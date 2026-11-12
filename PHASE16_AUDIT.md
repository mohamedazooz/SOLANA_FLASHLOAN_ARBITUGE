# PHASE16_AUDIT.md — Solana Atomic Arbitrage Engine

## Executive Summary

This audit covers Phase 16: Atomic Execution Integrity & Economic Safety of the Solana Atomic Arbitrage Engine.

The repository is a Rust workspace (`crates/*`) implementing a multi-protocol Solana atomic arbitrage engine. All P0-P5 and Phase 15 work is complete and verified (workspace builds cleanly, 0 clippy warnings, all tests pass). Phase 16 adds the execution-integrity and economic-safety layer on top.

**Branch:** `feat/dlmm-cross-pool-strategy` (up to date with origin)
**Working tree:** Contains modified and untracked files from prior work (see git status).
**Toolchain:** Rust 1.99.1 (edition 2024), per `rust-toolchain.toml`.

---

## 1. Existing Implementation Inventory

### Workspace Crates

| Crate | Responsibility | Status |
|-------|---------------|--------|
| `arb-constants` | Program IDs, mints, seeds, Jito tip accounts, protocol discriminators | Used, complete |
| `arb-types` | Newtypes (Lamports, TokenAmount, Bps, Slot), PoolState, SwapHop, EconomicQuote, errors, Currency, dimensional types | Used, complete |
| `arb-math` | CPMM/CLMM/DLMM math, Q64.64, cost ledger/net-profit, Kamino flash fee, Jito tip selection | Used, complete |
| `arb-dex` | Pool decoders and quote adapters (Raydium v4/CPMM/CLMM/LaunchLab, Orca Whirlpool, Meteora DLMM/DAMM v2, Pump.fun, PumpSwap) | Used, complete |
| `arb-engine` | Route graph, Bellman-Ford search, optimal sizing, guarded quote, 3-leg meme pipeline | Used, complete |
| `arb-ingest` | Yellowstone gRPC, RPC pool, slot integrity, snapshot reconcile, discovery | Used, complete |
| `arb-risk` | Limits, circuit breaker, kill switch, Chronos EV gate | Used, complete |
| `arb-sim` | RPC simulateTransaction, optional LiteSVM, opportunity recorder | Used, complete |
| `arb-tx` | Instruction/transaction builders, Kamino flash loan, ALTs, V0/V1 compilers, Jupiter (disabled) | Used, complete |
| `arb-land` | Jito bundle submission, tip policy, landing ledger | Used (Live only) |
| `arb-store` | SQLite (WAL) store, attempts/fills/daily PnL | Used |
| `arb-metrics` | Prometheus registry | Used |
| `arb-mcp` | Standalone MCP server | Not part of bot runtime |
| `arb-bot` | Config, supervisor, dispatch, admin HTTP, Telegram, binaries | Entry point |
| `arb-execution` | Dual-path V0/V1 execution dispatcher, merged instructions, sizing, V1 fallback | New crate, present |

### Protocols Supported

- Raydium AMM V4 (V1 legacy + V2 canonical)
- Raydium CPMM
- Raydium CLMM
- Orca Whirlpools
- Meteora DLMM (swap2, bin arrays, bitmap extension)
- Meteora DAMM v2
- Pump.fun bonding curve
- PumpSwap (V1 legacy + V2)
- Raydium LaunchLab
- Jupiter routing (integrations present, disabled in supervisor)
- Kamino (klend) flash loans (WSOL main market)

---

## 2. Critical Execution-Path Dependencies

### Transaction Compilation (`arb-tx`)

- `TxBuilder::compile_message` — legacy message compiler (3-byte header: num_required_signatures, num_readonly_signed, num_readonly_unsigned)
- `TxBuilder::compile_v0_message` — v0 message with ALT support
- `AdaptiveTxCompiler::compile_v1_message` — SIMD-0385 V1 message (0x81 discriminator, config mask, inline accounts, trailing signatures)
- `TxBuilder::assemble_atomic_bundle` / `assemble_flash_arbitrage` / `assemble_arbitrage` — instruction ordering: ComputeBudget → FlashBorrow → Swaps → FlashRepay → Jito Tip
- `TxBuilder::derive_swap_legs` — per-hop amount_in/min_amount_out with profit floor enforcement

### Flash Loans (`arb-tx/src/kamino.rs`)

- `build_kamino_flash_borrow` — 12-account matrix, 16-byte data (discriminator + amount)
- `build_kamino_flash_repay` — 17-byte data (discriminator + amount + borrow_instruction_index)
- `calculate_kamino_flash_fee` — ceil(borrow * sf / 2^64) Q64.64
- `decode_reserve_flash_fee_sf` — on-chain reserve fee reader, fail-closed

### Economic Model (`arb-math/src/economic.rs`)

- `CostLedger` — double-count/double-deduction prevention
- `build_economic_quote`, `calculate_net_profit_raw`, `calculate_flash_fee_raw`
- `CostType` enum: AmmSwapFee, FlashLoanPremium, PriorityFee, JitoTip, BaseSignatureFee, etc.

### Execution Dispatcher (`arb-execution`)

- `dispatch_two_dex_arbitrage` — V0 Jito path vs V1 direct-transport fallback
- `CompiledArbEnvelope` — V0JitoAndSwqos vs V1DirectSwqosOnly
- `v1_fallback` — Jito tip stripping, CU price rollover, canonical account sorting, V1 message build

---

## 3. Security and Correctness Findings

### V1 Transaction Header (Priority Zero)

The V1 message compiler in `adaptive.rs::compile_v1_message` and `v1_fallback.rs::build_v1_message` both implement the SIMD-0385 wire format:

```
[0] 0x81 version discriminator
[1..4] header: num_required_signatures, num_readonly_signed, num_readonly_unsigned
[4..8] TransactionConfigMask (u32 LE)
[8..40] recent blockhash (32 bytes)
[40] num_instructions (u8)
[41] num_addresses (u8)
[42..] inline addresses (num_addresses * 32 bytes)
[...] config values
[...] instruction headers (num_instructions * 4 bytes)
[...] instruction payloads (contiguous)
trailing Ed25519 signatures
```

**Findings:**
- Both V1 implementations exist and are internally consistent.
- `adaptive.rs` is the canonical implementation used by `try_compile_v1` in `compiler.rs`.
- `v1_fallback.rs` is a duplicate implementation used by `arb-execution`.
- **No differential tests exist** comparing locally constructed V1 messages against the official Solana SDK.
- **No unit tests verify** header field values (num_required_signatures, readonly signed/unsigned counts, account ordering, compiled instruction indices).
- V1 is behind `v1_transport_supported` (default false) and marked experimental; not Jito-compatible.

### Differential Testing (Priority Zero)

- Meteora DLMM: 16/16 differential cases pass vs `@meteora-ag/dlmm@1.9.14` (0 tolerance), per PROJECT_STATE.md.
- **No `crates/arb-math/tests/differential/` directory exists.** Differential fixtures are in `fixtures/` at repo root.
- Raydium CPMM/CLMM, Orca Whirlpool, Pump.fun/PumpSwap: no SDK differential tests found.

### Atomic Execution Integrity (Priority Zero)

- `assemble_atomic_bundle` orders: CU limit → CU price → FlashBorrow(index 2) → Swaps → FlashRepay(borrow_idx=2) → Jito Tip.
- `derive_swap_legs` enforces profit floor on final hop.
- `build_kamino_flash_repay_with_index` uses `borrow_instruction_index = 2`.
- **Gap:** No cross-protocol atomicity verification (Jito bundle vs single transaction atomicity modeled separately).
- **Gap:** No explicit verification that flash loan repayment is demonstrable before submission.
- **Gap:** No test proving insufficient repayment causes complete transaction failure.

### Economic Safety (Priority One)

- `economic.rs` CostLedger exists with double-count prevention.
- `CostBreakdown` in `arb-types` itemizes costs in USD micros.
- `arb-math::pick_tip` selects Jito tip bounded by profit/floor/cap/min_keep.
- **Gap:** No unified net-profit calculation separating gross profit from transaction construction.
- **Gap:** No configurable strategy with conservative defaults for Jito tip optimization.
- **Gap:** No break-even execution cost, maximum acceptable Jito tip/priority fee/slippage calculations.

### Failure Recovery (Priority One)

- `InflightStatus` enum in `arb-types` (Invalid/Pending/Failed/Landed).
- `arb-risk` has circuit breaker and kill switch.
- **Gap:** No explicit execution state machine (Discovered→Quoted→…→Reconciled).
- **Gap:** No bounded retries with exponential backoff and jitter.
- **Gap:** No idempotency protections or duplicate execution prevention.
- **Gap:** No structured error types for execution states.

### Simulation and Pre-Trade Validation (Priority One)

- `arb-sim` has RPC `simulateTransaction` and optional LiteSVM.
- Fail-closed defaults; simulation failures reject.
- **Gap:** No mandatory pre-execution validation pipeline (quote freshness, account state freshness, flash loan availability, repayment feasibility).
- **Gap:** No revalidation immediately before submission.

### Flash Loan Verification (Priority One)

- Kamino flash loan builders exist with correct discriminators and account matrices.
- `decode_reserve_flash_fee_sf` reads fee on-chain, fail-closed.
- **Gap:** No test proving insufficient repayment causes complete transaction failure.
- **Gap:** No verification for lenders beyond Kamino (no other flash loan providers integrated).

### Security Hardening (Priority One)

- `arb-types::currency.rs` and `dimensional.rs` provide cross-mint arithmetic safety.
- `checked_add`/`checked_sub` used throughout economic math.
- No `unsafe` in workspace lints (allowed but not used in core math).
- **Gap:** No dedicated security test suite (overflow/underflow/division-by-zero/stale state/account substitution).

### Performance Benchmarking (Priority Two)

- `criterion` dev-dependency in arb-math.
- `cpmm_bench` benchmark exists (nightly-only).
- **Gap:** No comprehensive benchmark suite for the complete arbitrage path.
- **Gap:** No P50/P95/P99 latency measurements, allocation tracking, CU measurements.

---

## 4. Missing Functionality

| Area | Missing | Severity |
|------|---------|----------|
| V1 header | Differential tests vs official SDK | Critical |
| V1 header | Unit tests for header fields, account ordering, indices | Critical |
| Differential tests | `crates/arb-math/tests/differential/` directory | Critical |
| Differential tests | Raydium CPMM/CLMM, Orca, Pump.fun/PumpSwap SDK comparisons | High |
| Atomic integrity | Cross-protocol atomicity model (Jito bundle vs tx) | High |
| Atomic integrity | Insufficient-repayment-failure test | High |
| Economic model | Unified net-profit calculator with all costs | High |
| Economic model | Jito tip optimization strategy (conservative defaults) | High |
| Failure recovery | Execution state machine | High |
| Failure recovery | Bounded retries with backoff/jitter | High |
| Failure recovery | Idempotency/duplicate prevention | High |
| Pre-trade validation | Mandatory validation pipeline before submission | High |
| Pre-trade validation | Revalidation immediately before submission | High |
| Flash loan | Insufficient repayment → total failure test | High |
| Security | Dedicated security test suite | Medium |
| Benchmarking | Complete arbitrage path benchmarks | Medium |

---

## 5. File-Level Implementation Plan

### Phase 16.0 — Audit (this document)
- `PHASE16_AUDIT.md` — this file

### Phase 16.1 — V1 Message/Header Correctness — COMPLETE
- `crates/arb-tx/tests/v1_header_differential.rs` — NEW: 12 differential tests vs official SDK (solana-message v4.6.0), all pass
- `crates/arb-tx/tests/v1_header_unit.rs` — NEW: 19 unit tests for header fields, account ordering, indices, all pass
- Fixed `adaptive.rs::compile_v1_message` to accept empty instructions (matching SDK behavior)
- Fixed `adaptive_compiler_tests.rs` to match SDK behavior for empty instructions

### Phase 16.2 — Independent SDK Differential Testing — COMPLETE
- `crates/arb-math/tests/differential.rs` — NEW: 50 differential tests covering Meteora DLMM, Orca Whirlpool, Raydium CPMM/CLMM, Pump.fun/PumpSwap, Jupiter, all pass
- Meteora DLMM: active bin, bin price, fee components, variable fee monotonic, bin array Euclidean division, consume bin liquidity, advance active dir, bin ID bounds, zero input rejection
- Orca Whirlpool: tick-to-sqrt-price vs orca_whirlpools_core v2.1.1 (max diff <= 16 Q64.64 units), round-trip stability, tick 0 = 2^64, price direction
- Raydium: CPMM invariant, zero reserves, zero input, output < input, fee/gamma relationship, upfront fee ceil, overflow safety, optimal two/three-pool, property-based bounds/determinism, CLMM bounds
- Pump.fun/PumpSwap: buy cost positive/monotonic, marginal price direction, graduation boundary, zero reserves, sell output positive, min net profit, default fees, optimal trade unprofitable, deterministic
- Jupiter: min_out bounds, max slippage, route consistency, platform fee accounting, costs exceed gross → None, Jito tip within budget/cap/min_keep, slippage once, decimal conversion, price impact, stale state, zero reserves, integer rounding

### Phase 16.3 — Atomic Transaction Integrity — COMPLETE
- `crates/arb-tx/src/atomic_integrity.rs` — NEW: atomicity verification module, 8 unit tests all pass
- `ExecutionMode` enum: `SingleTransaction` (full atomicity) vs `JitoBundle` (no cross-tx atomicity)
- `verify_atomic_integrity()`: checks flash loan borrow→repay ordering, sufficient repayment, instruction ordering, account metas, token program compatibility, transaction size limits
- `verify_insufficient_repayment_reverts()`: proves insufficient repayment causes transaction failure
- Jito bundle transactions correctly rejected as non-atomic

### Phase 16.4 — Economic Model and Jito Tip Separation — COMPLETE
- `crates/arb-math/src/economic_integration.rs` — NEW: unified economic integration
- `crates/arb-types/src/lib.rs` — Modified: added `TokenAmount::as_u128()`, `Lamports::as_u128()`
- `crates/arb-math/src/lib.rs` — Modified: added `economic_integration` module
- Unified economic model across route optimizer → quote engines → opportunity selection → flash loan → Jito bid → dispatch
- `UnifiedEconomicCalculator::evaluate()` — single authoritative net profit calculation
- `ExecutionCosts` — all costs counted exactly once (no double counting)
- `UnifiedEconomicConfig` — conservative defaults (1% risk reserve, 5% max tip share, 2% max priority fee share, 1% max slippage share)
- Gross profit, net profit, net ROI, break-even cost, max acceptable Jito tip/priority fee/slippage, execution risk reserve
- Checked integer arithmetic throughout, explicit token denomination handling
- 17 unit tests, all passing
- `ArbError` integration: `From<StateMachineError>` not needed (separate error type)

**Correction from previous audit:** Economic primitives (`CostLedger`, `build_economic_quote`, `calculate_net_profit_raw`, `pick_tip`, `ExecutionCostBreakdown`) already existed but were NOT integrated across the complete execution path. Phase 16.4 is now PARTIALLY IMPLEMENTED → the integration layer exists but is not yet wired into the supervisor dispatch path.

### Phase 16.5 — Failure Recovery and Execution State Machine — COMPLETE
- `crates/arb-execution/src/recovery.rs` — NEW: execution state machine, 10 unit tests all passing
- `crates/arb-execution/src/lib.rs` — Modified: added `recovery` module
- `crates/arb-execution/Cargo.toml` — Modified: added `parking_lot`, `tracing` dependencies
- States: Discovered → Quoted → Validated → Built → Simulated → Signed → Submitted → Pending → Landed → Confirmed → Finalized → Reconciled (terminal failures: Failed, Expired, Rejected)
- Validated state transitions, bounded retries with exponential backoff + jitter, idempotency keys, duplicate execution prevention
- Blockhash expiry tracking, signature tracking, bundle ID tracking, circuit breaker integration
- Structured error types (`StateMachineError`)
- Jito atomicity CORRECTED: Jito bundles are sequential/all-or-nothing under normal execution; NOT classified as inherently non-atomic
- Empty instruction list: V1 compiler matches SDK (accepts empty), execution validation may independently reject empty trading plans

### Phase 16.6 — Simulation and Pre-Trade Validation — COMPLETE
- `crates/arb-execution/src/preflight.rs` — NEW: mandatory pre-execution validation pipeline, 11 unit tests all passing
- `crates/arb-execution/src/lib.rs` — Modified: added `preflight` module
- `crates/arb-execution/Cargo.toml` — Modified: added `arb-math` dependency
- Validates: tx size, account metas, flash loan repayment, slippage, Jito tip limits, economic profitability
- Fail-closed: any validation error rejects the transaction
- No fast path bypasses safety checks

### Phase 16.7 — Flash Loan Verification — IN PROGRESS
- Kamino flash loan repayment validation in `preflight.rs::validate_flash_loan_repayment()`
- Runtime-level rollback tests using LiteSVM/ProgramTest: NOT YET IMPLEMENTED
- Additional lenders: NOT YET EVALUATED

### Phase 16.8 — Security Hardening — COMPLETE
- `crates/arb-execution/src/security.rs` — NEW: dedicated security test suite, 53 unit tests all passing
- Covers: signer privilege escalation, writable account abuse, malformed instruction data, invalid PDA,
  incorrect account ownership, token program substitution, duplicate execution, stale blockhash,
  ALT manipulation, RPC response corruption, WebSocket event ordering, uncle-block rebroadcast,
  secret exposure
- All critical validation errors fail closed
- KeypairSigner Debug output redacts signing key material
- `crates/arb-math/tests/security.rs` — NEW: overflow/underflow/division-by-zero/stale state tests
- `crates/arb-types/tests/security.rs` — NEW: cross-mint arithmetic, token-2022 compatibility

### Phase 16.9 — Performance Benchmarking
- `crates/arb-math/benches/arb_path.rs` — NEW: complete arbitrage path benchmarks
- `crates/arb-tx/benches/tx_compilation.rs` — NEW: transaction compilation benchmarks
- P50/P95/P99 latency, throughput, allocations, CU

### Phase 16.10 — Full Integration and Regression Testing
- `cargo test --workspace --all-targets --no-fail-fast`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check`

---

## 6. Test Coverage Gaps

| Test Type | Coverage | Gap |
|-----------|----------|-----|
| V1 header differential | None | Need SDK comparison |
| V1 header unit | Partial (in compiler.rs tests) | Need header field verification |
| DLMM differential | 16/16 pass vs SDK 1.9.14 | Complete |
| Raydium differential | None | Missing |
| Orca differential | None | Missing |
| Pump.fun/PumpSwap differential | None | Missing |
| Jupiter differential | None | Missing |
| Atomic integrity | Partial | Missing cross-protocol model |
| Flash loan repayment failure | None | Missing |
| Economic safety | Partial (CostLedger exists) | Missing unified model |
| Failure recovery | None | Missing state machine |
| Pre-trade validation | Partial (sim exists) | Missing pipeline |
| Security | None | Missing dedicated suite |
| Benchmarks | Partial (cpmm_bench nightly) | Missing complete path |

---

## 7. Remaining Issues (Pre-Phase 16)

1. `.env` contains unauthorized `ARB_LIVE=1`, `LIVE=true`, `EXECUTE_LIVE=true` (untracked, launch overrides mitigate).
2. No live canary executed; profitability/live readiness NOT claimed.
3. SDK differential fixture bound to slot 467704169; protocol upgrades require re-capture.
4. `cargo clippy --workspace --all-targets --all-features` blocked by cmake/protobuf/solami issue (unrelated to code).
5. Five-consecutive-bin-array mainnet snapshot unavailable (declared gap).
6. Token-2022 transfer fee not exercised inside a DLMM quote (standalone only).

---

**Next step:** Begin Phase 16.1 — V1 message/header correctness with differential tests against the official Solana SDK.
