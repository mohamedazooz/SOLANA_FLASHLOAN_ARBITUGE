//! Adaptive Flash Loan Sizing — calculates optimal borrow amount based on
//! pool depth, liquidity, and risk parameters. No fixed amounts.

use crate::meteora_quote_engine::MeteoraDlmmCrossPoolQuoteEngine;
use arb_math::dlmm::dlmm_active_bin_max_input;
use arb_types::{MintId, PoolState};

/// Kamino uses U68F60 (60 fractional bits) for flash loan fees, NOT Q64.64 (64 bits).
/// This matches klend's `Fraction = fixed::types::U68F60`.
pub const KAMINO_FRACTION_FRAC_BITS: u32 = 60;
/// 2^60 - the denominator for Kamino's U68F60 fraction type.
pub const KAMINO_FRACTION_ONE_BITS: u128 = 1u128 << KAMINO_FRACTION_FRAC_BITS;

/// Configuration for adaptive flash loan sizing
#[derive(Debug, Clone)]
pub struct FlashLoanSizingConfig {
    /// Safety factor applied to max capacity (0.0 - 1.0)
    /// 0.9 = use 90% of max capacity to avoid crossing bins
    pub safety_factor: f64,
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
    /// Reserve fraction to keep in pool (don't drain completely)
    pub reserve_fraction: f64,
}

impl Default for FlashLoanSizingConfig {
    fn default() -> Self {
        Self {
            safety_factor: 0.85,
            min_flash_amount: 100_000,       // 0.0001 WSOL
            max_flash_amount: 5_000_000_000, // 5 WSOL
            max_slippage_bps: 50,            // 0.5%
            target_profit_bps: 100,          // 1%
            optimize_for_profit: true,
            max_bins_crossed: 3,
            reserve_fraction: 0.1, // Keep 10% in pool
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
    /// Whether sizing was constrained by max amount
    pub capped_at_max: bool,
    /// Whether sizing was constrained by pool capacity
    pub capped_by_capacity: bool,
    /// Sizing confidence score (0.0 - 1.0)
    pub confidence: f64,
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

    // 2. Apply safety factors
    let safe_cap1 = (cap1 as f64 * config.safety_factor) as u64;
    let safe_cap2 = (cap2 as f64 * config.safety_factor) as u64;
    let safe_cap3 = (cap3 as f64 * config.safety_factor) as u64;

    // 3. The bottleneck is the minimum capacity across all legs
    let bottleneck = safe_cap1.min(safe_cap2).min(safe_cap3);

    // 4. Apply reserve fraction (don't drain pools completely)
    let reserve_adjusted = (bottleneck as f64 * (1.0 - config.reserve_fraction)) as u64;

    // 5. Clamp to configured bounds
    let borrow_amount = reserve_adjusted
        .max(config.min_flash_amount)
        .min(config.max_flash_amount);

    let capped_at_max = borrow_amount == config.max_flash_amount;
    let capped_by_capacity =
        borrow_amount == reserve_adjusted && reserve_adjusted < config.max_flash_amount;

    // 6. Simulate the 3-leg chain at this size
    let (leg_amounts, leg_min_outputs, bins_crossed, dex_fees) =
        simulate_three_leg_chain(pool1, pool2, pool3, borrow_amount, config.max_slippage_bps);

    // 7. Calculate flash loan fee
    let flash_fee = calculate_kamino_flash_fee(borrow_amount, kamino_flash_fee_sf);

    // 8. Total costs
    let network_costs = network_cost_lamports + priority_fee_lamports + jito_tip_lamports;
    let total_costs = flash_fee + dex_fees + network_costs;

    // 9. Final output after 3 legs
    let final_output = leg_amounts[2];
    let gross_profit = final_output as i64 - borrow_amount as i64;
    let net_profit = gross_profit - total_costs as i64;
    let profit_bps = if borrow_amount > 0 {
        (net_profit * 10_000) / borrow_amount as i64
    } else {
        0
    };

    // 10. If profit is below target, try to optimize
    let (
        final_borrow,
        final_profit,
        final_bps,
        final_amounts,
        final_outputs,
        final_bins,
        final_dex_fees,
    ) = if net_profit < config.target_profit_bps as i64 * borrow_amount as i64 / 10_000 {
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
        (
            borrow_amount,
            net_profit,
            profit_bps,
            leg_amounts,
            leg_min_outputs,
            bins_crossed,
            dex_fees,
        )
    };

    // Confidence based on how close to capacity we are
    let capacity_ratio = final_borrow as f64 / bottleneck.max(1) as f64;
    let confidence = (1.0 - capacity_ratio).clamp(0.3, 1.0);

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
        capped_at_max,
        capped_by_capacity,
        confidence,
    }
}

/// Get maximum input capacity for a single leg
fn get_leg_capacity(pool: &PoolState, swap_for_y: bool) -> u64 {
    if let Some(dlmm) = &pool.dlmm {
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

/// Simulate 3-leg chain at given borrow amount
fn simulate_three_leg_chain(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    borrow_amount: u64,
    max_slippage_bps: u64,
) -> ([u64; 3], [u64; 3], [u32; 3], u64) {
    // Get mints from pool states
    let wsol = MintId::default(); // Would be passed in real implementation
    let mint_a = MintId::default();
    let mint_b = MintId::default();

    let mut amount = borrow_amount;
    let mut amounts = [0u64; 3];
    let mut min_outputs = [0u64; 3];
    let mut bins = [0u32; 3];
    let mut total_fees = 0u64;

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
    }

    (amounts, min_outputs, bins, total_fees)
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
fn optimize_size(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    initial_borrow: u64,
    config: &FlashLoanSizingConfig,
    kamino_flash_fee_sf: u64,
    network_costs: u64,
) -> (u64, i64, i64, [u64; 3], [u64; 3], [u32; 3], u64) {
    let mut best_borrow = initial_borrow;
    let mut best_profit = i64::MIN;
    let mut best_bps = 0i64;
    let mut best_amounts = [0u64; 3];
    let mut best_outputs = [0u64; 3];
    let mut best_bins = [0u32; 3];
    let mut best_fees = 0u64;

    // Coarse search: powers of 1.5 from min to max
    let mut test_amount = config.min_flash_amount;
    while test_amount <= config.max_flash_amount {
        let (amounts, outputs, bins, fees) =
            simulate_three_leg_chain(pool1, pool2, pool3, test_amount, config.max_slippage_bps);

        let flash_fee = calculate_kamino_flash_fee(test_amount, kamino_flash_fee_sf);
        let total_costs = flash_fee + fees + network_costs;
        let final_out = outputs[2];
        let net = final_out as i64 - test_amount as i64 - total_costs as i64;

        if net > best_profit {
            best_profit = net;
            best_borrow = test_amount;
            best_bps = if test_amount > 0 {
                (net * 10_000) / test_amount as i64
            } else {
                0
            };
            best_amounts = amounts;
            best_outputs = outputs;
            best_bins = bins;
            best_fees = fees;
        }

        test_amount = (test_amount as f64 * 1.5) as u64;
    }

    // Fine search around best
    let fine_start = best_borrow
        .saturating_sub(initial_borrow / 10)
        .max(config.min_flash_amount);
    let fine_end = (best_borrow + initial_borrow / 10).min(config.max_flash_amount);
    let step = ((fine_end - fine_start) / 20).max(1);

    for test_amount in (fine_start..=fine_end).step_by(step as usize) {
        let (amounts, outputs, bins, fees) =
            simulate_three_leg_chain(pool1, pool2, pool3, test_amount, config.max_slippage_bps);

        let flash_fee = calculate_kamino_flash_fee(test_amount, kamino_flash_fee_sf);
        let total_costs = flash_fee + fees + network_costs;
        let final_out = outputs[2];
        let net = final_out as i64 - test_amount as i64 - total_costs as i64;

        if net > best_profit {
            best_profit = net;
            best_borrow = test_amount;
            best_bps = if test_amount > 0 {
                (net * 10_000) / test_amount as i64
            } else {
                0
            };
            best_amounts = amounts;
            best_outputs = outputs;
            best_bins = bins;
            best_fees = fees;
        }
    }

    (
        best_borrow,
        best_profit,
        best_bps,
        best_amounts,
        best_outputs,
        best_bins,
        best_fees,
    )
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
        assert_eq!(config.safety_factor, 0.85);
        assert_eq!(config.min_flash_amount, 100_000);
        assert_eq!(config.max_flash_amount, 5_000_000_000);
    }
}
