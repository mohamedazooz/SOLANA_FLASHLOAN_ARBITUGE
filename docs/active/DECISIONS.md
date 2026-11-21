# Engineering Decisions & Architecture Decision Records (ADR)

This document is the authoritative register of binding engineering decisions, single sources of truth, and architectural trade-offs across the codebase.

---

## 1. Single Source of Truth (SSOT) Architecture

To eliminate conflicting behaviors and duplicate logic, the following subsystems have strictly ONE canonical implementation:

| Domain | Canonical Implementation | Prohibited Patterns |
|---|---|---|
| **Economic Calculation** | `arb-math/src/economic.rs` (re-exported by `arb-tx`) | Parallel profit estimators in engine or bot; floating-point profit checks |
| **Kamino Flash Fee** | `arb-math/src/economic.rs::calculate_kamino_flash_fee` | Ad-hoc fee multiplication without U68F60 fixed-point rounding |
| **Market Data State** | `arb-ingest/src/account_store.rs` & `cache.rs` | Local unvalidated pool state maps; serving stale cached state on feed failure |
| **Transaction Construction** | `arb-tx/src/primitives/builder.rs` & `compiler.rs` | Multiple competing transaction assemblers; legacy unversioned builders |
| **Execution Entry Point** | `crates/arb-bot/src/supervisor/` | Standalone execution scripts bypassing risk gates and simulation |
| **Operator Control** | `crates/arb-bot/src/control_service.rs` | Independent state mutations across Telegram and Web dashboard |

---

## 2. Binding Repository Architectural Invariants

### Invariant 1: Pure Integer & Fixed-Point Arithmetic
All financial calculations, token amounts, exchange rates, and profit assertions MUST use integer arithmetic (`u64`, `u128`, `ruint::Uint<256, 4>`). Floating-point arithmetic (`f32`/`f64`) is strictly forbidden on all authoritative decision and routing paths. Floats are permitted solely in display formatting (e.g. logging to console or Prometheus UI).

### Invariant 2: No Stale-Cache Fallback (Fail-Closed Data Plane)
If an authoritative market data feed fails or disconnects, the system MUST fail closed. The bot rotates through configured providers (`ProviderSupervisor`); if valid, fresh state is unavailable, it stops evaluating affected routes immediately. Never serve stale cached state as fresh.

### Invariant 3: Single-Transaction Flash Loan Atomicity
Flash loans must follow strict intra-transaction instruction ordering:
$$\text{FlashBorrow} \longrightarrow \text{Swap}_1 \longrightarrow \dots \longrightarrow \text{Swap}_N \longrightarrow \text{ProfitGuard} \longrightarrow \text{FlashRepay}$$
Borrow, swaps, and repayment must execute in a single Solana transaction. Splitting a flash loan across multiple transactions or bundles is prohibited.

### Invariant 4: Solana V1 Wire Standard (SIMD-0385, Activated 2026-09-15)
All transactions must adhere to the V1 envelope, verified against the activated
SIMD-0385 specification:
- 0x81 wire discriminator.
- Maximum wire ceiling of 4,096 bytes.
- Up to 64 inline accounts without relying on Address Lookup Tables (ALTs).
- Resource limits live in the message config (TransactionConfigMask), NOT in
  ComputeBudget instructions: priority fee as an absolute total in lamports
  (mask bits [0,1]), compute-unit limit (bit 2), loaded-accounts-data limit
  (bit 3), heap size (bit 4). ComputeBudget instructions are no-ops in V1 and
  are stripped at compile time (klend `borrow_instruction_index` is rewritten
  accordingly). See `crates/arb-tx/src/transport/adaptive.rs`.
- Explicit loaded-accounts data limit in 1..=64 MiB (flash routes request the
  64 MiB maximum; see `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT`).

### Invariant 5: Canonical Kamino Repayment Formula
Repayment is defined as:
$$\text{Repayment} = \text{Principal} + \text{Fee}$$
where the fee is derived using the official Kamino Klend formula:
$$\text{fee\_bits} = \max(\text{amount} \cdot \text{sf}, 2^{60})$$
with ties rounded away from zero. The `BorrowTooSmall` guard compares raw bits pre-rounding.

### Invariant 6: Safety Defaults are Load-Bearing
- `ARB_LIVE=0`
- `DRY_RUN=true`
- Shadow mode by default.
Autonomous agents are prohibited from altering these safety defaults. Live execution requires explicit operator authorization.

---

## 3. Architecture Decision Records (ADRs) Summary

The complete history of formal ADRs is maintained in [`docs/adr/`](../../docs/adr). The active decisions are summarized below:

| ADR ID | Title | Status | Decision Summary |
|---|---|---|---|
| [`ADR-0001`](../../docs/adr/0001-remove-arb-live-gate.md) | Retain ARB_LIVE Safety Gate | **Accepted** | Rejected proposal to remove `ARB_LIVE`; safety gate remains mandatory. |
| [`ADR-0002`](../../docs/adr/0002-kill-switch-persistence.md) | Kill Switch Persistence | **Accepted** | Kill switch is evaluated on every tick, reading from persistent local file and authenticated memory state. |
| [`ADR-0003`](../../docs/adr/0003-protocol-guards-retention.md) | Protocol Guard Retention | **Accepted** | Retain strict fail-closed guards for pool status, bin arrays, and fee rates across all DEX adapters. |
| [`ADR-0004`](../../docs/adr/0004-risk-gate-bypass-config.md) | Disallow Risk Gate Bypass | **Accepted** | Prohibit environment flags from silently bypassing the risk gate or preflight simulation. |
| [`ADR-0005`](../../docs/adr/0005-simulation-policy.md) | Mandatory Preflight Simulation | **Accepted** | Mandatory live RPC preflight simulation for all transaction candidates before dispatch. |
| [`ADR-0006`](../../docs/adr/0006-stream-readiness-guard.md) | Stream Readiness & Slot Guards | **Accepted** | Pipeline halts opportunity generation until live account streams confirm healthy slot progression. |
| [`ADR-0007`](../../docs/adr/0007-enable-pumpswap-adapter.md) | PumpSwap Protocol Adapter | **Accepted** | Enable PumpSwap adapter subject to verified fee config and coherent reserve states. |
| [`ADR-0008`](../../docs/adr/0008-enable-jupiter-live.md) | Prohibit Direct Jupiter Routing | **Accepted** | Prohibit third-party routing APIs on latency-sensitive paths; maintain native internal route engine. |
| [`ADR-0009`](../../docs/adr/0009-split-routing-implementation.md) | Supervisor Module Split | **Accepted** | Split monolithic supervisor into cohesive submodules (`run`, `strategy`, `opportunity`, `dispatch`, `recovery`). |
| [`ADR-0010`](../../docs/adr/0010-guard01-quote-guarded-wiring.md) | Pre-Dispatch State Guard (GUARD-01) | **Accepted** | Revalidate stream generation and pool freshness immediately prior to transaction dispatch. |
| [`ADR-0011`](../../docs/adr/0011-durable-execution-state-machine.md) | Durable Execution State Machine | **Accepted** | Persist dispatch intents and reconcile ambiguous transactions on startup or timeout. |
| [`ADR-0012`](../../docs/adr/0012-fork-simulation-integration.md) | Simulation Engine Policy | **Accepted** | Use Solana JSON-RPC `simulateTransaction` as authoritative preflight; LiteSVM as optional local test accelerator. |
| [`ADR-0013`](../../docs/adr/0013-dex-account-resolution-and-builders.md) | DEX Account Resolution Architecture | **Accepted** | Require complete account dependency resolution before candidate compute scheduling. |
| [`ADR-0014`](../../docs/adr/ADR-0014_METEORA_DLMM_0_12_COMPATIBILITY.md) | Meteora DLMM 0.12+ Compatibility | **Accepted** | Adopt Meteora DLMM `swap2` instruction and account layout compatibility standards. |
