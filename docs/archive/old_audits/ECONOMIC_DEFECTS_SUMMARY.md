> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Economic Defects Summary

**Report date:** 2026-11-16
**Branch:** newrepair
**Status:** CRITICAL - Multiple defects present and some fixes reverted

---

## Critical Defects Status

### ✅ B4 — Priority fee double charge (FIXED)
**Location:** `crates/arb-bot/src/supervisor/opportunity.rs` and `crates/arb-tx/src/primitives/builder.rs`
**Problem:** Supervisor computed capped price, builder re-elevated it with gross-based addition
**Fix applied:** 
- Supervisor now uses `arb_tx::priority_fee_lamports(cu_price, cu_limit)`
- Builder encodes `priority_fee_micro_lamports` verbatim
- `calculate_dynamic_cu_price` function removed
**Impact:** ✅ COMPLETE

### ❌ B1 — Hop-input contract violation (REVERTED)
**Location:** `crates/arb-bot/src/supervisor/run.rs` line 371-376
**Problem:** Supervisor feeds `amount_out` (optimistic) instead of `amount_in` (conservative) to `hop_inputs`
**Required fix:** 
```rust
let hop_inputs = vec![
    size_in,                    // borrow
    result.legs[0].amount_in,   // CONSERVATIVE: previous hop's min-out
    result.legs[1].amount_in,   // CONSERVATIVE: previous hop's min-out  
    result.expected_min_final_output, // CONSERVATIVE: final leg's min-out
];
```
**Current broken code:**
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_out,   // OPTIMISTIC: quotes out1
    result.legs[1].amount_out,   // OPTIMISTIC: quotes out2
    result.legs[2].amount_out,   // OPTIMISTIC: quotes out3
];
```
**Impact:** ❌ BLOCKED - violates invariant `previous.min_out >= next.amount_in`

### ❌ B2 — Double slippage application (PRESENT)
**Location:** Root cause is B1
**Problem:** Pipeline applies slippage once, supervisor feeds optimistic outputs, builder applies slippage again
**Effect:** `(1-s)^2` slippage on intermediate hops instead of `(1-s)`
**Impact:** ❌ BLOCKED

### ❌ B3 — Profit gate ranking inconsistency (PRESENT)
**Location:** `crates/arb-engine/src/three_leg_meme_pipeline.rs`
**Problem:** Two different profit calculators disagree:
- Pipeline's `net_profit_of` uses optimistic `quote.out[2]`
- Sizing's `net_profit_lamports` uses conservative `leg_min_outputs[2]`
**Impact:** ❌ BLOCKED - ranking inflation

### ❌ B5 — Kamino fee unchecked wrapper (PRESENT)
**Location:** `crates/arb-bot/src/supervisor/run.rs` line 576
**Problem:** Uses `arb_tx::kamino::calculate_kamino_flash_fee` (unchecked) instead of `arb-math::economic::calculate_flash_fee_raw` (checked)
**Impact:** ❌ BLOCKED - fail-closed only on-chain

### ❌ B6 — No canonical execution-plan (PRESENT)
**Location:** Struct does not exist anywhere
**Problem:** Competing encodings: `hop_inputs: Vec<u64>`, `PlannedHop`, `LegSwapParams`, etc.
**Required:** Single `HopExecutionPlan` struct with all fields in one place
**Impact:** ❌ BLOCKED

---

## Directive §5.1 Fix - One-Sentence Summary

**B1/B2:** Fix hop_inputs to use conservative `[borrow, in2, in3, final_min_out]` where `in2 = floor(out1 * (1-s))` and `in3 = floor(out2 * (1-s))`, NOT optimistic `[borrow, out1, out2, out3]`. This ensures `previous.min_out >= next.amount_in` and prevents double slippage.

---

## Action Required

### 1. IMMEDIATE - Fix hop_inputs in run.rs
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_in,           // CONSERVATIVE: in2 = floor(out1 * (1-s))
    result.legs[1].amount_in,           // CONSERVATIVE: in3 = floor(out2 * (1-s))
    result.expected_min_final_output,   // CONSERVATIVE: final leg's min-out
];
```

### 2. CRITICAL - Add validation loop
```rust
for i in 0..hops.len() {
    let prev_min_out = result.legs[i].amount_out;
    let next_in = result.legs.get(i + 1)
        .map(|l| l.amount_in)
        .unwrap_or(expected_min_final_output);
    if next_in > prev_min_out {
        // Reject inconsistent route
        return Vec::new();
    }
}
```

### 3. REQUIRED - Implement HopExecutionPlan
Create single authoritative representation:
```rust
struct HopExecutionPlan {
    pool_id,
    protocol,
    mint_in,
    mint_out,
    amount_in,
    expected_out,
    min_amount_out,
    direction,
    token_programs,
    required_accounts,
    state_slot,
}
```

### 4. REQUIRED - Fix Kamino fee path
- Change `calculate_kamino_flash_fee` to use checked `calculate_flash_fee_raw`
- Propagate `BorrowTooSmall` errors through sizing pipeline

### 5. REQUIRED - Add regression tests
- 1% slippage with multiple hops
- `min_out == next.amount_in` boundary case
- `min_out < next.amount_in` rejection case
- Integer rounding boundaries
- Zero output case
- U64 overflow case
- Final repayment floor case
- Inverted route orientations

---

## Current State Assessment

**Total Progress:** ~20% of required fixes completed
**Blockers:** 5 out of 6 economic defects still present
**Toolchain limitation:** Cannot compile or test changes
**Recommendation:** Cannot proceed with on-chain implementation until economic fixes are applied and verified

**IMMEDIATE ACTION:** Restore the correct hop_inputs fix and add validation loop before any on-chain work proceeds.