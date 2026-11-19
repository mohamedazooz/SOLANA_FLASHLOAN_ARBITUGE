# Three-Pool Execution Audit (PHASE F)

## Target route

WSOL → Token A → Token B → WSOL across three distinct Meteora DLMM pools, financed by a Kamino flash loan.

## Production wiring — PROVEN, not standalone

The three-leg builder is wired into the real Supervisor dispatch path:

1. `crates/arb-bot/src/supervisor/run.rs:79` — `capture_three_leg_dlmm()` runs inside the Supervisor scan loop.
2. `run.rs:163` — calls `arb_engine::build_three_leg_meme_pipeline(fresh_pools, &params)`.
3. The pipeline (`arb-engine/src/three_leg_meme_pipeline.rs`) performs chained DLMM quotes, coarse→fine size search, and net-profit computation via `arb_math::economic`.
4. The winning chain is converted to a standard `ExecutionOpportunity` with three `SwapHop`s (protocol `MeteoraDlmm`, per-hop inputs chained from each leg's conservative minimum output) — `run.rs:176-215`.
5. The opportunity then flows through the SAME economic gate (`supervisor/opportunity.rs`), risk gates, and dispatch (`supervisor/dispatch.rs`) as every other route — no separate execution path exists.

Call-graph proof: `cargo test -p arb-engine --test three_leg_meme_pipeline_tests` and
`three_leg_net_economics_tests` exercise the pipeline; `hop_two_input_is_bound_to_hop_one_minimum_output`
pins the chained-minimum invariant.

## Validation enforced

* Pool distinctness (`ThreeLegMemeArbitrageContext::validate`): P1≠P2, P2≠P3, P1≠P3.
* Mint chain closure: leg N's output mint = leg N+1's input mint; final output = WSOL.
* Per-leg `amount_in` = previous leg's conservative `min_amount_out` (never the optimistic output).
* Per-leg `min_amount_out` from the slippage config; final leg's minimum gates profitability.
* Bin arrays per leg planned by `plan_bin_arrays` with the corrected Bitmap Extension rule.
* Rejects: repeated pools, broken mint chains, missing bin arrays, insufficient liquidity, non-closing routes.

## Transaction assembly

`compile_3leg_meme_dlmm_arb` (`arb-tx/src/flash/compiler.rs`):
* V0 + Static Master ALT when ≤ 1232 bytes → `V0JitoCompatible`.
* V1 direct transport (≤ 4096, accounts inline, ComputeBudget stripped into message config) only when `v1_transport_supported` — currently fail-closed `false` in the 3-leg path (BUG-004, documented blocker).
* Never silently downgrades: oversized routes return `Rejected(V0Oversized/V1Oversized)`.

## Status

WIRED AND TESTED. Live-mainnet evidence (previous phase): full fresh-pipeline cycle
borrow 0.1 USDC → 2-hop → net −1,906 raw → correctly rejected; 942-byte V0 wire,
76,549 CU, slot lag 1, simulation success, zero broadcast.
