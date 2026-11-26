//! Adaptive Flash Loan Sizing — calculates optimal borrow amount based on
//! pool depth, liquidity, and risk parameters. No fixed amounts.

use crate::meteora_quote_engine::MeteoraDlmmCrossPoolQuoteEngine;
use arb_dex::engines::MeteoraDlmmQuoteEngine;
use arb_math::dlmm::{DlmmBin, dlmm_active_bin_max_input, dlmm_multi_bin_max_input};
use arb_types::{DexProtocol, MintId, PoolState, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

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
            min_position_lamports: arb_constants::MIN_KAMINO_FLASH_BORROW_LAMPORTS,
            max_position_lamports: arb_constants::MAX_KAMINO_FLASH_BORROW_LAMPORTS,
        }
    }
}

impl Default for FlashLoanSizingConfig {
    fn default() -> Self {
        Self {
            safety_factor_bps: 8_500,
            min_flash_amount: arb_constants::MIN_KAMINO_FLASH_BORROW_LAMPORTS,
            max_flash_amount: arb_constants::MAX_KAMINO_FLASH_BORROW_LAMPORTS,
            max_slippage_bps: arb_constants::DEFAULT_HOP_SLIPPAGE_BPS,
            target_profit_bps: 100, // 1%
            optimize_for_profit: true,
            max_bins_crossed: arb_constants::DEFAULT_MAX_BINS_PER_LEG,
            reserve_fraction_bps: 1_000, // Keep 10% in pool
            sizing_method: SizingMethod::SafetyFactor,
            kelly_params: KellyParams::default(),
        }
    }
}

/// A protocol/LP fee amount whose raw units remain attached to its mint.
/// Fees from different route legs must never be summed without a verified conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeAmountByMint {
    pub mint: MintId,
    pub amount_raw: u64,
}

impl Default for FeeAmountByMint {
    fn default() -> Self {
        Self {
            mint: MintId(Pubkey::default()),
            amount_raw: 0,
        }
    }
}

/// Result of flash loan sizing calculation
#[derive(Debug, Clone)]
pub struct FlashLoanSizingResult {
    /// Recommended borrow amount in lamports
    pub borrow_amount: u64,
    /// Conservative minimum-out net profit in lamports (after all costs).
    pub expected_profit: i64,
    /// Whether the conservative net result is strictly positive.
    pub is_profitable: bool,
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
    /// Per-leg DEX fees, each preserved in its own fee mint.
    pub dex_fees_by_leg: [FeeAmountByMint; 3],
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

fn wsol_mint_id() -> MintId {
    MintId(Pubkey::from_str(arb_constants::WSOL_MINT).expect("valid WSOL mint"))
}

fn other_mint(pool: &PoolState, mint: MintId) -> Option<MintId> {
    if pool.base_mint == mint {
        Some(pool.quote_mint)
    } else if pool.quote_mint == mint {
        Some(pool.base_mint)
    } else {
        None
    }
}

fn token_program_for(pool: &PoolState, mint: MintId) -> Option<TokenProgramKind> {
    if pool.base_mint == mint {
        Some(pool.token_programs[0])
    } else if pool.quote_mint == mint {
        Some(pool.token_programs[1])
    } else {
        None
    }
}

fn three_leg_mints(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
) -> Option<(MintId, MintId)> {
    let wsol = wsol_mint_id();
    let mint_a = other_mint(pool1, wsol)?;
    let mint_b = other_mint(pool3, wsol)?;
    if mint_a == wsol
        || mint_b == wsol
        || mint_a == mint_b
        || ![pool2.base_mint, pool2.quote_mint].contains(&mint_a)
        || ![pool2.base_mint, pool2.quote_mint].contains(&mint_b)
    {
        return None;
    }

    let pools = [pool1, pool2, pool3];
    if pools.iter().any(|pool| {
        pool.protocol != DexProtocol::MeteoraDlmm
            || !pool.is_active
            || pool.has_risky_token
            || pool.transfer_fee_bps != 0
            || pool.token_programs.contains(&TokenProgramKind::Token2022)
    }) {
        return None;
    }
    if pool1.pool_id == pool2.pool_id
        || pool1.pool_id == pool3.pool_id
        || pool2.pool_id == pool3.pool_id
    {
        return None;
    }

    // A and B must use the same token program in every pool they traverse.
    if token_program_for(pool1, mint_a)? != token_program_for(pool2, mint_a)?
        || token_program_for(pool2, mint_b)? != token_program_for(pool3, mint_b)?
        || token_program_for(pool1, wsol)? != token_program_for(pool3, wsol)?
    {
        return None;
    }
    Some((mint_a, mint_b))
}

fn empty_sizing_result(network_costs: u64, capped_by_capacity: bool) -> FlashLoanSizingResult {
    FlashLoanSizingResult {
        borrow_amount: 0,
        expected_profit: i64::MIN,
        is_profitable: false,
        profit_bps: 0,
        leg_amounts: [0; 3],
        leg_min_outputs: [0; 3],
        bins_crossed: [0; 3],
        flash_fee: 0,
        dex_fees_by_leg: [FeeAmountByMint::default(); 3],
        network_costs,
        expected_final_output: 0,
        capped_at_max: false,
        capped_by_capacity,
        confidence_bps: 0,
    }
}

/// Calculate optimal flash loan size for a three-pool WSOL cycle.
/// Capacity, quotes, fee denominations and min-out accounting all use the
/// same verified Meteora DEX quote path; unhydrated pools return a non-profitable result.
pub fn calculate_optimal_flash_loan(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    kamino_flash_fee_sf: u64,
    config: &FlashLoanSizingConfig,
    network_cost_lamports: u64,
    priority_fee_lamports: u64,
    jito_tip_lamports: u64,
) -> FlashLoanSizingResult {
    let Some(network_costs) = network_cost_lamports
        .checked_add(priority_fee_lamports)
        .and_then(|v| v.checked_add(jito_tip_lamports))
    else {
        return empty_sizing_result(u64::MAX, true);
    };
    let hard_max = config
        .max_flash_amount
        .min(arb_constants::MAX_KAMINO_FLASH_BORROW_LAMPORTS);
    if config.min_flash_amount == 0
        || config.min_flash_amount > hard_max
        || config.safety_factor_bps > 10_000
        || config.max_slippage_bps > 10_000
        || config.reserve_fraction_bps > 10_000
        || config.max_bins_crossed == 0
        || config.kelly_params.win_probability_bps > 10_000
        || config.kelly_params.fractional_kelly_bps > 10_000
    {
        return empty_sizing_result(network_costs, true);
    }
    let Some((mint_a, mint_b)) = three_leg_mints(pool1, pool2, pool3) else {
        return empty_sizing_result(network_costs, true);
    };
    let wsol = wsol_mint_id();
    let dir1 = pool1.base_mint == wsol;
    let dir2 = pool2.base_mint == mint_a;
    let dir3 = pool3.base_mint == mint_b;

    // The direction flag means X -> Y; derive it from the actual input mint.
    let cap1 = get_leg_capacity_with_limit(pool1, dir1, config.max_bins_crossed);
    let cap2 = get_leg_capacity_with_limit(pool2, dir2, config.max_bins_crossed);
    let cap3 = get_leg_capacity_with_limit(pool3, dir3, config.max_bins_crossed);
    let safe_cap = |cap: u64| -> u64 {
        u64::try_from(u128::from(cap) * u128::from(config.safety_factor_bps) / 10_000)
            .unwrap_or(u64::MAX)
    };
    let bottleneck = safe_cap(cap1).min(safe_cap(cap2)).min(safe_cap(cap3));
    if bottleneck == 0 {
        return empty_sizing_result(network_costs, true);
    }
    let reserve_adjusted = u64::try_from(
        u128::from(bottleneck) * u128::from(10_000u64 - config.reserve_fraction_bps) / 10_000,
    )
    .unwrap_or(u64::MAX);
    let max_allowed = reserve_adjusted.min(hard_max);
    if max_allowed < config.min_flash_amount {
        return empty_sizing_result(network_costs, true);
    }

    let borrow_amount = match config.sizing_method {
        SizingMethod::SafetyFactor => max_allowed,
        SizingMethod::Kelly => calculate_kelly_optimal_size(
            max_allowed,
            &config.kelly_params,
            config.min_flash_amount,
            max_allowed,
        ),
    };
    if borrow_amount < config.min_flash_amount || borrow_amount > max_allowed {
        return empty_sizing_result(network_costs, true);
    }
    let (leg_amounts, leg_min_outputs, bins_crossed, dex_fees_by_leg, final_output) =
        simulate_three_leg_chain(pool1, pool2, pool3, borrow_amount, config.max_slippage_bps);
    if leg_amounts.contains(&0) || leg_min_outputs[2] == 0 {
        return empty_sizing_result(network_costs, true);
    }
    let net_profit = net_profit_lamports(
        leg_min_outputs[2],
        borrow_amount,
        kamino_flash_fee_sf,
        network_costs,
    );
    let profit_bps = if borrow_amount > 0 {
        i64::try_from(i128::from(net_profit) * 10_000 / i128::from(borrow_amount))
            .unwrap_or(if net_profit >= 0 { i64::MAX } else { i64::MIN })
    } else {
        0
    };

    let outcome = if i128::from(net_profit)
        < i128::from(config.target_profit_bps) * i128::from(borrow_amount) / 10_000
    {
        optimize_size(
            pool1,
            pool2,
            pool3,
            borrow_amount,
            max_allowed,
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
            dex_fees_by_leg,
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
        dex_fees_by_leg: final_dex_fees_by_leg,
        expected_output: final_expected_output,
    } = outcome;

    let capacity_ratio_bps =
        u64::try_from(u128::from(final_borrow) * 10_000 / u128::from(bottleneck.max(1)))
            .unwrap_or(u64::MAX)
            .min(10_000);
    let confidence_bps = (10_000 - capacity_ratio_bps).max(3_000);
    FlashLoanSizingResult {
        borrow_amount: final_borrow,
        expected_profit: final_profit,
        is_profitable: final_profit > 0,
        profit_bps: final_bps,
        leg_amounts: final_amounts,
        leg_min_outputs: final_outputs,
        bins_crossed: final_bins,
        flash_fee: calculate_kamino_flash_fee(final_borrow, kamino_flash_fee_sf),
        dex_fees_by_leg: final_dex_fees_by_leg,
        network_costs,
        expected_final_output: final_expected_output,
        capped_at_max: final_borrow == hard_max,
        capped_by_capacity: final_borrow == reserve_adjusted && reserve_adjusted < hard_max,
        confidence_bps,
    }
}

/// Get maximum input capacity for a single leg. Requires a complete, bitmap-proven
/// BinArray snapshot and uses the configured traversal depth.
fn get_leg_capacity_with_limit(pool: &PoolState, swap_for_y: bool, max_bins: u32) -> u64 {
    if pool.protocol != DexProtocol::MeteoraDlmm
        || !pool.is_active
        || pool.has_risky_token
        || pool.transfer_fee_bps != 0
        || pool.token_programs != [TokenProgramKind::SplToken; 2]
        || max_bins == 0
        || MeteoraDlmmQuoteEngine::validate_pool_mint_snapshot(pool).is_err()
    {
        return 0;
    }
    let Some(dlmm_state) = pool.dlmm.as_ref() else {
        return 0;
    };
    if MeteoraDlmmQuoteEngine::validate_bin_array_snapshot(
        pool.pool_id,
        dlmm_state,
        pool.execution_accounts.as_ref(),
    )
    .is_err()
    {
        return 0;
    }
    // Reuse the canonical DLMM state/bin conversion used by the quote engine;
    // capacity math must never maintain a second bin-field mapping.
    let Ok((math_state, bins)) = MeteoraDlmmQuoteEngine::build_dlmm_math_state(pool) else {
        return 0;
    };
    let active_id = math_state.active_id;
    let limit = max_bins.saturating_sub(1).min(i32::MAX as u32) as i32;
    let filtered_bins: Vec<DlmmBin> = bins
        .into_iter()
        .filter(|bin| {
            if swap_for_y {
                bin.id <= active_id && bin.id >= active_id.saturating_sub(limit)
            } else {
                bin.id >= active_id && bin.id <= active_id.saturating_add(limit)
            }
        })
        .collect();
    if let Some(capacity) =
        dlmm_multi_bin_max_input(active_id, math_state.bin_step, &filtered_bins, swap_for_y)
    {
        if capacity > 0 {
            return capacity;
        }
    }
    let Some(active_bin) = filtered_bins.iter().find(|bin| bin.id == active_id) else {
        return 0;
    };
    dlmm_active_bin_max_input(
        active_id,
        math_state.bin_step,
        active_bin.amount_x,
        active_bin.amount_y,
        swap_for_y,
    )
    .unwrap_or(0)
}

fn fee_mint_for_mode(pool: &PoolState, input_mint: MintId) -> Option<MintId> {
    MeteoraDlmmQuoteEngine::fee_mint_for_quote(pool, input_mint).ok()
}

#[cfg(test)]
fn apply_slippage(amount: u64, slippage_bps: u64) -> u64 {
    arb_constants::slippage_floor_bps(amount, slippage_bps)
}

/// Quote a three-pool WSOL cycle through the single canonical DLMM quote engine.
/// Each fee stays paired with the mint in which the protocol charges it.
fn simulate_three_leg_chain(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    borrow_amount: u64,
    max_slippage_bps: u64,
) -> ([u64; 3], [u64; 3], [u32; 3], [FeeAmountByMint; 3], u64) {
    let empty = ([0; 3], [0; 3], [0; 3], [FeeAmountByMint::default(); 3], 0);
    if borrow_amount == 0 || max_slippage_bps > 10_000 {
        return empty;
    }
    let Some((mint_a, mint_b)) = three_leg_mints(pool1, pool2, pool3) else {
        return empty;
    };
    let wsol = wsol_mint_id();

    let Ok(slippage_bps) = u16::try_from(max_slippage_bps) else {
        return empty;
    };
    let Some((quote1, _)) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop_with_slippage(
        pool1,
        borrow_amount,
        wsol,
        mint_a,
        slippage_bps,
    ) else {
        return empty;
    };
    let Some(fee_mint1) = fee_mint_for_mode(pool1, wsol) else {
        return empty;
    };
    let min1 = quote1.minimum_amount_out_raw;
    if min1 == 0 {
        return empty;
    }

    let Some((quote2, _)) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop_with_slippage(
        pool2,
        min1,
        mint_a,
        mint_b,
        slippage_bps,
    ) else {
        return empty;
    };
    let Some(fee_mint2) = fee_mint_for_mode(pool2, mint_a) else {
        return empty;
    };
    let min2 = quote2.minimum_amount_out_raw;
    if min2 == 0 {
        return empty;
    }

    let Some((quote3, _)) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop_with_slippage(
        pool3,
        min2,
        mint_b,
        wsol,
        slippage_bps,
    ) else {
        return empty;
    };
    let Some(fee_mint3) = fee_mint_for_mode(pool3, mint_b) else {
        return empty;
    };
    let min3 = quote3.minimum_amount_out_raw;
    if min3 == 0 {
        return empty;
    }

    let amounts = [borrow_amount, min1, min2];
    let min_outputs = [min1, min2, min3];
    let bins = [
        quote1.dlmm_bins_crossed.unwrap_or(0),
        quote2.dlmm_bins_crossed.unwrap_or(0),
        quote3.dlmm_bins_crossed.unwrap_or(0),
    ];
    let fees = [
        FeeAmountByMint {
            mint: fee_mint1,
            amount_raw: quote1.fees_breakdown.total_fee,
        },
        FeeAmountByMint {
            mint: fee_mint2,
            amount_raw: quote2.fees_breakdown.total_fee,
        },
        FeeAmountByMint {
            mint: fee_mint3,
            amount_raw: quote3.fees_breakdown.total_fee,
        },
    ];
    (amounts, min_outputs, bins, fees, quote3.amount_out_raw)
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
    // This sizing path is explicitly WSOL-denominated: all amounts and the
    // combined network-cost input are lamports, so the canonical rational
    // conversion is the identity 1/1. Preserve a signed shortfall; any
    // calculation error fails closed as the most negative representable value.
    let net_i128 = match arb_math::economic::calculate_net_profit_raw_signed(
        final_output,
        borrow,
        flash_fee,
        0, // DEX fees are already reflected in the chained quote output
        0, // transfer fees are already reflected in the per-leg minimum outputs
        0, // no separate account-creation charge in this sizing model
        0, // network/priority/tip are supplied together as lamports below
        arb_types::Lamports(network_cost_lamports),
        arb_types::Lamports(0),
        arb_types::Lamports(0),
        1,
        1,
    ) {
        Ok(value) => value,
        Err(_) => return i64::MIN,
    };
    match i64::try_from(net_i128) {
        Ok(value) => value,
        Err(_) if net_i128.is_negative() => i64::MIN,
        Err(_) => i64::MAX,
    }
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
    dex_fees_by_leg: [FeeAmountByMint; 3],
    expected_output: u64,
}

fn consider_size_candidate(
    best: &mut SizeSearchOutcome,
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    test_amount: u64,
    upper_bound: u64,
    config: &FlashLoanSizingConfig,
    kamino_flash_fee_sf: u64,
    network_costs: u64,
) {
    if test_amount < config.min_flash_amount || test_amount > upper_bound {
        return;
    }
    let (amounts, min_outputs, bins, fees, expected_output) =
        simulate_three_leg_chain(pool1, pool2, pool3, test_amount, config.max_slippage_bps);
    if amounts.contains(&0) || min_outputs[2] == 0 {
        return;
    }
    let profit = net_profit_lamports(
        min_outputs[2],
        test_amount,
        kamino_flash_fee_sf,
        network_costs,
    );
    let bps = i64::try_from(i128::from(profit) * 10_000 / i128::from(test_amount))
        .unwrap_or(if profit >= 0 { i64::MAX } else { i64::MIN });
    let better = if config.optimize_for_profit {
        profit > best.profit
    } else {
        profit > 0 && test_amount > best.borrow
    };
    if better {
        *best = SizeSearchOutcome {
            borrow: test_amount,
            profit,
            bps,
            amounts,
            min_outputs,
            bins,
            dex_fees_by_leg: fees,
            expected_output,
        };
    }
}

fn optimize_size(
    pool1: &PoolState,
    pool2: &PoolState,
    pool3: &PoolState,
    initial_borrow: u64,
    max_allowed: u64,
    config: &FlashLoanSizingConfig,
    kamino_flash_fee_sf: u64,
    network_costs: u64,
) -> SizeSearchOutcome {
    let upper_bound = max_allowed
        .min(config.max_flash_amount)
        .min(arb_constants::MAX_KAMINO_FLASH_BORROW_LAMPORTS);
    let mut best = SizeSearchOutcome {
        borrow: 0,
        profit: i64::MIN,
        bps: 0,
        amounts: [0; 3],
        min_outputs: [0; 3],
        bins: [0; 3],
        dex_fees_by_leg: [FeeAmountByMint::default(); 3],
        expected_output: 0,
    };
    if upper_bound < config.min_flash_amount {
        return best;
    }

    // Always include the initial candidate, then scan a bounded coarse grid.
    consider_size_candidate(
        &mut best,
        pool1,
        pool2,
        pool3,
        initial_borrow.min(upper_bound),
        upper_bound,
        config,
        kamino_flash_fee_sf,
        network_costs,
    );
    let mut test_amount = config.min_flash_amount;
    while test_amount <= upper_bound {
        consider_size_candidate(
            &mut best,
            pool1,
            pool2,
            pool3,
            test_amount,
            upper_bound,
            config,
            kamino_flash_fee_sf,
            network_costs,
        );
        let next = test_amount.saturating_mul(3) / 2;
        if next <= test_amount {
            break;
        }
        test_amount = next;
    }

    // Fine search around the best coarse point; never exceed verified capacity.
    if best.borrow > 0 {
        let radius = (initial_borrow / 10).max(1);
        let fine_start = best
            .borrow
            .saturating_sub(radius)
            .max(config.min_flash_amount);
        let fine_end = best.borrow.saturating_add(radius).min(upper_bound);
        let step = ((fine_end.saturating_sub(fine_start)) / 20).max(1);
        let step = usize::try_from(step).unwrap_or(usize::MAX).max(1);
        for amount in (fine_start..=fine_end).step_by(step) {
            consider_size_candidate(
                &mut best,
                pool1,
                pool2,
                pool3,
                amount,
                upper_bound,
                config,
                kamino_flash_fee_sf,
                network_costs,
            );
        }
    }
    best
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
        assert_eq!(
            config.min_flash_amount,
            arb_constants::MIN_KAMINO_FLASH_BORROW_LAMPORTS
        );
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
            params: Some(arb_types::DlmmFeeParams::default()),
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 0,
            ..Default::default()
        };

        let pool_id = PoolId::new([1u8; 32]);
        let pool_pubkey = Pubkey::new_from_array([1u8; 32]);
        let bin_array_pdas = [-1_i64, 0_i64]
            .map(|index| MeteoraDlmmQuoteEngine::derive_bin_array_pda(&pool_pubkey, index).0)
            .to_vec();
        let base_mint = MintId(Pubkey::new_unique());
        let quote_mint = MintId(Pubkey::new_unique());
        let execution_accounts = arb_types::PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas,
            bitmap_extension: None,
            pool_token_x: base_mint.0,
            pool_token_y: quote_mint.0,
        };
        let pool = PoolState {
            pool_id,
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint,
            quote_mint,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 0,
            reserve_out: 0,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(dlmm_state),
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: Some(execution_accounts),
            last_update_slot: 1,
            last_update_ms: 0,
        };

        // swap_for_y = true: active bin has 0 Y, but multi-bin reaches bin -1
        let cap = get_leg_capacity_with_limit(&pool, true, 2);
        assert!(
            cap > 0,
            "Capacity should not be 0 when adjacent bin has liquidity"
        );
    }
}
