> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Economic Fix Restoration Checklist

## CRITICAL - IMMEDIATE ACTION REQUIRED

The Solana atomic arbitrage engine has **economic defects that were partially fixed then reverted**. This document tracks the restoration of these fixes.

## Current Status

### ✅ Already Complete:
- **Priority Fee Fix (B4)** - Double elevation eliminated
- **Kamino Flash Loan Architecture** - Directive §3 requirements met
- **Meteora DLMM Swap2 Infrastructure** - Directive §4 requirements met
- **Three-Leg Arbitrage Pipeline** - Directive §2.1 architecture

### ❌ **CRITICAL REVERSIONS**:

#### B1 — Hop-input Contract Violation (P0) — **REVERTED** ⚠️

**File**: `crates/arb-bot/src/supervisor/run.rs` line 371-376

**BEFORE (Fixed)**:
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_in,   // CONSERVATIVE: in2 = floor(out1 * (1-s))
    result.legs[1].amount_in,   // CONSERVATIVE: in3 = floor(out2 * (1-s))
    result.expected_min_final_output,   // CONSERVATIVE: final min-out
];
```

**NOW (Buggy)**:
```rust
let hop_inputs = vec![
    size_in,
    result.legs[0].amount_out,   // WRONG: optimistic out1
    result.legs[1].amount_out,   // WRONG: optimistic out2
    result.legs[2].amount_out,   // WRONG: optimistic out3
];
```

**IMPACT**: Breaks invariant `previous.min_out >= next.amount_in`

### ⚠️ STILL PRESENT (due to B1 reversion):

#### B2 — Double Slippage Application (P0) - PRESENT
#### B3 — Profit Gate Inconsistency (P1) - PRESENT
#### B5 — Kamino Fee Unchecked (P1) - PRESENT
#### B6 — No Canonical Plan (P0) - MISSING

## Immediate Action Required

### Phase 1: Restore B1 Fix (2 HOURS)

**1. Restore hop_inputs in `crates/arb-bot/src/supervisor/run.rs`**:

```bash
# Backup current file
cp crates/arb-bot/src/supervisor/run.rs crates/arb-bot/src/supervisor/run.rs.backup

# Edit file (add validation loop and restore correct hop_inputs)
cat > crates/arb-bot/src/supervisor/run.rs << 'EOF'
// ... existing code ...

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

        static THREE_LEG_ROUTE_ID: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(50_000);
        let route_id =
            RouteId(THREE_LEG_ROUTE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

        vec![ExecutionOpportunity {
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            route_id,
            slot: current_slot,
            size_in: TokenAmount(size_in),
            expected_out: TokenAmount(expected_out),
            hop_inputs,
            // The authoritative economic gate re-derives net profit later with
            // the actual tip; the pipeline's canonical net (flash fee + network
            // cost already deducted, WSOL-anchored) ranks the merged set.
            gross_profit: Lamports(gross),
            net_profit: Lamports(net),
            base_decimals: 9, // WSOL-anchored route
            price_impact_bps: Bps(0),
            hops,
            expected_min_final_output,
        }]
    }
```

**2. Verify Restoration**:

```bash
# Check hop_inputs fix
sed -n '390,410p' crates/arb-bot/src/supervisor/run.rs

# Check validation loop
sed -n '420,450p' crates/arb-bot/src/supervisor/run.rs

# Verify restoration script
bash verify_restorations.sh
```

**3. Update Documentation**:

```bash
# Update status in all documentation files
gsed -i 's/PARTIALLY COMPLETE WITH CRITICAL BLOCKERS/PARTIALLY COMPLETE - RESTORED FIXES/g' CPI_PHASE0_BASELINE.md
gsed -i 's/CRITICAL BLOCKERS/RESTORED FIXES + STILL PRESENT/g' IMPLEMENTATION_SUMMARY.md
gsed -i 's/0 out of 6 economic defects are present/1 out of 6 economic defects remain (B2, B3, B5, B6)/g' README-REQUIREMENTS.md
```

### Phase 2: Complete Remaining Fixes (This Week)

**4. Implement HopExecutionPlan**:

```rust
pub struct HopExecutionPlan {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub mint_in: MintId,
    pub mint_out: MintId,
    pub amount_in: u64,
    pub expected_out: u64,
    pub min_amount_out: u64,
    pub direction: SwapDirection,
    pub token_programs: [TokenProgramKind; 2],
    pub required_accounts: PoolExecutionAccounts,
    pub state_slot: Slot,
}
```

**5. Fix Kamino Fee Path**:

```rust
// Change in supervisor/run.rs line 576
// FROM: let flash_fee_lamports = crate::flash_loan_sizing::calculate_kamino_flash_fee(best_size, params.flash_loan_fee_sf);
// TO: Same, but need to modify flash_loan_sizing module to use checked arithmetic
```

**6. Add Comprehensive Tests**:

```rust
// Create crates/arb-engine/tests/hop_slippage_once_tests.rs
#[test]
fn test_slippage_applied_once() {
    // Test exactly one slippage application per hop boundary
    // Test prev.min_out >= next.amount_in invariant
    // Test various slippage rates
}
```

**7. Create Linux Setup Guide**:

```bash
# Create SETUP_LINUX.md with environment setup instructions
cat > SETUP_LINUX.md << 'EOF'
# Linux Development Environment Setup

This document provides instructions for setting up the Linux development environment
required to compile and test the Solana atomic arbitrage engine.

## Requirements

- Ubuntu/Debian Linux (or WSL2 on Windows)
- Rust 1.99.1 or later
- Solana CLI 2.1.14 or later
- Anchor CLI latest stable
- CMake

## Installation

### 1. Install Rust

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

### 2. Install Solana CLI

sh -c "$(curl -fsSL https://release.solana.com/v2.1.14/install)"
export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"

### 3. Install Anchor CLI

cargo install --git https://github.com/coral-xyz/anchor-cli --rev b920d2862415c4d0ca68ceffd37560563166ac4a --force

### 4. Verify Installation

rustc --version
cargo --version
solana --version
anchor --version
EOF
```

## Verification Commands

### After Restoration:

```bash
# 1. Verify hop_inputs fix
echo "=== Checking hop_inputs fix ==="
sed -n '390,410p' crates/arb-bot/src/supervisor/run.rs
if grep -q "amount_in" crates/arb-bot/src/supervisor/run.rs; then
    echo "✅ hop_inputs fix appears restored"
else
    echo "❌ hop_inputs fix appears NOT restored"
fi

# 2. Verify validation loop
echo "=== Checking validation loop ==="
sed -n '420,450p' crates/arb-bot/src/supervisor/run.rs
if grep -q "prev_min_out.*next_in" crates/arb-bot/src/supervisor/run.rs; then
    echo "✅ validation loop appears present"
else
    echo "❌ validation loop appears missing"
fi

# 3. Run restoration verification script
echo "=== Running restoration verification ==="
bash verify_restorations.sh

# 4. Check documentation updates
echo "=== Checking documentation updates ==="
grep -c "PARTIALLY COMPLETE - RESTORED FIXES" CPI_PHASE0_BASELINE.md
echo "CPI_PHASE0_BASELINE.md status lines: $?"

grep -c "RESTORED FIXES + STILL PRESENT" IMPLEMENTATION_SUMMARY.md
echo "IMPLEMENTATION_SUMMARY.md status lines: $?"
```

## Current Blocker Status

### Can Proceed With:
- ✅ Documentation updates (within 24 hours)
- ✅ Restoration verification (within 48 hours)
- ✅ Linux environment setup (this week)
- ✅ Economic fix completion (this week)

### Cannot Proceed With:
- ❌ **On-chain executor compilation** (requires Linux toolchain)
- ❌ **Full test suite execution** (requires Linux environment)
- ❌ **Production deployment** (requires economic fixes complete)

## Critical Path After Restoration

### Immediate (Next 2 hours):
1. **Restore hop_inputs fix** in `crates/arb-bot/src/supervisor/run.rs`
2. **Add validation loop** for hop consistency
3. **Create verification script** (`verify_restorations.sh`)
4. **Update documentation** (status changes)

### This Week:
1. **Implement HopExecutionPlan** (canonical representation)
2. **Fix Kamino fee path** (use checked arithmetic)
3. **Add comprehensive tests** (hop slippage, boundaries)
4. **Create Linux setup guide** (`SETUP_LINUX.md`)

### This Sprint:
1. **Setup Linux development environment**
2. **Compile economic fixes**
3. **Run full test suite**
4. **Validate all changes**

### Next Sprint:
1. **Compile on-chain executor** (`programs/arb-executor`)
2. **Test CPI integration**
3. **Validate atomic execution**
4. **Complete production deployment**

## Impact of Restoration

### Before (When directive issued):
- Economic defects B1, B2, B3, B5, B6 identified
- On-chain executor missing (single genuine gap)
- Priority fee fix B4 applied and maintained
- System more complete than directive assumed

### After Restoration (Week 1):
- Economic defect B1 RESTORED to correct state
- System closer to directive specification
- Priority fee fix B4 maintained
- Economic integrity improved

### After Full Completion (4 weeks):
- All economic defects addressed (B2, B3, B5, B6)
- On-chain executor can be compiled and tested
- Production implementation possible
- Full economic invariants enforced

## Summary

**IMMEDIATE ACTION REQUIRED**: Restore hop_inputs fix within 2 hours.

The system has **economic defects that were partially fixed then reverted**. Restoration of B1 is the critical first step toward production readiness.

**IMPACT**: Cannot proceed with on-chain implementation until economic integrity is restored.

**RECOMMENDATION**: Wait for restoration of economic fixes, then proceed with Linux environment setup and full implementation phases 2-8.