# Evidence-based audit of supplied snapshot

## Scope and limitations

Reviewed the four supplied files and unpacked the supplied source archive. The archive has no `.git` directory and does not include the referenced `AGENTS.md` or `docs/reports/FINDINGS.md`; commit history, key exposure history, and the original gate definitions therefore cannot be independently reconstructed. The local environment has no Rust toolchain, so compilation and tests were not run.

## Confirmed directly in the supplied source

| Severity | Finding | Direct evidence | Decision |
|---|---|---|---|
| Critical | Helius credential is embedded in root `config.toml` | A full `mainnet.helius-rpc.com/?api-key=...` credential is present in the supplied file | Treat as exposed: revoke/rotate at provider, remove from tracked config/history, inject via secret manager/env, verify logs and artifacts |
| Critical | Webhook example contains a fixed auth token | `.env.example` has a non-placeholder `WEBHOOK_AUTH_TOKEN` value | Rotate/revoke it and replace with an unmistakable placeholder; never reuse it |
| Critical | Local simulation can be a no-op when feature is disabled | `arb-sim/src/litesvm_sim.rs` returns `litesvm feature disabled` under `cfg(not(feature = "litesvm"))`; feature is not enabled by default | A caller must treat Disabled as rejection, never as successful simulation |
| High | Split impact is estimated by dividing old impact by leg count | `supervisor.rs` computes `opp.price_impact_bps / num_legs` | Remove heuristic; calculate aggregate impact from re-quoted split execution, or use a conservative maximum until that quote exists |
| High | Split builder copies first-hop mints and fee to every leg | Split loop constructs each `SwapHop` using `first_hop.mint_in`, `first_hop.mint_out`, and `first_hop.fee_bps` | Each leg must use metadata read from that exact pool and its exact protocol decoder |
| High | Split return leg uses estimated summed output and a simple haircut | `supervisor.rs` sums `leg.expected_out`, then derives return `min_out` by applying the same slippage setting | Re-quote the return hop using the aggregate intermediate amount and real pool state; calculate protocol-specific min-out and costs |
| High | CLMM ceiling division uses saturating increment | `clmm.rs::div_ceil_u64` uses `quot.saturating_add(1)` before conversion | Use checked addition; overflow must return a typed error |
| High | CLMM A-to-B tick search permits current tick | `find_next_tick_boundary` tests `tick.tick_index <= tick_current` | Exclude current tick for strict progress, and test no-progress/loop bounds against protocol semantics |
| High | DLMM fixed-point exponentiation silently substitutes zero/max on conversion failures | `dlmm.rs::pow_q64` contains `unwrap_or(0)` and `unwrap_or(u128::MAX)` in arithmetic path | Propagate `MathOverflow`; do not manufacture a fallback price |
| High | `min_profit_raw` unit contract is internally inconsistent | `build_economic_quote` doc says inputs are base-mint raw units; parameter is `TokenAmount`; inline comment treats it as lamports and converts it | Resolve the public contract and every caller first. Rename to `min_profit_lamports` or keep `min_profit_base_raw`; never infer units from the wrapper type |
| Medium | CPMM marginal guard is unreachable | `clamped = min(candidate_size, optimal)` then tests `clamped > optimal` | Simplify to the intended clamp semantics and add boundary/property tests |

## Important report corrections

1. The report's dead-code count is inconsistent: the prose says D-01…D-24, while its own table includes D-22…D-28. Use the table's actual IDs and recount before tracking closure.
2. The report's logical-error count is not reconciled to its enumerated IDs. Track findings by stable ID and machine-check the count.
3. FIN-01 is not accurately described as a single unresolved constant in the main supervisor path. The supplied source calls `build_verified_inputs`, computes the fee from the verified `flash_loan_fee_sf`, and passes that value to several builders. However, `arb-tx/src/lib.rs` contains a legacy assembly path that calls `assemble_flash_arbitrage` with `fee_sf: 0` when the caller uses the older wrapper. Keep FIN-01 **partially resolved / path-dependent** until every public builder and binary is audited.
4. The proposed one-line M-01 fix (“use min_profit_raw directly”) is not safe until units are decided. Current docs and type imply base-token raw units, while the comment and conversion imply lamports. The fix must correct the API contract and call sites, not just delete a conversion.
5. `arb-math::chronos` being unused is not proof it should be wired into the hot path. Benchmark and validate its score against realized outcomes first; otherwise it can add CPU cost or duplicate existing risk logic.
6. “Unused” does not mean “delete”: `arb-mcp`, test binaries, metrics, and simulator are operational or future surfaces. Classify each as retain, wire, archive, or remove, with ownership and tests.

## Not independently verified

- SEC-01 historical key exposure: the current archive itself contains a live-looking Helius API credential, but the missing Git history prevents determining when/how widely it was exposed.
- LIVE-01/LIVE-02 fabricated execution claims and Gates A–E: source fragments show canary logging and execution controls, but the referenced original finding definitions are absent.
- Exact fee direction/rounding for every DEX: requires protocol-versioned IDL/source and differential fixtures; the report's broad warning is valid, but each pool implementation needs independent proof.
- Actual production performance, landed rate, profitability, and RPC feed latency: cannot be inferred from static source.
