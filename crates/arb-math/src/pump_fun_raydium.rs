//! Cross-venue Pump.fun <-> Raydium price discovery and optimal trade sizing.
//!
//! Uses exact integer wide arithmetic (`U384` / `isqrt_u384`) for closed-form
//! sizing: buying `delta_t` tokens from a constant-product bonding curve costs
//! `R * delta_t / (T - delta_t)` (rounded up for safety) and selling `delta_t`
//! on Raydium yields `gamma * y * delta_t / (D * (x + delta_t))`.

use crate::cpmm::{U384, isqrt_u384};
use ruint::aliases::U256;
use std::cmp;

pub const DEFAULT_FLASH_LOAN_FEE_BPS: u64 = 30;
pub const DEFAULT_RAYDIUM_FEE_BPS: u64 = 25;
pub const MIN_NET_PROFIT_LAMPORTS: u64 = 30_000;

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

/// Pure wide-integer optimal trade sizing for buying on Pump.fun and selling on Raydium.
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
