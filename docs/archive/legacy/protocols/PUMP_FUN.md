> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SPECIFICATION — PUMP.FUN

## 1. Protocol Identity & Addresses

- **Program ID:** `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`
- **Global Config PDA:** `4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf` (seeds: `["global"]`)
- **Fee Recipient:** `CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM`
- **Bonding Curve PDA:** `["bonding-curve", token_mint.as_ref()]`
- **Bonding Curve Associated Token Account:** Derived via Associated Token Program for the bonding curve PDA and token mint.

## 2. On-Chain Account Layout

The Pump.fun `BondingCurve` account layout is 83 bytes:

| Field | Offset | Type | Description |
|---|---|---|---|
| `discriminator` | 0 | `[u8; 8]` | `0x17b7f83760d8ac60` (`[23, 183, 248, 55, 96, 216, 172, 96]`) |
| `virtual_token_reserves` | 8 | `u64` | Virtual token supply in atomic units (6 decimals) |
| `virtual_sol_reserves` | 16 | `u64` | Virtual SOL reserves in lamports |
| `real_token_reserves` | 24 | `u64` | Actual tokens remaining on curve available for sale |
| `real_sol_reserves` | 32 | `u64` | Actual SOL lamports accumulated on curve |
| `token_total_supply` | 40 | `u64` | Total token supply (typically 1,000,000,000 * 10^6 = 10^15) |
| `complete` | 48 | `bool` | True when curve reaches graduation threshold; trading halts |
| `creator` | 49 | `Pubkey` | Token creator wallet address (32 bytes) |
| `is_mayhem_mode` | 81 | `bool` | Optional protocol mode flag (V2) |
| `is_cashback_coin` | 82 | `bool` | Optional protocol cashback flag (V2) |

## 3. Mathematical Quoting Model

The bonding curve functions as a virtual Constant-Product AMM:
$$k = \text{virtual\_sol} \times \text{virtual\_token}$$

### 3.1 Buy Quote (Exact SOL In $\to$ Tokens Out)
Given $\Delta \text{sol}$ input lamports:
1. Apply protocol fee $f$ (typically 100 bps / 1%):
   $$\Delta \text{sol}_{\text{net}} = \Delta \text{sol} \times \frac{10000 - f}{10000}$$
2. Compute new virtual reserves:
   $$\text{virtual\_sol}_{\text{new}} = \text{virtual\_sol} + \Delta \text{sol}_{\text{net}}$$
   $$\text{virtual\_token}_{\text{new}} = \left\lfloor \frac{k}{\text{virtual\_sol}_{\text{new}}} \right\rfloor$$
   $$\Delta \text{token} = \text{virtual\_token} - \text{virtual\_token}_{\text{new}}$$
3. Bound check: $\Delta \text{token} \le \text{real\_token\_reserves}$.

### 3.2 Sell Quote (Exact Tokens In $\to$ SOL Out)
Given $\Delta \text{token}$ input tokens:
1. Compute new virtual reserves:
   $$\text{virtual\_token}_{\text{new}} = \text{virtual\_token} + \Delta \text{token}$$
   $$\text{virtual\_sol}_{\text{new}} = \left\lfloor \frac{k}{\text{virtual\_token}_{\text{new}}} \right\rfloor$$
   $$\Delta \text{sol}_{\text{gross}} = \text{virtual\_sol} - \text{virtual\_sol}_{\text{new}}$$
2. Apply protocol fee:
   $$\Delta \text{sol}_{\text{net}} = \left\lfloor \frac{\Delta \text{sol}_{\text{gross}} \times (10000 - f)}{10000} \right\rfloor$$
3. Bound check: $\Delta \text{sol}_{\text{gross}} \le \text{real\_sol\_reserves}$.

## 4. Graduation & Migration Behavior

1. **Completion Trigger:** When $\text{real\_sol\_reserves} \ge 85\text{ SOL}$, the `complete` flag is set to `true`.
2. **Trading Halt:** All subsequent `Buy` and `Sell` instructions on the bonding curve fail with program error `0x1775` (`BondingCurveComplete`).
3. **Migration:** Remaining tokens (206,900,000 tokens) and accumulated SOL are transferred to seed a newly created AMM pool (Raydium or PumpSwap).
4. **Race Condition Prevention:** An arbitrageur MUST NOT assume the new AMM pool is immediately readable upon `CompleteEvent`. The pool account creation and initialization transaction must confirm on-chain before valid AMM quotes can be generated.
