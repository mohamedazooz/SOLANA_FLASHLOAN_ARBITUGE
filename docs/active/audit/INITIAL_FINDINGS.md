# Initial Forensic Audit Findings (`origin/main` @ `b9b516c`)

**Audit Date**: 2026-11-26  
**Baseline Commit**: `b9b516c` (PR #171)  
**Scope**: Complete source audit of `crates/*`, `programs/arb-executor`, `webhook-service/`, `web/`, `.github/workflows/`, `config/*`, and active documentation.

> Note: Several issues from older audit branches (`QUOTE-002`, `DLMM-003`, `DLMM-NEW-003`, `ECON-002`, `ARCH-001`) were already resolved in prior PRs merged onto `origin/main` before `b9b516c`. Only **verified live defects present in `b9b516c`** are registered below.

---

## Summary Table

| ID | Severity | Component | Location | Status at `b9b516c` |
|---|---|---|---|---|
| `PUMPSWAP-01` | **P1** | Ingestion / Execution | `crates/arb-ingest/src/pump_state.rs:251-267, 485-495, 706` | **OPEN** (drops verified Token-2022 program IDs) |
| `SWAP-01` | **P1** | Transaction IX Builder | `crates/arb-tx/src/protocols/swap_hop.rs:561-572` | **OPEN** (dimensionally invalid CLMM `sqrt_price_limit`) |
| `CPI-003` | **P1** | Flash Loan Builders | `cross_dex_flash_arb.rs:172`, `dlmm_flash_arb.rs:234`, `dlmm_4leg_flash_arb.rs:238` | **OPEN** (`saturating_add` on repayment calculation) |
| `CPI-001` | **P1** | Flash Loan Validators | `cross_dex_flash_arb.rs:206,311`, `dlmm_flash_arb.rs:301`, `dlmm_4leg_flash_arb.rs:322` | **OPEN** (missing closing `token_program` equality check) |
| `TOKEN-003` | **P1/P2** | ATA & Token-2022 Guards | `swap_hop.rs:581-591`, `pump_fun.rs:23-40`, `pump_fun_raydium.rs:178-198`, `meteora_accounts.rs:184,281` | **OPEN** (hardcoded `TOKEN_PROGRAM_ID` / unguarded fallback `1` / missing hook guard) |
| `SIZE-001` | **P2** | Sizing Optimizer | `crates/arb-engine/src/three_pool_router.rs:359-362` | **OPEN** (uses `f64` Golden Section Search instead of integer optimizer) |
| `CFN-01` | **P2** | Config & Env Parsing | `crates/arb-bot/src/config.rs:551-552, 657-805` | **OPEN** (silently ignores malformed risk env vars; `from_arb_config` drops DLMM toggles) |
| `TM12` | **P2** | Store & Reconciliation | `crates/arb-bot/src/supervisor/dispatch.rs` (14 sites) | **OPEN** (`record_outcome` used instead of `record_outcome_detail`) |
| `DOC-DRIFT-01` | **P2** | Docs & Root README | `README.md`, `docs/active/CURRENT_STATUS.md`, root/legacy `docs/*` | **OPEN** (stale crate count, false CLMM claim, false LICENSE claim, unarchived legacy docs) |
| `ISSUE-P0-04` | **P0 (Historical)** | Parity / Replay | `docs/active/OPEN_ISSUES.md` | **OPEN** (legacy `0x1773` capture not yet replayed on same snapshot; fails closed) |

---

## Detailed Defect Specifications

### 1. `PUMPSWAP-01` (P1) — PumpSwap Joiner Drops Verified Token Program IDs on Extensionless Token-2022 Mints
- **Exact Source Location**: `crates/arb-ingest/src/pump_state.rs:251-267`, `:440-495`, `:706`.
- **Root Cause**: `join_pump_swap_state` decodes `base_token_program` (`:440`) and `quote_token_program` (`:448`) and explicitly allows extension-free 82-byte Token-2022 mints past lines `:485-495` (`base_mint.data.len() > MINT_BASE_LEN || token2022_has_risky_extension(...)`). However, `PumpSwapJoinedState` does not store `base_token_program` / `quote_token_program`, and pool promotion at line `:706` stamps `token_programs: Default::default()` (`[TokenProgramKind::SplToken; 2]`).
- **Reproduction**: Construct a synthetic PumpSwap `Pool`, vaults owned by `TOKEN_2022_PROGRAM_ID`, and an 82-byte Token-2022 mint; call `join_pump_swap_state` -> promote `PoolState` and inspect `pool.token_programs`.
- **Impact**: Swap ATA derivation and instruction program selection pass `TOKEN_PROGRAM_ID` instead of `TOKEN_2022_PROGRAM_ID`, causing guaranteed on-chain `IncorrectProgramId` / `InvalidAccountData` reverts.
- **Evidence**: Direct read of `crates/arb-ingest/src/pump_state.rs:440-495, 706`.
- **Recommended Correction**: Store `base_token_program: TokenProgramKind` and `quote_token_program: TokenProgramKind` in `PumpSwapJoinedState` and map `[base_token_program, quote_token_program]` directly into `PoolState.token_programs` (and audit PumpSwap V2 remaining accounts against `pump-public-docs`).
- **Required Regression Tests**: Test confirming an 82-byte Token-2022 mint through `PumpSwapJoinedState` propagates `TokenProgramKind::Token2022` into `PoolState.token_programs` (and rejects non-SPL/non-T22 owners).
- **Acceptance Criteria**: Zero hardcoded `Default::default()` on `token_programs` when vault/mint owners are decoded.

### 2. `SWAP-01` (P1) — Dimensionally Broken `sqrt_price_limit` in `RaydiumClmm` Builder Arm
- **Exact Source Location**: `crates/arb-tx/src/protocols/swap_hop.rs:561-572`.
- **Root Cause**: When `amount_in > 0 && min_amount_out > 0`, the `DexProtocol::RaydiumClmm` arm sets `sqrt_price_limit = (min_amount_out * Q64) / amount_in` (for `zero_for_one`) or `(amount_in * Q64) / min_amount_out` (for `!zero_for_one`), computing a raw output/input ratio instead of a Q64.64 **square root** price (matching decimal-scaled `sqrt(P) * 2^64`).
- **Reproduction**: Build any `RaydiumClmm` `SwapHop` with non-zero `amount_in` and `min_amount_out` (especially SOL/USDC decimals 9 vs 6 where raw ratio diverges wildly from `sqrt(price)`) and compare with `MIN_SQRT_PRICE_X64` / `MAX_SQRT_PRICE_X64` (`orca.rs:132-148`).
- **Impact**: On-chain Raydium CLMM `swap_v2` either aborts immediately with `SqrtPriceLimitOverflow` (`6028`) or truncates fill before completing the swap.
- **Evidence**: Direct inspection of `swap_hop.rs:561-572`, `orca.rs:132-148`, and verified `raydium-io/raydium-clmm` `tick_math.rs` / `swap.rs` (`other_amount_threshold` enforces slippage when limit is unbounded).
- **Recommended Correction**: Pass bounded directional extreme `MIN_SQRT_PRICE_X64` / `MAX_SQRT_PRICE_X64` (`4_295_048_016 + 1` / `79_226_673_515_401_499_993_453_864_427 - 1`, matching `raydium_clmm.rs` strict inequality `MIN < limit < current` or `0` unbounded per `raydium-clmm`), letting `other_amount_threshold` (`min_amount_out`) enforce slippage cleanly.
- **Required Regression Tests**: Regression test verifying `DexProtocol::RaydiumClmm` emits directional extreme limits (`> MIN_SQRT_PRICE_X64` for `zero_for_one`, `< MAX_SQRT_PRICE_X64` for `!zero_for_one`) rather than raw `min_amount_out / amount_in` ratios even on 9dp/6dp SOL/USDC swaps.
- **Acceptance Criteria**: Matches `orca.rs` directional limit semantics and Raydium `swap_v2` bounds.

### 3. `CPI-003` (P1) — `saturating_add` in Flash Loan `repay_amount()` Can Silently Mask Overflow
- **Exact Source Location**: `crates/arb-tx/src/flash/cross_dex_flash_arb.rs:172`, `crates/arb-tx/src/flash/dlmm_flash_arb.rs:234`, `crates/arb-tx/src/flash/dlmm_4leg_flash_arb.rs:238`.
- **Root Cause**: Unlike `dlmm_3leg_flash_arb.rs:244` (which returns `Result<u64, FlashArb3LegError>` using `checked_add`), these three flash builders still compute `self.borrow_amount.saturating_add(self.flash_fee)`.
- **Reproduction**: Construct params with `borrow_amount = u64::MAX - 5` and `flash_fee = 10` and call `repay_amount()`.
- **Impact**: Saturates silently at `u64::MAX` instead of returning an explicit arithmetic overflow error before transaction compilation.
- **Evidence**: Direct code inspection of all four flash modules (`Select-String 'fn repay_amount'`).
- **Recommended Correction**: Convert `repay_amount(&self)` in all three files to return `Result<u64, ...>` via `checked_add` and propagate errors through `validate()` and assemblers.
- **Required Regression Tests**: Boundary unit tests asserting `u64` overflow returns `Err(...)` in all three modules.
- **Acceptance Criteria**: Zero `saturating_add` calls on repayment math across `arb-tx`.

### 4. `CPI-001` (P1) — Missing Closing Token-Program Parity Check on Flash Arbitrage Loops
- **Exact Source Location**: `crates/arb-tx/src/flash/cross_dex_flash_arb.rs:206-213, 311-318`, `dlmm_flash_arb.rs:301-306`, `dlmm_4leg_flash_arb.rs:322-328`.
- **Root Cause**: Validation checks `first_mint == last_mint` (`NonClosingRoute`) without verifying that the closing swap leg's output token program (`token_out_program` / `token_programs[1]`) matches the borrowed input leg's token program (`token_in_program` / `token_programs[0]`), unlike `dlmm_3leg_flash_arb.rs:330-338`.
- **Reproduction**: Pass a closing loop with matching WSOL mints where leg 0 enters via `SplToken` and final leg exits via `Token2022`.
- **Impact**: Allows mis-matched SPL vs Token-2022 ATAs in a flash loop to pass pre-build validation.
- **Evidence**: Direct code inspection comparing `cross_dex_flash_arb.rs` / `dlmm_flash_arb.rs` / `dlmm_4leg_flash_arb.rs` against `dlmm_3leg_flash_arb.rs`.
- **Recommended Correction**: Enforce closing `token_program` equality in `CrossDexFlashArbParams::validate`, `validate_cross_dex_opportunity`, `dlmm_flash_arb::validate`, and `dlmm_4leg_flash_arb::validate` (and intermediate token-program continuity across adjacent legs).
- **Required Regression Tests**: Negative tests proving mismatched closing token programs fail validation.
- **Acceptance Criteria**: All 2/3/4-leg and cross-DEX flash validators match `dlmm_3leg_flash_arb` invariants.

### 5. `TOKEN-003` (P1/P2) — Hardcoded SPL ATA Helpers & Missing Guardrails on Legacy Protocol Paths
- **Exact Source Location**: `crates/arb-tx/src/protocols/swap_hop.rs:581-591` & `pump_fun.rs:23-40` (`PumpFunAccounts::for_mint` uses `get_associated_token_address`), `crates/arb-tx/src/protocols/pump_fun_raydium.rs:178-198` (hardcodes `get_associated_token_address` + silent `else { 1 }` slippage floor when `intermediate_amount == 0`), `crates/arb-tx/src/protocols/meteora_accounts.rs:184,281` (hardcodes empty `RemainingAccountsInfo` without explicit transfer-hook check in builder).
- **Root Cause**: Unparameterized `get_associated_token_address` calls ignore `TokenProgramKind::Token2022` and `PumpFunAccounts::for_mint` assumes legacy SPL `TOKEN_PROGRAM_ID`, while `pump_fun_raydium` substitutes `1` raw unit on zero intermediate output.
- **Impact**: Token-2022 Pump.fun `create_v2` tokens or malformed intermediate legs can emit wrong ATA addresses or zero-protection min-out floors.
- **Recommended Correction**: Parameterize `PumpFunAccounts::for_mint_with_program` / fail closed on `Token2022` in `swap_hop.rs` until `Pump.fun` V3/Token-2022 execution parity is proven; parameterize `pump_fun_raydium.rs` and reject zero `intermediate_amount` instead of falling back to `1`; add explicit fail-closed `MintInfo` / hook guard documentation & check in `meteora_accounts.rs`.
- **Required Regression Tests**: Token-2022 rejection test in `swap_hop` for `PumpFun`, zero-intermediate error test in `pump_fun_raydium`.

### 6. `SIZE-001` (P2) — Floating-Point Golden Section Search in `three_pool_router.rs`
- **Exact Source Location**: `crates/arb-engine/src/three_pool_router.rs:359-362`.
- **Root Cause**: `KaminoBorrowOptimizer` uses `f64` bounds (`PHI_INV = 0.618...`, `a = min_borrow as f64`) to search optimal borrow size.
- **Impact**: Violates repository zero-float invariant in sizing decisions and can diverge from integer non-monotonic curve optima on steep CLMM/DLMM pools.
- **Recommended Correction**: Replace with integer-only `crate::route_sizing::optimize_bounded_non_monotonic`.
- **Required Regression Tests**: Determinism and integer boundary tests on `KaminoBorrowOptimizer`.

### 7. `CFN-01` (P2) — Silent Ignore of Malformed Risk Env Overrides in `arb-bot/src/config.rs`
- **Exact Source Location**: `crates/arb-bot/src/config.rs:551-552, 657-805`.
- **Root Cause**: Legacy `Config::load` silently skips malformed values via `if let Ok(value) = raw.trim().parse::<...>()` for `MAX_TRADE_SIZE`/`MAX_TRADE_SIZE_USD`, `MAX_INFLIGHT_BUNDLES`/`MAX_CONCURRENT_TRADES`, `MIN_NET_PROFIT`/`MIN_NET_PROFIT_LAMPORTS`, `MAX_SLIPPAGE_BPS`, `MAX_PRIORITY_FEE_SHARE_BPS`/`MAX_PRIORITY_FEE`, and `MAX_CONSECUTIVE_FAILURES`/`MAX_TRANSACTION_FAILURES` (plus duplicate parse of `MAX_DAILY_LOSS_USD` at line 670 and 753). Also `AppConfig::from_arb_config` (`551-552`) drops `dlmm_cross_pool`/`dlmm_three_leg`.
- **Impact**: An operator typo in an env limit (`MAX_SLIPPAGE_BPS=50_0` or `MAX_INFLIGHT_BUNDLES=abc`) silently falls back to permissive defaults instead of aborting startup fail-closed.
- **Recommended Correction**: Return `Err(...)` explicitly when any set risk override fails parsing or range checks, deduplicate `MAX_DAILY_LOSS_USD`, and map `dlmm_cross_pool` / `dlmm_three_leg` cleanly if present or document explicitly.
- **Required Regression Tests**: Unit tests verifying malformed risk env strings fail startup instead of falling back.

### 8. `TM12` (P2) — `supervisor/dispatch.rs` Drops Failure Reasons via Legacy `record_outcome`
- **Exact Source Location**: `crates/arb-bot/src/supervisor/dispatch.rs` (14 call sites: lines 155, 193, 241, 263, 282, 700, 767, 796, 887, 1163, 1269, 1313, 1400, 1451).
- **Root Cause**: Calls `store.record_outcome(...)` which delegates with `actual_output_raw = None, failure_reason = None`.
- **Impact**: Post-mortem DB queries (`outcomes`) lose exact failure reasons (simulation error string, kill-switch abort reason, submission error, confirmation failure) and actual output amounts when known.
- **Recommended Correction**: Call `store.record_outcome_detail(...)` with concrete failure reasons and actual output values where available.
- **Required Regression Tests**: In-memory SQLite persistence test checking non-null `failure_reason` for simulated/aborted dispatches.

### 9. `DOC-DRIFT-01` (P2) — Root `README.md` Inaccuracies & Active/Archive Hygiene
- **Exact Source Location**: `README.md:1-169`, loose root/legacy files (`AUDIT_BASELINE.md`, `FINAL_PRODUCTION_READINESS_REPORT.md`, `docs/DAMM_V2_FEE_SCHEDULER_AUDIT.md`, `docs/{architecture,audit,performance,quality,research,adr}/`), stale `docs/active/CURRENT_STATUS.md` (stops at PR #168).
- **Root Cause**: Rapid iterative PRs (#169-#171) updated code faster than top-level docs.
- **Recommended Correction**: Refresh active docs in-sync, archive superseded legacy files into `docs/archive/`, and rewrite root `README.md` from verified source-of-truth facts.

### 10. `ISSUE-P0-04` (Historical Blocker) — Unreproduced `0x1773` Same-Snapshot Replay
- **Status**: Still OPEN in `docs/active/OPEN_ISSUES.md`; remains fail-closed until a matching same-slot snapshot capture reproduces or rules out the failure.
