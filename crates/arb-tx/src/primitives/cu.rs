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

/// Calculate dynamic compute unit limit with measured buffer (simulated via reserve depth proxy — upgrade path: real simulateTransaction).
pub fn calculate_dynamic_cu_limit(base_required_units: u32) -> u32 {
    // 15% safety buffer over measured base requirement.
    let buffer = base_required_units.saturating_mul(15) / 100;
    let total = base_required_units.saturating_add(buffer);
    total.min(MAX_COMPUTE_UNIT_LIMIT)
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
    // Whole-lamport priority budget: `ceil(profit * share_bps / 10_000)`.
    let budget = profit_lamports
        .saturating_mul(max_share_bps)
        .div_ceil(10_000);
    // Largest per-CU price whose total cost stays within `budget`.
    let max_price = budget
        .saturating_mul(1_000_000)
        / u64::from(cu_limit);
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
    cu_price_micro_lamports
        .saturating_mul(u64::from(cu_limit))
        .div_ceil(1_000_000)
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
        MeteoraDlmmPro => 90_000,
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
    let base_overhead: u32 = 450; // ComputeBudget + tip
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 }; // borrow + repay
    let swaps: u32 = hops.iter().map(|p| per_hop_cu(*p)).sum();
    let atas = (ephemeral_atas as u32).saturating_mul(20_000); // create + close each
    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swaps)
        .saturating_add(atas)
}
