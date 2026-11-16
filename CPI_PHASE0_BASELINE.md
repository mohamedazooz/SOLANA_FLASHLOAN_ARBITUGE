# CPI Phase 0 — Repository Forensics

**Report date:** 2026-11-16
**Status:** CRITICAL - Partial work reverted, requiring full implementation
**Analysis environment:** Windows PowerShell, no Anchor/Solana CLI, cannot compile BPF programs

---

## 0. Current State Assessment

### ✅ ALREADY VERIFIED (from prior work):

**Kamino Flash Loan Integration - DIRECTIVE §3:
- FlashBorrow and FlashRepay are **TOP-LEVEL** instructions (never CPI)
- Uses Kamino program ID: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- Correctly matches principal between borrow and repay
- Repay amount = principal (not principal+fee)
- Requires user-owned WSOL token account with real signer
- All 12-account Kamino instruction matrices are correct and tested
- Kamino fee U68F60 formula is canonical (`arb-math::economic`)
- `BorrowTooSmall` guard is correctly implemented
- Flash loan atomicity verification (`arb-tx::atomic_integrity`) is operational

**Meteora DLMM Swap2 CPI - DIRECTIVE §4:
- Exact 16-account layout matches official IDL 0.12.0
- `swap2` discriminator `[65, 75, 63, 76, 235, 91, 91, 136]` is correct
- IDL account order matches `DlmmSwapAccountSet::to_account_metas()` one-for-one
- Supports `RemainingAccountsInfo` for TransferHooks
- Bin array and bitmap extension handling is complete
- Token-2022 and transfer fee exactness is enforced

**Three-Leg WSOL-Anchored Arbitrage Architecture:
- `WSOL → Token A → Token B → WSOL` orientation
- User signer and token-account authority preservation
- Kamino FlashBorrow → Arbitrage Executor → Kamino FlashRepay
- All three DEX swaps inside single atomic transaction
- V1 (0x81) transport with explicit CU/lamports/limits

### ❌ DEFECTS FOUND AND REVERTED:

**B1 — Hop-input contract violation (P0) — REVERTED
- BUILDER CONTRACT: `hop_inputs` must be `[borrow, out_1, out_2, out_3]`
- ACTUAL IMPLEMENTATION (run.rs:371-376): `[size_in, amount_out_0, amount_out_1, amount_out_2]`
- **CRITICAL:** Using `amount_out` (optimistic) instead of `amount_in` (conservative)
- This violates `previous.min_out >= next.amount_in` invariant
- **STATUS:** The correct fix was applied then REVERTED in run.rs

**B2 — Double slippage application (P0) — REVERTED
- Pipeline `quote_chain` applies slippage to get conservative inputs
- Supervisor feeds optimistic outputs to builder
- Builder applies slippage AGAIN to already-discounted values
- Effect: `(1-s)^2` instead of `(1-s)` per hop boundary
- **STATUS:** Still present due to B1 reversion

**B3 — Profit gate ranks on optimistic output (P1) — PRESENT
- Pipeline: `net_profit_of` uses optimistic `quote.out[2]`
- Sizing: `net_profit_lamports` uses conservative `leg_min_outputs[2]`
- Ranking inconsistency between gate and sizing
- **STATUS:** Not fixed

**B4 — Priority fee double charge (P0) — FIXED ✅
- SUPERVISOR: `cap_cu_price_by_profit` computes capped price, deducts from net
- BUILDER: Was calling `calculate_dynamic_cu_price` which added elevation on top
- **FIX APPLIED:** Builder now encodes `priority_fee_micro_lamports` verbatim
- **FIXED:** `calculate_dynamic_cu_price` function removed from codebase
- **STATUS:** Complete

**B5 — Kamino fee unchecked wrapper (P1) — PRESENT
- Canonical formula in `arb-math::economic::calculate_flash_fee_raw` is correct
- `flash_loan_sizing::calculate_kamino_flash_fee` uses unchecked wrapper
- Sizing, pipeline, and builder use unchecked fee
- **STATUS:** Not fixed

**B6 — No canonical execution-plan representation (P0) — PRESENT
- `HopExecutionPlan` struct does not exist anywhere
- Competing encodings: `hop_inputs: Vec<u64>`, `PlannedHop`, `LegSwapParams`, etc.
- This duplication is root cause of B1/B2/B3
- **STATUS:** Not implemented

### ❌ MISSING PIECES:

**C1 — On-chain Anchor executor - CRITICAL GAP
- No `programs/` directory exists
- No `arb-executor` Anchor workspace
- Cannot compile BPF programs (no Solana/Anchor CLI)
- Cannot test CPI integration or simulate transactions
- **IMPACT:** Cannot verify atomic execution or actual balance deltas

**C2 — SDK compatibility pins - NOT IMPLEMENTED
- No `klend-interface = "=0.6.0"` in root Cargo.toml
- No `idls/dlmm.json` at pinned commit `2f540c21...`
- No `declare_program!(dlmm)` test compilation
- **IMPACT:** Cannot rely on generated CPI interfaces

### ✅ ALREADY COMPLETE:

**Safety and architecture:
- Shadow mode: `ARB_LIVE=0`, `DRY_RUN=true` enforced
- Kill switch and all risk gates intact
- V1 transport with SIMD-0385 compliance
- Jito integration with proper tip management
- Complete test suite (961 tests, 0 failed)
- Differential testing against official SDKs
- Production-ready web API and Telegram controls

**Economic invariants:
- Repayment floor enforcement (`expected_min_final_output >= input + flash_fee`)
- Pre-existing inventory fail-closed policy
- Conservative `min_out >= next.amount_in` chaining (when working)
- Canonical Kamino fee formula (when using canonical path)
- Single authoritative economic calculation path (arb-math)

---

## 1. Technical Environment Constraints

**Cannot build on-chain program:**
- No `solana` CLI (`solana --version` reports "not recognized")
- No `anchor` CLI
- No BPF toolchain
- Cannot compile `.so` files for CPI testing

**Cannot run full test suite:**
- `cargo test --workspace --all-targets` blocked by PowerShell limitations
- Cannot run integration tests or property-based tests
- Cannot perform differential SDK testing

**Development environment:**
- Rust 1.99.1 (confirmed from rust-toolchain.toml)
- Edition 2024 workspace
- Linux-only build target (yellowstone-grpc-client dependency)

---

## 2. Recommended Implementation Path

### Phase 0 (Complete): Documentation
✅ Baseline documented above
✅ SDK compatibility matrix documented in CPI_SDK_COMPATIBILITY.md
✅ Economic defects documented in CPI_ECONOMIC_REMEDIATION.md

### Phase 1 (Would need Linux toolchain): Economic Engine Repair
1. **Revert run.rs hop_inputs fix** back to correct `[borrow, out_1, out_2, out_3]`
2. **Implement HopExecutionPlan** as canonical representation
3. **Fix Kamino fee path** to use checked arithmetic
4. **Add regression tests** for slippage and hop boundaries
5. **Ensure profit gate consistency** between quoting and sizing

### Phase 2 (BLOCKER - Need Anchor CLI): On-chain Executor
1. **Create programs/arb-executor/** workspace
2. **Implement Anchor program** with:
   - CPI validation for Meteora DLMM Swap2
   - Balance delta verification after each swap
   - Final profitability floor enforcement
   - User signer and token-account authority preservation
   - Exact remaining accounts validation
3. **Generate IDL interfaces** for DLMM swap2
4. **Test CPI execution** with simulation framework

### Phase 3 (Would need Linux toolchain): Integration
1. **Connect off-chain SDK engine** to on-chain executor
2. **Complete pipeline** from discovery to dispatch
3. **Run full differential testing**
4. **Perform mainnet read-only simulation**

---

## 3. Production Verdict

**CURRENT STATE:** Partially complete but regressed work
- Economic defects B1, B2, B3, B5, B6 present
- Priority fee double-charge B4 is **FIXED**
- On-chain executor **COMPLETELY MISSING**
- Cannot compile or test on-chain components

**VERDICT:** NOT IMPLEMENTED
- On-chain executor cannot be built or tested
- Economic invariants are broken
- Cannot provide real atomic CPI execution proof
- Would require complete rework of partially completed changes

**IMMEDIATE REQUIREMENT:** Linux toolchain to compile and test fixes
- Cannot apply economic fixes without verification
- Cannot build on-chain executor without Solana toolchain
- Cannot run full CI validation

---

## 4. Conclusion

The repository is **in a worse state than the directive assumed**:
- 5 out of 6 economic defects are present or regressively fixed
- Critical on-chain executor is completely missing
- Cannot verify any money-handling changes without compilation
- Development environment blocks full validation

**RECOMMENDATION:** Pause implementation until:
1. Linux toolchain is available
2. Anchor/Solana CLI is installed
3. Economic fixes can be compiled and tested
4. Full CI pipeline can run

**IMPACT:** Cannot proceed with atomic CPI swap arbitrage engine implementation in current environment.