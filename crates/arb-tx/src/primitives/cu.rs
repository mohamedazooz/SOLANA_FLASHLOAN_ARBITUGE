//! Compute-unit estimation and dynamic priority-fee calculation.

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

/// Calculate dynamic compute unit price in micro-lamports, dynamically elevated based on profit.
pub fn calculate_dynamic_cu_price(profit_lamports: u64, base_priority_fee: u64) -> u64 {
    let dynamic_elevation = (profit_lamports / 25_000).min(500_000);
    base_priority_fee.saturating_add(dynamic_elevation)
}

/// Calculate fee-deduplicated compute unit price.
/// In bundle-only mode (`bundle_only = true`), returns 0 to eliminate priority fee duplication
/// and concentrate 100% of priority budget into dynamic JitoTip.
pub fn calculate_fee_deduplicated_cu_price(
    profit_lamports: u64,
    base_priority_fee: u64,
    bundle_only: bool,
) -> u64 {
    if bundle_only {
        0
    } else {
        calculate_dynamic_cu_price(profit_lamports, base_priority_fee)
    }
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
    let base_overhead: u32 = 450; // ComputeBudget + tip
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 }; // borrow + repay
    let swaps: u32 = hops.iter().map(|p| per_hop_cu(*p)).sum();
    let atas = (ephemeral_atas as u32).saturating_mul(20_000); // create + close each
    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swaps)
        .saturating_add(atas)
}

/// Cap a compute-unit price so the total priority fee stays within
/// `max_share_bps` of `profit_lamports`.
///
/// The priority fee in lamports is `cu_price (µlamports/CU) * cu_limit / 1e6`.
/// Bounding it by a share of the edge stops a runaway priority bid from eating
/// the profit it was meant to capture.
pub fn cap_cu_price_by_profit(
    profit_lamports: u64,
    cu_limit: u32,
    desired_cu_price: u64,
    max_share_bps: u64,
) -> u64 {
    if cu_limit == 0 || max_share_bps == 0 {
        return 0;
    }
    let budget = profit_lamports.saturating_mul(max_share_bps) / 10_000;
    let max_price = budget.saturating_mul(1_000_000) / u64::from(cu_limit);
    desired_cu_price.min(max_price)
}
