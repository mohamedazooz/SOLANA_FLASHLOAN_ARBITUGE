# DEX Pricing & Fee Reference — Forensic Audit

**Audit date:** 2026-11-26
**Branch:** `feat/forensic-pricing-fee-unification`
**Scope:** Every DEX adapter and quote engine in `crates/arb-dex`, `crates/arb-math`, and `crates/arb-engine`.
**Method:** Static code inspection of the current tree (no reliance on prior reports). Source code is the authority; documentation is descriptive only.

> **Honesty note.** This audit did **not** perform a live Mainnet differential
> against official protocol SDKs in this session. Commit SHAs below are left as
> "unpinned" where they could not be verified from the local checkout. A
> follow-up pass must pin them and record exact SDK versions used for differential
> vectors (see `OPEN_ISSUES.md` DEX-F-2026-11-25-N2).

## 1. Common quote contract (`crates/arb-dex/src/quote_engine.rs`)

All adapters implement `QuoteEngine` with:

- `quote_exact_in(state, request) -> QuoteResult`
- `quote_exact_out(state, request) -> QuoteResult`

`QuoteResult` carries `amount_in_raw`, `amount_out_raw`, `minimum_amount_out_raw`,
`total_fee_mint`, `creator_fee_mint`, `fees_breakdown`, `transfer_fee_breakdown`,
`price_impact`, `unfilled_input_raw`, `required_accounts`, `state_slot`,
`state_hash`, `calculation_version`. `QuoteRequest::validate` enforces:
pool active, non-zero amount, slippage ≤ 10000 bps, distinct mints, mint
orientation matches pool, risky-token rejection, family match, Token-2022
schedule presence/absence consistency, transfer-fee bps ≤ 10000.

**Invariant (enforced by tests, not by construction):** if `amount_out_raw` already
reflects DEX fees + Token-2022 output transfer fee, the profit engine MUST NOT
subtract them again. See `economic.rs` and the fee-ledger section of
`HARDCODED_VALUES_AUDIT.md`.

## 2. Protocol table

| Protocol | Pricing type | Price source | Fee computation | Fee deduction side | Rounding | Required accounts | Support status |
|---|---|---|---|---|---|---|---|
| Meteora DLMM (`meteora_dlmm.rs`, `engines/meteora_dlmm.rs`, `arb-math/src/dlmm/*`) | Bin traversal (active bin + neighbours via bitmap) | `LbPair.active_bin_id`, bin arrays, bitmap extension | Base fee + dynamic fee (volatility accumulator) + protocol share; `collect_fee_mode` handled in decoder | Fee on input (swap into pool) | Floor/ceil per bin per official `get_exact_in_fill_amount_result` | Pool, bin arrays, bitmap extension, vaults, token programs, oracle (if needed) | Quote + execution gated; Token-2022 path fail-closed unless verified |
| Orca Whirlpool (`orca_whirlpool.rs`, `engines/orca_whirlpool.rs`, `arb-math/src/clmm.rs`) | CLMM tick-step | `sqrt_price` Q64.64, current tick, active liquidity, tick arrays | Per-step fee rate + adaptive fee + protocol fee | Fee on input; protocol fee split | Per official swap_math rounding | Pool, tick arrays, oracle (adaptive), vaults | Quote-only where adaptive fee state unverified |
| Raydium CPMM (`raydium_cpmm.rs`, `engines/raydium_cpmm.rs`, `arb-math/src/cpmm.rs`) | CPMM `x*y=k` | Reserves (vaults minus accrued fees) | `trade_fee_rate` / 1e6 (exact); protocol/fund/creator split from AmmConfig | Fee on input | Floor per SDK v2 calculator | Pool, AmmConfig, vaults, mints | Differential-tested (54 vectors) |
| Raydium CLMM (`raydium_clmm.rs`, `engines/raydium_clmm.rs`) | CLMM tick-step | sqrt price, tick, liquidity, tick arrays | `AmmConfig.trade_fee_rate` / 1e6; dynamic fee if present | Fee on input; protocol fee | Per official math | Pool, AmmConfig, tick arrays, bitmap extension, vaults | **NOT priceable** — AmmConfig not live-joined (DEX-F-2026-11-25-N3) |
| Raydium V4 (`raydium_v4.rs`, `engines/raydium_v4.rs`) | Hybrid orderbook/AMM | Reserves | Config fee numerator/denominator | Fee on input | Floor | Pool, vaults, mints, open-orders (if applicable) | Quote-only / fail-closed where vaults undecodable |
| Meteora DAMM v2 (`meteora_damm_v2.rs`, `engines/meteora_damm_v2.rs`, `arb-math/src/damm_v2.rs`) | CPMM/CLMM family by `is_compounding` | Reserves + config | Dynamic fee (volatility) + base fee; compounding fee added to reserve B | Fee on input | Per official DAMM v2 | Pool, config, vaults, mints | Quote pipeline present; execution gated |
| Meteora DBC (`meteora_dbc.rs`) | Bonding curve | Curve state | Swap fee from config | Fee on input/output per curve | Floor | Pool, config, vaults | Quote-only, not executable |
| Pump.fun bonding curve (`pump_fun.rs`, `engines/pump_bonding_curve.rs`) | Bonding curve `k` | Curve reserves | Flat 1.25% (125 bps) per official | Buy fee on input (SOL), sell fee on output (SOL) | Floor | Bonding curve, global, fee recipient | Quote + execution gated (Token-2022 rejected) |
| PumpSwap (`pump_swap.rs`, `engines/pump_swap.rs`) | CPMM-style | Reserves | LP/protocol/creator PPM (exact) | Direction-correct placement | Floor + 2-raw-unit margin | Pool, fee config, vaults | Quote-only |
| Orca (Whirlpool) adaptive fee | CLMM tick-step | As above + oracle | Adaptive fee from oracle EMA | Fee on input | Floor | + oracle account | Fail-closed if oracle missing |

## 3. Kamino flash loan fee (`arb-math/src/economic.rs`, re-exported by `arb-tx`)

- `flash_loan_fee_sf` is a **scaled fraction** (sf) read from the Kamino `Reserve` state, NOT a fixed bps.
- Conversion: `fee = max(amount * sf / 2^frac_bits, 2^60)` style scaled-fraction math; the `BorrowTooSmall` guard compares **raw bits pre-rounding**.
- Repayment = principal + fee (exact). Never assume a global fee.
- Different reserves (WSOL vs USDC) have different configs; `select_kamino_flash_reserve` must match flash mint to reserve mint.
- Flash loans disabled state must be checked.

## 4. Solana network costs

- Base fee: 5000 lamports per signature (legacy/V0).
- Priority fee: `CU_limit * CU_price / 1e6` (micro-lamports), using the **limit** requested in the message, not consumed units.
- Compute budget instructions: `SetComputeUnitLimit`, `SetComputeUnitPrice`, `SetLoadedAccountsDataSizeLimit`.
- Transaction format: V1 wire (0x81 discriminator, 4096-byte ceiling, 64 inline accounts, no ALT). Never silently downgrade.

## 5. Token-2022 transfer fees

- `TransferFeeConfig` is epoch-aware; missing schedule = fail closed (not zero fee).
- Input transfer fee applied **before** venue math; output transfer fee applied **after**.
- Transfer hooks: unsupported until exact account resolution (fail closed).
- See `crates/arb-dex/src/token2022.rs`.

## 6. Jito tip

- Tip is **not** a fixed constant. It is only charged when a tip instruction is
  present or an explicit expected-cost is recorded for the opportunity.
- Tip accounts must stay inline (not ALT-loaded) — see `OPEN_ISSUES.md` TX-CORR-2026-11-23-N1.

## 7. Differences between protocol model and project implementation

1. **Raydium CLMM** is deliberately non-priceable pending AmmConfig live-join (safety gate). Discovery still emits metadata but exact quote returns `Unsupported`.
2. **Token-2022** pools are excluded from the live pilot path; standalone quote helpers exist but are not wired end-to-end with epoch-aware mint data.
3. **`MarketEdge.fee_bps`** is an integer BPS ceiling; exact sub-bps ratios (Raydium CPMM `trade_fee_rate` / 1e6) are preserved only inside the adapter, not in graph metadata. This may under-rank opportunities (conservative).
4. **DLMM two-pool engine** (`dlmm_flash_arbitrage.rs`) is test/example only and lacks on-chain retained-profit floor (FL-F-2026-11-22-N2). Must not be wired without the floor.
5. **Bellman-Ford** uses f64 only as a pre-screen; authoritative pricing/sizing uses integer math. This boundary must be preserved.

## 8. Unverified items

- Official SDK commit SHAs were not re-fetched in this session.
- Mainnet differential vectors for Orca adaptive fee, Raydium CLMM dynamic fee, Meteora DLMM limit-order non-zero layers, and PumpSwap creator-fee attribution are queued (see OPEN_ISSUES).
- Live pool decode coverage gaps (DATA-COVERAGE-2026-11-25-N1).
