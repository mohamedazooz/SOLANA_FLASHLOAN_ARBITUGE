> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SPECIFICATION — METEORA

## 1. Supported Programs

| Product | Program ID | Instruction | Architecture |
|---|---|---|---|
| **Meteora DLMM** | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | `Swap2` (Discriminator: `[65, 75, 63, 115, 120, 150, 146, 21]`) | Discrete Liquidity Market Maker with discrete pricing bins |
| **Meteora DAMM / Dynamic Pool** | `Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB` | `Swap` | Constant-product with dynamic dynamic volatility fee |

---

## 2. Meteora DLMM (Discretized Liquidity)

### 2.1 Bin Array Mechanics
- Each bin in DLMM represents a single discrete price level.
- Bins are grouped into `BinArray` accounts of **70 bins each** (`METEORA_BINS_PER_BIN_ARRAY = 70`).
- **Euclidean Bin Array Index Arithmetic:**
  $$\text{bin\_array\_index} = \left\lfloor \frac{\text{bin\_id}}{70} \right\rfloor$$
  Using Euclidean floor division (critical for negative bin IDs: $-1$ belongs to bin array $-1$, not $0$).
- PDA Seed: `["bin_array", lb_pair.as_ref(), &bin_array_index.to_le_bytes()]`.
- Bitmap Extension Account: Required if active bins span beyond $\pm 512$ bins from center.

### 2.2 Account Requirements for `Swap2`
1. `lb_pair`: LB pair state account (`[writable]`)
2. `bin_array_bitmap_extension`: Bitmap extension account (optional / pass program if unused)
3. `reserve_x`: Token X vault (`[writable]`)
4. `reserve_y`: Token Y vault (`[writable]`)
5. `user_token_in`: User token in account (`[writable]`)
6. `user_token_out`: User token out account (`[writable]`)
7. `token_x_mint`: Mint address of Token X
8. `token_y_mint`: Mint address of Token Y
9. `oracle`: LB pair oracle account (`[writable]`)
10. `host_fee_in`: Optional host fee account
11. `user`: Payer / Trader wallet (`[signer]`)
12. `token_x_program`: Token Program (Tokenkeg or Token-2022)
13. `token_y_program`: Token Program (Tokenkeg or Token-2022)
14. `event_authority`: Event authority PDA
15. `program`: Meteora DLMM Program ID
16. *Trailing Bin Arrays:* 1 to 4 `BinArray` accounts covering the active and neighboring bins in trade direction.

### 2.3 Quoting Mathematics
Within an active bin $i$:
$$P(i) = \left(1 + \frac{\text{bin\_step}}{10000}\right)^i$$
- **Swap X to Y (Price Decreasing):**
  Input Token X is deposited into the bin, output Token Y is withdrawn at exact price $P(i)$ until Token Y in the bin is depleted. If $\Delta x$ remains, the active bin decrements to $i - 1$.
- **Swap Y to X (Price Increasing):**
  Input Token Y is deposited, output Token X is withdrawn at $\frac{1}{P(i)}$ until Token X is depleted. If $\Delta y$ remains, active bin increments to $i + 1$.
- **Dynamic Fee:** Total fee includes base fee + variable volatility fee calculated from `volatility_accumulator`.
- Checked integer arithmetic via `ruint::aliases::U256` prevents overflow during bin-crossing iterations.
