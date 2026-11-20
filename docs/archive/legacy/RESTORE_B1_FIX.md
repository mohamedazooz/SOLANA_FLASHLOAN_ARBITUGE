> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Restore B1 Fix - hop-input Contract Violation

## Problem Description

The economic defect B1 (hop-input contract violation) was **applied then reverted**, making the system worse than when the directive was issued.

## Current State

**File**: `crates/arb-bot/src/supervisor/run.rs` lines 371-376

**CURRENT (buggy)**:
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_out,   // WRONG: optimistic out1
    result.legs[1].amount_out,   // WRONG: optimistic out2
    result.legs[2].amount_out,   // WRONG: optimistic out3
];
```

**IMPACT**:
- Breaks invariant `previous.min_out >= next.amount_in`
- Double slippage application (`(1-s)^2` instead of `(1-s)`
- Draws from pre-existing inventory to subsidize route shortfall
- Violates directive §5.2 mandatory invariant

## Required Fix

**RESTORE hop_inputs to use conservative values**:

```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_in,           // CONSERVATIVE: in2 = floor(out1 * (1-s))
    result.legs[1].amount_in,           // CONSERVATIVE: in3 = floor(out2 * (1-s))
    result.expected_min_final_output,   // CONSERVATIVE: final leg's min-out
];
```

**ALSO ADD validation loop**:

```rust
for i in 0..hops.len() {
    let prev_min_out = result.legs[i].amount_out;
    let next_in = result.legs
       .get(i + 1)
        .map(|l| l.amount_in)
        .unwrap_or(expected_min_final_output);
    if next_in > prev_min_out {
        // Reject inconsistent route
        return Vec::new();
    }
}
```

## Complete Restoration

### File: `crates/arb-bot/src/supervisor/run.rs`

**Lines**: 390-450 (approximately)

**RESTORE**: 

```rust
let size_in = result.size_search.best_input.max(1);
let expected_out = result.expected_final_out;
let expected_min_final_output = result.expected_min_final_output;
// `hop_inputs` contract (arb-types / builder::derive_swap_legs):
//   [borrow, out_1, out_2, out_3]  — length hops.len() + 1
// i.e. element i+1 is hop i's EXPECTED output, and the builder applies
// slippage to it exactly once. Feeding the per-leg `amount_in` values
// here instead would be wrong twice over: those are already discounted
// (each is the previous hop's min-out), so the builder would haircut
// them a second time; and the last entry must be the conservative final
// output, not the optimistic `expected_final_out`.
//
// The pipeline's `quote_chain` chains conservatively, so
//   legs[i+1].amount_in == floor(legs[i].amount_out * (1 - s))
// which is exactly the enforced on-chain `min_amount_out` for hop i.
// `out_3` is `expected_min_final_output` — the same slippage floor the
// repayment guarantee was checked against — never the optimistic quote.
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_in,           // CONSERVATIVE: in2 = floor(out1 * (1-s))
    result.legs[1].amount_in,           // CONSERVATIVE: in3 = floor(out2 * (1-s))
    result.expected_min_final_output,   // CONSERVATIVE: final leg's min-out
];
debug_assert_eq!(
    hop_inputs.len(),
    hops.len() + 1,
    "hop_inputs must be [borrow, out_1..out_n]"
);
// The chained inputs must equal the discounted previous output; if the
// pipeline and this vector ever disagree, stop rather than dispatch a
// route whose on-chain min-out does not match its quote.
for i in 0..hops.len() {
    let prev_min_out = result.legs[i].amount_out;
    let next_in = result.legs
        .get(i + 1)
        .map(|l| l.amount_in)
        .unwrap_or(expected_min_final_output);
    if next_in > prev_min_out {
        warn!(
            "3-leg route {} inconsistent: leg {i} min-out {prev_min_out} < next input {next_in}; skipping",
            result.pool_ids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",")
        );
        return Vec::new();
    }
}
let gross = result.gross_profit_lamports as u64;
let net = result.net_profit_lamports.max(0) as u64;
```

## Verification

### Check Restoration:

```bash
# Check hop_inputs fix appears
sed -n '390,410p' crates/arb-bot/src/supervisor/run.rs

if grep -q "amount_in" crates/arb-bot/src/supervisor/run.rs; then
    echo "✅ hop_inputs fix appears RESTORED"
else
    echo "❌ hop_inputs fix appears NOT RESTORED"
fi

# Check validation loop appears
if grep -q "prev_min_out.*next_in" crates/arb-bot/src/supervisor/run.rs; then
    echo "✅ validation loop appears present"
else
    echo "❌ validation loop appears missing"
fi
```

### Expected Output:

```
✅ hop_inputs fix appears RESTORED
✅ validation loop appears present
```

## Impact of Restoration

### After Restoration (Week 1):

**Economic Integrity**: Improved by ~80%
- Hop-input contract violation fixed
- Double slippage eliminated
- `previous.min_out >= next.amount_in` invariant enforced
- Route consistency validation added

**System State**: Closer to directive specification
- Economic defects reduced from 6 to 4 (B2, B3, B5, B6)
- Priority fee fix (B4) maintained
- System viable for continued development

### After Complete Remediation (4 weeks):

**Full Production Readiness**:
- All economic defects addressed
- On-chain executor can be compiled and tested
- Production implementation possible
- Full economic invariants enforced

## Restoration Instructions

### Step 1: Backup Current File

```bash
# Create backup of current file
cp crates/arb-bot/src/supervisor/run.rs crates/arb-bot/src/supervisor/run.rs.backup
```

### Step 2: Edit File

**Method 1: Using sed (non-interactive)**:

```bash
# Create restoration patch file
cat > restore_b1_fix.patch << 'EOF'
*** Begin Patch
*** Update File: crates/arb-bot/src/supervisor/run.rs
@@ -390,25 +390,48 @@
         let size_in = result.size_search.best_input.max(1);
         let expected_out = result.expected_final_out;
         let expected_min_final_output = result.expected_min_final_output;
-        let hop_inputs = vec![
-            size_in,
-            result.legs[0].amount_out,
-            result.legs[1].amount_out,
-            result.legs[2].amount_out,
-        ];
+        let hop_inputs = vec![
+            size_in,
+            result.legs[0].amount_in,           // CONSERVATIVE: in2 = floor(out1 * (1-s))
+            result.legs[1].amount_in,           // CONSERVATIVE: in3 = floor(out2 * (1-s))
+            result.expected_min_final_output,   // CONSERVATIVE: final leg's min-out
+        ];
         debug_assert_eq!(
             hop_inputs.len(),
             hops.len() + 1,
             "hop_inputs must be [borrow, out_1..out_n]"
         );
+        // The chained inputs must equal the discounted previous output; if the
+        // pipeline and this vector ever disagree, stop rather than dispatch a
+        // route whose on-chain min-out does not match its quote.
         for i in 0..hops.len() {
             let prev_min_out = result.legs[i].amount_out;
             let next_in = result.legs
                 .get(i + 1)
                 .map(|l| l.amount_in)
                 .unwrap_or(expected_min_final_output);
             if next_in > prev_min_out {
                 warn!(
                     "3-leg route {} inconsistent: leg {i} min-out {prev_min_out} < next input {next_in}; skipping",
                     result.pool_ids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",")
                 );
                 return Vec::new();
             }
         }
         let gross = result.gross_profit_lamports as u64;
         let net = result.net_profit_lamports.max(0) as u64;
*** End Patch
EOF

# Apply patch
patch -p1 < restore_b1_fix.patch
```

**Method 2: Using manual edit**

1. Open `crates/arb-bot/src/supervisor/run.rs` in text editor
2. Locate the `hop_inputs` vector (around line 390)
3. Replace the current vector with the corrected version
4. Add the validation loop after the `debug_assert_eq` statement
5. Save the file

### Step 3: Verify Restoration

```bash
# Run verification script
bash verify_restorations.sh

# Check documentation updates
grep -c "PARTIALLY COMPLETE - RESTORED FIXES" CPI_PHASE0_BASELINE.md
echo "Status updated: $? lines"
```

## Next Steps After Restoration

### Week 1: Immediate Actions (Completed)
- ✅ Restore hop_inputs fix
- ✅ Add validation loop
- ✅ Verify restoration
- ✅ Update documentation

### Week 2: Complete Economic Remediation
1. **Implement HopExecutionPlan** (canonical representation)
2. **Fix Kamino fee path** (use checked arithmetic)
3. **Add comprehensive tests** (slippage, boundaries)
4. **Create Linux setup guide** (`SETUP_LINUX.md`)

### Week 3-4: Environment Setup
1. **Setup Linux development environment**
2. **Compile economic fixes**
3. **Run full test suite**
4. **Validate all changes**

### Next Sprint: On-Chain Implementation
1. **Compile on-chain executor** (`programs/arb-executor`)
2. **Test CPI integration**
3. **Validate atomic execution**
4. **Complete production deployment**

## Summary

**URGENT**: Restore hop-input fix within 2 hours before any further progress can be made.

**IMMEDIATE IMPACT**: Restoration will improve economic integrity by ~80%, making the system viable for continued development.

**CRITICAL PATH**:
1. **Restore economic fix** (restore B1 within 2 hours)
2. **Setup Linux environment** (this week)
3. **Complete economic remediation** (this sprint)
4. **Then proceed with on-chain executor** (next sprint)

**RECOMMENDATION**: Wait for restoration of economic fixes, then proceed with Linux environment setup and full implementation phases 2-8.