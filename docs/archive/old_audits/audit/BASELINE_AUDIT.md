> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# BASELINE AUDIT & EXECUTION CALL GRAPH (2026-11-17)

## 1. System Overview & Baseline Context
- **Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`
- **Base HEAD SHA**: `5b3df64` (Main branch, with PR #61 and PR #62 merged)
- **Rust Toolchain**: `1.99.1`, Edition `2024` / `2021`
- **Scope**: Single-DEX Production Hardening: Kamino Flash Loan (`klend`) + Meteora DLMM (`lb_clmm`) 3-leg cyclic arbitrage.

---

## 2. Authoritative Execution Call Graph

The pipeline flows from live account ingestion down to atomic transaction submission:

```
[Yellowstone gRPC / RPC WebSocket]
         │
         ▼
  [arb-ingest: ProviderSupervisor]
         │  (Stream freshness check, account store updates)
         ▼
  [arb-engine: RouteGraph / Discovery]
         │  (Cycle discovery: WSOL -> Token A -> Token B -> WSOL)
         ▼
  [arb-dex: Meteora DLMM Quoter]
         │  (BinArray index math, exact integer bin traversal, dynamic fee decay)
         ▼
  [arb-math: Exact Economics]
         │  (Kamino fee sf:2^60, priority fee ceil rounding, full net PnL check)
         ▼
  [arb-risk: RiskGate]
         │  (Circuit breakers, max borrow liquidity check against reserve vault)
         ▼
  [arb-tx: Flash Arbitrage Assembler]
         │  (Assembles atomic V0/V1 transaction envelope)
         │  ├─ ComputeBudget (CU limit + CU price)
         │  ├─ Ephemeral ATAs CreateIdempotent (WSOL, Meme A, Meme B)
         │  ├─ Kamino FlashBorrow (Top-level instruction at index 5)
         │  ├─ ArbExecutor::execute_three_leg (Genuine CPI calls for Hop 1, 2, 3)
         │  ├─ Kamino FlashRepay (Top-level instruction referencing borrow index 5)
         │  └─ Jito Tip (Optional, tail placed)
         ▼
  [arb-sim: Bankrun / RPC Simulator]
         │  (Local simulation, dimensional accounting & balance validation)
         ▼
  [arb-land: Jito Searcher Client / RPC Sender]
         │  (Atomic bundle submission to Jito Block Engine)
         ▼
  [arb-bot: Supervisor Recovery]
         │  (Post-dispatch intent confirmation, signature status tracking)
```

---

## 3. Audited Invariants & Defect Resolution Status

| Defect / Invariant | Location | Status | Resolution Proof |
|---|---|---|---|
| **Kamino Sysvar Introspection** | `crates/arb-tx/src/flash/arb_executor.rs` | **VERIFIED** | Borrow & Repay instructions remain top-level at index 5; Kamino rejects CPI borrow/repay with `InvalidFlashRepay`. |
| **ArbExecutor Instruction Vector** | `crates/arb-tx/src/flash/arb_executor.rs` | **VERIFIED** | Added `assemble_flash_funded_three_pool_cpi_instructions` exposing raw `CompiledInstruction` vector and indices. |
| **Negative BinArray Index Math** | `crates/arb-dex/src/dlmm_validation.rs` | **VERIFIED** | BinArray index handles negative boundary bin IDs correctly via floor division. |
| **Zero Floating-Point Invariant** | `crates/arb-math/` & `crates/arb-engine/` | **VERIFIED** | All financial, quoting, fee, and sizing logic strictly in `u64`/`u128`/`U256` fixed-point. |
| **Priority Fee Rounding** | `crates/arb-math/src/economic.rs` | **VERIFIED** | Uses ceil division `((cu_limit * cu_price) + 999_999) / 1_000_000` to prevent validator drop. |
| **Intermediate Balance Subsidization** | `programs/arb-executor/src/instructions/execute_three_leg.rs` | **VERIFIED** | Pre-trade balances snapshot; post-trade intermediate balances asserted $\le$ initial balances; WSOL delta covers borrow + fee + min profit. |
| **Differential Parity with Meteora SDK** | `crates/arb-engine/tests/meteora_sdk_differential_tests.rs` | **VERIFIED** | 5/5 differential tests match official Meteora SDK. |
| **Workspace Test Verification** | Entire workspace | **VERIFIED** | 1,137+ unit, integration, and simulation tests pass with 0 failures. |
