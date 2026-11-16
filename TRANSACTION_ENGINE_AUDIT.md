# TRANSACTION_ENGINE_AUDIT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## V1-only profile

- V1 discriminator 0x81, 4096-byte wire ceiling, 64 inline accounts, no ALT.
- Config: `V1ConfigParams { priority_fee_lamports, compute_unit_limit, loaded_accounts_data_size_limit, heap_size }`.
- `AdaptiveTxCompiler::compile_v1_only` is the only compilation path in dispatch (no Legacy/V0/ALT fallback).
- V1 transport opt-in per channel via `SOLANA_TX_V1_DIRECT_ENABLED` / `SOLANA_TX_V1_JITO_ENABLED` (default false).
- `maxSupportedTransactionVersion: 1` required on reads.

## Compute budget

- `estimate_route_cu` (protocol-specific per-hop + ephemeral ATAs).
- `calculate_dynamic_cu_limit` (15% headroom).
- `cap_cu_price_by_profit` (tip share of gross, competition-aware).
- `priority_fee_lamports(cu_price, cu_limit)` (ceiling rounding).
- No hardcoded 600k/900k/1.4M magic numbers for the limit; measured CU used when available (Phase E).

## Verification gaps

- V1 size ladder (4096-byte ceiling) executed in prior evidence (reports/v1_canary/) but not re-run this session.
- Loaded accounts data size limit set to 67,108,864 (8 MiB) in opportunity.rs — verify against current spec.
- `priority_fee_lamports == u64::MAX` fail-closed check present.
- Simulation (`unitsConsumed`, loaded data) integration not re-verified.

## Status

V1-only discipline intact; live verification pending.
