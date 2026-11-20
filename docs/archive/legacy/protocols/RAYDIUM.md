> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SPECIFICATION — RAYDIUM

## 1. Supported Market Types & Programs

Raydium encompasses three distinct market architectures:

| Protocol Type | Program ID | Instruction / Opcode | Key Characteristics |
|---|---|---|---|
| **Raydium V4 (AmmV4)** | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Opcode `9` (SwapBaseIn) or `16` (SwapV2) | Classic CPMM integrated with OpenBook order book market |
| **Raydium CPMM (CP-Swap)** | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Discriminator `0x8fbe6ad4cdda2c5d` | Modern standard CPMM with Token-2022 support and AmmConfig fee tiers |
| **Raydium CLMM** | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Discriminator `swap` or `swap_v2` | Concentrated liquidity with directional tick arrays and sqrt-price math |

---

## 2. Raydium CPMM (CP-Swap)

### 2.1 Account Requirements (13 Accounts for `swap_base_input`)
1. Payer / Trader wallet (`[signer, writable]`)
2. Authority PDA (`vault_and_lp_mint_auth_seed`)
3. AmmConfig Account (`D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2` or pool config)
4. Pool State Account (`[writable]`)
5. User Input Token Account (`[writable]`)
6. User Output Token Account (`[writable]`)
7. Input Token Vault (`[writable]`)
8. Output Token Vault (`[writable]`)
9. Input Token Program (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` or Token-2022)
10. Output Token Program (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` or Token-2022)
11. Input Token Mint
12. Output Token Mint
13. Observation State Account (`[writable]`, seeds: `["observation", pool_id]`)

### 2.2 Quoting Mathematics
Constant-Product invariant with trade fee deducted from input:
$$\text{amount\_in\_net} = \left\lfloor \frac{\text{amount\_in} \times (10000 - \text{fee\_bps})}{10000} \right\rfloor$$
$$\text{amount\_out} = \left\lfloor \frac{\text{reserve\_out} \times \text{amount\_in\_net}}{\text{reserve\_in} + \text{amount\_in\_net}} \right\rfloor$$

---

## 3. Raydium CLMM (Concentrated Liquidity)

### 3.1 Account Requirements & Tick Arrays
- CLMM pools represent liquidity concentrated within price intervals.
- The `swap` instruction requires up to 3 contiguous initialized `TickArray` accounts in the direction of the trade:
  - If selling Token 0 for Token 1 (price decreasing), tick arrays must descend.
  - If selling Token 1 for Token 0 (price increasing), tick arrays must ascend.
- PDA Seed: `["tick_array", pool_id.as_ref(), &start_tick_index.to_be_bytes()]`.
- Incorrect tick array order or missing active tick arrays causes on-chain abort `0x177a`.

### 3.2 Quoting Mathematics
- Uses Q64.64 fixed-point representation for $\sqrt{P}$.
- When active tick crosses a boundary, liquidity changes by $\pm \Delta L$.
- Checked integer arithmetic avoids floating-point precision loss.

---

## 4. Raydium AMM V4 (Legacy / OpenBook)

- Interacts directly with OpenBook market accounts (Bids, Asks, Event Queue, Open Orders).
- Requires canonical account order: AMM Pool, Authority, Open Orders, Target Orders, Coin Vault, Pc Vault, OpenBook Market, Bids, Asks, Event Queue, Coin Vault, Pc Vault, Vault Signer PDA.
