> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# METEORA DLMM INTEGRATION & ZERO-COPY CPI SPECIFICATION

## 1. Protocol Architecture & Key Mechanics

Meteora Dynamic Liquidity Market Maker (DLMM) organizes pool liquidity into discrete price bins (concentrated liquidity). Swaps occur by traversing active bins in order.

### Core Accounts:
- **`LbPair`**: The central state of the pool, containing `active_id`, `bin_step`, `token_x_mint`, `token_y_mint`, `reserve_x`, `reserve_y`, `protocol_fee`, and `dynamic_fee` state.
- **`BinArray`**: Contiguous slices of 70 bins per account. An active trade accesses up to 3 `BinArray` accounts (active bin array, plus up to 2 adjacent bin arrays depending on trade direction and tick traversal).
- **`EventAuthority` & `MemoProgram`**: Required anchor authority accounts for logging swap events via CPI.

---

## 2. BinArray PDA Derivation

Bin arrays are derived Program Derived Addresses (PDAs) under the Meteora DLMM program (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`):

$$\text{BinArray PDA} = \text{find\_program\_address}\left([\text{"bin\_array"}, \text{LbPair Pubkey}, \text{index as } i64 \text{ LE bytes}], \text{ProgramId}\right)$$

### Bin Index Calculation:
Given an active bin ID $B$ and `BIN_ARRAY_CAPACITY = 70`:
$$\text{bin\_array\_index} = \left\lfloor \frac{B}{70} \right\rfloor$$
When traversing:
- **Swap X -> Y (Price Down)**: Next bin arrays have descending indices ($\text{index} - 1, \text{index} - 2$).
- **Swap Y -> X (Price Up)**: Next bin arrays have ascending indices ($\text{index} + 1, \text{index} + 2$).

The off-chain indexer dynamically determines which `BinArray` accounts must be loaded and passes them into the `DlmmSwapAccountSet`.

---

## 3. Dynamic Fee Decay Math

Meteora DLMM implements dynamic fees that surge during volatility and decay exponentially toward base fee during stability:

$$f_{\text{dynamic}} = f_{\text{base}} + f_{\text{variable}}$$
Where $f_{\text{variable}}$ scales with the volatility accumulator $V_A$:
$$V_A(t) = V_A(t_0) \times \exp\left(-\frac{\Delta t}{\tau}\right) + \Delta \text{bin\_crossings}$$

The off-chain quoter (`arb-dex/src/protocols/meteora_dlmm/mod.rs` and `arb-math`) evaluates the current decayed dynamic fee using exact integer arithmetic matching the on-chain DLMM contract.

---

## 4. On-Chain CPI Invocation (`arb-executor`)

`arb-executor` executes three sequential swaps via genuine CPI to the Meteora DLMM `swap2` instruction:

```rust
// Discriminator: sha256("global:swap2")[..8]
pub const DLMM_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 115, 122, 98, 14, 185];

// Data layout:
// [0..8]: Discriminator
// [8..16]: amount_in (u64 LE)
// [16..24]: min_amount_out (u64 LE)
```

### CPI Account Set (18 Accounts per Swap):
1. `lb_pair` (writable)
2. `bin_array_bitmap_extension` (optional / sentinel)
3. `reserve_x` (writable)
4. `reserve_y` (writable)
5. `user_token_in` (writable)
6. `user_token_out` (writable)
7. `token_x_mint` (readonly)
8. `token_y_mint` (readonly)
9. `oracle` (writable)
10. `host_fee_in` (optional / sentinel)
11. `user` / `authority` (signer, writable)
12. `token_x_program` (readonly)
13. `token_y_program` (readonly)
14. `event_authority` (readonly)
15. `program` (readonly)
16. `bin_array_0` (writable)
17. `bin_array_1` (writable / sentinel)
18. `bin_array_2` (writable / sentinel)

---

## 5. Token-2022 Handling

Meteora DLMM supports standard SPL Token and SPL Token-2022 (including transfer-hook and transfer-fee extensions).
- In `arb-executor`, the correct token program ID (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` vs `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`) is resolved per hop and passed explicitly to CPI.
- Transfer fee adjustments are accounted for in both the quoter and on-chain intermediate balance assertions.
