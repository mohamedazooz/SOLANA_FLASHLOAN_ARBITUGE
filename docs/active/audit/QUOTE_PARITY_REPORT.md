# Quote Parity Report — Phase 3 (Mission §8)

Objective: prove that repo quote math equals official protocol math for every SUPPORTED venue, using committed differential suites, and honestly list where parity is not proven.

## 1. Parity evidence per venue

| Venue | Official reference | Repo differential suite | Result (this branch) |
|---|---|---|---|
| Orca Whirlpool | orca-so/whirlpools SDK math | `arb-math/tests/orca_sdk_parity.rs`, `whirlpool_mainnet_sdk_differential` (arb-engine), `orca_whirlpool_boundary_tests` (arb-dex) | PASS (one fixture corrected 2026-11-20 for zero-capacity tick boundary, PR #97) |
| Raydium CLMM | Raydium CLMM swap formula | `raydium_clmm_swap_math_parity_tests`, `raydium_clmm_tick_parity_tests`, `raydium_clmm_mainnet_sdk_differential`, `clmm_sqrt_bounds_tests`, `clmm_tick_array_layout_tests` | PASS |
| Meteora DLMM | Meteora lb_clmm math (`github.com/MeteoraAg/dlmm-sdk`) | `meteora_mainnet_sdk_differential`, `meteora_sdk_differential_tests`, `dlmm_bin_array_boundary_tests`, `dlmm_drain_breakpoints_tests`, `dlmm_exact_out_fee_invariant_tests`, `dlmm_q64_volatility_boundary_tests`, `phase17_pda_differential` (bin-array & oracle PDA derivations) | PASS |
| Raydium CPMM | CP-Swap (constant product + exact trade fee; reserve accounting from PoolState) | `raydium_cpmm_boundary_tests`, `cpmm_real_bytes_fixture`, `cross_dex_dlmm_cpmm_v1_atomic_tests` / `cross_dex_dlmm_cpmm_supervisor_execution` | PARTIAL / GATED: exact 1e6 fee ratio and accrued-fee reserve accounting covered; creator-fee-enabled pools fail closed; full official-SDK quote differential still required |
| Raydium V4 | constant-product with on-chain AMM fee fields | `raydium_v4_boundary_tests`, `raydium_v4_v2_execution_tests` | PASS for decoded account snapshots; invalid/missing fee state is fail-closed |
| Pump.fun | bonding-curve `virtual_reserves` formula | `pump_fun_boundary_tests`, `pump_boost_arb_tests`, `pump_state_tests` | PASS (embedded state fixtures; no live snapshot) |
| PumpSwap | constant-product post-grad pool | `pumpswap_boundary_tests`, `pumpswap_engine_tests` | PASS |
| Kamino flash fee | klend `flash_borrow_reserve` fee (see Phase 2 audit) | `kamino_fee_boundary_tests`, `differential.rs` | PASS (`U68F60`, ties-away, `2^60` floor) |
| Cross-venue net economics | — | `three_leg_net_economics_tests`, `differential_fuzzing_boundary_tests`, `fuzzing_suite`, `differential` proptest suite, `python_oracle_differential` (independent oracle), `adversarial_*` | PASS |

## 2. Where parity is NOT proven (honest gaps)

| Venue | Gap | Class |
|---|---|---|
| DAMM v2 / DBC / LaunchLab | No complete verified live quote path: DAMM v2/DBC state is incomplete; LaunchLab requires verified fee/config joins; no full discovery/mainnet parity | DEX-F-2026-11-22-N1 (P1 safety gate) |
| Meteora DLMM Pro (retired) | Removed from the active enum, quote/math, decoder, hydration and transaction paths; research-only documentation remains | Explicit retirement decision |
| Raydium CLMM fee | Discovery fallback (pool-decode path) uses a structural `fee_bps: 25`; operative fee comes from the Raydium catalog `feeRate` + millionths→bps conversion with fail-closed `fee_on`/`dynamic_fee_info` exclusions | DEX-F-2026-11-22-N2 (P3 telemetry, registered) |
| Pump.fun / PumpSwap | No committed Mainnet account snapshots in the test corpus (fixtures are synthetic but formula-faithful) | Improvement item (P3) |
| Token-2022 transfer-fee venues | Unsupported by policy; quote helpers exist but are not wired to discovery | Per ISSUE-P1-01, unchanged |

**Raydium CPMM fee attribution:** quote state carries the exact aggregate trade-fee ratio; generic `FeesBreakdown` component fields remain zero when allocation rates are not carried into `PoolState`. `total_fee` is authoritative. Graph prefiltering currently uses conservative integer-bps ceiling (safe but may under-rank sub-bps opportunities); exact fee ratios are retained in the quote engines. Carry exact ratios into `MarketEdge` and live allocation rates into `PoolState` in the follow-up tracked as DEX-F-2026-11-25-N2.

## 3. Nonlinear-math policy check

- No linear approximations in the SUPPORTED path: DLMM bin traversal is exact per-bin ladder; CLMM/Whirlpool exact tick math; Pump bonding-curve exact formula; CP pools constant-product (their true model).
- Bellman-Ford is used in the route **graph only for candidate discovery/white-listing of profitable cycles**; final evaluation re-quotes every hop with the exact pool functions (`graph.rs` route-by-size search, per-hop re-quote — see ISSUE-P0-02 resolution). BF negative-log cycle output is never used directly as an executed quote.
- Integer-only economics in decisions (`arb-math`); f64 appears only in USD notional telemetry, never in gates (verified in Phase 2/4 pass-throughs).

## 4. Test evidence (this branch)

```text
cargo test --workspace --all-targets
151 test binaries: 1,758 passed / 0 failed / 7 ignored

cargo clippy --workspace --all-targets -- -D warnings
PASS

cargo fmt --all -- --check
PASS
```
