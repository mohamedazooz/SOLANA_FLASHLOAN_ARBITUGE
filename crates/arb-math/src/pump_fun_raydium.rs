//! Cross-venue Pump.fun <-> Raydium price discovery and optimal trade sizing.
//!
//! Uses exact integer wide arithmetic (`U384` / `isqrt_u384`) for closed-form
//! sizing: buying `delta_t` tokens from a constant-product bonding curve costs
//! `R * delta_t / (T - delta_t)` (rounded up for safety) and selling `delta_t`
//! on Raydium yields `gamma * y * delta_t / (D * (x + delta_t))`.

use crate::cpmm::{U384, isqrt_u384};
use ruint::aliases::U256;
use std::cmp;

pub const MIN_NET_PROFIT_LAMPORTS: u64 = arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpFunParams {
    pub reserve: u128,
    pub supply: u128,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaydiumParams {
    pub token_reserve: u128,
    pub sol_reserve: u128,
    pub fee_bps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptimalTrade {
    pub delta_t: u128,
    pub buy_cost: u128,
    pub sell_output: u128,
    pub net_profit: i128,
    /// Generic percentage-based financing fee supplied by the caller (BPS); not a Kamino U68F60 fee.
    pub flash_loan_fee_bps: u64,
}

impl OptimalTrade {
    pub fn is_profitable(&self) -> bool {
        self.net_profit > 0
    }
}

pub fn pump_fun_marginal_price(reserve: u128, supply: u128) -> Option<u128> {
    if reserve == 0 || supply == 0 {
        return None;
    }
    Some(reserve / supply)
}

/// Exact SOL cost to buy `delta_t` tokens out of `(reserve, supply)`:
/// `ceil(reserve * delta_t / (supply - delta_t))` when `delta_t < supply`.
/// Returns `None` if `delta_t >= supply` (asymptotically impossible to drain 100%).
pub fn pump_fun_buy_cost(reserve: u128, supply: u128, delta_t: u128) -> Option<u128> {
    if delta_t == 0 {
        return Some(0);
    }
    if reserve == 0 || supply == 0 || delta_t >= supply {
        return None;
    }
    let denom = supply.checked_sub(delta_t)?;
    let num = U256::from(reserve).checked_mul(U256::from(delta_t))?;
    let denom_u = U256::from(denom);
    let cost = (num + denom_u - U256::from(1u64)) / denom_u;
    u128::try_from(cost).ok()
}

pub fn raydium_sell_output(
    sol_reserve: u128,
    token_reserve: u128,
    delta_t: u128,
    fee_bps: u64,
) -> Option<u128> {
    if delta_t == 0 || sol_reserve == 0 || token_reserve == 0 {
        return Some(0);
    }
    let fee_factor = 10_000_u128.checked_sub(u128::from(fee_bps.min(10_000)))?;
    let num = U256::from(sol_reserve)
        .checked_mul(U256::from(delta_t))?
        .checked_mul(U256::from(fee_factor))?;
    let denom = U256::from(token_reserve)
        .checked_add(U256::from(delta_t))?
        .checked_mul(U256::from(10_000u64))?;
    if denom.is_zero() {
        return None;
    }
    u128::try_from(num / denom).ok()
}

pub fn verify_marginal_decreasing(
    pump: PumpFunParams,
    raydium: RaydiumParams,
    flash_fee_bps: u64,
    delta_t: u128,
) -> bool {
    let delta_t = cmp::max(delta_t, 1);
    if delta_t >= pump.supply {
        return true;
    }
    let ray_fee = 10_000_u128.saturating_sub(u128::from(raydium.fee_bps.min(10_000)));
    let loan_factor = 10_000_u128.saturating_add(u128::from(flash_fee_bps.min(10_000)));

    let lhs =
        U384::from(ray_fee) * U384::from(raydium.sol_reserve) * U384::from(raydium.token_reserve);
    let rhs = U384::from(loan_factor) * U384::from(pump.reserve) * U384::from(pump.supply);

    let x_plus = U384::from(raydium.token_reserve.saturating_add(delta_t));
    let t_minus = U384::from(pump.supply.saturating_sub(delta_t));
    let denom_left = x_plus * x_plus;
    let denom_right = t_minus * t_minus;
    if denom_left.is_zero() || denom_right.is_zero() {
        return true;
    }
    (lhs / denom_left) <= (rhs / denom_right)
}

/// Pure wide-integer sizing for a generic BPS-financed Pump.fun/Raydium route.
/// The caller must supply current protocol-specific fee inputs; this helper does
/// not decode Kamino U68F60 fees or Raydium CPMM/CLMM AmmConfig fees.
pub fn calculate_optimal_trade_integer(
    pump: PumpFunParams,
    raydium: RaydiumParams,
    flash_fee_bps: u64,
    max_delta_t: u128,
) -> Option<OptimalTrade> {
    calculate_optimal_trade(pump, raydium, flash_fee_bps, max_delta_t)
        .ok()
        .filter(|trade| trade.is_profitable())
}

/// Closed-form integer optimal trade sizing with zero float casts:
/// `delta_t* = (T * A - x_r * B) / (A + B)` where
/// `A = isqrt(ray_fee * y_r * x_r)` and `B = isqrt(loan_factor * R * T)`.
pub fn calculate_optimal_trade(
    pump: PumpFunParams,
    raydium: RaydiumParams,
    flash_fee_bps: u64,
    max_delta_t: u128,
) -> Result<OptimalTrade, String> {
    if pump.reserve == 0
        || pump.supply <= 1
        || raydium.token_reserve == 0
        || raydium.sol_reserve == 0
    {
        return Err("pump/raydium reserves cannot be zero".to_string());
    }

    let safe_max = max_delta_t
        .min(pump.supply.saturating_sub(1))
        .min(raydium.token_reserve);
    if safe_max == 0 {
        return Err("max_delta_t is zero".to_string());
    }

    let flash_fee_bps = flash_fee_bps.min(10_000);
    let ray_fee_bps = raydium.fee_bps.min(10_000);

    let ray_fee = 10_000_u128.saturating_sub(u128::from(ray_fee_bps));
    let loan_factor = 10_000_u128.saturating_add(u128::from(flash_fee_bps));

    let rad_a =
        U384::from(ray_fee) * U384::from(raydium.sol_reserve) * U384::from(raydium.token_reserve);
    let rad_b = U384::from(loan_factor) * U384::from(pump.reserve) * U384::from(pump.supply);

    let a = isqrt_u384(rad_a);
    let b = isqrt_u384(rad_b);

    let left = U384::from(pump.supply) * a;
    let right = U384::from(raydium.token_reserve) * b;

    if left <= right {
        return Err("No positive optimum under the current curve geometry".to_string());
    }

    let numerator = left - right;
    let denominator = a + b;
    if denominator.is_zero() {
        return Err("Zero denominator".to_string());
    }

    let delta_t_wide = numerator / denominator;
    let delta_t = u128::try_from(delta_t_wide)
        .unwrap_or(safe_max)
        .min(safe_max);

    if delta_t == 0 {
        return Err("No profitable trade sizing exists".to_string());
    }

    let buy_cost = pump_fun_buy_cost(pump.reserve, pump.supply, delta_t)
        .ok_or_else(|| "pump buy cost overflow".to_string())?;
    let sell_output = raydium_sell_output(
        raydium.sol_reserve,
        raydium.token_reserve,
        delta_t,
        ray_fee_bps,
    )
    .ok_or_else(|| "raydium sell output overflow".to_string())?;

    let flash_cost = buy_cost
        .saturating_mul(u128::from(flash_fee_bps))
        .saturating_div(10_000);
    let total_cost = buy_cost
        .saturating_add(flash_cost)
        .saturating_add(u128::from(MIN_NET_PROFIT_LAMPORTS));
    let net_profit = i128::try_from(sell_output)
        .map_err(|_| "sell output overflow".to_string())?
        .saturating_sub(i128::try_from(total_cost).map_err(|_| "total cost overflow".to_string())?);

    Ok(OptimalTrade {
        delta_t,
        buy_cost,
        sell_output,
        net_profit,
        flash_loan_fee_bps: flash_fee_bps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic test fixtures only. Production fee inputs must come from the
    // relevant protocol state; these are not Kamino or universal Raydium rates.
    const TEST_FINANCING_FEE_BPS: u64 = 30;
    const TEST_RAYDIUM_FEE_BPS: u64 = 25;

    /// Profitable geometry: Pump.fun marginal price 1 lamport/token vs
    /// Raydium 2 lamports/token — a 100% gross spread far above both fees.
    fn profitable_pair() -> (PumpFunParams, RaydiumParams) {
        (
            PumpFunParams {
                reserve: 1_000_000_000,
                supply: 1_000_000_000,
            },
            RaydiumParams {
                token_reserve: 1_000_000_000,
                sol_reserve: 2_000_000_000,
                fee_bps: TEST_RAYDIUM_FEE_BPS,
            },
        )
    }

    #[test]
    fn min_net_profit_constant_matches_arb_constants() {
        assert_eq!(
            MIN_NET_PROFIT_LAMPORTS,
            arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS
        );
    }

    #[test]
    fn marginal_price_rejects_zero_reserves() {
        assert_eq!(pump_fun_marginal_price(0, 1_000), None);
        assert_eq!(pump_fun_marginal_price(1_000, 0), None);
        assert_eq!(pump_fun_marginal_price(1_000, 500), Some(2));
    }

    #[test]
    fn buy_cost_rounds_up_and_rejects_unfunded_sizes() {
        assert_eq!(pump_fun_buy_cost(0, 0, 0), Some(0));
        assert_eq!(pump_fun_buy_cost(1, 3, 1), Some(1)); // ceil(1/2) = 1
        assert_eq!(pump_fun_buy_cost(3, 10, 5), Some(3)); // ceil(15/5) = 3 exact
        assert_eq!(pump_fun_buy_cost(1_000, 1_000, 1_000), None); // delta >= supply
        assert_eq!(pump_fun_buy_cost(0, 1_000, 1), None);
        assert_eq!(pump_fun_buy_cost(1_000, 0, 1), None);
    }

    #[test]
    fn raydium_sell_output_applies_fee_and_floor_rounding() {
        assert_eq!(raydium_sell_output(1_000, 1_000, 0, 0), Some(0));
        // No fee: 1000 * 100 * 10000 / (1100 * 10000) = 90.909 -> 90 (floor).
        assert_eq!(raydium_sell_output(1_000, 1_000, 100, 0), Some(90));
        // 25 bps fee: 1000 * 100 * 9975 / (1100 * 10000) = 90.68 -> 90 (floor,
        // conservative). Larger reserves make the fee visible: 90_909 vs 90_681.
        assert_eq!(
            raydium_sell_output(1_000_000, 1_000_000, 100_000, 0),
            Some(90_909)
        );
        assert_eq!(
            raydium_sell_output(1_000_000, 1_000_000, 100_000, 25),
            Some(90_681)
        );
        // Fee >= 100% clamps to zero output factor, never panics.
        assert_eq!(raydium_sell_output(1_000, 1_000, 100, 10_000), Some(0));
    }

    #[test]
    fn optimal_trade_rejects_degenerate_reserves() {
        let (pump, ray) = profitable_pair();
        assert!(calculate_optimal_trade(PumpFunParams::default(), ray, 30, 1_000).is_err());
        assert!(
            calculate_optimal_trade(
                pump,
                RaydiumParams {
                    sol_reserve: 0,
                    ..ray
                },
                30,
                1_000
            )
            .is_err()
        );
        assert!(calculate_optimal_trade(pump, ray, 30, 0).is_err());
    }

    #[test]
    fn optimal_trade_finds_no_optimum_when_prices_are_aligned() {
        // Identical marginal prices: 25 bps Raydium fee + 30 bps flash fee
        // make every size unprofitable -> closed form must error, not fake.
        let pump = PumpFunParams {
            reserve: 1_000_000_000,
            supply: 1_000_000_000,
        };
        let ray = RaydiumParams {
            token_reserve: 1_000_000_000,
            sol_reserve: 1_000_000_000,
            fee_bps: TEST_RAYDIUM_FEE_BPS,
        };
        let err = calculate_optimal_trade(pump, ray, TEST_FINANCING_FEE_BPS, 1_000_000)
            .expect_err("aligned prices must not produce a trade");
        assert_eq!(err, "No positive optimum under the current curve geometry");
        assert!(
            calculate_optimal_trade_integer(pump, ray, TEST_FINANCING_FEE_BPS, 1_000_000).is_none()
        );
    }

    #[test]
    fn optimal_trade_is_profitable_and_internally_consistent() {
        let (pump, ray) = profitable_pair();
        let trade = calculate_optimal_trade_integer(pump, ray, TEST_FINANCING_FEE_BPS, u128::MAX)
            .expect("profitable geometry must yield a trade");
        assert!(trade.is_profitable());
        assert!(trade.delta_t > 0);
        // Recomputed leg economics must match the reported trade.
        let buy = pump_fun_buy_cost(pump.reserve, pump.supply, trade.delta_t).unwrap();
        let sell = raydium_sell_output(
            ray.sol_reserve,
            ray.token_reserve,
            trade.delta_t,
            ray.fee_bps,
        )
        .unwrap();
        assert_eq!(trade.buy_cost, buy);
        assert_eq!(trade.sell_output, sell);
        // Sell proceeds must cover the borrow + flash fee + min-profit floor.
        assert!(trade.sell_output > trade.buy_cost);
        // Optimal size stays inside protocol physics.
        assert!(trade.delta_t < pump.supply);
        assert!(trade.delta_t <= ray.token_reserve);
    }

    #[test]
    fn verify_marginal_decreasing_brackets_the_optimum() {
        // At the optimum, marginal revenue == marginal cost (condition holds);
        // well below it, marginal revenue still exceeds marginal cost (false).
        let (pump, ray) = profitable_pair();
        let trade = calculate_optimal_trade(pump, ray, TEST_FINANCING_FEE_BPS, u128::MAX)
            .expect("optimum exists");
        assert!(verify_marginal_decreasing(
            pump,
            ray,
            TEST_FINANCING_FEE_BPS,
            trade.delta_t
        ));
        assert!(!verify_marginal_decreasing(
            pump,
            ray,
            TEST_FINANCING_FEE_BPS,
            (trade.delta_t / 10).max(1)
        ));
        // Degenerate size beyond supply short-circuits to true (fail-safe).
        assert!(verify_marginal_decreasing(
            pump,
            ray,
            TEST_FINANCING_FEE_BPS,
            pump.supply + 1
        ));
    }
}
