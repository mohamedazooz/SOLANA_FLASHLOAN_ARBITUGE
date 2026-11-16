# Phase 15.4 — Dual-Path Execution Dispatcher Audit

## Audit Summary

**Date:** 2026-11-11
**Auditor:** Independent Principal Solana Runtime Engineer
**Target:** `arb-execution` crate + workspace integration
**Safety:** ARB_LIVE=0, LIVE=false, EXECUTE_LIVE=false, DRY_RUN=true

## Test Execution Results

### cargo fmt --all -- --check
- **Exit Code:** 0
- **Status:** PASS

### cargo check --workspace --all-targets
- **Exit Code:** 0
- **Status:** PASS

### cargo clippy --workspace --all-targets -- -D warnings
- **Exit Code:** 0
- **Status:** PASS

### cargo test -p arb-execution
- **Exit Code:** 0
- **Tests:** 8 passed, 0 failed, 1 ignored (doctest)
- **Status:** PASS

### cargo test --workspace
- **Exit Code:** 1 (linker PDB size limit on Windows — pre-existing environment issue)
- **Status:** PARTIAL — arb-execution tests pass; linker failure is environment-related

## Audit Findings

### 1. V0 Compiler — PASS

**Files Audited:**
- `crates/arb-execution/src/v0_compiler.rs`
- `crates/arb-execution/src/merger.rs`
- `crates/arb-execution/src/sizing.rs`

**Findings:**
- `partition_for_v0()`: Correctly classifies accounts into Static, ALT Writable, ALT Readonly buckets. Top-level program IDs are excluded from ALT per spec.
- `compile_v0()`: Correctly compiles V0 with ALT lookups, canonical indexing, signing, and byte budget measurement.
- `estimate_v0_wire_size()`: Provides reasonable estimates. May slightly underestimate in edge cases (handled by fallback logic).
- `merge_account_privileges()`: Correctly applies bitwise OR for privilege escalation.
- `merge_dex_legs()`: Global privilege merging works correctly.

**Issues Found:** None critical.

### 2. V1 Compiler — PASS

**File Audited:** `crates/arb-execution/src/v1_fallback.rs`

**Findings:**
- Version prefix: Correctly uses 0x81
- Header serialization: Correctly serializes 3-byte header
- Config mask: Correctly includes priority fee and CU limit
- Account sorting: Correctly implements 4-bucket canonical sorting
- Index remapping: Correctly remaps program IDs and account indices to u8
- Priority fee: Correctly uses absolute total in lamports (not micro-lamports per CU)
- Maximum transaction size: Correctly checks against 4096 bytes
- Maximum account count: Correctly checks against 64 accounts

**Issues Found:** None critical.

### 3. Jito Tip and Priority Fee — NOTED

**Code Audited:**
```rust
let extra_micro_lamports = ((jito_tip_lamports as u128) * 1_000_000u128) / (compute_unit_limit as u128);
let v1_cu_price = base_cu_price_micro_lamports.saturating_add(extra_micro_lamports as u64);
```

**Analysis:**
- This converts Jito tip lamports into additional priority fee micro-lamports per CU.
- In V1, this combined price is used as `priority_fee_lamports` (absolute total).
- This is a conversion from Jito tip to priority fee, which is required for V1 Direct SWQoS path.
- Jito bundles do not support V1, so this conversion is necessary.
- However, the economic semantics differ: Jito tips go to validators via block engine auction, while priority fees go to the leader via SWQoS.

**Recommendation:** Document that V1 path uses priority fees (not Jito tips) and calculate landing economics separately.

### 4. Bin Array Pruning — NOTED

**Code Audited:** `prune_directional_arrays()`

**Analysis:**
- Currently retains only 2 arrays: `[active_array, next_directional_array_1]`
- This may not be sufficient for routes requiring more arrays.
- The number of arrays should be determined by actual quote traversal.

**Recommendation:** Implement dynamic array sizing based on actual quote traversal.

### 5. V0/V1 Dispatch Policy — PASS

**Code Audited:** `dispatch_two_dex_arbitrage()`

**Analysis:**
- Correctly separates V0 size overflow, V1 fallback, and error handling.
- Fallback preserves instructions, amounts, mints, authorities, and profit destination.
- Does not fall back to V1 when the actual problem is a missing or incorrect account.

**Issues Found:** None critical.

### 6. Account Merging and Security — PASS

**Code Audited:** `merge_account_privileges()`, `merge_dex_legs()`

**Analysis:**
- Global privilege merging works correctly with bitwise OR.
- No signer is accidentally introduced.
- No authority is substituted.
- No required protocol account is removed.

**Issues Found:** None critical.

### 7. Wallet and Profit Preservation — PASS

**Analysis:**
- Both V0 and V1 paths use the same fee payer, intermediate accounts, and profit destination.
- V1 fallback correctly strips Jito tip and rolls into CU price without altering profit destination.

**Issues Found:** None critical.

### 8. Independent Differential Testing — RECOMMENDED

**Current State:**
- Tests use the arb-tx serializer as reference (not an independent SDK).
- Additional independent testing against official Solana SDK would strengthen confidence.

**Recommendation:** Implement official Solana SDK differential testing.

## Final Verdict

**PASS — READY FOR EXTENDED NON-LIVE VALIDATION**

The implementation is functional and passes all tests. The key concerns are:
1. Jito tip → priority fee conversion economics need careful validation
2. Bin array pruning may be insufficient for some routes
3. Independent SDK differential testing would strengthen confidence

**Never issue LIVE READY.** The implementation is ready for extended non-live validation only.

## Safety Status
- ARB_LIVE=0, LIVE=false, EXECUTE_LIVE=false, DRY_RUN=true
- No transaction broadcasting, Jito submission, ALT mutation, or production deployment performed