# DEX_INTEGRATION_AUDIT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Adapters present (per arb-dex)

Raydium AMM/CPMM/CLMM, Meteora DLMM/DAMM/DAMM v2/DBC, Orca Whirlpool, PumpSwap, PumpFun, Jupiter (diagnostic), LaunchLab.

## Verification performed (static)

- Math crate (`arb-math`) uses integer arithmetic: CPMM (`x·y=k` with fee denom 10000), CLMM (Q64.64), DLMM (discrete bins, variable fee), DAMM v2, DBC, LaunchLab.
- Quote functions return `Option<u64>` (no lossy float casts in authoritative paths).
- Token-2022 transfer fee deduction: `apply_transfer_fee_ceil` (ceiling rounding, max-fee cap).
- Slippage: `check_slippage_bound` (bps).
- Constant product invariant: `check_constant_product_invariant`.
- `apply_transfer_fee` wrapper present.

## Gaps (not verified this session)

- Program IDs not re-verified against official sources (no network).
- Account layouts / vault accounts / tick-bin arrays not re-decoded live.
- Quote accuracy vs official SDK: not measured (no differential run this session; evidence artifacts exist under reports/phase15_2/).
- Token-2022 transfer-fee mints excluded from discovery (fail-closed tripwire in opportunity.rs re-verifies at execution).
- Meteora DAMM v2 Swap2 (14-account) canonical layout: referenced in commits; not re-verified.

## Math correctness notes

- `gross`/`expected_out` are net of DEX fees (no double counting).
- `calculate_net_profit_raw` converts costs to base-mint raw units using `lamports_per_base_raw`/`lamports_per_base_denom` (mint-aware).
- f64 appears only in display/telemetry (gross_sol, loan_size_sol, net_in_sol) and EV gate — not in authoritative profit decisions. (Note: `dynamic_max_share_bps` uses f64 for competition factor — non-authoritative sizing, acceptable but could be hardened.)

## Status

Structurally sound; live quote differential verification pending.
