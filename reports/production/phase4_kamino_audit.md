# Phase 4 — Kamino Flash Loan Forensic Audit

## 4.1 Verify Actual Deployed Program Behavior

### 4.1.1 Program ID and Deployment
- **Kamino Lend Program ID**: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **Verified against**: On-chain deployment data, main market address `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF`
- **SDK version**: Anchor-based program; offsets verified on 2026-11-04

### 4.1.2 Reserve Account Layout (verified offsets from 2026-11-04)
| Field | Offset (from account base) | Type | Description |
|---|---|---|---|
| `liquidity_mint` | `8 + 120` | Pubkey (32) | Liquidty mint address |
| `mint_decimals` | `8 + 264` | u8 (1) | Token decimals |
| `status` | `8 + 4848` | u8 (1) | 0 = Active, ≠ 0 = Inactive |
| `fee_sf` | `8 + 4896` | u64 (8) | Flash loan fee in Q64.64 fixed-point |

**Total account size**: `8 + 4896 + 8 = 4912 bytes`

### 4.1.3 Flash Loan Discriminators
| Instruction | Discriminator (8 bytes) | Source |
|---|---|---|
| `flash_borrow_reserve_liquidity` | `[135, 231, 52, 167, 7, 52, 212, 193]` | `sha256("global:flash_borrow_reserve_liquidity")[..8]` |
| `flash_repay_reserve_liquidity` | `[185, 117, 0, 203, 96, 245, 180, 186]` | `sha256("global:flash_repay_reserve_liquidity")[..8]` |

### 4.1.4 Reserve Status
- `KAMINO_RESERVE_STATUS_ACTIVE = 0` → reserve is active and usable
- Any non-zero status → reserve inactive, flash loans rejected

### 4.1.5 Main-Market Fee Values (verified on 2026-11-04)
| Reserve | `flash_loan_fee_sf` (Q64.64) | Approx. bps | Notes |
|---|---|---|---|
| WSOL | `11_529_215_046_068` | ≈ 0.006 bps | Very small; only Q64.64 exact math represents it |
| USDC | `11_529_215_046_068` | ≈ 0.006 bps | Same as WSOL (both main-market reserves) |

**Critical**: A bps integer cannot represent this fee. The value `11_529_215_046_068` in Q64.64 ≈ 6.25e-7 ≈ 0.0000625%. Any hardcoded 0 or 5 bps misprices quotes (800x on main market).

## 4.2 Flash Loan Instruction Sequence

### 4.2.1 Required Sequence: Atomic Three-Instruction Flow

The flash loan instruction sequence must follow this exact atomic order:

1. **Flash Borrow** (`flash_borrow_reserve_liquidity`)
   - Borrows liquidity from the reserve
   - Records the borrow instruction index for later repayment reference
   - Emits/records the borrow index internally on-chain

2. **Arbitrage Instructions** (user's instructions)
   - Swap(s) against DEX pools (Meteora DLMM, Raydium, Orca, etc.)
   - User's profit-generating logic

3. **Flash Repay** (`flash_repay_reserve_liquidity`)
   - Repays the borrowed amount + protocol fee
   - References the original borrow instruction index
   - Atomizes the flash loan (if repay fails, entire transaction reverts)

**Verification that repayment references the correct borrow instruction**:
- The `build_kamino_flash_repay` function accepts `borrow_instruction_index: u8`
- This must exactly match the index assigned during borrow
- Default value: `2` (the borrow instruction is at index 2 within the transaction)
- The repay instruction data includes this index byte: `data.push(borrow_instruction_index)`

**Same expected reserve and liquidity mint**:
- Both borrow and repay must use the same reserve address
- Both must use the same liquidity mint (the reserve's liquidity mint)
- Mismatched reserve/mint → transaction fails (instruction error)

**Exact borrowed amount**:
- The `amount` parameter in `build_kamino_flash_borrow` specifies the borrowed amount
- The repay amount must be `borrowed_amount + protocol_fee` (calculated from `fee_sf`)
- Under-repayment → transaction fails (on-chain check)
- Over-repayment → funds lost (no refund mechanism)

## 4.3 Balance Invariants

### 4.3.1 Minimum Repayment Invariant

At minimum:
```
final_repay_amount >= borrowed_amount + protocol_fee
```

The user must receive positive net profit after ALL other costs (fees, slippage, tip, priority fee).

### 4.3.2 Intermediate Token Balance Checks

1. **Reserve liquidity must be sufficient** to lend the requested amount
   - `reserve_liquidity >= borrowed_amount` (on-chain check)
   - If insufficient → transaction fails with appropriate error

2. **User destination ATA must be valid** for the liquidity mint
   - If ATA doesn't exist, it's created idempotently (CreateIdempotent opcode)
   - If ATA is wrong mint → funds lost

3. **Fee vault must hold sufficient fees** for repayment
   - `reserve_fee_vault` balance must cover the protocol fee component

### 4.3.3 Reject Unsupported Token Extensions

- Token-2022 transfers with `transfer_fee_bps != 0` or `has_risky_token` features → rejected
- Mayhem/cashback Pump.fun modes → rejected
- Complete/migrated bonding curves → rejected

### 4.3.4 Reject Stale Reserve Data

- `reserve.status != 0` (not active) → `ReserveNotActive` error
- Reserve account too short → `AccountTooShort` error
- Discriminator mismatch → `BadDiscriminator` error
- Liquidity mint mismatch → `MintMismatch` error

## 4.4 Negative Testing

### 4.4.1 Test Matrix for Invalid Scenarios

| Scenario | Expected Failure | Error Type |
|---|---|---|
| Missing reserve account | Err | `PoolNotFound` / `AccountTooShort` |
| Wrong owner on reserve | Err | Instruction error (not signer) |
| Wrong discriminator on instruction | Err | `ArbError::SimulationFailed` |
| Wrong liquidity mint | Err | `MintMismatch` (from `decode_reserve_flash_fee_sf`) |
| Inactive reserve (status != 0) | Err | `ReserveNotActive` |
| Invalid fee_sf (0 with non-zero borrow) | Err | `calculate_kamino_flash_fee` returns 0 but protocol may reject |
| Overflow in borrow amount | Err | `ArbError::Overflow` |
| Wrong borrow index in repay | Err | Instruction fails (index out of range or mismatched) |
| Incorrect instruction order (repay before borrow) | Err | Transaction simulation fails |
| Missing repay instruction | Err | Flash loan not atomic; funds not returned |
| Incorrect account flags (e.g., user not signer) | Err | Preflight simulation fails |
| Insufficient repayment (amount < borrowed + fee) | Err | Transaction reverts on-chain |
| Wrong token program (Token-2022 without verification) | Err | `adapters.rs` rejects risky mint |

### 4.4.2 Key Negative Test Results (from existing test suite)

All 10 existing Kamino tests pass, covering:
- ✓ Market authority PDA derivation (bump = 248)
- ✓ Flash borrow layout and discriminator (12 accounts, 16-byte data)
- ✓ Flash repay layout and discriminator (12 accounts, 17-byte data with borrow_ix_index)
- ✓ Decode returns known fee fixture (mainnet sf = 11_529_215_046_068)
- ✓ Decrypt rejects wrong mint
- ✓ Decrypt rejects bad discriminator
- ✓ Decrypt rejects short account
- ✓ Decrypt rejects inactive reserve
- ✓ Calculate flash fee: sf=0 → 0, 5% Q64.64 → 50M, mainnet sf → 625 for 1 SOL

**Additional negative tests that must pass** (not yet in test suite, but required for production readiness):

1. **Wrong borrow instruction index**: `build_kamino_flash_repay` with `borrow_instruction_index != 2` when using default, or any index that doesn't match the actual borrow position
2. **Mismatched reserve/mint**: Different mint in reserve decode vs expected_mint parameter
3. **Stale reserve data**: Reserve with old slot/dead bank → `decode_reserve_flash_fee_sf` rejects with `ReserveNotActive`
4. **Overflow in fee calculation**: `calculate_kamino_flash_fee(u64::MAX, u64::MAX)` → must return `u64::MAX` or error, never wrap
5. **Zero borrow amount**: `calculate_kamino_flash_fee(0, sf)` → returns 0 (guarded)
6. **Referrer handling**: With `referrer_token_state` and `referrer_account` set vs defaulting to `klend_program_id`

## 4.5 Instruction Sequence Atomicity

### 4.5.1 Fail-Closed Behavior

The flash loan mechanism is designed to be atomic:

- **If repay fails**: Entire transaction reverts; borrowed funds are returned to reserve
- **If any instruction fails**: All on-state changes are rolled back (Solana's native program atomicity)
- **No partial execution**: User cannot receive arbitrage profit without full repayment

### 4.5.2 Borrow/Repay Index Consistency

The `borrow_instruction_index` must satisfy:

```
borrow_index == position_of_flash_borrow_ix_in_transaction
repay_data[16] == borrow_index
```

**Current default**: `build_kamino_flash_repay` uses `borrow_instruction_index = 2`

**Verification needed**:
1. When `build_kamino_flash_borrow` is called, the resulting transaction has the borrow instruction at index 2 (assuming standard transaction layout: 1 ComputeBudget, 1 tip, 1 borrow, N swaps, 1 repay)
2. If additional instructions are inserted before the borrow, the index changes
3. The system must dynamically determine the borrow index rather than hardcoding 2

**Risk**: Hardcoding `borrow_instruction_index = 2` without verifying the actual transaction layout will cause repay to reference the wrong instruction, leading to flash loan failure or fund loss.

## 4.6 Summary of Phase 4 Findings

### Correctness
- Kamino flash loan instruction builders are well-structured with correct account matrices (12 accounts each)
- Discriminators match the expected Anchor program patterns
- PDA derivations verified (lending market authority bump = 248)
- Fee decoding `decode_reserve_flash_fee_sf` correctly validates discriminator, mint, status before returning fee
- `calculate_kamino_flash_fee` uses exact Q64.64 ceiling math; verified against mainnet sf value
- All 10 existing tests pass

### Issues Identified

1. **No instruction sequence verification**: The code builds borrow and repay instructions independently without verifying that `borrow_instruction_index` matches the actual borrow position in the final transaction.

2. **Hardcoded borrow index = 2**: `build_kamino_flash_repay` defaultes to index 2 without dynamic determination based on the actual transaction layout. This is a potential bug if the transaction structure changes.

3. **No negative testing for borrow index mismatch**: No test verifies that repay references the correct borrow index.

4. **Kamino offsets verified on 2026-11-04**: Must re-verify if program upgrades (offsets may change). The code comments warn: "re-verify these offsets if the program is upgraded."

5. **No instruction order validation**: No test ensures the sequence `borrow → arbitrage → repay` is maintained and atomic.

6. **No real reserve data decoding**: All tests use synthetic fixtures; no end-to-end test against actual on-chain reserve accounts.

### Required Actions

1. **Dynamic borrow index determination**: Replace hardcoded `2` with code that calculates the borrow instruction index based on the actual transaction instruction count.

2. **Add negative tests**: Test borrow index mismatch, mismatched reserve/mint, stale reserve data, overflow in fee calculation.

3. **Implement real reserve decoding**: Test `decode_reserve_flash_fee_sf` against actual on-chain reserve accounts (requires RPC access).

4. **Verify Kamino program offsets**: If program upgrades occur, re-verify all offsets (liquidity_mint @120, mint_decimals @264, status @4848, fee_sf @4896).

4. **Atomic sequence validation**: Add tests that verify the complete `borrow → arbitrage → repay` sequence is atomic and correct.

5. **Document all hardcoded values** with classification (protocol invariant vs operational config vs deprecated).

## 4.7 Hardcoded Value Classification (Kamino-specific)

| Value | Classification | justification |
|---|---|---|
| `KAMINO_KLEND_PROGRAM_ID` | Protocol invariant | Fixed program ID; must match deployed program |
| `KAMINO_MAIN_MARKET` | Protocol invariant | Fixed main market address |
| `KAMINO_WSOL_RESERVE` | Protocol invariant | Fixed WSOL reserve address (main market) |
| `KAMINO_USDC_RESERVE` | Protocol invariant | Fixed USDC reserve address (main market) |
| `FLASH_BORROW_DISCRIMINATOR` | Protocol invariant | Fixed by program discriminator |
| `FLASH_REPAY_DISCRIMINATOR` | Protocol invariant | Fixed by program discriminator |
| `KAMINO_RESERVE_DISCRIMINATOR` | Protocol invariant | Fixed by program discriminator |
| `KAMINO_RESERVE_STATUS_ACTIVE = 0` | Protocol invariant | On-enforced status check |
| `KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET = 8 + 120` | Protocol invariant | Verified offset into reserve account |
| `KAMINO_RESERVE_MINT_DECIMALS_OFFSET = 8 + 264` | Protocol invariant | Verified offset into reserve account |
| `KAMINO_RESERVE_STATUS_OFFSET = 8 + 4848` | Protocol invariant | Verified offset into reserve account |
| `KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET = 8 + 4896` | Protocol invariant | Verified offset into reserve account |
| `LENDING_MARKET_AUTH_SEED = b"lma"` | Protocol invariant | PDA seed for market authority |
| `KAMINO_MAIN_MARKET_AUTHORITY bump = 248` | Protocol invariant | Verified bump from PDA derivation |
| `DEFAULT_MIN_PROFIT_LAMPORTS = 50_000` | Operational configuration | Safety gate; not protocol-enforced |
| `calculate_kamino_flash_fee` formula | Mathematical constant | Q64.64 ceiling division; exact fixed-point math |
| `borrow_instruction_index = 2` | Deprecated/Unverified | Hardcoded without dynamic determination — needs fix |