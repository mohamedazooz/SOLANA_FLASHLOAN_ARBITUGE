> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Immediate Actions - Restore Economic Fixes

## CRITICAL STATUS UPDATE

The Solana atomic arbitrage engine implementation is **PARTIALLY COMPLETE but regressed**. Economic fixes were applied then **REVERTED**, leaving the system in a worse state than when the directive was issued.

## Immediate Action Required

### 1. Restore Hop-input Contract Fix (CRITICAL - 2 hours)

**File**: `crates/arb-bot/src/supervisor/run.rs`

**CURRENT (broken)**:
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_out,   // WRONG: optimistic
    result.legs[1].amount_out,   // WRONG: optimistic
    result.legs[2].amount_out,   // WRONG: optimistic
];
```

**RESTORED (correct)**:
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_in,           // CORRECT: conservative in2 = floor(out1 * (1-s))
    result.legs[1].amount_in,           // CORRECT: conservative in3 = floor(out2 * (1-s))
    result.expected_min_final_output,   // CORRECT: final leg's min-out
];
```

**PLUS validation loop**:
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

**Instructions**:

1. Open `crates/arb-bot/src/supervisor/run.rs`
2. Locate lines 371-376 (the `hop_inputs` vector)
3. Replace current code with corrected version
4. Locate lines 398-398 (validation loop)
5. Add validation loop after the `debug_assert_eq`
6. Save file

### 2. Update Documentation (24 hours)

**Update these files**:

1. **`CPI_PHASE0_BASELINE.md`** - Mark economic defects as partially fixed
2. **`CPI_ECONOMIC_REMEDIATION.md`** - Add restoration notes
3. **`ECONOMIC_DEFECTS_SUMMARY.md`** - Update status to "PARTIALLY FIXED"
4. **`IMPLEMENTATION_SUMMARY.md`** - Add restoration instructions
5. **`README-REQUIREMENTS.md`** - Update status and requirements

**Key updates to make**:

- Change status from "NOT IMPLEMENTED" to "PARTIALLY COMPLETE with RESTORED FIXES"
- Add restoration instructions for B1 hop_inputs fix
- Document that B4 priority fee fix remains applied
- Update blockers list to reflect restored economic fix

### 3. Create Restoration Verification Script (48 hours)

**File**: `verify_restorations.sh`

```bash
#!/bin/bash

# Verify that economic fixes have been properly restored
# Run this after making the changes in step 1

set -e

echo "Verifying economic fix restorations..."

# Check hop_inputs fix
echo "1. Checking hop_inputs fix in run.rs..."
if grep -A 8 "let hop_inputs = vec!\[" crates/arb-bot/src/supervisor/run.rs | head -10 | grep -q "amount_in"); then
    echo "✅ hop_inputs fix appears restored (using amount_in)"
else
    echo "❌ hop_inputs fix appears NOT restored"
    exit 1
fi

# Check validation loop
echo "2. Checking validation loop..."
if grep -q "prev_min_out.*next_in" crates/arb-bot/src/supervisor/run.rs; then
    echo "✅ validation loop appears present"
else
    echo "⚠️  validation loop may be missing"
fi

# Check documentation updates
echo "3. Checking documentation updates..."
if [ -f "CPI_PHASE0_BASELINE.md" ] && grep -q "PARTIALLY COMPLETE" CPI_PHASE0_BASELINE.md; then
    echo "✅ Documentation updated"
else
    echo "⚠️  Documentation may need updating"
fi

echo "✅ Economic fix restoration verification complete!"
echo "Next step: Setup Linux development environment"
```

## Current State Assessment

### ✅ Successfully Implemented:
1. **Kamino Flash Loan Architecture** (Directive §3)
2. **Meteora DLMM Swap2 Infrastructure** (Directive §4)
3. **Three-Leg Arbitrage Pipeline** (Directive §2.1)
4. **Priority Fee Fix (B4)** (Directive §6.3)

### ❌ CRITICAL REVERSION:
1. **B1 — Hop-input Contract Violation** - RESTORED TO BUGGY STATE
   - Supervisor uses `amount_out` instead of `amount_in`
   - Breaks `prev.min_out >= next.amount_in` invariant

### ⚠️ STILL PRESENT:
2. **B2 — Double Slippage Application** - Due to B1 reversion
3. **B3 — Profit Gate Inconsistency** - Due to B1/B2
4. **B5 — Kamino Fee Unchecked** - Need to fix
5. **B6 — No Canonical Plan** - Need to implement

## Impact of Reversion

### Before (When directive issued):
- Economic defects B1, B2, B3, B5, B6 identified
- On-chain executor missing (single genuine gap)
- Priority fee fix B4 applied and maintained
- System more complete than directive assumed

### After (Current):
- Economic defect B1 RESTORED to buggy state
- System worse than directive assumed
- Cannot proceed with on-chain work
- Economic integrity compromised

## Immediate Timeline

### Hours 0-2: **Restore Economic Fixes**
- [ ] Restore hop_inputs fix in run.rs
- [ ] Add validation loop
- [ ] Verify changes

### Hours 2-24: **Update Documentation**
- [ ] Update all status documentation
- [ ] Create restoration verification script
- [ ] Document changes made

### Hours 24-48: **Setup Environment**
- [ ] Create Linux setup guide
- [ ] Document environment requirements
- [ ] Setup verification script

### This Week:
- [ ] Implement HopExecutionPlan
- [ ] Fix Kamino fee path
- [ ] Add comprehensive tests
- [ ] Setup Linux development environment

### This Sprint:
- [ ] Compile economic fixes
- [ ] Run full test suite
- [ ] Validate all changes

### Next Sprint:
- [ ] Compile on-chain executor (`programs/arb-executor`)
- [ ] Test CPI integration
- [ ] Validate atomic execution
- [ ] Complete production deployment

## Required Files

### Must Be Created/Updated:

1. **`crates/arb-bot/src/supervisor/run.rs`** - Restore hop_inputs fix
2. **`CPI_PHASE0_BASELINE.md`** - Update status
3. **`CPI_ECONOMIC_REMEDIATION.md`** - Add restoration notes
4. **`ECONOMIC_DEFECTS_SUMMARY.md`** - Update status
5. **`IMPLEMENTATION_SUMMARY.md`** - Add restoration instructions
6. **`README-REQUIREMENTS.md`** - Update requirements
7. **`verify_restorations.sh`** - Verification script
8. **`SETUP_LINUX.md`** - Linux setup guide

### Already Created:

1. **`programs/arb-executor/`** - On-chain executor prototype
2. **`programs/arb-executor/src/lib.rs`** - Anchor program
3. **`programs/arb-executor/README.md`** - Program documentation
4. **`programs/arb-executor/scripts/build.sh`** - Build script
5. **`programs/arb-executor/tests.rs`** - Test skeleton

## Verification Commands

### After Restoration:

```bash
# Verify hop_inputs fix
sed -n '371,380p' crates/arb-bot/src/supervisor/run.rs

# Check validation loop
sed -n '380,410p' crates/arb-bot/src/supervisor/run.rs

# Run verification script
bash verify_restorations.sh

# Check documentation
grep -l "PARTIALLY COMPLETE" *.md
```

### Documentation Status Check:

```bash
# Check key documentation files
grep -c "PARTIALLY COMPLETE" CPI_PHASE0_BASELINE.md
echo "CPI_PHASE0_BASELINE.md status lines: $?"

grep -c "RESTORED" CPI_ECONOMIC_REMEDIATION.md
echo "CPI_ECONOMIC_REMEDIATION.md restored lines: $?"

grep -c "PARTIALLY COMPLETE" IMPLEMENTATION_SUMMARY.md
echo "IMPLEMENTATION_SUMMARY.md status lines: $?"
```

## Current Blocker Status

### Can Proceed With:
- Documentation updates
- Linux environment setup
- Restoration verification
- Planning for full remediation

### Cannot Proceed With:
- **On-chain executor compilation** (no Solana toolchain)
- **Economic remediation** (fixes were reverted)
- **Full test suite execution** (no Linux environment)
- **Production deployment** (incomplete economic fixes)

## Critical Path

### Immediate (Next 2 hours):
1. Restore hop_inputs fix in run.rs
2. Add validation loop
3. Create verification script
4. Update documentation

### This Week:
1. Implement HopExecutionPlan
2. Fix Kamino fee path
3. Add comprehensive tests
4. Create Linux setup guide

### This Sprint:
1. Setup Linux development environment
2. Compile economic fixes
3. Run full test suite
4. Validate all changes

### Next Sprint:
1. Compile on-chain executor (`programs/arb-executor`)
2. Test CPI integration
3. Validate atomic execution
4. Complete production deployment

## Summary

**CRITICAL ISSUE:** Economic fix B1 was applied then **REVERTED**, making the system worse than when the directive was issued.

**IMMEDIATE ACTION REQUIRED:** Restore hop_inputs fix within 2 hours before any further progress can be made. The system cannot deliver a working solution with the current economic defects.

**IMPACT:** Cannot proceed with production implementation until economic integrity is restored.

**RECOMMENDATION:** Wait for proper restoration of economic fixes, then proceed with Linux environment setup and full implementation phases 2-8.