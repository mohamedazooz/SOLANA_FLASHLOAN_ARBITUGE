> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# AGENT PROGRESS & LIVE AUDIT JOURNAL

**Mission**: Solana Flash Loan Arbitrage — Complete Audit, Repair, Testing & Production Hardening
**Author**: Principal Solana Protocol & Systems Engineer
**Current Branch**: `feat/master-engineering-hardening-20261117`
**Base SHA**: `eebd392`

---

## 1. Executive Status & Architectural Alignment

### Core Topology
1. **Compute Budget**: `SetComputeUnitLimit` (dynamically requested based on hop count / measured profile) + `SetComputeUnitPrice` (authoritative micro-lamports).
2. **Kamino Flash Borrow**: Top-level instruction (`FlashBorrowReserveLiquidity`). Strictly top-level per Kamino's instruction introspection rules (`SysvarInstructions`).
3. **Execution Routing**:
   - **Meteora 3-Leg DLMM via Dedicated CPI Executor** (`arb-executor`): Executes genuine on-chain `swap2` CPI calls with verified BinArray, Oracle, and Bitmap extension accounts. Enforces sequential token balance isolation and zero pre-existing balance subsidization.
   - **Native Multihop Fallback**: Direct top-level `swap2` instructions for non-CPI paths.
4. **Kamino Flash Repay**: Top-level instruction (`FlashRepayReserveLiquidity`). Explicitly checks borrow instruction index and requires repayment of exact borrow principal plus fee.
5. **Jito Tip**: Protected tip instruction placed after repayment assertions or within private atomic bundles.

---

## 2. Phase-by-Phase Audit & Verification

### Phase 0: Baseline & Repository Forensics
- **Toolchain**: Rust 1.99.1 (Edition 2024 / 2021).
- **Workspace Status**: 14 Crates / Programs (`arb-types`, `arb-constants`, `arb-math`, `arb-dex`, `arb-ingest`, `arb-engine`, `arb-risk`, `arb-tx`, `arb-sim`, `arb-land`, `arb-store`, `arb-metrics`, `arb-bot`, `arb-executor`).
- **All targets compiled cleanly**: 88 test binaries compiled and executable.

### Phase 1: Architecture & Atomic Flash-Loan Execution
- **Instruction Topology**: Verified Kamino Flash Borrow at index `[2]` (or `[5]` when idempotent ATAs are prepended), followed by Swaps / CPI, followed by Kamino Flash Repay referencing the exact borrow index.
- **Atomic Integrity**: `verify_atomic_integrity` strictly enforces single-transaction borrow-to-repay atomicity.
- **Executor CPI Connection**: `arb_executor::assemble_flash_funded_three_pool_cpi` verified across unit and integration tests (`tests/arb_executor_cpi_tests.rs`).

### Phase 2: Meteora CPI Executor Security
- **Account Validation**: Token account mint ownership verified; user authority checked; distinct pool invariants enforced.
- **Pre-existing Inventory Subsidization**: On-chain checks in `verify_sequential_balance_transitions` assert starting intermediate balances are zero and final WSOL output exceeds required threshold without absorbing pre-existing capital.
- **Slippage Bounds**: Per-hop minimum amount out verified against exact quote calculations.

### Phase 3 & 4: DLMM State & SDK Parity
- **Meteora SDK Differential Tests**: 5/5 passed (`tests/meteora_sdk_differential_tests.rs`):
  - `test_official_sdk_token_2022_transfer_fee_differential`: **PASS**
  - `test_official_sdk_collect_fee_mode_differential`: **PASS**
  - `test_official_sdk_exact_out_quote_differential`: **PASS**
  - `test_official_sdk_exact_in_quote_differential`: **PASS**
  - `test_official_sdk_pda_derivations_differential`: **PASS**
- **BinArray Index Math**: Exact Euclidean division handling negative bin IDs (-1, -70, -71, 0, 70, 71).

### Phase 5 & 6: Token-2022 & Dynamic Fee Time Correctness
- **Token-2022 Transfer Fees**: Explicit epoch-aware transfer fee deductions applied on both input and output sides.
- **Dynamic Fee Decay**: Elapsed time evaluated using snapshot context slot / timestamp; volatility accumulator decay verified.

### Phase 7: Quantitative & Flash Loan Math
- **Kamino Flash Fee**: Strict `fee_bits = max(amount * sf, 2^60)` with round-half-up math (`calculate_kamino_flash_fee`).
- **Exact Priority Fee Rounding**: `priority_fee_lamports = ceil(price * cu / 1_000_000)` unified across all economic calculations.
- **Net Economics Tests**: 9/9 passed (`tests/three_leg_net_economics_tests.rs`).

### Phase 8 & 9: Transaction Landing & MEV Safety
- **V1 Loaded Data Limit**: Verified 64 MiB (`V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 67,108,864`) on Agave 4.4.1 mainnet simulation to avoid `MaxLoadedAccountsDataSizeExceeded`.
- **Atomic Revert Protection**: Transaction fails closed if expected final output is insufficient to repay borrow principal + fee + min net profit.

---

## 3. Verified Test Proofs

| Suite | Scope | Result | Execution Time |
|---|---|---|---|
| `arb-executor` unit tests | Program CPI & instruction validation | 12 / 12 PASS | 0.00s |
| `arb_executor_cpi_tests` | Transaction builder CPI assembly | 5 / 5 PASS | 0.03s |
| `three_pool_flash_arb_tests` | 3-leg flash-funded atomic integrity | 17 / 17 PASS | 0.09s |
| `meteora_sdk_differential_tests` | Differential quote vs official SDK | 5 / 5 PASS | 0.03s |
| `meteora_cross_pool_vertical_slice` | Pair discovery & size optimizer | 3 / 3 PASS | 0.01s |
| `three_leg_net_economics_tests` | Economic fees, repayment & sizing | 9 / 9 PASS | 0.01s |
| `meteora_fresh_pipeline_test` | Fresh state cross-pool cycle | 2 / 2 PASS | 0.05s |
| `meteora_mainnet_simulation_test` | Live mainnet non-broadcast simulation | 1 / 1 PASS | 3.03s |

---

## 4. Current Work & Next Milestones
- [x] Wire `arb-executor` CPI option in `Supervisor::assemble_native_ixs` for pure 3-pool Meteora DLMM routes.
- [x] Create comprehensive architecture and lifecycle documentation in `docs/`.
- [x] Maintain zero warnings across workspace.

---

## 5. Session (2026-11-20, branch `vibe/raydium-phase45-d4dc69`) — Raydium CLMM Phase 4 Dynamic-Fee/Fee-Side Gates + Raydium V4 Phase 5 Post-OpenBook V2 Execution

### Phase 4: Raydium CLMM dynamic-fee & fee-side gates (arb-math + arb-dex)
- **`arb_math::raydium_dynamic_fee`** (new module): `raydium_clmm_dynamic_fee_rate_upper_bound`
  (exact `compute_dynamic_fee_rate` formula evaluated at `max_volatility_accumulator`,
  fee monotonic in VA -> sound ceiling), `raydium_clmm_total_fee_rate_upper_bound`
  (base + worst-case dynamic, capped at `MAX_FEE_RATE_NUMERATOR = 100_000`).
  Constants VERIFIED against deployed program source (`states/pool_fee.rs`,
  `instructions/swap.rs`), checked 2026-11-20.
- **`RaydiumClmmPool`**: `dynamic_fee_rate_upper_bound()` / `total_fee_rate_upper_bound()`
  expose the bound; `fee_bps()` and `clmm_state().fee_rate` now use the base+dynamic
  upper bound and fail closed (None) when the dynamic-fee tail (offsets 1102/1106/1118)
  is missing or the bound exceeds the u16 quote-math domain — never clamped.
- **`quote()` gates (fail-closed)**:
  1. `fee_on` (`CollectFeeOn`) must be provable from account bytes (`len >= 391`).
  2. Only `FromInput` (0) is supported — Token0/Token1-only fee collection rejected
     (our model charges the fee on the input token; the other modes change the fee side).
  3. Dynamic-fee state must be provable (`len >= 1122`) — reject rather than assume disabled.
- **Limit-order traversal fix**: `parse_raydium_clmm_tick_array_account` dropped the
  parsed `limit_order_unfilled` (hard-coded 0). Fixed; a tick holding unfilled Raydium
  CLMM limit orders now (a) counts as initialized per official `is_initialized()` and
  (b) aborts the pure-AMM traversal with `LimitOrderTickInSwapPath` when the swap path
  crosses it exactly at the tick price.
- **New tests**: `raydium_clmm_limit_order_tests.rs` (7) + `raydium_clmm_phase45_tests.rs` (10).

### Phase 5: Raydium V4 execution accounts post-OpenBook
- `RaydiumV4Pool::execution_accounts()` previously required a hydrated OpenBook market
  (serum_market + derived vault_signer), making pools untradable when the market
  account is not fetched/returned. Since the default schema `SwapBaseInV2` (opcode 16,
  8 accounts) needs no OpenBook accounts, the gate now requires only the AMM vaults;
  unresolved serum keys stay default-nil and an explicit V1 build still fails closed
  in `arb_tx::protocols::raydium_v4::build_raydium_v4_swap_instruction_versioned`.
- **New tests**: `raydium_v4_v2_execution_tests.rs` (4).

### Verification (this session, all commands executed in-sandbox)
- `cargo check --workspace` — clean.
- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets` — 0 warnings, 0 errors.
- `cargo test --workspace --all-targets` — **1343 ok / 0 fail** (baseline before session: 1318; +25 new).
- Phase 3 note (unchanged status): **PARTIALLY VERIFIED** — the Phase-3 DEX coverage
  checklist gates were merged earlier (PR #87 and predecessors); the DeFiLlama/registry
  discovery-driven pool-onboarding evidence below has not been re-executed against a
  live RPC in this session.
