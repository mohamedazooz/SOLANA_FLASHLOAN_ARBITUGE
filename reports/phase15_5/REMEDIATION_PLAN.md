# Phase 15.6 — Remediation Plan: Solving All Identified Issues

## Overview

This plan addresses all issues identified in Phase 15.4 and Phase 15.5 audits.

## Priority 1: Critical Issues (Must Fix Before Production)

### 1.1 Jito Tip → Priority Fee Economic Conversion

**Problem:** Converting Jito tips to priority fees changes economic semantics between V0 and V1 paths.

**Current Code (v1_fallback.rs):**
```rust
let extra_micro_lamports = ((jito_tip_lamports as u128) * 1_000_000u128) / (cu_limit as u128);
let v1_cu_price = base_cu_price_micro_lamports.saturating_add(extra_micro_lamports as u64);
```

**Remediation:**
1. Create separate economic models for V0 (Jito) and V1 (SWQoS)
2. V0: Calculate Jito tip economics (auction-based, guaranteed inclusion)
3. V1: Calculate priority fee economics (SWQoS-based, probabilistic inclusion)
4. Add `TransportEconomics` struct to `CompiledArbEnvelope`
5. Never describe converted fees as equivalent

**Files to modify:**
- `crates/arb-execution/src/v1_fallback.rs` — Add economic model
- `crates/arb-execution/src/lib.rs` — Add economics to envelope
- `crates/arb-types/src/lib.rs` — Add `TransportEconomics` type

### 1.2 Dynamic Bin Array Sizing

**Problem:** Fixed 2-array retention may truncate required arrays.

**Current Code (merger.rs):**
```rust
pub fn prune_directional_arrays(...) -> Vec<Pubkey> {
    // Always retains only 2 arrays
    result.push(bin_arrays[0]);
    if bin_arrays.len() > 1 { result.push(bin_arrays[1]); }
    result
}
```

**Remediation:**
1. Replace with dynamic sizing based on quote traversal
2. Pass actual `active_bin_id` from LbPair data
3. Return `Result<Vec<Pubkey>, TxError>` to reject missing arrays
4. Add bitmap extension traversal support

**Files to modify:**
- `crates/arb-execution/src/merger.rs` — Rewrite `prune_directional_arrays`
- `crates/arb-dex/src/meteora_dlmm.rs` — Use actual active bin ID

### 1.3 Independent SDK Differential Testing

**Problem:** Custom serializer is its own reference.

**Remediation:**
1. Add `solana-sdk` as dev-dependency
2. Create differential test suite comparing:
   - Message bytes
   - Serialized transaction bytes
   - Account key lists
   - Signer lists
   - Writable flags
   - Program IDs
   - Instruction account indexes
   - Instruction data
3. Use official Solana SDK as ground truth

**Files to create:**
- `crates/arb-execution/tests/differential_sdk_tests.rs`

## Priority 2: High Issues (Should Fix Before Extended Validation)

### 2.1 Failure Recovery Implementation

**Problem:** No retry logic, idempotency, or failure recovery.

**Remediation:**
1. Implement retry with exponential backoff
2. Add blockhash freshness validation
3. Implement idempotency checks (transaction signature dedup)
4. Add transaction expiration detection
5. Implement safe recovery for all failure modes

**Files to create:**
- `crates/arb-execution/src/recovery.rs`

### 2.2 Performance Benchmarking

**Problem:** No actual benchmark measurements.

**Remediation:**
1. Add criterion benchmarks for all compilation paths
2. Measure p50, p95, p99 latencies
3. Profile memory allocations
4. Test with production-like route complexity

**Files to create:**
- `crates/arb-execution/benches/compiler_bench.rs`

### 2.3 V1 Header Computation

**Problem:** `compute_v1_header()` uses heuristics instead of exact counts.

**Remediation:**
1. Track privilege information alongside sorted accounts
2. Compute exact header counts from actual account metadata
3. Remove heuristic approximations

**Files to modify:**
- `crates/arb-execution/src/v1_fallback.rs` — Rewrite `compute_v1_header`

## Priority 3: Medium Issues (Nice to Have)

### 3.1 Documentation Improvements
- Add comprehensive doc comments to all public functions
- Document economic models and trade-offs

### 3.2 Test Coverage Expansion
- Add edge case tests
- Add adversarial test cases
- Add property-based tests

### 3.3 Code Cleanup
- Remove unused functions
- Fix all clippy warnings

## Implementation Order

```
Phase 1: Critical (1-2 weeks)
├── 1.1 Jito Tip Economics (3 days)
├── 1.2 Dynamic Bin Arrays (3 days)
└── 1.3 SDK Differential Tests (4 days)

Phase 2: High (1-2 weeks)
├── 2.1 Failure Recovery (4 days)
├── 2.2 Performance Benchmarks (3 days)
└── 2.3 V1 Header Computation (2 days)

Phase 3: Medium (1 week)
├── 3.1 Documentation (2 days)
├── 3.2 Test Coverage (2 days)
└── 3.3 Code Cleanup (1 day)
```

## Success Criteria

- [ ] All critical issues resolved
- [ ] Independent SDK differential tests pass
- [ ] Dynamic bin array sizing implemented
- [ ] Failure recovery implemented
- [ ] Performance benchmarks established
- [ ] All clippy warnings resolved
- [ ] Documentation complete

## Safety Constraints

Throughout all phases:
- ARB_LIVE=0
- LIVE=false
- EXECUTE_LIVE=false
- DRY_RUN=true
- No transaction broadcasting
- No production deployment