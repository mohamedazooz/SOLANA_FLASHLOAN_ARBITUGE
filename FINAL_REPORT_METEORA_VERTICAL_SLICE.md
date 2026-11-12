# Final Report — Meteora DLMM Two-Pool Arbitrage Vertical Slice

## 1. Git state
- Branch: `feat/dlmm-cross-pool-strategy`
- HEAD: `738589cef6ff62815623f31cd0a559c04d286c70`
- Base preserved commit: `c47d9f7fc81691da4afc2d28be60b921d497453f` (via `3883760` baseline stash)
- Working tree: modified + untracked vertical-slice files (no reset/clean performed)
- Note: this report supersedes the earlier revision that recorded HEAD
  `0d967282da16fd58ac3bb6ff1c2d504bbb5cf76b`; the tree has advanced one commit and
  the accounting section below has been corrected.

## 2. Changed files
Modified:
- `Cargo.lock`
- `crates/arb-engine/src/lib.rs`
- `crates/arb-ingest/src/registry.rs`
- `crates/arb-math/src/cpmm.rs`
- `crates/arb-math/tests/fuzzing_suite.proptest-regressions`
- `crates/arb-sim/Cargo.toml`
- `crates/arb-sim/src/lib.rs`
- `crates/arb-tx/src/lib.rs`

New:
- `crates/arb-engine/src/meteora_pair_discovery.rs`
- `crates/arb-engine/src/meteora_quote_engine.rs`
- `crates/arb-engine/src/meteora_size_optimizer.rs`
- `crates/arb-engine/tests/meteora_cross_pool_vertical_slice.rs`
- `crates/arb-sim/src/meteora_simulation_harness.rs`
- `crates/arb-sim/tests/meteora_mainnet_simulation_test.rs`
- `crates/arb-tx/src/meteora_atomic_builder.rs`
- `crates/arb-tx/tests/meteora_atomic_builder_tests.rs`

## 3. Completed phases
- P0: preserved working state, audited diffs, fixed failing tests.
- P1: RPC reliability already present (token buckets, circuit breakers, cooldowns); Alchemy primary used.
- P2-P4: Meteora DLMM discovery, canonical pair discovery, exact multi-bin quoting.
- P5: bounded coarse-to-fine trade size optimizer.
- P6: atomic two-swap transaction builder with distinct pools, intermediate isolation, MTU enforcement.
- P7: real mainnet non-broadcast simulation harness.
- P8: net-profit gate via existing profitability engine.
- P9: paper/shadow mode execution (bot running, live disabled).
- P12 (partial): Meteora vertical slice only; other DEX builders untouched by design.

## 4. Build / test results
- `cargo fmt --all -- --check`: CLEAN
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS (0 warnings)
- `cargo test --workspace --all-targets`: PASS (0 failures)
- Targeted:
  - `meteora_cross_pool_vertical_slice`: 3/3 pass
  - `meteora_atomic_builder_tests`: 3/3 pass
  - `meteora_mainnet_simulation_test`: 1/1 pass (real mainnet RPC, non-broadcast)
  - `arb-math` fuzz suite: pass (after gamma==0 fix)

## 5. Meteora discovery metrics (from live simulation test)
- Meteora pools discovered: >= 1 pair found at runtime
- Distinct eligible pool pairs: >= 1
- Candidate routes evaluated: both directions (A and B)
- Simulation: non-broadcast `simulateTransaction` against live mainnet RPC

## 6. Net-profit accounting
- Canonical profitability engine used (`P_net = final - initial - costs`).
- Fees deducted exactly once; SOL costs converted conservatively.
- Minimum net-profit hurdle enforced; zero-profit candidates rejected.

### CORRECTION (this revision)
An earlier revision of this report stated a net result of `-9,086` and `-908 bps` for
the observed route. That was a **cross-currency accounting defect**, not a market
result. The 86-unit quote delta (raw USDC, 6 decimals) was added directly to 9,000
lamports (SOL, 9 decimals) and the sum labelled "lamports"; the bps figure then
divided a lamport total by a USDC denominator.

Correct accounting for the same observation:

| Step | Value |
|---|---|
| Initial quote balance | 100,000 raw USDC |
| Final quote balance | 99,914 raw USDC |
| Gross profit | **−86 raw USDC** |
| Base fee | 5,000 lamports |
| Priority fee | 4,000 lamports |
| Network cost (SOL) | 9,000 lamports |
| SOL/USDC rate (timestamped floor) | 180.00 |
| Network cost (quote) | 9,000 × 180 × 10^6 / 10^9 = **1,620 raw USDC** |
| Net profit | −86 − 1,620 = **−1,706 raw USDC** |
| Net profit basis points | floor(−1,706 × 10,000 / 100,000) = **−170 bps** |
| DEX fee | 400 + 400 raw, **embedded in quote — not re-deducted** |
| Refundable rent | tracked separately, **not deducted** |

The route is **not profitable**. The dominant cost is network fee, not swap fee.

Full derivation, the unit-safety mechanism, and the guarding tests are in
`docs/P0_FEE_ACCOUNTING_EVIDENCE.md`.

## 7. Performance
- Pipeline latency not separately benchmarked in this pass (correctness first).
- Existing RPC P50/P95 not captured in this report.

## 8. RPC health
- Alchemy primary endpoint responsive (getSlot OK).
- Solami endpoints return 429 under load; bot uses cooldown/rotation.
- gRPC/Yellowstone remains disabled (`GRPC_ENABLED=false`).

## 9. Process state
- Trading mode: SHADOW (paper)
- ARB_LIVE=0 enforced in launched process
- Process: PID 30232 (`arb-bot`, running from clean temp dir)
- Start command: `arb-bot --mode shadow` with `ARB_LIVE=0`
- Stop command: `Stop-Process -Id <PID> -Force`
- Health check: process alive + `logs/shadow_run.log` tail

## 10. Residual bugs / blocked capabilities
- Live discovery in the long-running bot has not completed within observation window (RPC 429 / slow discovery). Code-level slice validated via tests.
- No live broadcast performed; no Jito submission.
- Yellowstone integration not enabled (no valid endpoint configured).
- Multi-DEX expansion (Raydium/Orca/PumpSwap cross) deferred per priority.

## 11. Claim labels
- Code changes: VERIFIED
- Unit/integration tests: TESTED
- Mainnet simulation: SIMULATED
- Bot operation: PAPER (SHADOW)
- Live trading: NOT-AUTHORIZED / DISABLED
- Realized profit: NOT-VERIFIED (none claimed)
