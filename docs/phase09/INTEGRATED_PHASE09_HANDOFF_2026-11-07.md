# Integrated Phase-09 handoff — 2026-11-07

## What is this archive?

A single full Rust workspace based on the later Phase-09 remediation archive, with the separate remediation-v1 audit/reference material preserved inside the same repository. The reference proof pipeline is not compiled into production until its types and risk/execution interfaces are reconciled.

## First work delivered: CLMM/DLMM safety pass

- Removed the fake single-step Orca quote path based on an eight-tick heuristic and ±1% fallback.
- Orca quoting now requires supplied tick arrays and uses the multi-tick CLMM quote function.
- The native Orca fee-rate integer is passed directly; it is not rounded down to integer bps and multiplied back.
- CLMM swap-step errors are propagated instead of replaced by synthetic output/price values.
- CLMM multi-step quote returns explicit errors for zero progress, traversal limit and incomplete input consumption; arithmetic totals are checked.
- DLMM Q64.64 power calculation no longer maps overflow/underflow to fabricated prices.
- CLMM/DLMM engine input and reserve conversions reject out-of-range values rather than clamping.
- Added focused regression tests for those failure paths.

## Remaining Phase 09A work

- Implement Raydium CLMM as a distinct adapter and account decoder. The current Orca-shaped 88-tick parser must not be reused for Raydium.
- Verify Orca tick-array parser offsets against official account layouts and golden fixtures.
- Replace fixed three-array ingestion with dynamic tick-array traversal and slot/fork-coherent account state.
- Replace fixed three-bin-array ingestion with dynamic Meteora BinArray coverage.
- Differential-test fee rounding, tick/bin crossing, dynamic fees, volatility update order and exact-in/exact-out amounts against official SDK/program code.
- Add instruction/account parity before enabling either protocol for live execution.

## Validation performed

Only static checks are available in this environment. Rust compilation and tests have not run because `cargo`/`rustc` are not installed. The archive must pass `cargo fmt`, `cargo check`, `cargo test`, `cargo clippy` on the deployment/build machine before acceptance.

## Runtime posture

Shadow/Simulate only. No live execution, transaction signing, or broadcast is authorized by this package. Failed or incomplete quotes must reject.
