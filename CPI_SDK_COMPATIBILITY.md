# CPI_SDK_COMPATIBILITY.md: Official SDK, IDL & Protocol Compatibility Specification

**Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`  
**Active Branch**: `feat/meteora-dlmm-swap2-atomic-cpi`  
**Protocol References**:
- Kamino Klend: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- Meteora DLMM: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
- SPL Token: `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`
- Token-2022: `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`
- Memo Program: `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`

---

## 1. Meteora DLMM `swap2` Protocol Architecture

### 1.1 Instruction Interface & Discriminator
Meteora DLMM `swap2` is defined in `fixtures/dlmm.json` (and official Meteora SDK `@mercurial-finance/dlmm`):
- **Anchor Discriminator**: `[65, 75, 63, 76, 235, 91, 91, 136]` (`0x414b3f4ceb5b5b88`)
- **Serialized Parameters**:
  ```text
  Offset 0..8   : Discriminator [u8; 8]
  Offset 8..16  : amount_in: u64 (Little-Endian)
  Offset 16..24 : min_amount_out: u64 (Little-Endian)
  Offset 24..   : RemainingAccountsInfo (Borsh-serialized)
  ```

### 1.2 `RemainingAccountsInfo` & Transfer Hook Slices
```rust
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemainingAccountsInfo {
    pub slices: Vec<RemainingAccountsSlice>,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct RemainingAccountsSlice {
    pub accounts_type: AccountsType,
    pub length: u8,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountsType {
    TransferHookA,
    TransferHookB,
    TransferHookReward,
    TransferHookMultiReward(u8),
    TransferHookReferral,
}
```
When no Token-2022 transfer hooks are required:
- `slices` is empty: serialized as `[0u8, 0, 0, 0]` (length 0).
- Remaining accounts passed after index 15 consist solely of the required `bin_arrays`.

### 1.3 Account Matrix & Mutability Rules
The 16 fixed accounts must follow exact order:
```text
Index | Name                       | Signer | Writable | Description
------+----------------------------+--------+----------+---------------------------------------
  0   | lb_pair                    |   No   |   Yes    | Pool state account
  1   | bin_array_bitmap_extension |   No   |   Yes    | Bitmap extension PDA or sentinel
  2   | reserve_x                  |   No   |   Yes    | Token X vault
  3   | reserve_y                  |   No   |   Yes    | Token Y vault
  4   | user_token_in              |   No   |   Yes    | User source token ATA
  5   | user_token_out             |   No   |   Yes    | User destination token ATA
  6   | token_x_mint               |   No   |    No    | Mint for token X
  7   | token_y_mint               |   No   |    No    | Mint for token Y
  8   | oracle                     |   No   |   Yes    | Observation / oracle PDA
  9   | host_fee_in                |   No   |   Yes    | Host fee account or sentinel
 10   | user                       |  Yes   |    No    | Authority / Owner of user token ATAs
 11   | token_x_program            |   No   |    No    | SPL Token or Token-2022
 12   | token_y_program            |   No   |    No    | SPL Token or Token-2022
 13   | memo_program               |   No   |    No    | MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr
 14   | event_authority            |   No   |    No    | PDA derived from ["__event_authority"]
 15   | program                    |   No   |    No    | LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo
16..  | bin_arrays                 |   No   |   Yes    | Dynamic bin arrays in traversal order
```

**Sentinel Convention**: When optional accounts (index 1: `bin_array_bitmap_extension`, index 9: `host_fee_in`) are not used, Meteora official SDK mandates encoding the DLMM Program ID (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) as a readonly non-signer account.

### 1.4 Bin Array PDA Derivation
Bin arrays are PDAs derived using:
```rust
seeds = [
    b"bin_array",
    lb_pair.as_ref(),
    &bin_array_index.to_le_bytes(),
]
```
Where `bin_array_index = floor(active_bin_id / 70)`.

---

## 2. Kamino Klend Flash Loan Protocol Specification

### 2.1 Program & Discriminators
- **Program ID**: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **FlashBorrow Discriminator**: `[167, 137, 246, 12, 19, 240, 194, 252]`
- **FlashRepay Discriminator**: `[218, 129, 222, 153, 56, 142, 142, 229]`

### 2.2 Instruction Layouts
#### `flash_borrow_reserve_liquidity`:
- Arguments: `liquidity_amount: u64` (Little-Endian)
- Transferred: `liquidity_amount` lamports of reserve liquidity deposited directly into `user_destination_liquidity` ATA.

#### `flash_repay_reserve_liquidity`:
- Arguments:
  ```text
  Offset 0..8  : Discriminator [u8; 8]
  Offset 8..9  : borrow_instruction_index: u8 (0-indexed position of flash_borrow in tx)
  Offset 9..17 : liquidity_amount: u64 (Little-Endian, principal d_star)
  ```
- Repaid: Kamino Klend pulls `liquidity_amount + fee` from `user_source_liquidity` ATA.
- **Critical Invariant**: The `liquidity_amount` parameter passed to `flash_repay` is the **original borrow principal** ($d^*$), NOT the principal + fee. The fee calculation is executed internally by klend, and klend debits $(d^* + \text{fee})$ from the user's account.

### 2.3 Canonical Kamino Flash Loan Fee Math
The klend fee formula implemented in `crates/arb-math/src/economic.rs`:
$$\text{fee\_bits} = \max\left(\text{borrow\_amount} \times \text{flash\_fee\_sf}, 2^{60}\right)$$
$$\text{fee} = \left\lfloor \frac{\text{fee\_bits} + 2^{63}}{2^{64}} \right\rfloor$$
Ties are rounded away from zero. Minimum borrow guard: if `borrow_amount * sf < 2^60`, the transaction fails closed.

---

## 3. Token-2022 & Transfer Hook Compatibility Matrix

| Feature | Supported by Arb Engine | Action / Enforcement |
|:---|:---:|:---|
| **Standard SPL Token** | Yes | Full native support |
| **Token-2022 without extensions** | Yes | Full native support |
| **Token-2022 Transfer Fee** | Restricted | Validated against fee bps. If net output after fee does not satisfy minimum profit, fails closed before broadcast. |
| **Token-2022 Transfer Hook** | Restricted | If transfer hook requires unresolvable accounts or mutable state beyond allowed transaction limits, rejects route (`FailClosed`). |
| **Token-2022 Permanent Delegate** | No | Fails closed (security risk) |
| **Token-2022 Non-Transferable** | No | Fails closed (tokens cannot be swapped) |
