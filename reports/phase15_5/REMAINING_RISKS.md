# Remaining Risks

## Summary

This document tracks remaining risks for the arb-execution crate.

## Critical Risks

### 1. Jito Tip → Priority Fee Conversion
- **Risk:** Economic semantics change between V0 and V1 paths
- **Impact:** Profit calculations may be incorrect
- **Mitigation:** Calculate landing economics separately for each transport
- **Status:** Documented, not remediated

### 2. Bin Array Pruning
- **Risk:** Fixed 2-array retention may truncate required arrays
- **Impact:** Transaction may fail on-chain
- **Mitigation:** Implement dynamic sizing based on quote traversal
- **Status:** Documented, not remediated

### 3. Independent Differential Testing
- **Risk:** Custom serializer is its own reference
- **Impact:** Serialization bugs may go undetected
- **Mitigation:** Implement official Solana SDK differential testing
- **Status:** Recommended, not implemented

## Medium Risks

### 4. Failure Recovery
- **Risk:** No retry logic for RPC timeouts, stale blockhash, etc.
- **Impact:** Failed transactions not recovered
- **Mitigation:** Implement retry with exponential backoff
- **Status:** Recommended, not implemented

### 5. Idempotency
- **Risk:** Duplicate dispatch attempts not handled
- **Impact:** Double execution possible
- **Mitigation:** Implement idempotency checks
- **Status:** Recommended, not implemented

### 6. Performance
- **Risk:** No actual benchmark measurements
- **Impact:** Performance regressions may go undetected
- **Mitigation:** Implement criterion benchmarks
- **Status:** Recommended, not implemented

## Low Risks

### 7. Documentation
- **Risk:** Some code paths lack documentation
- **Impact:** Maintenance difficulty
- **Mitigation:** Add comprehensive documentation
- **Status:** Ongoing

### 8. Test Coverage
- **Risk:** Some edge cases not tested
- **Impact:** Bugs may go undetected
- **Mitigation:** Add more test cases
- **Status:** Ongoing

## Final Verdict

**PASS — READY FOR EXTENDED NON-LIVE VALIDATION**

All critical safety constraints are enforced. The implementation is functional and passes all tests. Remaining risks are documented for future remediation.

**Never issue LIVE READY.**