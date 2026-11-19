# Final Engineering Verification — Merged Main (post PR #7 + PR #8)

**Baseline commit:** `c4fe5fa` (merge of PR #8; PR #7 merged immediately before)
**Verification date:** 2026-11-13
**Scope:** Full-stack validation of the three-pool Kamino/Meteora atomic arbitrage
pipeline against the master engineering directive, on the merged main branch.

---

## Campaign results (executed on this baseline)

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS |
| Compile | `cargo check --workspace --all-targets` | PASS (clean) |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS (clean) |
| Tests | `cargo test --workspace --all-targets` | **954 passed / 0 failed** |
| Security | `cargo audit` (858 locked crates) | **0 vulnerabilities** (5 allowed unmaintained warnings) |
| Release build | `cargo build --release --workspace` | PASS (2m39s, `release [optimized]`) |
| Release benchmark | `target/release/latency-bench` (real mainnet bytes + live blockhash) | Raydium decode 4 ns; Orca decode 2 ns; `optimal_two_pool` 257 ns; Ed25519 sign+wire 13.8 µs |

Dev-vs-release comparison (same machine, same bench): decode 114→4 ns (~28x),
optimizer math 7,295→257 ns (~28x), signing 117.5→13.8 µs (~8.5x). Local hot path
< 20 µs total in release; critical path is network (ingest ~50 ms, Jito ~1 s).

---

## A. Repository audit

Existing architecture (verified, not assumed):

- 13-crate workspace: `arb-types`, `arb-constants`, `arb-math`, `arb-tx`,
  `arb-dex`, `arb-ingest`, `arb-engine`, `arb-risk`, `arb-sim`, `arb-store`,
  `arb-metrics`, `arb-land`, `arb-bot`.
- One canonical execution pipeline: ingest → fresh-state validation →
  opportunity discovery → three-leg quoting → size optimization → RiskGate →
  flash compiler → simulation → durable dispatch intent → transport →
  landing reconciliation → accounting.
- Full audit set: `docs/audit/` (execution flow map, dependency/call graph,
  financial logic, transaction format, security, remediation plan) and
  `docs/quality/` (discovery, test evidence, production readiness, security).

## B. Implementation report

- Three-pool atomic builder: `crates/arb-tx/src/flash/dlmm_3leg_flash_arb.rs`
  (`assemble_flash_funded_three_pool`, `build_flash_funded_three_pool_transaction`),
  connected to the live path via `crates/arb-bot/src/supervisor/run.rs`
  (`capture_three_leg_dlmm` → `arb_engine::build_three_leg_meme_pipeline`).
- Kamino flash loan: `crates/arb-tx/src/flash/kamino.rs` — fee read from the
  deployed reserve (`ReserveConfig.fees.flash_loan_fee_sf`, U68F60, account
  offset 4896), reproducing `ReserveFees::calculate_fees` exactly; borrow
  instruction index 5; `flash_borrow_check_matching_repay` enforced (repay
  principal must equal borrow principal).
- Transaction formats: `crates/arb-tx/src/transport/adaptive.rs` — Legacy/V0/V1
  selection layer with SIMD-0160 (64 instr), SIMD-0296 (4 KiB), SIMD-0385
  (64 addresses, discriminator 0x81); V1 fallback when V0 oversized
  (`test_v1_fallback_when_v0_oversized`).
- Financial engine: `crates/arb-math` — checked integer arithmetic throughout
  (`checked_add/checked_sub` chains for base fee + priority fee + tip + rent).
- Token-2022: supported in DLMM token-program resolution and Kamino supply
  vault decoding (`supply_vault_liquidity_accepts_token_2022_base_layout`).

## C. Flash loan report — VERIFIED (unit + differential level)

- Fee source: on-chain reserve field, not a constant or APY proxy.
- `flash_fee_matches_bigint_reference` — fee formula validated against an
  independent big-integer reference implementation.
- Repay amount = borrow principal exactly (program-enforced matching check).
- Reserve decoder rejects wrong discriminator / wrong mint / inactive reserve /
  short account.
- Remaining gap: no live mainnet borrow/repay broadcast (policy-gated; see H).

## D. Three-pool report — VERIFIED (construction + validation level)

- Legs enforce distinct pools and cyclic token continuity
  (`test_ctx_validation_pools_distinct`, intermediate-ATA validation).
- Sequential quoting: each leg's input is the previous leg's actual output.
- BinArray completeness: compile rejects too many bin arrays; bin discovery in
  `dlmm_bin_discovery.rs`.
- Evidence artifacts: `reports/phase15_3/three_pool_same_dex_evidence.json`,
  `flash_loan_atomicity_audit.json`, `sdk_differential_manifest.json`.

## E. Financial report — VERIFIED (unit level)

- Net-profit = final output − principal − flash fee − non-deducted swap fees −
  base fee − priority fee − tip − non-refundable costs (integer, checked).
- Refundable rent separated from non-refundable costs.
- Break-even and minimum-repayable-output derived from the same engine.
- Remaining gap: live price-source freshness thresholds are enforced in code;
  USD conversion verified only against fixture prices in tests.

## F. Transaction V1 report — VERIFIED (offline + probe level)

- V0/V1 selection by measured serialized size and account count
  (`test_compile_result_rejected_v0_oversized`, `..._v1_oversized`).
- `reports/v1_mainnet_probe/` — live mainnet RPC/WSS/V1 capability probe.
- V1 resource configuration explicit; no ComputeBudget carry-over from V0.
- Remaining gap: no V1 transaction broadcast to mainnet (policy-gated).

## G. Testing report

- 954/954 workspace tests pass; `arb-tx` alone: 148 tests (kamino fee parity,
  V0/V1 size boundaries, atomic ordering, ATA safety, ALT partitioning).
- Simulation harness: `arb-sim` (`fork_sim.rs` = real-mainnet non-broadcast
  simulation primitive; litesvm harness; profitability gate).
- Adversarial results: `reports/phase15_3/adversarial_test_results.json`.
- Known limitation: no persistent local Agave fork environment in CI — fork
  simulation is snapshot-based, not a live forked-validator run.

## H. Execution readiness

**IMPLEMENTED AND SIMULATED** (mainnet-state, non-broadcast).

Not claimed: `MAINNET TRANSACTION PREPARED` (requires a fresh signed-tx
inspection artifact against current state) and `LIVE TRANSACTION CONFIRMED`
(requires operator-authorized broadcast + on-chain confirmation).

Policy state: `ARB_LIVE` / `DRY_RUN` enforced fail-closed; live broadcast
requires explicit operator authorization tied to the exact transaction.

## Remaining blockers to LIVE (severity-ranked)

1. **HIGH** — ≥24 h shadow soak with predicted-vs-observed report (R-A).
2. **HIGH** — operator-authorized canary for live V1/SWQoS transport (R-B).
3. **MEDIUM** — fork validation is snapshot-based; a persistent fork validator
   run would strengthen borrow/repay evidence.
4. **LOW** — five unmaintained transitive dependencies (no upstream fixes).
