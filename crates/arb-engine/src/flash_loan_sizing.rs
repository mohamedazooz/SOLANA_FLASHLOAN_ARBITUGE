//! Adaptive Flash Loan Sizing — calculates optimal borrow amount based on
//! pool depth, liquidity, and risk parameters. No fixed amounts.

use crate::meteora_quote_engine::MeteoraDlmmCrossPoolQuoteEngine;
use arb_math::dlmm::{DlmmBin, dlmm_active_bin_max_input, dlmm_multi_bin_max_input};
use arb_types::{MintId, PoolState};

/// Kamino uses U68F60 (60 fractional bits) for flash loan fees, NOT Q64.64 (64 bits).
/// This matches klend's `Fraction = fixed::types::U68F60`.
pub const KAMINO_FRACTION_FRAC_BITS: u32 = 60;
/// 2^60 - the denominator for Kamino's U68F60 fraction type.
pub const KAMINO_FRACTION_ONE_BITS: u128 = 1u128 << KAMINO_FRACTION_FRAC_BITS;

/// Configuration for adaptive flash loan sizing
#[derive(Debug, Clone)]
pub struct FlashLoanSizingConfig {
    /// Safety factor applied to max capacity, in basis points (0 - 10_000).
    /// 8500 = use 85% of max capacity to avoid crossing bins. Integer-only:
    /// token-amount math never touches floating point.
    pub safety_factor_bps: u64,
    /// Minimum flash loan amount (lamports)
    pub min_flash_amount: u64,
    /// Maximum flash loan amount (lamports) - hard cap
    pub max_flash_amount: u64,
    /// Maximum slippage per leg (basis points)
    pub max_slippage_bps: u64,
    /// Target profit margin after all costs (basis points)
    pub target_profit_bps: u64,
    /// Whether to optimize for max profit or max volume
    pub optimize_for_profit: bool,
    /// Max number of bins to cross per leg
    pub max_bins_crossed: u32,
    /// Reserve fraction to keep in pool (don't drain completely), in basis
    /// points. 1000 = keep 10% of pool liquidity untouched.
    pub reserve_fraction_bps: u64,
    /// Sizing methodology to use
    pub sizing_method: SizingMethod,
    /// Kelly Criterion parameters (used when sizing_method == Kelly)
    pub kelly_params: KellyParams,
}

/// Sizing methodology for flash loan amount optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizingMethod {
    /// Safety-factor based sizing (current default): capacity * safety_factor_bps / 10000
    SafetyFactor,
    /// Kelly Criterion optimal bet sizing: f* = p - (1-p)/R
    /// where p = win probability, R = reward/risk ratio
    /// Uses half-Kelly (fractional_kelly = 0.5) by default for volatility reduction
    Kelly,
}

/// Kelly Criterion parameters for position sizing
#[derive(Debug, Clone)]
pub struct KellyParams {
    /// Estimated win probability in basis points (0 - 10_000). Derived from historical landing rate.
    pub win_probability_bps: u64,
    /// Reward-to-risk ratio scaled by 1000 (e.g. 2000 = 2.0). Derived from historical PnL.
    pub reward_risk_ratio_scaled: u64,
    /// Fractional Kelly multiplier in basis points (0 - 10_000). 5000 = half-Kelly.
    /// Reduces volatility and drawdowns while preserving ~75% of peak compounding speed.
    pub fractional_kelly_bps: u64,
    /// Minimum position size floor (lamports) - Kelly can suggest very small sizes
    pub min_position_lamports: u64,
    /// Maximum position size cap (lamports) - hard limit regardless of Kelly
    pub max_position_lamports: u64,
}

impl Default for KellyParams {
    fn default() -> Self {
        Self {
            win_probability_bps: 6500,      // Conservative estimate from landing rate
            reward_risk_ratio_scaled: 2000, // Conservative R:R
            fractional_kelly_bps: 5000,     // Half-Kelly (recommended by practitioners)
            min_position_lamports: 100_000, // 0.0001 WSOL
            max_position_lamports: 5_000_000_000, // 5 WSOL
        }
    }
}

impl Default for FlashLoanSizingConfig {
    fn default() -> Self {
        Self {
            safety_factor_bps: 8_500,
            min_flash_amount: 100_000,       // 0.0001 WSOL
            max_flash_amount: 5_000_000_000, // 5 WSOL
            max_slippage_bps: 50,            // 0.5%
            target_profit_bps: 100,          // 1%
            optimize_for_profit: true,
            max_bins_crossed: 3,
            reserve_fraction_bps: 1_000, // Keep 10% in pool
            sizing_method: SizingMethod::SafetyFactor,
            kelly_params: KellyParams::default(),
        }
    }
}

/// Result of flash loan sizing calculation
#[derive(Debug, Clone)]
pub struct FlashLoanSizingResult {
    /// Recommended borrow amount in lamports
    pub borrow_amount: u64,
    /// Expected profit in lamports (after all costs)
    pub expected_profit: i64,
    /// Expected profit in basis points
    pub profit_bps: i64,
    /// Amount that will be input to each leg
    pub leg_amounts: [u64; 3],
    /// Conservative min output for each leg
    pub leg_min_outputs: [u64; 3],
    /// Bins crossed per leg
    pub bins_crossed: [u32; 3],
    /// Total flash loan fee
    pub flash_fee: u64,
    /// Total DEX fees
    pub dex_fees: u64,
    /// Total network costs
    pub network_costs: u64,
    /// Expected final WSOL output of the chain (before the slippage haircut)
    pub expected_final_output: u64,
    /// Whether sizing was constrained by max amount
    pub capped_at_max: bool,
    /// Whether sizing was constrained by pool capacity
    pub capped_by_capacity: bool,
    /// Sizing confidence score in basis points (0 - 10_000). Integer-only;
    /// 10_000 = borrow is negligible relative to pool capacity, lower values
    /// mean the borrow approaches the capacity bottleneck.
    pub confidence_bps: u64,
}

/// Calculate Kelly Criterion optimal position size
///
/// Kelly formula: f* = p - (1-p)/R
/// where p = win probability, R = reward/risk ratio
///
/// Fractional Kelly: f = fractional_kelly * f*
///
/// Returns position size in lamports, clamped to [min_position, max_position]
fn calculate_kelly_optimal_size(
    capacity_based_size: u64,
    kelly: &KellyParams,
    min_position: u64,
    max_position: u64,
) -> u64 {
    // Kelly fraction: f* = p - (1-p)/R, computed in fixed-point (x10_000).
    let p_bps = kelly.win_probability_bps.min(10_000) as u128;
    let r_scaled = kelly.reward_risk_ratio_scaled.max(1) as u128;
    let one_minus_p_over_r = (10_000u128 - p_bps) * 1000 / r_scaled;
    let kelly_fraction_scaled = p_bps.saturating_sub(one_minus_p_over_r);

    // Apply fractional Kelly (basis points)
    let fractional_bps = kelly.fractional_kelly_bps.min(10_000) as u128;
    let adjusted_scaled = kelly_fraction_scaled * fractional_bps / 10_000;

    // If Kelly suggests negative or zero, use minimum
    if adjusted_scaled == 0 {
        return min_position;
    }

    // Position size = fraction * capacity (or max_position, whichever is smaller)
    let capacity = capacity_based_size.min(max_position);
    let kelly_size = (capacity as u128 * adjusted_scaled / 10_000) as u64;

    // Clamp to [min_position, max_position]
    kelly_size.clamp(min_position, max_position)
}

/// Calculate optimal flash loan size for a 3-pool arbitrage
pub fn calculate_optimal_flash_loan(
    pool1: &PoolState, // WSOL -> A
    pool2: &PoolState, // A -> B
    pool3: &PoolState, // B -> WSOL
    kamino_flash_fee_sf: u64,
    config: &FlashLoanSizingConfig,
    network_cost_lamports: u64,
    priority_fee_lamports: u64,
    jito_tip_lamports: u64,
) -> FlashLoanSizingResult {
    // 1. Get max capacity for each leg
    let cap1 = get_leg_capacity(pool1, true); // WSOL -> A
    let cap2 = get_leg_capacity(pool2, true); // A -> B (depends on pool1 output)
    let cap3 = get_leg_capacity(pool3, false); // B -> WSOL

    // 2. Apply safety factors. Integer-only: floor division keeps the cap at
    // or below the exact factor, i.e. rounding errs toward smaller borrows.
    let safe_cap1 = cap1.saturating_mul(config.safety_factor_bps) / 10_000;
    let safe_cap2 = cap2.saturating_mul(config.safety_factor_bps) / 10_000;
    let safe_cap3 = cap3.saturating_mul(config.safety_factor_bps) / 10_000;

    // 3. The bottleneck is the minimum capacity across all legs
    let bottleneck = safe_cap1.min(safe_cap2).min(safe_cap3);

    // 4. Apply reserve fraction (don't drain pools completely). Floor again:
    // the kept reserve can only be larger than the exact fraction.
    let reserve_adjusted = bottleneck
        .saturating_mul(10_000u64.saturating_sub(config.reserve_fraction_bps.min(10_000)))
        / 10_000;

    // 5. Determine borrow amount based on sizing method
    let borrow_amount = match config.sizing_method {
        SizingMethod::SafetyFactor => {
            // Original safety-factor based sizing
            reserve_adjusted
                .max(config.min_flash_amount)
                .min(config.max_flash_amount)
        }
        SizingMethod::Kelly => {
            // Kelly Criterion optimal sizing
            calculate_kelly_optimal_size(
                reserve_adjusted,
                &config.kelly_params,
                config.min_flash_amount,
                config.max_flash_amount,
            )
        }
    };

    let capped_at_max = borrow_amount == config.max_flash_amount;
    let capped_by_capacity =
        borrow_amount == reserve_adjusted && reserve_adjusted < config.max_flash_amount;

    // 6. Simulate the 3-leg chain at this size
    let (leg_amounts, leg_min_outputs, bins_crossed, dex_fees, final_output) =
        simulate_three_leg_chain(pool1, pool2, pool3, borrow_amount, config.max_slippage_bps);

    // 7/8. Costs: the flash fee is charged inside `net_profit_lamports` and
    // DLMM quote outputs are already NET of protocol/LP fees (the fee is taken
    // from the swap input before bin traversal), so the quoted `dex_fees` are
    // informational and MUST NOT be subtracted again. DLMM quote outputs are already NET of protocol/LP fees
    // (the fee is taken from the swap input before bin traversal), so the
    // quoted `dex_fees` are informational and MUST NOT be subtracted again.
    let network_costs = network_cost_lamports + priority_fee_lamports + jito_tip_lamports;

    // 9. Final output after 3 legs (WSOL, flash-loan denomination). Net profit
    // is computed on the CONSERVATIVE slippage-haircut floor of the final leg,
    // never on the optimistic expected output.
    let net_profit = net_profit_lamports(
        leg_min_outputs[2],
        borrow_amount,
        kamino_flash_fee_sf,
        network_costs,
    );
    let profit_bps = if borrow_amount > 0 {
        (net_profit * 10_000) / borrow_amount as i64
    } else {
        0
    };

    // 10. If profit is below target, try to optimize
    let outcome = if net_profit < config.target_profit_bps as i64 * borrow_amount as i64 / 10_000 {
        optimize_size(
            pool1,
            pool2,
            pool3,
            borrow_amount,
            config,
            kamino_flash_fee_sf,
            network_costs,
        )
    } else {
        SizeSearchOutcome {
            borrow: borrow_amount,
            profit: net_profit,
            bps: profit_bps,
            amounts: leg_amounts,
            min_outputs: leg_min_outputs,
            bins: bins_crossed,
            dex_fees,
            expected_output: final_output,
        }
    };
    let SizeSearchOutcome {
        borrow: final_borrow,
        profit: final_profit,
        bps: final_bps,
        amounts: final_amounts,
        min_outputs: final_outputs,
        bins: final_bins,
        dex_fees: final_dex_fees,
        expected_output: final_expected_output,
    } = outcome;

    // Confidence based on how close to capacity we are, in integer bps with
    // a 3000 bps floor (a saturated borrow still reports 30% confidence).
    let capacity_ratio_bps = (final_borrow
        .saturating_mul(10_000)
        .saturating_div(bottleneck.max(1)))
    .min(10_000);
    let confidence_bps = (10_000 - capacity_ratio_bps).max(3_000);

    FlashLoanSizingResult {
        borrow_amount: final_borrow,
        expected_profit: final_profit,
        profit_bps: final_bps,
        leg_amounts: final_amounts,
        leg_min_outputs: final_outputs,
        bins_crossed: final_bins,
        flash_fee: calculate_kamino_flash_fee(final_borrow, kamino_flash_fee_sf),
        dex_fees: final_dex_fees,
        network_costs,
        expected_final_output: final_expected_output,
        capped_at_max,
        capped_by_capacity,
        confidence_bps,
    }
}

/// Get maximum input capacity for a single leg across up to `max_bins` (default 3)
fn get_leg_capacity_with_limit(pool: &PoolState, swap_for_y: bool, max_bins: u32) -> u64 {
    if let Some(dlmm) = &pool.dlmm {
        if dlmm.bins.is_empty() {
            return 0;
        }

        // Limit the bin traversal window to max_bins along the direction of traversal
        // swap_for_y: active_bin_id downwards (decreasing ID)
        // !swap_for_y: active_bin_id upwards (increasing ID)
        let active_id = dlmm.active_bin_id;
        let limit = (max_bins.max(1) - 1) as i32;

        let filtered_bins: Vec<DlmmBin> = dlmm
            .bins
            .iter()
            .filter(|b| {
                if swap_for_y {
                    b.bin_id <= active_id && b.bin_id >= active_id.saturating_sub(limit)
                } else {
                    b.bin_id >= active_id && b.bin_id <= active_id.saturating_add(limit)
                }
            })
            .map(|b| DlmmBin {
                id: b.bin_id,
                amount_x: u64::try_from(b.amount_x).unwrap_or(0),
                amount_y: u64::try_from(b.amount_y).unwrap_or(0),
                price_q64: 0,
                open_order_amount: b.open_order_amount,
                processed_order_remaining_amount: b.processed_order_remaining_amount,
                limit_order_ask_side: b.limit_order_ask_side,
            })
            .collect();

        if let Some(multi_cap) =
            dlmm_multi_bin_max_input(active_id, dlmm.bin_step_bps, &filtered_bins, swap_for_y)
        {
            if multi_cap > 0 {
                return multi_cap;
            }
        }

        // Fallback to single active bin if multi-bin evaluation yielded nothing
        if let Some(active_bin) = dlmm.bins.iter().find(|b| b.bin_id == dlmm.active_bin_id) {
            return dlmm_active_bin_max_input(
                dlmm.active_bin_id,
                dlmm.bin_step_bps,
                u64::try_from(active_bin.amount_x).unwrap_or(0),
                u64::try_from(active_bin.amount_y).unwrap_or(0),
                swap_for_y,
            )
            .unwrap_or(0);
        }
    }
    0
}

/// Get maximum input capacity for a single leg (default up to 3 bins)
fn get_leg_capacity(pool: &PoolState, swap_for_y: bool) -> u64 {
    get_leg_capacity_with_limit(pool, swap_for_y, 3)
}

/// Simulate 3-leg chain at given borrow amount
fn simulate_three_leg_chain(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    borrow_amount: u64,
    max_slippage_bps: u64,
) -> ([u64; 3], [u64; 3], [u32; 3], u64, u64) {
    // Derive the actual route mints from the pool states, anchored on WSOL.
    // Leg 1 must trade WSOL <-> A, leg 3 must trade B <-> WSOL, and leg 2 must
    // bridge A <-> B. Any other shape quotes nothing and yields zero output.
    let wsol = wsol_mint_id();
    let empty = ([0u64; 3], [0u64; 3], [0u32; 3], 0u64, 0u64);
    if pool1.base_mint == wsol && pool1.quote_mint != wsol {
        // A is pool1's quote mint
    } else if pool1.quote_mint == wsol && pool1.base_mint != wsol {
        // A is pool1's base mint
    } else {
        return empty;
    }
    let mint_a = if pool1.base_mint == wsol {
        pool1.quote_mint
    } else {
        pool1.base_mint
    };
    let mint_b = if pool3.base_mint == wsol {
        pool3.quote_mint
    } else if pool3.quote_mint == wsol {
        pool3.base_mint
    } else {
        return empty;
    };
    if mint_b == wsol || mint_b == mint_a {
        return empty;
    }
    let p2_mints = [pool2.base_mint, pool2.quote_mint];
    if !p2_mints.contains(&mint_a) || !p2_mints.contains(&mint_b) {
        return empty;
    }

    let mut amount = borrow_amount;
    let mut amounts = [0u64; 3];
    let mut min_outputs = [0u64; 3];
    let mut bins = [0u32; 3];
    let mut total_fees = 0u64;
    let mut final_output = 0u64;

    // Leg 1: WSOL -> A
    if let Some((out1, quote1, _)) =
        MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(pool1, amount, wsol, mint_a)
    {
        amounts[0] = amount;
        min_outputs[0] = apply_slippage(out1, max_slippage_bps);
        bins[0] = quote1.bins_crossed;
        total_fees += quote1.fee_amount;
        amount = min_outputs[0];
    }

    // Leg 2: A -> B
    if let Some((out2, quote2, _)) =
        MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(pool2, amount, mint_a, mint_b)
    {
        amounts[1] = amount;
        min_outputs[1] = apply_slippage(out2, max_slippage_bps);
        bins[1] = quote2.bins_crossed;
        total_fees += quote2.fee_amount;
        amount = min_outputs[1];
    }

    // Leg 3: B -> WSOL
    if let Some((out3, quote3, _)) =
        MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(pool3, amount, mint_b, wsol)
    {
        amounts[2] = amount;
        min_outputs[2] = apply_slippage(out3, max_slippage_bps);
        bins[2] = quote3.bins_crossed;
        total_fees += quote3.fee_amount;
        final_output = out3;
    }

    (amounts, min_outputs, bins, total_fees, final_output)
}

fn wsol_mint_id() -> MintId {
    use std::str::FromStr;
    MintId(solana_pubkey::Pubkey::from_str(arb_constants::WSOL_MINT).expect("valid wsol mint"))
}

/// Canonical NET profit accounting for a WSOL-denominated flash-loan route:
/// gross output minus the borrowed principal, minus the Kamino flash fee at
/// the on-chain U68F60 rate, minus the flat network cost. Single source of
/// truth shared by the size optimizer and the 3-leg pipeline. All integer,
/// all checked; a disabled reserve (`u64::MAX` fee sentinel) fails closed.
pub fn net_profit_lamports(
    final_output: u64,
    borrow: u64,
    flash_fee_sf: u64,
    network_cost_lamports: u64,
) -> i64 {
    let flash_fee = calculate_kamino_flash_fee(borrow, flash_fee_sf);
    let net_i128 = i128::from(final_output)
        - i128::from(borrow)
        - i128::from(flash_fee)
        - i128::from(network_cost_lamports);
    if net_i128 > i128::from(i64::MAX) {
        i64::MAX
    } else {
        i64::try_from(net_i128).unwrap_or(i64::MIN)
    }
}

/// Apply slippage to get conservative minimum output
fn apply_slippage(amount: u64, slippage_bps: u64) -> u64 {
    amount.saturating_mul(10_000 - slippage_bps.min(10_000)) / 10_000
}

/// Calculate Kamino flash loan fee using U68F60 (60 fractional bits).
///
/// Kamino's on-chain fee is stored as a U68F60 value (60 fractional bits).
/// The fee in atomic units is: round(borrow * sf / 2^60) with a 1-unit floor.
///
/// This matches klend's `ReserveFees::calculate_fees` implementation exactly.
///
/// # Examples
///
/// For the main-market WSOL reserve with `sf = 11_529_215_046_068` (1e-5):
/// - Borrowing 1 SOL (1e9 lamports) yields a fee of 10,000 lamports
/// - Borrowing 0.5 SOL yields a fee of 5,000 lamports
/// - Borrowing 1 lamport yields a fee of 1 lamport (minimum floor)
///
/// # Arguments
///
/// * `borrow` - The amount to borrow in atomic token units (lamports for WSOL)
/// * `fee_sf` - The flash loan fee as a U68F60 value read from the reserve account
///
/// # Returns
///
/// The flash loan fee in atomic token units. Returns `u64::MAX` if flash loans
/// are disabled (`fee_sf == u64::MAX`) or if the calculation overflows.
pub fn calculate_kamino_flash_fee(borrow: u64, fee_sf: u64) -> u64 {
    // Single authoritative implementation (klend `FeeCalculation::Exclusive`,
    // round-half-away-from-zero). The previous local copy truncated, which
    // understated the fee; see `arb_tx::kamino::calculate_kamino_flash_fee`.
    arb_tx::kamino::calculate_kamino_flash_fee(borrow, fee_sf)
}

/// Optimize borrow size for maximum profit
/// Winning candidate of the bounded size search.
struct SizeSearchOutcome {
    borrow: u64,
    profit: i64,
    bps: i64,
    amounts: [u64; 3],
    min_outputs: [u64; 3],
    bins: [u32; 3],
    dex_fees: u64,
    expected_output: u64,
}

fn optimize_size(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    initial_borrow: u64,
    config: &FlashLoanSizingConfig,
    kamino_flash_fee_sf: u64,
    network_costs: u64,
) -> SizeSearchOutcome {
    let mut best_borrow = initial_borrow;
    let mut best_profit = i64::MIN;
    let mut best_bps = 0i64;
    let mut best_amounts = [0u64; 3];
    let mut best_outputs = [0u64; 3];
    let mut best_bins = [0u32; 3];
    let mut best_fees = 0u64;
    let mut best_expected_output = 0u64;

    // Coarse search: powers of 1.5 from min to max
    let mut test_amount = config.min_flash_amount;
    while test_amount <= config.max_flash_amount {
        let (amounts, outputs, bins, fees, expected_out) =
            simulate_three_leg_chain(pool1, pool2, pool3, test_amount, config.max_slippage_bps);

        let final_out = outputs[2];
        let net = net_profit_lamports(final_out, test_amount, kamino_flash_fee_sf, network_costs);

        if net > best_profit {
            best_profit = net;
            best_borrow = test_amount;
            best_bps = if test_amount > 0 {
                let bps = i128::from(net) * 10_000 / i128::from(test_amount);
                if bps > i128::from(i64::MAX) {
                    i64::MAX
                } else {
                    i64::try_from(bps).unwrap_or(i64::MIN)
                }
            } else {
                0
            };
            best_amounts = amounts;
            best_outputs = outputs;
            best_bins = bins;
            best_fees = fees;
            best_expected_output = expected_out;
        }

        // Integer 1.5x growth (floor). The +1 guard guarantees termination
        // when floor division would stall (e.g. test_amount == 1).
        let next = test_amount.saturating_mul(3) / 2;
        test_amount = if next <= test_amount {
            test_amount.saturating_add(1)
        } else {
            next
        };
    }

    // Fine search around best
    let fine_start = best_borrow
        .saturating_sub(initial_borrow / 10)
        .max(config.min_flash_amount);
    let fine_end = (best_borrow + initial_borrow / 10).min(config.max_flash_amount);
    let step = ((fine_end - fine_start) / 20).max(1);

    for test_amount in (fine_start..=fine_end).step_by(step as usize) {
        let (amounts, outputs, bins, fees, expected_out) =
            simulate_three_leg_chain(pool1, pool2, pool3, test_amount, config.max_slippage_bps);

        let final_out = outputs[2];
        let net = net_profit_lamports(final_out, test_amount, kamino_flash_fee_sf, network_costs);

        if net > best_profit {
            best_profit = net;
            best_borrow = test_amount;
            best_bps = if test_amount > 0 {
                let bps = i128::from(net) * 10_000 / i128::from(test_amount);
                if bps > i128::from(i64::MAX) {
                    i64::MAX
                } else {
                    i64::try_from(bps).unwrap_or(i64::MIN)
                }
            } else {
                0
            };
            best_amounts = amounts;
            best_outputs = outputs;
            best_bins = bins;
            best_fees = fees;
            best_expected_output = expected_out;
        }
    }

    SizeSearchOutcome {
        borrow: best_borrow,
        profit: best_profit,
        bps: best_bps,
        amounts: best_amounts,
        min_outputs: best_outputs,
        bins: best_bins,
        dex_fees: best_fees,
        expected_output: best_expected_output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slippage_application() {
        assert_eq!(apply_slippage(1_000_000, 100), 990_000); // 1% slippage
        assert_eq!(apply_slippage(1_000_000, 50), 995_000); // 0.5%
        assert_eq!(apply_slippage(1_000_000, 10_000), 0); // 100%
    }

    #[test]
    fn net_profit_clamp_never_flips_loss_into_profit() {
        let fee_sf = 11_529_215_046_068u64;
        // Ordinary profitable case stays exact (30_000 gross − 10_000 fee − 10_000 network).
        assert_eq!(
            net_profit_lamports(1_000_030_000, 1_000_000_000, fee_sf, 10_000),
            10_000
        );
        // Ordinary loss stays exact and negative (flash fee 10_000 + network 10_000).
        assert_eq!(
            net_profit_lamports(1_000_000_000, 1_000_000_000, fee_sf, 10_000),
            -20_000
        );
        // Extreme output saturates at i64::MAX, never wraps negative.
        assert_eq!(net_profit_lamports(u64::MAX, 0, 0, 0), i64::MAX);
        // Extreme loss saturates at i64::MIN, never wraps positive. A disabled
        // reserve (fee_sf == u64::MAX) yields the u64::MAX fee sentinel.
        let net = net_profit_lamports(0, u64::MAX, u64::MAX, u64::MAX);
        assert_eq!(net, i64::MIN);
        // Zero-liquidity output with a real borrow is a strict loss.
        assert!(net_profit_lamports(0, 1_000_000_000, fee_sf, 9_000) < 0);
    }

    #[test]
    fn test_flash_fee_calculation() {
        // Kamino main-market WSOL reserve: flash_loan_fee_sf = 11_529_215_046_068 = 1e-5 in U68F60
        // Fee = round(1_000_000_000 * 11_529_215_046_068 / 2^60) = 10_000 lamports
        let fee_sf = 11_529_215_046_068; // 1e-5 in U68F60 (60 fractional bits)
        assert_eq!(calculate_kamino_flash_fee(1_000_000_000, fee_sf), 10_000);
        // 0.5 SOL borrows half the fee
        assert_eq!(calculate_kamino_flash_fee(500_000_000, fee_sf), 5_000);
        // 1 lamport borrow with 1e-5 fee: floor of 1 unit
        assert_eq!(calculate_kamino_flash_fee(1, fee_sf), 1);
        // Disabled reserve (fee_sf == u64::MAX)
        assert_eq!(
            calculate_kamino_flash_fee(1_000_000_000, u64::MAX),
            u64::MAX
        );
        // Zero fee or zero borrow
        assert_eq!(calculate_kamino_flash_fee(1_000_000_000, 0), 0);
        assert_eq!(calculate_kamino_flash_fee(0, fee_sf), 0);
    }

    #[test]
    fn test_config_defaults() {
        let config = FlashLoanSizingConfig::default();
        assert_eq!(config.safety_factor_bps, 8_500);
        assert_eq!(config.min_flash_amount, 100_000);
        assert_eq!(config.max_flash_amount, 5_000_000_000);
    }

    #[test]
    fn test_get_leg_capacity_multi_bin_support() {
        use arb_types::{DexProtocol, DlmmBin as TypeDlmmBin, DlmmState, PoolId, PoolKind, Slot};

        // Create a pool where active bin 0 has 0 output liquidity, but bin -1 has 10_000_000
        let dlmm_state = DlmmState {
            active_bin_id: 0,
            bin_step_bps: 25,
            status: 0,
            bins: vec![
                TypeDlmmBin {
                    bin_id: 0,
                    amount_x: 5_000_000,
                    amount_y: 0, // 0 Y in active bin
                    ..Default::default()
                },
                TypeDlmmBin {
                    bin_id: -1,
                    amount_x: 0,
                    amount_y: 10_000_000, // neighbouring bin has Y
                    ..Default::default()
                },
            ],
            params: None,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 0,
            ..Default::default()
        };

        let pool = PoolState {
            pool_id: PoolId::new([1u8; 32]),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId::default(),
            quote_mint: MintId::default(),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 0,
            reserve_out: 0,
            fee_numerator: 25,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(dlmm_state),
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        };

        // swap_for_y = true: active bin has 0 Y, but multi-bin reaches bin -1
        let cap = get_leg_capacity(&pool, true);
        assert!(
            cap > 0,
            "Capacity should not be 0 when adjacent bin has liquidity"
        );
    }
}
