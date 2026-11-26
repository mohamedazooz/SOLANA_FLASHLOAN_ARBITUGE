//! Compute-unit estimation and dynamic priority-fee calculation.
//!
//! One authoritative priority-fee path: [`cap_cu_price_by_profit`] caps the
//! per-CU price against a share of the edge, and [`priority_fee_lamports`]
//! converts that same price to lamports with ceiling rounding. Callers must
//! encode the capped price verbatim and must not add a second elevation.

use crate::*;

/// Estimate the minimal compute units strictly required by the transaction
/// before safety buffer is applied, keeping instruction execution minimal and fast.
pub fn estimate_minimal_cu(has_flash_loan: bool, hops_count: usize) -> u32 {
    let base_overhead: u32 = 450; // Compute Budget + Tip
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 }; // Kamino Borrow + Repay
    let swap_cu = (hops_count as u32).saturating_mul(35_000); // Minimal needed per DEX swap hop
    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swap_cu)
}

/// Calculate a dynamic CU limit with a caller-selected safety margin.
/// The margin is rounded up so integer truncation cannot understate the requested
/// headroom, then clamped to Solana's runtime transaction cap.
pub fn calculate_dynamic_cu_limit_with_buffer(
    base_required_units: u32,
    safety_margin_bps: u16,
) -> u32 {
    if base_required_units == 0 {
        return 0;
    }
    let margin_bps = u128::from(safety_margin_bps.min(10_000));
    let numerator = u128::from(base_required_units) * margin_bps;
    let buffer = numerator.div_ceil(10_000);
    let total = u128::from(base_required_units).saturating_add(buffer);
    u32::try_from(total.min(u128::from(MAX_COMPUTE_UNIT_LIMIT))).unwrap_or(MAX_COMPUTE_UNIT_LIMIT)
}

/// Conservative fallback for unmeasured route estimates: 15% headroom.
pub fn calculate_dynamic_cu_limit(base_required_units: u32) -> u32 {
    calculate_dynamic_cu_limit_with_buffer(base_required_units, 1_500)
}

/// Apply the authoritative profit cap to a desired compute-unit price.
///
/// The supervisor has already decided what share of the edge this route may
/// spend on priority. This function clamps the desired price to that ceiling and
/// is the ONLY place the cap is applied — callers must encode its result
/// verbatim.
///
/// It deliberately does NOT add anything. The previous implementation added a
/// second profit-scaled elevation on top of the caller's already-capped price,
/// which meant the transaction could pay more than the profitability gate had
/// approved, and could resurrect a non-zero price from a cap that had correctly
/// rounded down to zero.
///
/// Rounding: the cap must satisfy the exact integer invariant
/// `price * cu_limit <= budget * 1e6`, where `budget` is the whole-lamport
/// amount `ceil(profit * share_bps / 10_000)`. Taking `floor` of the division
/// guarantees that invariant; rounding up would let the capped price's own
/// lamport cost exceed the very budget it was derived from (e.g. profit 1 with a
/// 3-CU limit would cost 2 lamports against a 1-lamport budget).
///
/// The budget itself rounds UP (away from zero) so an infinitesimal share of a
/// real profit is never truncated to "free priority". Saturating arithmetic
/// throughout; a zero `cu_limit` or `max_share_bps` yields `0` (no budget, no
/// bid).
pub fn cap_cu_price_by_profit(
    profit_lamports: u64,
    cu_limit: u32,
    desired_cu_price: u64,
    max_share_bps: u64,
) -> u64 {
    if cu_limit == 0 || max_share_bps == 0 {
        return 0;
    }
    // Use u128 throughout: saturating u64 multiplication here can turn an
    // overflow into a smaller-than-real budget after the division, or into a
    // misleading cap. u64 × u64 fits exactly in u128.
    let budget = (u128::from(profit_lamports) * u128::from(max_share_bps)).div_ceil(10_000);
    // Largest per-CU price whose total cost stays within `budget`.
    let max_price = budget.saturating_mul(1_000_000) / u128::from(cu_limit);
    let max_price = u64::try_from(max_price).unwrap_or(u64::MAX);
    desired_cu_price.min(max_price)
}

/// Convert an authoritative per-CU price into the total lamport priority fee.
///
/// Ceiling division: the priority fee is a **non-refundable** cost, so
/// truncating would understate it and overstate net profit. Kept next to
/// [`cap_cu_price_by_profit`] so the profitability gate and the encoded
/// instruction cannot drift apart.
///
/// Returns `u64::MAX` on overflow so a caller comparing against available funds
/// rejects rather than silently clamping a cost downward.
#[must_use]
pub fn priority_fee_lamports(cu_price_micro_lamports: u64, cu_limit: u32) -> u64 {
    // The shared V1 fee converter uses checked u128 arithmetic and returns
    // None on an unrepresentable result. Preserve a fail-closed sentinel:
    // never saturate the product in u64 and then divide the saturated value.
    crate::adaptive::total_priority_fee_lamports(cu_price_micro_lamports, u64::from(cu_limit))
        .unwrap_or(u64::MAX)
}

/// Per-hop compute units by protocol. A CLMM crosses ticks and a DLMM walks
/// bins, so they cost materially more than a constant-product swap. Used to size
/// the CU limit from the REAL route instead of a flat per-hop guess.
pub fn per_hop_cu(protocol: arb_types::DexProtocol) -> u32 {
    use arb_types::DexProtocol::*;
    match protocol {
        RaydiumV4 | RaydiumCpmm => 35_000,
        PumpFun => 42_000,
        PumpSwap => 48_000,
        RaydiumLaunchLab => 50_000,
        MeteoraDbc => 55_000,
        MeteoraDammV2 => 65_000,
        OrcaWhirlpool | RaydiumClmm => 85_000,
        MeteoraDlmm => 120_000,
    }
}

/// Estimate the CU a route actually needs: fixed overhead, flash-loan overhead,
/// per-hop cost (protocol-specific), and ephemeral ATA create/close. The caller
/// adds the safety buffer via `calculate_dynamic_cu_limit`.
pub fn estimate_route_cu(
    has_flash_loan: bool,
    hops: &[arb_types::DexProtocol],
    ephemeral_atas: usize,
) -> u32 {
    let intermediate_accounts = u16::try_from(ephemeral_atas).unwrap_or(u16::MAX);
    let create_count = u16::try_from(ephemeral_atas.saturating_add(1)).unwrap_or(u16::MAX);
    let counts = RouteAtaInstructionCounts {
        unique_token_accounts: create_count,
        intermediate_accounts,
        idempotent_create_instructions: create_count,
        close_token_account_instructions: 0,
    };
    estimate_route_cu_with_shape(has_flash_loan, hops, counts, 0, 0, 0)
}

/// Shape-aware CU estimate used by graph sizing and the final execution gate.
/// Estimates remain provisional until measured for the exact same CU shape.
pub fn estimate_route_cu_with_shape(
    has_flash_loan: bool,
    hops: &[arb_types::DexProtocol],
    ata_counts: RouteAtaInstructionCounts,
    dlmm_bins_crossed: u32,
    clmm_steps_taken: u32,
    required_accounts: u32,
) -> u32 {
    const ATA_CREATE_CU: u32 = 20_000;
    const ATA_CLOSE_CU: u32 = 7_500;
    const EXTRA_BIN_CU: u32 = 1_500;
    const EXTRA_TICK_STEP_CU: u32 = 3_000;
    const EXTRA_ACCOUNT_CU: u32 = 750;
    const BASE_ACCOUNTS_PER_HOP: u32 = 5;

    let base_overhead: u32 = 450;
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 };
    let swaps = hops.iter().fold(0u32, |total, protocol| {
        total.saturating_add(per_hop_cu(*protocol))
    });
    let creates =
        u32::from(ata_counts.idempotent_create_instructions).saturating_mul(ATA_CREATE_CU);
    let closes =
        u32::from(ata_counts.close_token_account_instructions).saturating_mul(ATA_CLOSE_CU);
    let dlmm_hops = u32::try_from(
        hops.iter()
            .filter(|p| **p == arb_types::DexProtocol::MeteoraDlmm)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let clmm_hops = u32::try_from(
        hops.iter()
            .filter(|p| {
                matches!(
                    p,
                    arb_types::DexProtocol::OrcaWhirlpool | arb_types::DexProtocol::RaydiumClmm
                )
            })
            .count(),
    )
    .unwrap_or(u32::MAX);
    let extra_bins = dlmm_bins_crossed.saturating_sub(dlmm_hops);
    let extra_steps = clmm_steps_taken.saturating_sub(clmm_hops);
    let base_accounts = u32::try_from(hops.len())
        .unwrap_or(u32::MAX)
        .saturating_mul(BASE_ACCOUNTS_PER_HOP);
    let extra_accounts = required_accounts.saturating_sub(base_accounts);

    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swaps)
        .saturating_add(creates)
        .saturating_add(closes)
        .saturating_add(extra_bins.saturating_mul(EXTRA_BIN_CU))
        .saturating_add(extra_steps.saturating_mul(EXTRA_TICK_STEP_CU))
        .saturating_add(extra_accounts.saturating_mul(EXTRA_ACCOUNT_CU))
}
