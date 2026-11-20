> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SPECIFICATION — PUMPSWAP

## 1. Protocol Identity & Addresses

- **Program ID:** `pmswP5B8eBfW2sEskkbhJkU6cAgqF5U4d2L3vD7GjQW` (constant `PUMP_SWAP_PROGRAM_ID`)
- **Global Config Account:** `PUMP_SWAP_GLOBAL_CONFIG`
- **Pool Account Discriminator:** `0x4b786c268804b901` (`[75, 120, 108, 38, 136, 4, 185, 1]`)
- **Vault Account Types:** SPL Token / Token-2022 Account holding raw token balances.

## 2. On-Chain Account Layout

The PumpSwap `Pool` account header has a minimum length of 245 bytes:

| Field | Offset | Type | Description |
|---|---|---|---|
| `discriminator` | 0 | `[u8; 8]` | Account discriminator |
| `pool_bump` | 8 | `u8` | PDA bump seed |
| `index` | 9 | `u16` | Pool index |
| `creator` | 11 | `Pubkey` | Pool creator authority |
| `base_mint` | 43 | `Pubkey` | Base token mint (graduated token) |
| `quote_mint` | 75 | `Pubkey` | Quote token mint (WSOL or USDC) |
| `lp_mint` | 107 | `Pubkey` | LP token mint |
| `base_vault` | 139 | `Pubkey` | Base token vault account |
| `quote_vault` | 171 | `Pubkey` | Quote token vault account |
| `lp_supply` | 203 | `u64` | Total LP token supply |
| `coin_creator` | 211 | `Pubkey` | Coin creator address |
| `is_mayhem_mode` | 243 | `bool` | Mayhem incentive mode active flag |
| `is_cashback_coin` | 244 | `bool` | Cashback incentive mode active flag |
| `virtual_quote_reserves`| 245 | `i128` | Virtual quote reserve offset |
| `creator_fee_bps` | 261 | `u64` | Creator fee basis points |
| `protocol_fees` | 277 | `u64` | Accumulated un-swept protocol fees |
| `creator_fees` | 285 | `u64` | Accumulated un-swept creator fees |

## 3. Account Separation & Tripartite Join

PumpSwap pools CANNOT be priced from the Pool account alone:
1. **Pool Header:** Supplies pool metadata, vault addresses, and creator fee basis points.
2. **Vault Accounts:** Two distinct SPL Token accounts (`base_vault` and `quote_vault`) supply live reserve balances.
3. **Global Config:** Supplies protocol trade fee basis points and fee recipient addresses.
4. **State Join:** The `PumpSwapJoinedState` joins all three accounts at the exact same slot watermark before quotes can be emitted.

## 4. Quoting Mathematics

PumpSwap executes as a Constant-Product AMM modified by virtual quote reserves:
$$\text{Effective Quote Reserve} = \text{Quote Vault Balance} + \text{Virtual Quote Reserves}$$

### Exact Out with Multi-Tier Fee:
Total fee basis points:
$$\text{fee\_bps} = \text{protocol\_fee\_bps} + \text{creator\_fee\_bps}$$
Given $\Delta x$ input tokens:
$$\Delta x_{\text{net}} = \left\lfloor \frac{\Delta x \times (10000 - \text{fee\_bps})}{10000} \right\rfloor$$
$$\Delta y = \left\lfloor \frac{\text{Reserve}_y \times \Delta x_{\text{net}}}{\text{Reserve}_x + \Delta x_{\text{net}}} \right\rfloor$$
All divisions round down conservatively.
