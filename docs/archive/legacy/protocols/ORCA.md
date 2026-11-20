> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SPECIFICATION — ORCA WHIRLPOOLS

## 1. Protocol Identity & Program Addresses

- **Program ID:** `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`
- **SDK Dependency:** `orca_whirlpools_core v2.1.1` & `orca_whirlpools_macros v1.0.5`
- **Discriminator:** `[248, 198, 158, 145, 225, 117, 135, 200]` (`global:swap`)
- **Tick Array Size:** 88 ticks per `TickArray` account

## 2. On-Chain Whirlpool Account Layout

The Whirlpool account is 653 bytes:

| Field | Offset | Type | Description |
|---|---|---|---|
| `discriminator` | 0 | `[u8; 8]` | Account discriminator |
| `whirlpools_config` | 8 | `Pubkey` | Global whirlpool config address |
| `whirlpool_bump` | 40 | `[u8; 1]` | PDA bump |
| `tick_spacing` | 41 | `u16` | Tick spacing (e.g. 1, 8, 64, 128) |
| `tick_spacing_seed` | 43 | `[u8; 2]` | Tick spacing seed bytes |
| `fee_rate` | 45 | `u16` | Fee rate in hundredths of a basis point (1/1,000,000) |
| `protocol_fee_rate` | 47 | `u16` | Protocol cut of trade fee |
| `liquidity` | 49 | `u128` | Active liquidity $L$ at current tick |
| `sqrt_price` | 65 | `u128` | Sqrt price in Q64.64 fixed point |
| `tick_current_index`| 81 | `i32` | Current active tick index |
| `protocol_fee_owed_a`| 85 | `u64` | Uncollected protocol fee token A |
| `protocol_fee_owed_b`| 93 | `u64` | Uncollected protocol fee token B |
| `token_mint_a` | 101 | `Pubkey` | Mint address of Token A |
| `token_vault_a` | 133 | `Pubkey` | Vault address of Token A |
| `token_mint_b` | 181 | `Pubkey` | Mint address of Token B |
| `token_vault_b` | 213 | `Pubkey` | Vault address of Token B |
| `reward_infos` | 261 | `[RewardInfo; 3]` | Liquidity mining rewards |

## 3. Account Requirements for Swap Instruction (11 Accounts)

1. `token_program`: SPL Token or Token-2022 Program ID
2. `token_authority`: Trader wallet (`[signer]`)
3. `whirlpool`: Whirlpool pool state account (`[writable]`)
4. `token_owner_account_a`: User Token A account (`[writable]`)
5. `token_vault_a`: Pool Token A vault (`[writable]`)
6. `token_owner_account_b`: User Token B account (`[writable]`)
7. `token_vault_b`: Pool Token B vault (`[writable]`)
8. `tick_array_0`: Active/first tick array (`[writable]`)
9. `tick_array_1`: Second tick array in trade direction (`[writable]`)
10. `tick_array_2`: Third tick array in trade direction (`[writable]`)
11. `oracle`: Whirlpool oracle account

## 4. Quoting Mathematics & Directional Invariants

- **A to B Swap:** Token A in $\to$ Token B out. Price $\sqrt{P}$ decreases, tick index decreases.
  - Tick arrays must be ordered descending by start tick index.
- **B to A Swap:** Token B in $\to$ Token A out. Price $\sqrt{P}$ increases, tick index increases.
  - Tick arrays must be ordered ascending by start tick index.
- **Cross-Tick Iteration:**
  Within a single tick step:
  $$\Delta x = \Delta \left(\frac{L}{\sqrt{P}}\right) = L \times \frac{\sqrt{P}_{\text{upper}} - \sqrt{P}_{\text{lower}}}{\sqrt{P}_{\text{upper}} \times \sqrt{P}_{\text{lower}}}$$
  $$\Delta y = \Delta (L \sqrt{P}) = L \times (\sqrt{P}_{\text{upper}} - \sqrt{P}_{\text{lower}})$$
  When active tick crosses an initialized tick boundary, $L \gets L \pm \text{liquidity\_net}$.
- **Fee Calculation:** Fee is charged on input amount per step using `fee_rate / 1_000_000`.
