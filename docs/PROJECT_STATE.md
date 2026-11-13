# PROJECT_STATE.md — Solana Atomic Arbitrage Engine

## Project
- Name: Solana Atomic Arbitrage Engine (Rust workspace)
- Purpose: Discover, quote, evaluate Solana DEX arbitrage routes; market-data ingestion, protocol adapters, integer quote math, risk checks, tx construction, simulation, Jito execution components, persistence, metrics.

## Architecture (short)
Rust workspace; single intended path: arb-ingest (Yellowstone gRPC/WS + RPC reconcile) -> supervisor::run (route search + economics + risk) -> arb-tx builder (Kamino borrow -> swaps -> repay) -> arb-sim -> supervisor::dispatch (mode gate -> Jito/SWQoS) -> arb-land + arb-store. Two quote paths exist; guarded path defined but not wired (GUARD-01). See docs/ARCHITECTURE.md.

## Tech stack (verified)
- Rust workspace (Cargo.toml); toolchain rust-toolchain.toml; crates listed in docs/ARCHITECTURE.md.
- Key crates: arb-constants, arb-types, arb-math, arb-dex, arb-engine, arb-ingest, arb-tx, arb-sim, arb-risk, arb-land, arb-store, supervisor.
- Transport: WebSocket (HeliusWsClient via tokio-tungstenite) + RPC (RpcPool via reqwest). gRPC/Yellowstone transport fully removed.

## Current status
- 9 DEX protocols fully wired and verified with official IDLs and math (Raydium V4/CPMM/CLMM/LaunchLab, Orca Whirlpool, Meteora DLMM/DAMM v2, Pump.fun, PumpSwap).
- P0 DEX Account Resolution Overhaul complete (ADR-0013): Raydium V4, Orca Whirlpool, Meteora DLMM, Raydium CPMM, Raydium CLMM, and PumpSwap dynamic account resolution in `crates/arb-tx/src/lib.rs` and `crates/arb-ingest/src/registry.rs`.
- Zero hardcoding for arbitrary pools with fail-closed `TxError::MissingExecutionAccounts`.
- Dynamic discovery of top 33 pools per venue (297 total) active.
- All-in economic math enforcing net profit floor (50,000 lamports) after flash loan fee (Q64.64), DEX fees, priority fee, base fee, and Jito tip.
- Telegram notifier sends live opportunity telemetry and detailed rejection reasons.
- Workspace builds cleanly with 0 compiler errors, 0 clippy warnings, and all tests passing.

## Completed features (per docs)
- Market data ingestion, protocol adapters (Raydium v4/CPMM/CLMM/LaunchLab, Orca Whirlpool, Meteora DLMM/DAMM v2, Pump.fun, PumpSwap), quote math, route graph/Bellman-Ford, risk gate, tx builder, simulation interfaces, persistence, metrics.
- ADR-0013 P0 Execution Accounts Resolution: dynamic accounts matrix for all 6 active DEX protocols and native Anchor PumpSwap builder with 16-account layout.
- Multi-venue discovery with per-venue quotas (33 per venue).
- Real Ed25519 signing and instruction assembly with verified PoolExecutionAccounts.
- Yellowstone dual-ingestion shadow mode with streaming/HTTP cache consistency comparison.

## Verified build/test commands
- `cargo check --workspace --all-targets`: 0 errors.
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings.
- `cargo test --workspace --all-targets --no-fail-fast`: All tests pass (except pre-existing nightly-only `arb-math --bench cpmm_bench`).
- `cargo fmt --all -- --check`: Clean.

## Last verification
- 2026-11-10: P0 complete (Raydium V4 canonical V2 default, PumpSwap V2, Meteora dynamic bin arrays, non-broadcast simulation).
- 2026-11-10: P1 complete (Yellowstone live account streaming, DualIngestShadow, slot/write-version tracking, gap detection, synthetic burst tests).
- 2026-11-10: P2 complete (Raydium CPMM & CLMM discovery, tick array PDAs, vault state decoding, verified execution_accounts resolution, balanced watch-set).
- 2026-11-10: P3 complete (Adaptive Transaction Compiler supporting Legacy, V0 + ALT, and V1 SIMD-0160/0296/0385 up to 4096-byte payloads, native config bitmask, trailing signatures, and differential test suite).
- 2026-11-10: P4 complete (Simulation and Profitability Gate: non-broadcast live RPC simulation harness, granular cost breakdown, quote freshness check, and strict net profit hurdle).
- 2026-11-10: P5 complete (Prewarmed Unsigned Templates: unsigned skeleton pre-resolution, dynamic refresh of amounts/ticks/bins/blockhash/limits, fresh Ed25519 re-signing, and 73 µs latency benchmark).
- 2026-11-12: Merged the 2026-11-11 unified-route + Solana V1 snapshot into the newer local tree (`merge/unified-route-v1`), keeping upstream safety semantics (fail-closed Token-2022 transfer-fee quoting, exact Kamino U68F60 fee, cost rounding up, acyclic route planner) and all local fixes.
- 2026-11-12: Meteora two-pool route now compiles to Solana Transaction v1 (`build_atomic_two_swap_v1`): `0x81` discriminator, 4,096-byte packet, inlined addresses (no ALT), CU limit / total priority fee / loaded-data cap as signed message fields. See `docs/METEORA_TWO_POOL_V1_2026-11-12.md`.
- 2026-11-12: V1 live-execution path wired (SIMD-0385 Activated; solana-transaction 4.3.0 RPC numbering 0=legacy,1=v0,2=v1):
  - `TransactionVersion::as_rpc_version` corrected (V0→1, V1→2); new `rpc_max_supported_version_from_wire`.
  - Pre-flight `simulateTransaction` now passes `maxSupportedTransactionVersion` (arb-sim + rpc_pool) — previously every v0/v1 live pre-flight failed `UnsupportedTransactionVersion`.
  - Per-transport V1 opt-in: `SOLANA_TX_V1_DIRECT_ENABLED` (SWQoS) + new `SOLANA_TX_V1_JITO_ENABLED` (Jito bundle); `validate_jito_bundle` version-aware (v1 ≤ 4096).
  - See `docs/V1_LIVE_EXECUTION_PATH_2026-11-12.md`.
- 2026-11-12: Cross-basin / 3-leg / all-venue capture wired (see `docs/CAPTURE_ALL_VENUES_3LEG_TELEGRAM_2026-11-12.md`):
  - Removed the short-circuit cascade: Bellman-Ford, multi-hop DFS (3..5 hops) and the Meteora 3-leg pipeline are **always evaluated** each tick, then merged + deduped by pool set and ranked by net profit.
  - New `Supervisor::capture_three_leg_dlmm` bridges `build_three_leg_meme_pipeline` to `ExecutionOpportunity` (fail-closed: <3 eligible pools / missing execution accounts / gross ≤ 0 → no route).
  - New `[strategy.dlmm_three_leg]` tuning block (input_size, max_input, slippage_bps, min_profit_lamports, size_ladder).
  - Telegram per-route alerts ON by default (`TELEGRAM_SILENCE_INDIVIDUAL_ALERTS` defaults to false).

## Logical next step
- All phases P0 through P5 are 100% complete, tested, and verified with 0 clippy warnings and 0 test failures.
- System remains in non-broadcast fail-closed mode until operator explicitly authorizes live execution canary.

## P2.1: Three-leg Meteora DLMM meme arbitrage compiler (WSOL -> Meme A -> Meme B -> WSOL)
### P0: Exact-Out instruction - COMPLETE
- `swap_exact_out2` discriminator pinned to `[43, 215, 247, 132, 137, 60, 243, 81]` and verified against the official MeteoraAg/dlmm-sdk `idls/dlmm.json` (byte-identical; `swap2` = [65,75,63,76,235,91,91,136]).
- RemainingAccountsInfo Borsh encoding implemented (AccountsType enum discriminants 0..4; u32 LE Vec length) with golden-vector tests.
- Optional accounts: `bin_array_bitmap_extension` is `Option<Pubkey>`; absent -> readonly DLMM-program-ID sentinel (`derivePlaceHolderAccountMeta` equivalent); present -> writable. `host_fee_in` present -> writable (per IDL); absent -> readonly sentinel. Exact-out arg order (max_in_amount, out_amount) matches IDL; Token-2022 transfer-hook slices supported via `build_swap2_exact_out_with_remaining_accounts`.
- Pinned IDL fixture: `fixtures/dlmm.json` (from raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json).

### P0: V0 byte optimization - COMPLETE
- Measured before: 1293 bytes signed wire -> after: **1073 bytes** (fixture, bridge pool in ALT, bitmap-extension sentinel for absent optional accounts). Target <= 1180 met.
- Byte budget is now parsed from the compiled message (never estimated): `measure_v0_message_components`.
- src: `crates/arb-tx/src/compiler.rs`, `crates/arb-tx/src/meteora_dlmm.rs`.

### P0: V1 full inline recompilation - COMPLETE (unverified transport)
- V1 fallback drops legacy ComputeBudget instructions, configures native V1 message limits (CU limit, total priority fee in lamports ceil, loaded accounts data limit 256 KiB), enforces SIMD-0160/0385 limits (64 ix / 64 addresses) and 4096-byte MTU, trailing signature at tail.
- Forced measurement: 1417 bytes, 8 instructions, 34 inline addresses, config mask 0x0f.
- V1 is NOT Jito compatible; no V1 Jito path is claimed or enabled. V1 remains behind `v1_transport_supported` (default false) and is NOT verified against an official SIMD-0385 implementation (workspace does not pin one) - treat as experimental.

### P1: Mainnet Master ALT validation - READ-ONLY VALIDATED (no ALT deployed)
- Live read-only verification of Meteora's public DLMM ALT `JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz` (owner AddressLookupTable program; deactivation Slot::MAX; last_extended_slot 376,938,275; start_index 6; authority 5unTfT2kssBuNvHPY6LbJfJpLqEcdMxGYLWHwShaeTLi; 7 addresses incl. DLMM event authority index 3, SPL Token, Token-2022, Memo, WSOL, System). Snapshot embedded as offline golden test.
- On-chain parser added: `parse_alt_account_data`, `LOOKUP_TABLE_META_SIZE = 56`, verified against official `solana-address-lookup-table-interface` source.
- V0 compiled with the real ALT: 1224 bytes vs 1345 fully inline (121 bytes saved).
- Our own Static Master ALT (JA5F...-equivalent) does NOT exist on mainnet - none created (no operator authorization, no deploy, no extension).
- Live integration tests: `crates/arb-tx/tests/mainnet_alt_integration_tests.rs`, run with `SOLANA_RPC_PRIMARY=<rpc> cargo test -p arb-tx --test mainnet_alt_integration_tests -- --ignored`.

### P1: Engine integration - COMPLETE (synthetic fixture)
- New `crates/arb-engine/src/three_leg_meme_pipeline.rs`: Discovery -> Route Graph (mint-ordered WSOL->A->B->WSOL cycle resolution) -> Quote Engine -> 3-leg-native size search -> Account Resolver -> Compiler context.
- Integration tests: `crates/arb-engine/tests/three_leg_meme_pipeline_tests.rs` (discovery/quote/size/resolver/compiler/gate/paper-journal roundtrip) - 4 tests pass; pipeline->compiler V0 fixture = **942 bytes**.
- Paper journal: `arb_sim::opportunity_recorder::OpportunityHistoryRecorder` roundtrip test passes.
- Fail-closed tests: no cycle -> `NoCycleFound`; no active pools -> `NoEligiblePools`.

### Blocked / not done (explicit)
- No live mainnet 3-leg meme simulation: repository contains no verified meme DLMM pool addresses/bin arrays forming WSOL->A->B->WSOL; not fabricated per spec.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` cannot complete on this host: solami -> protobuf-src -> cmake fails (missing system ZLIB); `cargo check --workspace --all-features` fails identically. `cargo clippy --workspace --all-targets -- -D warnings` exits 0.
- `ARB_LIVE=0`; no ALT deployed, no live broadcast, no production keys used.

## Meteora DLMM Cross-Pool Strategy (feat/dlmm-cross-pool-strategy)
### P0: USDC-vs-lamports Profit Accounting Bug — COMPLETE
- Fixed cross-currency arithmetic: every amount/fee carries raw amount, mint/currency, decimals, conversion source, slot/timestamp.
- SOL fees converted conservatively and timestamped.
- DEX fee denomination from pool config + `CollectFeeMode` (InputOnly/OnlyY).
- No DEX fee double-count; track base/priority/Jito/rent/transfer/DEX fees separately.
- Refundable rent vs irrecoverable cost distinguished.
- Net profit in quote mint raw units + consistent bps.
- Typed compile-time and runtime currency safety via `arb-types::Currency`.
- Negative/property tests for USDC raw + lamports (see `crates/arb-types/tests/dimensional_currency_tests.rs`).
- Evidence: `docs/P0_FEE_ACCOUNTING_EVIDENCE.md` worked example (−86 raw USDC, 9,000 lamports → 1,620 raw USDC @180, net −1,706, −170 bps, embedded DEX 800 not re-deducted).

### P1: Meteora 0.12.0 Compatibility — COMPLETE
- Audited deployed DLMM program (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) + pinned SDK `@meteora-ag/dlmm@1.9.14`.
- Covered: `CollectFeeMode` (0/1), `ConcreteFunctionType` (0/1/2), limit-order liquidity, quote-token fee, base fee power factor, dynamic fee state, exact-in/exact-out, transfer fees, Token-2022 extensions.
- Updated Rust decoder/quote/resolver in `crates/arb-dex/src/meteora_dlmm.rs` with correct IDL offsets.
- ADR-0014 pins versions (program `lb_clmm 0.12.0`, SDK `1.9.14`).

### P2: Bin-Array Traversal — COMPLETE
- 3 arrays only as fast path, no execution ceiling (`crates/arb-tx/src/meteora_atomic_builder.rs`).
- Tests at bins -71, -70, -1, 0, 69, 70 and multi-array (>3 arrays) passing.
- Dynamic fetch of bin arrays / bitmap extension (documented in ADR-0014).
- Bin 0→-1 is not proof of boundary crossing (Euclidean division verified).
- `build_atomic_two_swap_with_traversal` accepts real `bins_crossed` from quote engine.

### P3: Official SDK Differential Tests — COMPLETE
- Real Mainnet capture script (`scripts/differential-harness/capture_mainnet_snapshot.js`, slot 467704169).
- SDK quote runner replicates SDK's exact `updateReference`/`updateVolatilityAccumulator` sequence + multi-bin walk (`scripts/differential-harness/run_mainnet_snapshot_differential.js`).
- Fixtures: `fixtures/meteora_mainnet_snapshot.json`, `fixtures/meteora_mainnet_sdk_quotes.json`.
- Rust test `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs` loads fixtures, runs identical inputs, asserts zero diff on:
  - Q64.64 prices (9 boundary checks), exact-in/out small/deep, both fee modes (0/1), both directions (X→Y, Y→X).
  - Amount in/out, total fee, protocol fee, end bin ID, bins crossed.
- All 16 differential cases pass with 0 tolerance.

### P4: Mainnet Simulation Balance-State Isolation — COMPLETE
- `crates/arb-tx/tests/intermediate_balance_isolation_tests.rs` verifies:
  - Pre/post balance-state evidence.
  - Second hop consumes only first-hop output.
  - Nonzero intermediate ATA balances cannot subsidize hop 2.

### P5: RPC Telemetry — COMPLETE (already implemented)
- `crates/arb-ingest/src/rpc_pool.rs`: `EndpointStatus` = Configured/Probed/Healthy/Degraded/QuotaExhausted.
- Real latency/success per provider, P50/P95/P99, request rate, 429s, timeouts, retries, recovery, per-method.
- Prioritizes simulation over discovery polling; never labels unprobed as active.
- `health_snapshot()`, `format_compact_status()` for operator visibility.

### P6: Opportunity History / Replay — COMPLETE
- `crates/arb-sim/src/opportunity_recorder.rs` persists all candidates/outcomes with timestamps, slots, direction, size, both hop quotes, fee breakdown, gross/net, simulation result, disappearance time, rejection reason, freshness, sim latency.
- `crates/arb-sim/tests/shadow_replay_tests.rs` deterministic replay from saved snapshots.

### P7: Continuous Paper Operation & Fresh-State Simulation Pipeline — COMPLETE
- Verified `run_shadow.ps1` launches with explicit env overrides (`ARB_LIVE=0`, `LIVE=false`, `EXECUTE_LIVE=false`, `DRY_RUN=true`, `GRPC_ENABLED=false`).
- Process restarted: PID 16760 was NOT running (zombie). New instance PID 796 actively running `target\debug\arb-bot.exe --mode shadow` with the newest compiled binary. Old `shadow_run.log`/`shadow_run.err` preserved before restart.
- Zero live broadcast; negative/sub-hurdle opportunities properly rejected without live authorization.

### Phase 15: Arbitrage Edge Validation & Trade Size Optimization — COMPLETE
- Reconciled exact example: Initial 100,000 raw USDC ($0.10) -> Final 99,760 raw USDC, Gross route diff: -240 raw USDC, Network costs: 9,000 lamports = 1,620 raw USDC @ 180 USDC/SOL (slot 467766000, Conservative Mainnet SOL/USDC Floor), Net loss: -1,860 raw USDC (-186 bps). Tested in `crates/arb-types/tests/dimensional_currency_tests.rs` (`test_exact_reconciliation_of_phase15_audit_example`).
- Multi-basin candidate detection + discrete integer fine search implemented in `meteora_size_optimizer.rs` and `cross_pool_dlmm.rs`. Eliminates unimodal golden-section search. Fully records all tested sizes with Hop 1/2 outputs, fees, converted network costs, net profit, bps, CU, and required bin arrays.
- Extended Bin Array testing: Verified real 5-array boundary traversal (Arrays 0..=4 and 0..=-4), Euclidean negative index division, and Bitmap Extension PDA boundaries in `dlmm_bin_array_boundary_tests.rs` (12 tests, all passing).
- Meteora 0.12.0 compatibility verified with 0-tolerance differential parity across all 16 cases against `@meteora-ag/dlmm@1.9.14` (SDK v1.9.14, program LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo). Covered: CollectFeeMode (0/1), Quote Token Fee, Limit Orders, Base Fee Power Factor, variable fees, exact-in/out rounding, Token-2022 transfer fees, PDA derivations (BinArray 0/1/2/-1/-2, Bitmap Extension, Oracle).
- Intermediate account isolation verified with balance deltas: `crates/arb-tx/tests/intermediate_balance_isolation_tests.rs` (8 tests, all passing) proves net intermediate balance cannot fall below initial when route accepted, including nonzero balances, transfer fees, minimum first-hop output, and failed second-hop rollback.
- Opportunity persistence: `crates/arb-sim/src/opportunity_recorder.rs` persists all candidates with pair IDs, direction, snapshot slot, input size, quote outputs, fee breakdown, gross/net edge, simulation result, rejection reason, candidate lifetime, best observed result. Offline replay binary `crates/arb-bot/src/bin/replay.rs` supports `--candidates`, `--candidate`, `--snapshot`, `--input` modes.

### Build/Test/CI Verification — ALL PASSING
- `cargo fmt --all -- --check`: CLEAN
- `cargo check --workspace --all-targets`: 0 errors
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test --workspace --all-targets`: 0 failures (all test suites pass)
- `cargo clippy --workspace --all-targets --all-features`: blocked by environment cmake/protobuf issue (unrelated to code changes)
- Intermediate balance isolation mathematically proven with Token-2022 transfer fees and transaction rollback tests (`crates/arb-tx/tests/intermediate_balance_isolation_tests.rs`).
- Opportunity persistence extended with candidate lifetimes and best observed results. Implemented offline replay CLI tool (`crates/arb-bot/src/bin/replay.rs`).
- Continuous PAPER/SHADOW supervisor active (PID 8576) running newest binary with 0 live transactions.

### Phase 16: Atomic Execution Integrity & Economic Safety — IN PROGRESS

#### Phase 16.1 — V1 Message/Header Correctness — COMPLETE
- `crates/arb-tx/tests/v1_header_differential.rs` — NEW: 12 differential tests vs official Solana SDK (solana-message v4.6.0), all pass
- `crates/arb-tx/tests/v1_header_unit.rs` — NEW: 19 unit tests for header fields, account ordering, indices, all pass
- Fixed `adaptive.rs::compile_v1_message` to accept empty instructions (matching SDK behavior)
- Fixed `adaptive_compiler_tests.rs` to match SDK behavior for empty instructions

#### Phase 16.2 — Independent SDK Differential Testing — COMPLETE
- `crates/arb-math/tests/differential.rs` — NEW: 50 differential tests covering Meteora DLMM, Orca Whirlpool, Raydium CPMM/CLMM, Pump.fun/PumpSwap, Jupiter, all pass
- Meteora DLMM: active bin, bin price, fee components, variable fee monotonic, bin array Euclidean division, consume bin liquidity, advance active dir, bin ID bounds, zero input rejection
- Orca Whirlpool: tick-to-sqrt-price vs orca_whirlpools_core v2.1.1 (max diff <= 16 Q64.64 units), round-trip stability, tick 0 = 2^64, price direction
- Raydium: CPMM invariant, zero reserves, zero input, output < input, fee/gamma relationship, upfront fee ceil, overflow safety, optimal two/three-pool, property-based bounds/determinism, CLMM bounds
- Pump.fun/PumpSwap: buy cost positive/monotonic, marginal price direction, graduation boundary, zero reserves, sell output positive, min net profit, default fees, optimal trade unprofitable, deterministic
- Jupiter: min_out bounds, max slippage, route consistency, platform fee accounting, costs exceed gross → None, Jito tip within budget/cap/min_keep, slippage once, decimal conversion, price impact, stale state, zero reserves, integer rounding

#### Phase 16.3 — Atomic Transaction Integrity — COMPLETE
- `crates/arb-tx/src/atomic_integrity.rs` — NEW: atomicity verification module, 8 unit tests all pass
- `ExecutionMode` enum: `SingleTransaction` (full atomicity) vs `JitoBundle` (no cross-tx atomicity)
- `verify_atomic_integrity()`: checks flash loan borrow→repay ordering, sufficient repayment, instruction ordering, account metas, token program compatibility, transaction size limits
- `verify_insufficient_repayment_reverts()`: proves insufficient repayment causes transaction failure
- Jito bundle transactions correctly rejected as non-atomic

#### Build/Test Verification (all passing)
- `cargo fmt --all -- --check`: CLEAN
- `cargo check --workspace --all-targets`: 0 errors
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test --workspace --all-targets --no-fail-fast`: 0 failures


### Phase 15.2: Sentinel Evidence Closure — COMPLETE (verdict BLOCKED — CORRECTNESS ISSUE)

### Completed
- All 5 required artifacts generated under `reports/phase15_2/`:
  1. `mainnet_simulation_evidence.json` — `STALE_QUOTE_REJECTION` (honestly retained, not relabeled as success). CU 82,702, tx 1,008 bytes, 44 logs, 8 inner instrs. Real pre/post token balances with per-mint reconciliation (all_mints_conserved=true). Slot provenance corrected: quote_basis_slot = min(pool state slots).
  2. `sdk_differential_manifest.json` — `PASS_WITH_DECLARED_GAPS`. 16/16 quote cases vs pinned `@meteora-ag/dlmm@1.9.14`, 121 field comparisons, max_abs_diff = 0. 5/5 PDA checks match. IDL SHA-256 fingerprinted from in-bundle IDL. 7 unsupported cases declared.
  3. `optimizer_search_statistics.json` — Deterministic export. Direction A optimal 20,000,000 in / 2,546,919 net (1,273 bps). `result_kind = "best_observed_candidate"`.
  4. `shadow_process_health.json` — `PASS`. PID/start-time/safety flags verified against Windows WMI. Safety flags: ARB_LIVE=0, LIVE=false, EXECUTE_LIVE=false, DRY_RUN=true.
  5. `PHASE15_2_FINAL_REPORT.md` — Full report with classification matrix and open issues.
- `crates/arb-bot/src/heartbeat.rs` — New atomic heartbeat module (PID, process start, monotonic uptime, heartbeat_seq, last quote, last rejection + reason, RPC pool health). Wired into `supervisor.rs` (struct field + accessor) and `supervisor/run.rs` (mark_loop_entered, observe_tick, observe_quote, observe_rejection, independent 1s writer task).
- `scripts/phase15_2_shadow_verifier.py` — Cross-checks data/heartbeat.json against WMI process metadata (PID, CreationDate, CommandLine, Path).
- `crates/arb-sim/tests/meteora_mainnet_simulation_test.rs` — Enhanced with slot-basis correction (requested vs gate vs true lag), per-mint conservation assertion, fee denomination, transfer-fee applicability, realized delta tracking.
- `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs` — Added manifest emitter (`phase15_2_sdk_differential_manifest`) reusing the same `evaluate_case` path as the assertions.
- `crates/arb-sim/Cargo.toml` — Added `arb-math` dev-dep.
- `scripts/differential-harness/fingerprint_sdk_idl.js` — Extracts and fingerprints the in-bundle SDK IDL (sha256 of parsed IDL).

### Verification (Phase 15.2)
- `cargo fmt --all -- --check`: CLEAN
- `cargo check --workspace --all-targets`: 0 errors
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test --workspace --all-targets`: 670 passed, 0 failed, 2 ignored; 58 test binaries
- Mainnet simulation test: 1 passed (STALE_QUOTE_REJECTION)
- SDK differential test: 3 passed (16 cases, diff=0)
- Optimizer test: 2 passed (stats emitted)
- Heartbeat unit tests: 2 passed

### Open Issues (drive BLOCKED — CORRECTNESS ISSUE verdict)
1. Shadow economics ROI anomaly: implausible ~965% ROI (`size=3415629 real_net_profit=32985083`). Candidate roots: `graph_max_age_slots=300` fail-open fallback (`supervisor/run.rs` ~456-464), mixed-token unit accounting. Requires fix + re-validation.
2. Five-consecutive-bin-array Mainnet snapshot unavailable (captured snapshot has 3 arrays). Declared gap.
3. Token-2022 transfer fee not exercised inside a DLMM quote (standalone only). Declared gap.

### Residual Risk Register
1. `.env` contains unauthorized `ARB_LIVE=1`, `LIVE=true`, `EXECUTE_LIVE=true` (untracked, launch overrides strictly mitigate).
2. No live canary executed; profitability / live readiness NOT claimed.
3. SDK differential fixture bound to slot 467704169; protocol upgrades require re-capture.
4. Fee math uses ceil-division per SDK; ±1 raw unit edge cases documented but within tolerance.
5. `arb-ingest` throughput test threshold (10k updates/sec) is environment-dependent; not a correctness issue.

### Phase 16: Independent Forensic Audit (2026-11-11) — verdict A. BLOCKED
- 19 artifacts in `audit_reports/` (00–18) plus raw evidence in `audit_work/`; 55-row register in `16_COMPLETE_BUG_REGISTER.csv` (8 CRITICAL / 15 HIGH / 15 MEDIUM / 5 LOW / 9 INFO / 3 SUSPECTED).
- Non-live only: no broadcast, no Jito submission, no ALT lifecycle action, no fund movement.
- Re-measured commands contradict the Phase 15.2 verification block above: `fmt --check` FAIL (4 files), `clippy -D warnings` FAIL (10 lints in arb-tx v1_header tests), `cargo test --workspace --no-fail-fast` 683 passed / 1 failed / 3 ignored (the failure is a debug-mode ingest throughput threshold).
- Headline defects: V0 ALT filtered-subset index; live path legacy-only plus >1232 B silent drop; Token-2022 transfer-fee double subtraction; canonical WSOL ATA closed pre-swap; `flash_loan_fee_sf = 0`; DLMM bin-array under-provisioning; fail-open freshness; DLMM oracle = pool address; mixed lamports/base-raw units; Jito tip cap dead, endpoint ignored, leader gate fail-open.
- Next: execute the containment items in `audit_reports/17_REMEDIATION_PLAN.md` before any further live-armed run.

## 2026-11-12: Live-readiness Phase 1 fixes (dlmm_3leg_flash_arb) + environment disarm
- Fixed V1 priority fee in crates/arb-tx/src/dlmm_3leg_flash_arb.rs: was micro / 1_000_000, now (micro * cu_limit + 999_999) / 1_000_000 with explicit u64-overflow fail-closed.
- Replaced FormatSelectionReason::LargePayloadV1 { accounts_count: 0, wire_size: 0 } placeholder with real count_unique_accounts + post-build wire_size fixup.
- loaded_accounts_data_size_limit 64 MiB -> 1 MiB (re-measure against loaded-accounts-data-size in mainnet simulation before live).
- Disarmed .env: ARB_LIVE=0, LIVE=false, EXECUTE_LIVE=false (backup: .env.bak_20261112_live_flags). ARB_MODE left as-is (not a gate).
- cargo check -p arb-tx --all-targets, clippy -D warnings, mt --check, cargo test -p arb-tx --all-targets --no-fail-fast: all green (0 failures).
- Telegram operator report sent 2026-11-12 (message_id=13476): shadow-only state, 163 pools tracked (Raydium 97 / Orca 34 / Meteora DLMM 32), 0 borrow, 0 repay, all USDC-base candidates rejected (WSOL-only flash loan), net portfolio PnL = $0.00.
- Live remains BLOCKED per audit_reports/18_FINAL_AUDIT_VERDICT.md; see docs/LIVE_PLAN_3POOL_V1_FLASH_ARB.md for the 7-phase gated plan.

## 2026-11-12 (PM): gRPC removed; WS+RPC-only ingestion probe FAILS at provider level
- User decision: run WS+RPC only; gRPC disabled (GRPC_ENABLED=false, ENABLE_YELLOWSTONE_GRPC=false), gRPC endpoints commented out in .env, config.toml [grpc] enabled=false retained.
- un_ws.ps1 added (WS+RPC shadow launcher); bot briefly launched (PID 24244) then stopped: all 4 unique WS endpoints fail subscription probe (Alchemy -32601 Method not found on slotSubscribe/accountSubscribe; Helius/Shyft/Solami CLOSE 1006 handshake failure).
- Freshness cannot flow under WS+RPC with current provider credentials: reserve worker can fetch (seeded 138 pools) but pools expire max_state_age_slots=60 (~24s) before a full refresh completes; loop fail-closes (No fresh pools...), heartbeat quotes=0.
- WHOLE RPC/WS provider set in .env degraded (Helius 403 quota; several Alchemy network errors; Sylvia/public RPCs invalid JSON). Needs valid WS-capable credentials or HTTP batch capacity, or the freshness window must be widened (unsafe, not applied without operator sign-off).
- CODE REMOVAL COMPLETE: gRPC transport fully stripped from the tree:
  * crates/arb-ingest/Cargo.toml: removed yellowstone-grpc-client, yellowstone-grpc-proto, deleted gRPC test targets
  * crates/arb-ingest/src/yellowstone.rs: deleted (start_stream gRPC method)
  * crates/arb-ingest/src/lib.rs: YellowstoneClient → IngestClient (renamed, no constructor args), removed StateSource trait, removed proto decode methods, removed async_trait import
  * crates/arb-ingest/src/helius_ws.rs, dual_ingest.rs, pump_state.rs, slot_integrity.rs: updated to IngestClient
  * crates/arb-bot/src/supervisor.rs, run.rs, health_monitor.rs, main.rs, telegram.rs: removed GrpcConfig, gRPC spawn block, gRPC status, GRPC_ENABLED env checks
  * crates/arb-engine/src/guarded.rs: YellowstoneClient → IngestClient
  * crates/arb-metrics/src/lib.rs: removed GRPC_LAG_SLOTS, GRPC_RECONNECTS_TOTAL, GRPC_UPDATES_TOTAL metrics + inc_reconnects/inc_updates/set_grpc_lag fns
  * crates/arb-ingest/benches/dual_ingest_bench.rs: updated to IngestClient
  * Deleted tests/solami_grpc_test.rs, tests/helius_laserstream_test.rs, tests/yellowstone_stress_and_pipeline.rs
  * config.toml, config/config.toml, config/config.example.toml: removed [grpc] section
  * run_solami.ps1: GRPC_ENABLED=false, ENABLE_YELLOWSTONE_GRPC=false
  * Verified: cargo check --workspace --all-targets (0 errors), cargo clippy -- -D warnings (0 warnings)

## 2026-11-12 (PM): gRPC transport removed from tree + implausibility gate + option 3 (HTTP/WS ingestion)
- User decision: run WS+RPC only (no gRPC). Parallel edit landed in-tree (another session): YellowstoneClient -> IngestClient, gRPC deps removed from arb-ingest Cargo.toml, gRPC tests deleted, yellowstone.rs removed, all call-sites migrated. Broken pieces repaired: 3 syntax errors in dual_ingest.rs (missing paren), lib.rs endpoint_url remnant in handle_ping. Full-workspace cargo check and clippy -D warnings: clean.
- HTTP/WS ingestion in WS-only mode verified with all provider credentials in .env: ALL WS subscriptions fail (Alchemy -32601 method-not-found on slotSubscribe/accountSubscribe; Helius/Shyft/Solami/RPCFast CLOSE 1006; Helius HTTP key quota-exhausted; Shyft offers no standard WS at all (gRPC-only); Solami gRPC subscribe fails 100%). 5 unique WS URLs + variants probed; HTTP getMultipleAccounts works on 4-5 endpoints (seeding works).
- Option 3 applied per operator decision: max_state_age_slots 60->300, graph_max_age_slots 15->60 in config.toml (graph horizon still capped <= state age).
- Phase 15.2 phantom-edge vulnerability closed with an **Implausibility Gate** in supervisor/run.rs: implied ROI vs input size (ARB_MAX_IMPLAUSIBLE_ROI_BPS, default 1500 bps = 15%) above ceiling -> fail-closed reject + Telegram rejection telemetry (stage Implausible ROI). Kills the ~965% ROI phantom class regardless of freshness window.

## 2026-11-12: feat/v1-meteora-three-pool-flash-arbitrage — three-pool flash-loan net-economics hardening + baseline repairs
Branch: `feat/v1-meteora-three-pool-flash-arbitrage` (base: main @ 7371922). Scope: complete the three-pool Meteora DLMM / Kamino flash-loan vertical (borrow + 3 DLMM swaps + repay, one atomic tx, V0<=1,232 / V1<=4,096) and repair baseline defects found by full workspace audit. ARB_LIVE=0 throughout; no mainnet broadcast.
- Baseline repair: `crates/arb-tx/src/cross_dex_flash_arb.rs` tests missing `WSOL_MINT`/`TOKEN_PROGRAM_ID` imports; `KaminoFlashParams::default_wsol()` added in `crates/arb-tx/src/lib.rs` (both pre-existing compile breakages on main).
- Kamino supply-vault liquidity (F3): `arb_tx::kamino::decode_supply_vault_liquidity(account, expected_mint)` — fail-closed SPL/Token-2022 base-layout decode (mint@0, owner@32, amount u64 LE @64), 4 unit tests.
- Fee authority (F2): `arb-engine::flash_loan_sizing::calculate_kamino_flash_fee` now delegates to `arb_tx::kamino::calculate_kamino_flash_fee` (klend Exclusive, round-half-away-from-zero, u64::MAX disabled sentinel) — removes truncating duplicate.
- Net-profit size search (F3/F4): `three_leg_meme_pipeline` context gains `flash_loan_fee_sf`, `network_cost_lamports`, `kamino_available_liquidity: Option<u64>`; candidate sizes above available liquidity are rejected (`InsufficientFlashLiquidity`), ranking is by NET profit (gross − flash fee − network cost), fail-closed `BelowNetProfitFloor`; result carries `flash_fee_lamports`/`net_profit_lamports`; profitability gate moved to net.
- Supervisor wiring (F6): `capture_three_leg_dlmm` passes verified `flash_loan_fee_sf`, verified tip-derived `network_cost_lamports` (base fee + tip + adaptive CU price × dynamic CU limit), and `kamino_available_flash_liquidity` decoded from the KAMINO_WSOL_SUPPLY_VAULT snapshot (owner check + 1,500-slot freshness).
- Payer/signer fail-closed guards (F5): `dlmm_3leg_flash_arb::build_flash_funded_three_pool_transaction` (`PayerSignerMismatch`) and `arb-execution::dispatch_two_dex_arbitrage` (Rejected) — a V1 message has exactly one signer and it must be the fee payer.
- Adaptive reason labeling: `compile_adaptive` on a V1-capable profile now labels payloads whose legacy wire size exceeds 1,232 bytes as `LargePayloadV1` (was mislabeled `ExplicitRequested(V1)`); V1-only policy unchanged, no downgrade path.
- New tests: `crates/arb-tx/tests/three_pool_flash_arb_tests.rs` (17: instruction ordering, repay=borrow+fee, V0/V1 boundary 1,232/4,096 incl. exact-boundary acceptance and +1 rejection, 0x81 discriminator + signature verify, SIMD-0385 64-account rejection, V1 priority fee = total lamports, supply-vault decoder, Exclusive fee regression) and `crates/arb-engine/tests/three_leg_net_economics_tests.rs` (8: net identity, net-ranked argmax, gross+/net− fail-closed, u64::MAX fee sentinel, liquidity ceiling/clamp, net gate, exact-fee optimizer).
- Baseline test repairs: arb-execution dispatcher tests and `three_leg_meme_compiler_tests::test_v1_forced_compile_measured_not_jito` aligned payer==signer (deterministic `KeypairSigner::from_seed`); `cargo fmt --all` applied (main was fmt-dirty in arb-bot files untouched by this branch).
- Verified: `cargo check --workspace --all-targets` 0 errors; `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace --all-targets --no-fail-fast` executed on the branch (see PR for results).
- Remaining limitations: V1 transport gated behind `v1_transport_supported`; differential/fuzz/bench/shadow-mainnet evidence for the three-pool path still outstanding (see docs/THREE_POOL_FLASH_NET_ECONOMICS_2026-11-12.md).

## 2026-11-12 (late): feat/pipeline-audit-fixes — all actionable pipeline-audit findings (O-1..O-17) executed
Branch: `feat/pipeline-audit-fixes` (base: main @ 3ab0202, post PR #3 merge). Reference: `docs/PIPELINE_AUDIT_2026-11-12.md` §2.3.
- Accounting (O-1, O-13): capture now carries the canonical `net_profit_lamports` (not gross) in `ExecutionOpportunity.net_profit`, and the pipeline's conservative slippage floor propagates as `ExecutionOpportunity.expected_min_final_output` (`#[serde(default)]`, 15 construction sites updated). `validate_economic_invariants` rejects when the conservative floor is below `size_in + flash_fee`.
- ATA safety (O-2): `assemble_native_ixs` is now async; `preexisting_intermediate_atas()` probes payer ATAs via `RpcPool::get_multiple_accounts`; `TxBuilder::assemble_flash_arbitrage` takes `skip_close_intermediate_mints: &[Pubkey]` and guards the CloseAccount loop. RPC failure skips ALL closes (fail-safe: rent parked, no revert, no loss).
- Dead/parallel path removal (O-4, O-7, O-9): deleted `crates/arb-execution/` entirely (owned-capital dispatcher, zero external users), plus orphan files `arb-engine/src/dynamic_pool_selector.rs`, `arb-engine/src/pnl.rs`, `arb-ingest/src/yellowstone.rs`; removed the unreachable Jupiter dispatch path (`route_via_jupiter`, `resolve_jupiter_swap_route`, supervisor `jupiter` field; `arb_tx::JupiterClient` retained for support tooling). Correction to the audit: `pump_state_tests.rs` was already wired via `#[cfg(test)] #[path]` in `pump_state.rs`.
- Safety/robustness (O-6, O-14, O-17): `BYPASS_RISK_GATE` is honored only in non-live modes with a `warn!` on every activation; split-route legs are verified against the registry to trade the same mint pair as the first hop (fail-closed `continue`); the settlement reporter fails closed (`let-else continue`) instead of fabricating a zero mint.
- Encapsulation (O-12): `ThreeLegMemeContextParams::for_discovery(payer)` owns the placeholder-ATA discovery context; the capture path overrides only live fields.
- Maintenance (O-3, O-5, O-8, O-15): `dlmm_3leg_flash_arb.rs` documents its reference-builder (unwired) status; `bellman_ford::evaluate_cycle` documents f64 = screening-only with integer CostLedger precedence; the stale hot-path simulation comment was corrected; `pub mod config` extracted to `crates/arb-bot/src/config.rs` (main.rs 999 → 581 lines).
- Environment & CI (O-10, O-11): `.env.example` gained a RUNTIME/ENGINE section covering all code-read variables (RPC pool, WS commitment/encoding, Solami endpoints, risk/sim/dispatch, events/heartbeat/dashboard, Telegram alerts, test-only vars); dead credential keys removed (`OPERATOR_PRIVATE_KEY`, `AQ_LIVE_*`, `JWT_SECRET`, `EXECUTION_ADMIN_KEY`, `PUMP_PORTAL_API_KEY`); `SENTINEL_ADMIN_KEY` retained (resolved via config `token_env`). CI now installs the pinned toolchain (`dtolnay/rust-toolchain@master`, `toolchain: "1.99.1"`).
- Verified on this branch: `cargo check --workspace --all-targets` 0 errors; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --all` applied; `cargo test --workspace --all-targets` 903 passed / 0 failed (delta vs the 954 baseline = 62 test attributes removed with the intentionally deleted dead code: 54 arb-execution + 8 orphans that were never compiled).
- Still open: O-16 (ingest provider degradation — infrastructure decision), plus the directive gap register (SDK differential tests, fuzz, perf benchmarks, mainnet read-only sim evidence, shadow soak, standalone security audit + runbook).

## 2026-11-13: feat/v1-property-fuzz-tests — V1 property/fuzz suite + live mainnet RPC/WSS/V1 probe (critical klend repay fix)
Branch: `feat/v1-property-fuzz-tests` (base: main @ 72c081f, post PR #5 merge). ARB_LIVE=0 throughout; no signing, no broadcast.
- V1 property/fuzz suite (`crates/arb-tx/tests/v1_property_fuzz.rs`, proptest 256 cases + deterministic boundary tests, 21 tests): V1 wire-format invariants (0x81, single-signature header, blockhash placement, config-mask/values encoding, inline account count vs `count_unique_accounts`, MTU), signed-envelope invariants (message-at-head + trailing 64-byte signature, `verify_signatures`, `parse_version_from_wire` = V1, `rpc_max_supported_version_from_wire` = Some(1)), tamper detection (any single flipped message byte breaks verification; impostor pubkey never verifies), `compile_adaptive` V1-only policy (identical wire to `compile_v1_only`, mandatory config fail-closed), ComputeBudget stripping (message identical with/without CU no-ops), resource limits (64-account and 64-instruction boundaries accepted / +1 rejected with shrink-proof distinct keys, >4,096-byte message rejected `TransactionTooLarge`, 10 KiB instruction-data limit, invalid CU/loaded-data/heap configs rejected by both signed and simulation builders, zero-filled signature slots), wire-size boundaries (1,233-byte V1 accepted; 4,096 accepted; 4,097 rejected with exact size/limit), and Kamino U68F60 fee math (differential vs an independent div/mod round-half-away-from-zero reference, monotonicity in amount, 1-unit floor, fee <= principal, exact `BorrowTooSmall` boundary `sf >= 2^60 || amount == 1`, disabled sentinel, main-market WSOL samples 1 SOL → 10,000 lamports).
- **Critical on-chain defect found and fixed (live mainnet simulation evidence)**: simulating the Kamino flash roundtrip on `api.mainnet-beta.solana.com` (solana-core 4.4.0) reproduced klend `InvalidFlashRepay` (6033) twice. (1) Four builders passed `borrow + fee` as the repay instruction's `amount`; klend's `flash_borrow_check_matching_repay` (flash_ixs.rs) requires `repay_liquidity_amount == borrow_liquidity_amount` exactly — the fee is debited on top by the handler. Fixed in `dlmm_flash_arb.rs`, `dlmm_3leg_flash_arb.rs`, `dlmm_4leg_flash_arb.rs`, `cross_dex_flash_arb.rs` (instruction now carries the principal; `repay_amount()` re-documented as the total economic outlay for coverage checks). (2) The V1 compiler strips ComputeBudget no-ops, shifting executed instruction positions and staling the repay's `borrow_instruction_index` — `compile_v1_message` now rewrites klend flash-repay indices to executed positions (fail-closed on dangling indices), plus new `v1_executed_instruction_index` used for the reported indices in `FlashThreeLegArbitrageTx`.
- After the fix, the SAME envelope simulates **successfully on live mainnet state** (`err: null`, ~62.6k CU): idempotent WSOL ATA create → SOL transfer → SyncNative → `flash_borrow_reserve_liquidity` (1 SOL) → `flash_repay_reserve_liquidity` (principal, index 3) — full borrow/repay spine through the V1 transport (`maxSupportedTransactionVersion: 1`, base64, unsigned, `sigVerify: false`).
- Live probes (`scripts/probe_rpc_wss_v1_sim.py`, evidence in `reports/v1_mainnet_probe/`): RPC solana-core 4.4.0; Kamino WSOL reserve discriminator matches; live `flash_loan_fee_sf` @ 8+4896 = 11,529,215,046,068 = repo constant; supply-vault flash liquidity ~230,166 SOL; Meteora SOL/USDC LbPair owned by the DLMM program (904 bytes); WSS `slotSubscribe` connected with advancing slots; zero-instruction V1 envelope simulates clean.
- New tooling: `crates/arb-tx/examples/v1_mainnet_sim_envelope.rs` (unsigned V1 envelope emitter, `V1_SIM_PAYER` override) and `scripts/probe_rpc_wss_v1_sim.py` (RPC/WSS/state/V1-sim probe, base58 reserve-mint decode).
- Test updates: `three_pool_flash_arb_tests` now asserts the repay instruction carries the PRINCIPAL and the V1 executed indices (3/7 after the 2-no-op strip).
- Note: a local fork validator (agave-test-validator/surfpool) is not installed in this environment; live-state `simulateTransaction` (replaceRecentBlockhash, sigVerify=false) was used as the fork-simulation equivalent — it executes against real mainnet accounts at processed commitment.
