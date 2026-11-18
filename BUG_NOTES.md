# BUG NOTES - Solana Flash Loan Arbitrage Engine

**Date:** 2026-11-18  
**Agent:** Elite Solana Quantitative & Atomic Arbitrage Agent  
**Discovery Context:** Pre-execution codebase inspection for Pump.fun-Raydium arbitrage engine task

---

## 🚨 CRITICAL BUGS (Must Fix Before Production)

---

### BUG-001: Missing Transaction V1 Support

**Severity:** CRITICAL  
**Status:** DISCOVERED  
**Files Affected:** `crates/arb-tx/src/*` (all transaction building files)  
**Line Numbers:** Multiple files, entire transaction construction pipeline

#### Root Cause Analysis
The existing transaction building infrastructure uses Legacy/V0 transaction patterns with `ComputeBudgetProgram` instructions. Solana Transaction V1 (SIMD-0296 & SIMD-0385), activated on Mainnet on September 15, 2026, has the following breaking changes:

1. **ComputeBudget instructions are IGNORED** - V1 moves compute budget into `Message.config` header
2. **Signatures at tail** - V1 places signatures at the end of serialized transaction (not at beginning)
3. **Version byte 0x81** - V1 uses version byte 0x81 at offset 0
4. **No ALTs** - V1 uses inline static accounts (up to 64)
5. **Message.config required** - Must explicitly set compute_unit_limit, loaded_accounts_data_size_limit, heap_size, priority_fee_lamports

#### Impact
- All transactions built with current code will FAIL on Mainnet
- Compute budget instructions waste 150 CUs and are ignored
- Transactions may exceed size limits without proper V1 formatting
- Priority fees may not be applied correctly

#### Proposed Fix
1. Create new V1 transaction builder module in `crates/arb-tx/src/transport/v1_builder.rs`
2. Implement `Message.config` header population
3. Remove all `ComputeBudgetProgram` instructions from V1 transactions
4. Add V1 serialization/deserialization support
5. Update all transaction construction to use V1 format
6. Add feature flag for V0 fallback (for testing)

#### Verification
```rust
// Before (V0 - WRONG for V1):
let mut tx = Transaction::new_with_payer(&instructions, Some(&payer.pubkey()));
tx.sign(&[payer], recent_blockhash);

// After (V1 - CORRECT):
let message = MessageV1 {
    header: MessageHeader {
        num_required_signatures: 1,
        num_readonly_signed_accounts: 0,
        num_readonly_unsigned_accounts: 0,
    },
    accounts: account_metas,
    recent_blockhash,
    instructions,
    config: MessageConfig {
        compute_unit_limit: 200_000,
        loaded_accounts_data_size_limit: 10_000,
        heap_size: 32_768,
        priority_fee_lamports: 1_000_000,
    },
};
let tx = VersionedTransaction::V1(VersionedMessage::V1(message));
```

---

### BUG-002: Incomplete Pump.fun-Raydium Arbitrage Implementation

**Severity:** HIGH  
**Status:** DISCOVERED  
**File:** `crates/arb-math/src/pump_fun_raydium.rs`  
**Line Numbers:** Entire file (259 lines)

#### Root Cause Analysis
The file `pump_fun_raydium.rs` exists but contains only partial implementation:
- Missing Raydium CLMM support
- Missing Raydium CPMM support  
- Missing optimal trade sizing for Pump.fun ↔ Raydium cycles
- No integration with flash loan infrastructure
- No profitability gates

#### Impact
- Cannot perform arbitrage between Pump.fun and Raydium CLMM pools
- Cannot perform arbitrage between Pump.fun and Raydium CPMM pools
- No optimal trade sizing = suboptimal profits or losses
- Missing core functionality for the requested arbitrage engine

#### Proposed Fix
1. Extend with complete Raydium CLMM integration
2. Extend with complete Raydium CPMM integration
3. Add optimal trade sizing using analytical formulas
4. Add piecewise Newton-Raphson for non-CPMM pools
5. Add profitability gates
6. Add comprehensive error handling

#### Verification
```rust
// Must support all Raydium architectures:
pub enum RaydiumPoolType {
    AMMv4,
    CPMM,
    CLMM,
}

pub fn quote_raydium_pool(
    pool_type: RaydiumPoolType,
    dx: u128,
    pool_state: &RaydiumPoolState,
) -> Result<u128> {
    match pool_type {
        RaydiumPoolType::AMMv4 => self::amm_v4::quote(dx, pool_state),
        RaydiumPoolType::CPMM => self::cpmm::quote(dx, pool_state),
        RaydiumPoolType::CLMM => self::clmm::quote(dx, pool_state),
    }
}
```

---

### BUG-003: No On-Chain Profit Assertion Guard

**Severity:** CRITICAL  
**Status:** DISCOVERED  
**Files Affected:** `crates/arb-engine/src/*`, `crates/arb-tx/src/*`  
**Line Numbers:** Missing from all execution paths

#### Root Cause Analysis
The existing codebase does not implement on-chain profit assertion to prevent unprofitable arbitrage executions. This means:

1. If off-chain quote is wrong (due to state change between quote and execution)
2. If slippage exceeds threshold
3. If flash loan fees exceed profits
4. The transaction will STILL EXECUTE and LOSE PRINCIPAL

#### Impact
- **Catastrophic risk** of losing entire flash loan principal
- No protection against MEV sandwich attacks
- No protection against state changes between quote and execution
- Violates atomic arbitrage principle (all-or-nothing)

#### Proposed Fix
1. Create `ProfitAssertion` instruction that:
   - Records pre-balance of profit account
   - Executes all swap instructions
   - Records post-balance
   - Reverts if post-balance < pre-balance + min_profit
2. Implement as custom program or use existing guard infrastructure
3. Add to ALL arbitrage transaction paths

#### Verification
```rust
// Profit guard instruction structure:
#[derive(Debug, Clone)]
pub struct AssertMinimumProfit {
    pub profit_account: Pubkey,
    pub min_net_profit_lamports: u64,
}

// On-chain logic (pseudo-code):
fn assert_minimum_profit(
    profit_account: &AccountInfo,
    min_profit: u64,
    pre_balance: u64, // Stored in instruction data
) -> Result<()> {
    let post_balance = profit_account.lamports();
    if post_balance < pre_balance.checked_add(min_profit).ok_or(ArbError::Overflow)? {
        return Err(ProgramError::Custom(ArbError::UnprofitableArbitrage as u32));
    }
    Ok(())
}
```

---

## 🚨 CRITICAL BUGS (Must Fix Before Production)

---

### BUG-005: Outdated Solana SDK - BLOCKS Transaction V1 Support

**Severity:** CRITICAL - BLOCKING  
**Status:** DISCOVERED  
**Files Affected:** `Cargo.toml`, `crates/*/Cargo.toml`, `programs/*/Cargo.toml`  
**Line Numbers:** Multiple dependency declarations

#### Root Cause Analysis
The entire workspace uses **solana-* crates v2.x and v3.x** which do NOT support Transaction V1 (SIMD-0296 & SIMD-0385). Transaction V1 requires **solana-* crates v4.2.x or higher**.

**Current versions found:**
- `solana-pubkey = "2.2"` (workspace root)
- `solana-address = "=2.7.0"` (arb-tx)
- `solana-instruction = "=3.5.0"` (arb-tx)
- `solana-message = "=4.6.0"` (arb-tx) - ✅ This one is OK
- `solana-signer = "=3.0.1"` (arb-tx)
- Various v3.x in programs/

**Required versions for V1:**
- All `solana-*` crates >= **v4.2.0**
- `solana-transaction` >= **v4.2.0** (for VersionedTransaction)

#### Impact
- **BLOCKS** all Transaction V1 functionality
- Cannot build V1 transactions
- Cannot parse V1 transactions from RPC/Yellowstone
- Cannot use 4096-byte payload capacity
- Cannot use Message.config header
- **ENTIRE ARBITRAGE ENGINE WILL FAIL ON MAINNET**

#### Proposed Fix
1. Update workspace `Cargo.toml`:
   ```toml
   # Change from:
   solana-pubkey = "2.2"
   # To:
   solana-pubkey = "4.2"
   ```

2. Update all crate Cargo.toml files:
   ```toml
   # arb-tx/Cargo.toml:
   solana-address = "4.2"
   solana-hash = "4.2"
   solana-instruction = "4.2"
   solana-message = "4.2"
   solana-signer = "4.2"
   solana-transaction = "4.2"
   ```

3. Update programs/ Cargo.toml files similarly
4. Test compilation with new SDK
5. Verify V1 transaction building works

#### Verification
```rust
// After fix, this should compile and work:
use solana_transaction::VersionedTransaction;
use solana_message::v1::MessageV1;

let message = MessageV1::new(...);
let tx = VersionedTransaction::V1(message);
```

---

## ⚠️ MEDIUM PRIORITY ISSUES

---

### BUG-004: DLMM Implementation May Be Outdated

**Severity:** MEDIUM  
**Status:** NEEDS VERIFICATION  
**Files:** `crates/arb-dex/src/meteora_dlmm.rs`, `crates/arb-math/src/dlmm.rs`  
**Line Numbers:** Entire DLMM implementation

#### Root Cause Analysis
Need to verify:
1. DLMM account layout matches current Meteora program
2. Bin step calculations are correct
3. Volatility accumulator math matches on-chain
4. Dynamic fee calculation is accurate
5. Cross-bin traversal logic is correct

#### Impact
- Incorrect quotes = wrong arbitrage decisions
- May miss profitable opportunities
- May execute unprofitable trades

#### Proposed Fix
1. Verify against latest Meteora DLMM program IDL
2. Cross-check with on-chain state
3. Add comprehensive property-based tests
4. Add fuzz testing for edge cases

---

## 📋 SDK USAGE VERIFICATION NEEDED

**Status:** NEEDS INSPECTION

The user requested to "use SDK" - need to verify:
1. Which Solana SDK version is being used
2. Whether it supports Transaction V1
3. Whether all dependencies are up-to-date
4. Whether Yellowstone gRPC is properly integrated

**Current Cargo.toml shows:**
```toml
solana-pubkey = "2.2"
```

**Required for V1:** `solana-*` crates v4.2.x+

**Action:** Need to update dependencies to support Transaction V1

---

## 🎯 IMMEDIATE ACTIONS REQUIRED

### Before Starting Pump.fun-Raydium Arbitrage Engine:

1. **FIX BUG-001** - Add Transaction V1 support (BLOCKING)
2. **FIX BUG-003** - Add profit assertion guard (BLOCKING)
3. **VERIFY BUG-004** - Check DLMM implementation
4. **UPDATE SDK** - Ensure solana-* crates >= 4.2.x

### During Implementation:

1. **FIX BUG-002** - Complete Pump.fun-Raydium implementation

---

## 📊 VERIFICATION CHECKLIST

- [ ] Transaction V1 builder implemented and tested
- [ ] All ComputeBudgetProgram instructions removed from V1 paths
- [ ] Message.config properly populated
- [ ] Profit assertion guard implemented
- [ ] DLMM implementation verified against latest specs
- [ ] SDK dependencies updated to v4.2.x+
- [ ] Pump.fun-Raydium arbitrage completed

---

## 🔗 REFERENCES

1. [Solana Transaction V1 - SIMD-0296 & SIMD-0385](https://solana.com/upgrades/larger-transaction-sizes)
2. [Agave 4.2 Release Notes](https://solana.com/upgrades/agave-4-2-release-overview)
3. [Pump.fun Program Docs](https://pump.fun/docs/bonding-curve)
4. [Raydium Program Addresses](https://docs.raydium.io/reference/program-addresses)
5. [Meteora DLMM Documentation](https://github.com/meteora-ag/)

---

**Next Steps:**
1. Review and prioritize bugs
2. Grant permission for fixes
3. Proceed with Pump.fun-Raydium arbitrage engine implementation

**Permission Request:** هل تعطيني الإذن (Permission) للذهاب وتعديل هذه المشاكل الآن؟

---

## 2026-11-18 — Audit-phase appendix (branch `vibe/audit-final`, PR #77)

### BUG-005 — Quarantined duplicate Pump++Raydium legacy modules (default-off)

- **Context**: discovered during per-feature CI matrix work (CI-02) on
  `vibe/audit-final` after merging `origin/main`.
- **Location**: `arb-tx/src/pump_v1_legacy_builder.rs` behind feature
  `pump-v1-legacy-builder` (crates/arb-tx/Cargo.toml:37);
  `arb-engine/src/pump_raydium_{builder,engine,shared}.rs` behind feature
  `pump-raydium-legacy-engine` (crates/arb-engine/Cargo.toml:8). Both default-off.
- **Root cause**: these modules were merged from parallel Phase work and (a)
  depend on an upward `arb-engine` dependency from arb-tx and (b) duplicate logic
  that the canonical `arb-tx/src/compiler.rs` and `arb-math` already own,
  violating the one-source-of-truth invariant. They also failed to compile under
  the feature matrix.
- **Impact if enabled**: duplicate fee/quote implementations could diverge from
  the canonical calculators (double counting / drift). Disabled = no runtime impact.
- **Proposed fix**: permanently delete the quarantined modules and instead port
  any missing pump+raydium route capability into `arb-tx/src/compiler.rs` /
  `arb-engine` split-routing path, or keep the features forever default-off and
  drop them from the CI matrix. Requires owner decision.
- **Status**: quarantined, not deleted — needs explicit operator decision.

### BUG-006 — ProfitGuardValidator boundary: `>` instead of `>=` — FIXED in this branch

- **Location**: `arb-engine/src/pump_raydium_guard.rs:225`.
- **Root cause**: the guard used strict `>` when comparing net profit after
  single-pass fee deduction against `min_net_profit_lamports`, rejecting an
  opportunity whose net profit lands exactly on the configured floor —
  inconsistent with every other guard in the codebase (all use `>=`).
- **Fix**: comparison updated to `>=` with a single-pass fee deduction (fees are
  subtracted once; no double-count). Regression covered by the guard tests in
  `arb-engine` (4 guard tests in the lib suite).
- **Status**: fixed on `vibe/audit-final`.

### BUG-007 — Deploy-to-VPS workflow expects repository secrets (operator action)

- **Location**: `.github/workflows/deploy-vps.yml`.
- **Root cause**: the workflow needs `VPS_HOST`, `VPS_USER`, `VPS_SSH_KEY`
  repository secrets; they are not present (the CI token cannot create
  them). It also auto-restarts the bot on every push to `main`.
- **Impact**: workflow fails (or restarts the bot) until the operator either
  adds the secrets or disables the workflow. This is an operator decision, not a
  code fix.
- **Status**: open — operator action required.

**Permission note (per Law 5):** BUG-005 (quarantined-module deletion) and
BUG-007 (secrets) require user consent. BUG-006 was fixed as part of the
approved audit scope because it guards profit acceptance (financial-correctness
critical path) and has a regression test.
