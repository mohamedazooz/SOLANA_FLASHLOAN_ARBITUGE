# Quote Parity Report — Phase 3 (Mission §8)

Objective: prove that repo quote math equals official protocol math for every SUPPORTED venue, using committed differential suites, and honestly list where parity is not proven.

## 1. Parity evidence per venue

| Venue | Official reference | Repo differential suite | Result (this branch) |
|---|---|---|---|
| Orca Whirlpool | orca-so/whirlpools SDK math | `arb-math/tests/orca_sdk_parity.rs`, `whirlpool_mainnet_sdk_differential` (arb-engine), `orca_whirlpool_boundary_tests` (arb-dex) | PASS (one fixture corrected 2026-11-20 for zero-capacity tick boundary, PR #97) |
| Raydium CLMM | Raydium CLMM swap formula | `raydium_clmm_swap_math_parity_tests`, `raydium_clmm_tick_parity_tests`, `raydium_clmm_mainnet_sdk_differential`, `clmm_sqrt_bounds_tests`, `clmm_tick_array_layout_tests` | PASS |
| Meteora DLMM | Meteora lb_clmm math (`github.com/MeteoraAg/dlmm-sdk`) | `meteora_mainnet_sdk_differential`, `meteora_sdk_differential_tests`, `dlmm_bin_array_boundary_tests`, `dlmm_drain_breakpoints_tests`, `dlmm_exact_out_fee_invariant_tests`, `dlmm_q64_volatility_boundary_tests`, `phase17_pda_differential` (bin-array & oracle PDA derivations) | PASS |
| Raydium CPMM | CP-Swap (constant product + LP fee) | `raydium_cpmm_boundary_tests`, `cross_dex_dlmm_cpmm_v1_atomic_tests` / `cross_dex_dlmm_cpmm_supervisor_execution` | PASS |
| Raydium V4 | constant-product w/ 25bps fee | `raydium_v4_boundary_tests`, `raydium_v4_v2_execution_tests` | PASS (fee constant matches standard 0.25% vault fee; fee from `decode_raydium_fee_bps` when present, else 25) |
| Pump.fun | bonding-curve `virtual_reserves` formula | `pump_fun_boundary_tests`, `pump_boost_arb_tests`, `pump_state_tests` | PASS (embedded state fixtures; no live snapshot) |
| PumpSwap | constant-product post-grad pool | `pumpswap_boundary_tests`, `pumpswap_engine_tests` | PASS |
| Kamino flash fee | klend `flash_borrow_reserve` fee (see Phase 2 audit) | `kamino_fee_boundary_tests`, `differential.rs` | PASS (`U68F60`, ties-away, `2^60` floor) |
| Cross-venue net economics | — | `three_leg_net_economics_tests`, `differential_fuzzing_boundary_tests`, `fuzzing_suite`, `differential` proptest suite, `python_oracle_differential` (independent oracle), `adversarial_*` | PASS |

## 2. Where parity is NOT proven (honest gaps)

| Venue | Gap | Class |
|---|---|---|
| DAMM v2 / DBC / DLMM Pro / LaunchLab | No protocol-math quote at all — CP approximation with fixed fee; no discovery; no mainnet snapshots | DEX-F-2026-11-22-N1 (P3; fail-closed by discovery gate) |
| Raydium CLMM fee | Discovery fallback (pool-decode path) uses a structural `fee_bps: 25`; operative fee comes from the Raydium catalog `feeRate` + millionths→bps conversion with fail-closed `fee_on`/`dynamic_fee_info` exclusions | DEX-F-2026-11-22-N2 (P3 telemetry, registered) |
| Pump.fun / PumpSwap | No committed Mainnet account snapshots in the test corpus (fixtures are synthetic but formula-faithful) | Improvement item (P3) |
| Token-2022 transfer-fee venues | Unsupported by policy; quote helpers exist but are not wired to discovery | Per ISSUE-P1-01, unchanged |

## 3. Nonlinear-math policy check

- No linear approximations in the SUPPORTED path: DLMM bin traversal is exact per-bin ladder; CLMM/Whirlpool exact tick math; Pump bonding-curve exact formula; CP pools constant-product (their true model).
- Bellman-Ford is used in the route **graph only for candidate discovery/white-listing of profitable cycles**; final evaluation re-quotes every hop with the exact pool functions (`graph.rs` route-by-size search, per-hop re-quote — see ISSUE-P0-02 resolution). BF negative-log cycle output is never used directly as an executed quote.
- Integer-only economics in decisions (`arb-math`); f64 appears only in USD notional telemetry, never in gates (verified in Phase 2/4 pass-throughs).

## 4. Test evidence (this branch)

```text
cargo test -p arb-dex   → 127 passed / 0 failed
cargo test -p arb-math  → 342 passed / 0 failed
```
Full workspace (Phase 2 branch evidence, unchanged logic): 1,626 passed / 0 failed / 6 ignored.
