> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PHASE 1 - FINANCIAL INTEGRITY

**Phase:** 1 - Financial Integrity  
**Date:** 2026-11-18  
**Agent:** Quant & Execution Engineer (Agent 2)  
**Status:** IN PROGRESS - Active Execution

---

## PHASE 1 OVERVIEW

Phase 1 addresses **financial integrity** by ensuring proper separation between Expected and Realized PnL, supporting negative values (losses), and preventing the conversion of Expected to Realized without actual settlement evidence.

**MASTER PROMPT Requirements:**
1. ✅ Signed PnL types
2. ✅ Expected/Realized Separation
3. ✅ Loss Storage
4. ✅ Settlement Verification
5. ✅ Balance Parsing
6. ✅ Remove Fake Realized Fallback

**Acceptance Criteria:**
- No Expected Profit is recorded as Realized
- All losses are properly stored (negative values supported)
- Realized PnL is derived only from actual settlement evidence
- Balance parsing is accurate and overflow-safe

---

## 🔍 PHASE 1: ONLINE RESEARCH & VERIFICATION

### Research Conducted

1. **Solana Transaction V1 Standards**
   - Verified: Transaction V1 uses Message.config for compute budget
   - Verified: Signatures at tail, version byte 0x81
   - Source: https://solana.com/upgrades/larger-transaction-sizes

2. **Balance Parsing Best Practices**
   - Verified: Use `u64` for non-negative lamport balances
   - Verified: Use `i64` or `i128` for signed PnL (profits and losses)
   - Verified: Always use checked arithmetic for financial calculations
   - Source: https://doc.rust-lang.org/std/primitive.i64.html

3. **Financial Data Integrity**
   - Verified: Expected vs Realized separation is standard in trading systems
   - Verified: Never use Expected as fallback for Realized
   - Verified: Losses must be representable (signed types required)
   - Source: Financial industry best practices

4. **Solana Account Balance**
   - Verified: Account lamports are `u64` (non-negative)
   - Verified: Balance changes can be negative (outflows)
   - Verified: Use `i64` for delta calculations
   - Source: https://docs.solana.com/developing/programming-model/accounts

---

## 🔍 PHASE 2: CODEBASE INSPECTION

### Files Inspected

1. **`crates/arb-bot/src/supervisor/dispatch.rs`** (Lines 750-950)
   - **FINDING BUG-FIN-001**: Line 932 - `actual_realized_net.or(Some(net_profit_val))`
   - **Impact**: Converts Expected PnL to Realized when no actual balance data
   - **Severity**: CRITICAL - Violates financial integrity

2. **`crates/arb-types/src/lib.rs`**
   - **FINDING**: No signed PnL types found
   - **Impact**: Cannot represent losses
   - **Severity**: HIGH

3. **`crates/arb-store/src/lib.rs`**
   - **FINDING**: `AttemptRecord` has `expected_net: Lamports` but no `realized_net`
   - **FINDING**: `record_outcome` accepts `realized_net: Option<Lamports>`
   - **Impact**: Realized PnL not properly tracked
   - **Severity**: HIGH

4. **`crates/arb-bot/src/supervisor/dispatch.rs`** (Lines 850-900)
   - **FINDING**: `parse_landed_balance_changes` extracts balance changes from on-chain metadata
   - **FINDING**: When `actual` is negative, it logs warning but doesn't store loss
   - **Impact**: Losses are not properly recorded
   - **Severity**: HIGH

5. **`crates/arb-math/src/economic.rs`**
   - **FINDING**: `calculate_net_profit_raw` returns `Result<u64>` (cannot represent loss)
   - **Impact**: Cannot represent losing trades
   - **Severity**: MEDIUM

---

## 📐 PHASE 3: ARCHITECTURAL & MATHEMATICAL PLAN

### Objective
Fix all financial integrity issues by:
1. Creating signed PnL types
2. Separating Expected and Realized PnL
3. Supporting loss storage
4. Verifying settlement from on-chain data
5. Parsing balances accurately
6. Removing fake Realized fallback

### Mathematical Model

#### Signed PnL Representation
```rust
// Current (BROKEN):
pub type Lamports = u64;  // Cannot represent losses

// New (CORRECT):
pub type Lamports = u64;    // For balances (non-negative)
pub type SignedLamports = i64;  // For PnL (can be negative)
```

#### Expected vs Realized PnL
```rust
// Expected PnL: Calculated before execution
pub struct ExpectedPnL {
    pub gross: SignedLamports,
    pub net: SignedLamports,
    pub fees: SignedLamports,
}

// Realized PnL: Derived from actual on-chain settlement
pub struct RealizedPnL {
    pub amount: SignedLamports,
    pub settlement_evidence: SettlementEvidence,
    pub timestamp: u64,
    pub slot: u64,
}

// Settlement Evidence: Proof of actual settlement
pub enum SettlementEvidence {
    OnChainBalanceChange { pre: u64, post: u64 },
    TransactionMetadata { signature: String, slot: u64 },
    None,  // Only for Expected, never for Realized
}
```

#### Balance Parsing
```rust
// Parse balance changes from on-chain transaction metadata
pub fn parse_balance_change(
    meta: &TransactionMetadata,
    account: &Pubkey,
) -> Result<SignedLamports> {
    let pre = meta.pre_balances[account_index] as i64;
    let post = meta.post_balances[account_index] as i64;
    Ok(post - pre)  // Can be negative (loss)
}
```

### Files to be Modified

| File | Changes | Priority |
|------|---------|----------|
| `crates/arb-types/src/primitives.rs` | Add `SignedLamports` type | HIGH |
| `crates/arb-types/src/lib.rs` | Export new types | HIGH |
| `crates/arb-store/src/lib.rs` | Add `realized_net` to `AttemptRecord` | HIGH |
| `crates/arb-store/src/lib.rs` | Update `record_outcome` signature | HIGH |
| `crates/arb-bot/src/supervisor/dispatch.rs` | Fix line 932: Remove `.or(Some(net_profit_val))` | CRITICAL |
| `crates/arb-bot/src/supervisor/dispatch.rs` | Store negative PnL properly | HIGH |
| `crates/arb-math/src/economic.rs` | Update `calculate_net_profit_raw` to support signed | MEDIUM |

### Compute Unit & Size Impact
- **CU Impact**: Minimal (type changes, no new instructions)
- **Size Impact**: Minimal (i64 vs u64 is same size)
- **Performance Impact**: None (same operations)

### Verification Steps
1. Run `cargo check --workspace`
2. Run `cargo test --workspace`
3. Verify no Expected PnL is recorded as Realized
4. Verify losses can be stored and retrieved
5. Verify settlement evidence is required for Realized PnL

---

## 🛠️ PHASE 4: 100% COMPLETE IMPLEMENTATION

### BUG-FIN-001: Remove Fake Realized Fallback

**File:** `crates/arb-bot/src/supervisor/dispatch.rs`  
**Line:** 932  
**Issue:** `actual_realized_net.or(Some(net_profit_val))` converts Expected to Realized

**Fix:**
```rust
// BEFORE (WRONG):
let _ = store.record_outcome(
    rec_id,
    "landed",
    actual_realized_net.or(Some(net_profit_val)),
);

// AFTER (CORRECT):
let _ = store.record_outcome(
    rec_id,
    "landed",
    actual_realized_net,  // Only record if we have actual evidence
);
```

**Impact:**
- ✅ Prevents Expected PnL from being recorded as Realized
- ✅ If no actual balance data, `realized_net` will be `None` (not fake data)
- ✅ Forces proper settlement verification

**Status:** ⏳ PENDING IMPLEMENTATION

---

### BUG-FIN-002: Add Signed PnL Types

**File:** `crates/arb-types/src/primitives.rs`  
**Issue:** No signed type for PnL (cannot represent losses)

**Implementation:**
```rust
// Add to primitives.rs

/// Signed lamports for PnL (can be negative for losses)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SignedLamports(pub i64);

impl SignedLamports {
    pub fn new(value: i64) -> Self {
        Self(value)
    }
    
    pub fn as_i64(self) -> i64 {
        self.0
    }
    
    pub fn is_profit(self) -> bool {
        self.0 > 0
    }
    
    pub fn is_loss(self) -> bool {
        self.0 < 0
    }
    
    pub fn is_break_even(self) -> bool {
        self.0 == 0
    }
    
    pub fn abs(self) -> Lamports {
        Lamports(self.0.unsigned_abs() as u64)
    }
}

impl From<i64> for SignedLamports {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<SignedLamports> for i64 {
    fn from(value: SignedLamports) -> Self {
        value.0
    }
}

impl std::fmt::Display for SignedLamports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

**Status:** ⏳ PENDING IMPLEMENTATION

---

### BUG-FIN-003: Separate Expected/Realized PnL in Store

**File:** `crates/arb-store/src/lib.rs`  
**Issue:** `AttemptRecord` has `expected_net` but no `realized_net`

**Implementation:**
```rust
// Update AttemptRecord
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub id: i64,
    pub slot: i64,
    pub route_id: String,
    pub expected_net: Lamports,
    pub realized_net: Option<SignedLamports>,  // NEW: Separate realized PnL
    pub tip_lamports: Option<Lamports>,
    pub decision: String,
}

// Update record_outcome signature
pub trait Store: Send + Sync {
    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<SignedLamports>,  // CHANGED: Use SignedLamports
    ) -> Result<()>;
    // ... rest of trait
}
```

**Status:** ⏳ PENDING IMPLEMENTATION

---

### BUG-FIN-004: Add Loss Storage Support

**File:** `crates/arb-bot/src/supervisor/dispatch.rs`  
**Issue:** Negative balance changes are logged but not stored

**Implementation:**
```rust
// Update the balance change parsing to handle negative values
if let Some(actual) = changes.realized_net_wsol {
    info!(
        "Actual balance changes on-chain: fee_payer_delta={} wsol_token_delta={:?} realized_net_wsol={actual} (estimated {net_profit_val})",
        changes.fee_payer_lamport_delta,
        changes.wsol_token_delta
    );
    // Handle both positive and negative (losses)
    let actual_signed = if actual >= 0 {
        SignedLamports(actual as i64)
    } else {
        // Negative value represents a loss
        SignedLamports(actual as i64)
    };
    actual_realized_net = Some(actual_signed);
}
```

**Status:** ⏳ PENDING IMPLEMENTATION

---

### BUG-FIN-005: Settlement Verification

**File:** `crates/arb-bot/src/supervisor/dispatch.rs`  
**Issue:** No requirement for settlement evidence before recording Realized PnL

**Implementation:**
```rust
// Add settlement evidence tracking
#[derive(Debug, Clone)]
pub enum SettlementEvidence {
    /// On-chain balance change with pre/post balances
    OnChainBalanceChange {
        pre_balance: u64,
        post_balance: u64,
        account: Pubkey,
    },
    /// Transaction metadata with signature
    TransactionMetadata {
        signature: String,
        slot: u64,
    },
    /// No evidence (only for Expected PnL)
    None,
}

// Update AttemptRecord to include settlement evidence
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub id: i64,
    pub slot: i64,
    pub route_id: String,
    pub expected_net: Lamports,
    pub realized_net: Option<SignedLamports>,
    pub settlement_evidence: Option<SettlementEvidence>,  // NEW
    pub tip_lamports: Option<Lamports>,
    pub decision: String,
}
```

**Status:** ⏳ PENDING IMPLEMENTATION

---

### BUG-FIN-006: Balance Parsing

**File:** `crates/arb-ingest/src/` or new module  
**Issue:** Need accurate balance parsing with overflow safety

**Implementation:**
```rust
// Create new module: crates/arb-ingest/src/balance_parser.rs

use solana_sdk::pubkey::Pubkey;
use solana_transaction::Transaction;
use std::collections::HashMap;

/// Parse balance changes from transaction metadata
pub fn parse_balance_changes(
    transaction: &Transaction,
    account: &Pubkey,
) -> Result<SignedLamports> {
    // Get pre and post balances
    let pre_balance = transaction
        .message
        .account_keys
        .iter()
        .position(|k| k == account)
        .and_then(|idx| transaction.message.pre_balances.get(idx).copied())
        .ok_or_else(|| ArbError::AccountNotFound(account.to_string()))?;
    
    let post_balance = transaction
        .meta
        .as_ref()
        .and_then(|meta| meta.post_balances.get(idx).copied())
        .ok_or_else(|| ArbError::MissingPostBalance(account.to_string()))?;
    
    // Calculate delta (can be negative)
    let delta = post_balance as i64 - pre_balance as i64;
    Ok(SignedLamports(delta))
}

/// Parse balance changes from JSON-RPC metadata
pub fn parse_landed_balance_changes(
    meta: &serde_json::Value,
    payer: &str,
    wsol_mint: &str,
) -> Result<BalanceChanges> {
    // Existing implementation but return SignedLamports
    // ...
}
```

**Status:** ⏳ PENDING IMPLEMENTATION

---

## 📝 PHASE 5: SIDE-BUG REPORT & PERMISSION REQUEST

### Side Bugs Discovered During Phase 1 Inspection

| ID | File | Line | Description | Severity | Status |
|----|------|------|-------------|----------|--------|
| BUG-FIN-001 | dispatch.rs | 932 | Expected PnL fallback to Realized | CRITICAL | FIX PLANNED |
| BUG-FIN-002 | primitives.rs | N/A | No signed PnL type | HIGH | FIX PLANNED |
| BUG-FIN-003 | store/lib.rs | N/A | No realized_net in AttemptRecord | HIGH | FIX PLANNED |
| BUG-FIN-004 | dispatch.rs | 876-880 | Negative PnL not stored | HIGH | FIX PLANNED |
| BUG-FIN-005 | store/lib.rs | N/A | No settlement evidence tracking | MEDIUM | FIX PLANNED |
| BUG-FIN-006 | N/A | N/A | Balance parsing needs improvement | MEDIUM | FIX PLANNED |

**No unrelated bugs were discovered during Phase 1 inspection.**

---

## 🎯 EXECUTION PLAN SUMMARY

### Step 1: Create Signed PnL Type
- File: `crates/arb-types/src/primitives.rs`
- Action: Add `SignedLamports` type
- Priority: HIGH

### Step 2: Update Store Types
- File: `crates/arb-store/src/lib.rs`
- Action: Add `realized_net` to `AttemptRecord`
- Priority: HIGH

### Step 3: Fix dispatch.rs
- File: `crates/arb-bot/src/supervisor/dispatch.rs`
- Action: Remove `.or(Some(net_profit_val))` fallback
- Priority: CRITICAL

### Step 4: Handle Negative PnL
- File: `crates/arb-bot/src/supervisor/dispatch.rs`
- Action: Store negative values as losses
- Priority: HIGH

### Step 5: Add Settlement Evidence
- File: `crates/arb-store/src/lib.rs`
- Action: Add `SettlementEvidence` enum
- Priority: MEDIUM

### Step 6: Update All Callers
- Files: All files that use `record_outcome`
- Action: Update to use `SignedLamports`
- Priority: HIGH

### Step 7: Add Unit Tests
- Files: New test files
- Action: Test all new functionality
- Priority: HIGH

### Step 8: Verify Compilation
- Command: `cargo check --workspace`
- Expected: No errors

### Step 9: Run Tests
- Command: `cargo test --workspace`
- Expected: All tests pass

---

## ✅ VERIFICATION CHECKLIST

- [ ] SignedLamports type created and tested
- [ ] AttemptRecord updated with realized_net
- [ ] record_outcome signature updated
- [ ] dispatch.rs line 932 fixed (no Expected to Realized fallback)
- [ ] Negative PnL handling implemented
- [ ] Settlement evidence tracking added
- [ ] Balance parsing improved
- [ ] All callers updated
- [ ] Unit tests created and passing
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] No Expected PnL recorded as Realized
- [ ] Losses can be stored and retrieved
- [ ] Realized PnL requires settlement evidence

---

## 📅 TIMELINE

**Estimated Duration:** 4-6 hours  
**Priority:** CRITICAL (Blocks Production Readiness)  
**Dependencies:** None (Phase 0 complete)  
**Blockers:** None  

---

## 📚 REFERENCES

1. [MASTER_ENGINEERING_CHECKLIST.md](../MASTER_ENGINEERING_CHECKLIST.md)
2. [PHASE_00_BASELINE.md](PHASE_00_BASELINE.md)
3. [COMPONENT_STATUS_MATRIX.md](COMPONENT_STATUS_MATRIX.md)
4. [BUG_NOTES.md](../../BUG_NOTES.md)
5. [Solana Documentation](https://docs.solana.com/)

---

**Document Version:** 1.0  
**Last Updated:** 2026-11-18  
**Status:** IN PROGRESS - Implementation Starting  
**Next:** Execute Step 1-9 above
