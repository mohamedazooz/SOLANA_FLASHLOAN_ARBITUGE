# Bin Array Validation Report

## Summary

This report audits the dynamic bin array pruning policy in the arb-execution crate.

## Current Implementation

### `prune_directional_arrays()`
```rust
pub fn prune_directional_arrays(
    bin_arrays: &[Pubkey],
    active_bin_id: i32,
    swap_for_y: bool,
) -> Vec<Pubkey> {
    if bin_arrays.len() <= 2 {
        return bin_arrays.to_vec();
    }
    // Retains only first 2 arrays
    result.push(bin_arrays[0]);
    if bin_arrays.len() > 1 {
        result.push(bin_arrays[1]);
    }
    result
}
```

## Issues Identified

### 1. Fixed 2-Array Retention
- **Issue:** Always retains only 2 arrays regardless of route complexity
- **Impact:** May truncate required arrays for routes crossing multiple bin arrays
- **Severity:** HIGH

### 2. Placeholder Active Bin ID
- **Issue:** Uses `ctx.leg_1_accounts.lb_pair.to_bytes()[0] as i32` as active_bin_id
- **Impact:** Incorrect active bin identification
- **Severity:** MEDIUM

### 3. No Directional Validation
- **Issue:** Direction is computed but not used for array selection
- **Impact:** May select wrong arrays
- **Severity:** MEDIUM

### 4. No Missing Array Rejection
- **Issue:** Silently truncates arrays without error
- **Impact:** Transaction may fail on-chain
- **Severity:** HIGH

## Recommendations

### 1. Dynamic Array Sizing
```rust
pub fn prune_directional_arrays(
    bin_arrays: &[Pubkey],
    active_bin_id: i32,
    swap_for_y: bool,
    max_arrays: usize, // Based on quote traversal
) -> Result<Vec<Pubkey>, TxError> {
    if bin_arrays.is_empty() {
        return Err(TxError::MissingExecutionAccounts { ... });
    }
    // Retain all required arrays based on actual traversal
    let arrays_needed = calculate_arrays_needed(active_bin_id, swap_for_y, ...);
    if arrays_needed > bin_arrays.len() {
        return Err(TxError::MissingExecutionAccounts { ... });
    }
    Ok(bin_arrays[..arrays_needed].to_vec())
}
```

### 2. Use Actual Active Bin ID
- Pass the real active_bin_id from LbPair account data
- Do not use placeholder values

### 3. Direction-Aware Selection
- Select arrays in the correct direction
- Verify all required arrays are present

### 4. Explicit Rejection
- Return error if required arrays are missing
- Never silently truncate

## Test Cases Needed

| Arrays Required | Expected Behavior |
|-----------------|-------------------|
| 1 | Retain 1 array |
| 2 | Retain 2 arrays |
| 3 | Retain 3 arrays (currently broken) |
| 5 | Retain 5 arrays (currently broken) |
| 10 | Retain 10 arrays (currently broken) |

## Verdict

**CONDITIONAL** — Bin array pruning needs dynamic sizing implementation before production use.