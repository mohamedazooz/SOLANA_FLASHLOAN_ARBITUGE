# DEX Pricing & Fee Reference — Protocol Math vs. Repo Implementation

Date: 2026-11-26
Baseline: `main` at `9e389e4` (merge of PR #175). Audit branch: `vibe/forensic-pricing-fee-audit-1b099c`.
Author: forensic pricing/fee audit pass (external reference research + repo trace).

**Honesty boundary.** This document records what was actually verified by reading
upstream source at the cited commits and comparing it with this repository's
code and test suites. It does NOT claim live-mainnet parity for any venue.
"Support status" reflects the repo's own fail-closed gating
(`crates/arb-dex/src/engines/mod.rs`, `UniversalQuoteDispatcher`), not a
marketing claim.

---

## 1. Per-protocol pricing & fee matrix

| البروتوكول | نوع التسعير | مصدر السعر | طريقة احتساب الرسوم | مكان خصم الرسوم | التقريب | الحسابات المطلوبة | حالة الدعم |
|---|---|---|---|---|---|---|---|
| Meteora DLMM | Bin ladder exact (discrete bins, Q64.64 bin price) | Active bin id + bin arrays; `get_price_from_id` (`arb-math/src/dlmm/`) | Per-bin `compute_total_fee_rate` = base fee (`base_fee_power_factor`) + variable fee (`variable_fee_control × (VA × bin_step)²` / 1e11, ceil); protocol fee split from `fee_amount × protocol_share_bps` | On input token when `collect_fee_mode=0` (fee-on-input for X→Y in that mode); fee taken per bin during traversal, included in `amount_out` | Fee rate ceil; amounts integer, protocol-mirroring | LbPair, BinArrays (ordered), BitmapExtension when active bin at edge, StaticParameters, VariableParameters, oracle for fresh VA timestamp | Executable (`swap2`); math parity suites PASS |
| Meteora DAMM v2 | Concentrated sqrt-price curve (Q64.64) or tracked-reserve constant product for `Compounding` | Pool account (V1 layout: token amount counters 680..696, layout version at 696) | Fee scheduler: `fee_numerator` over 1e9 denominator, max 50% (V0) / 99% (V1); dynamic fee from scheduler state | On input; A→B next-sqrt rounds UP, B→A rounds DOWN; compounding mode uses tracked reserves not sqrt curve | Fee ceil (1e9); sqrt-price directional rounding | Pool, token mints (T22 extensions), vaults, Clock (slot- or ts-based activation), fee scheduler state | Discovery/quote only — FAIL-CLOSED for execution (hydration returns no executable account set; see `docs/active/audit/UNIFIED_SWAP_PARITY_AUDIT_2026-11-26.md` §2) |
| Raydium CPMM (CP-Swap) | Constant product `x·y=k`, PPM fee denominator | PoolState reserves + `AmmConfig` | Trade fee on input (`fee_rate/1e6`); creator fee on input XOR output per config; protocol fee & fund fee are SUB-ALLOCATIONS of the trade fee (not additive) | Trade fee taken from `amount_in` before curve; protocol/fund/creator portions move to their vaults, only LP share stays in reserves | Curve floor; fee ceil sub-allocations | PoolState (333 B), AmmConfig, authority PDA, both vaults, observation state, token programs | Executable; 54/54 exact-in vectors match Raydium TS SDK 0.2.87-alpha; creator-fee modes synthetic (mainnet pool has creator fee disabled); exact-out with creator fee / T22 fail-closed |
| Raydium CLMM | Tick-crossing sqrt-price curve | Pool sqrt_price + tick arrays + AmmConfig | Fee on input per step (`fee_rate/1e6`), Raydium dynamic fee from `DynamicFee` struct; protocol fee is a share of the trade fee | Per-step from remaining input before liquidity math | Amount-out floor, next-sqrt floor per Raydium `swap_math` | Pool (273 B), AmmConfig (owner-verified), both vaults, tick arrays, bitmap extension, observation state | FAIL-CLOSED in production: telemetry-only until same-slot AmmConfig + dynamic-fee snapshot verified (`UniversalQuoteDispatcher` rejects). Math parity suites PASS in isolation |
| Raydium AMM v4 | Constant product with OpenBook market scaffolding | AMM account reserves (with market orderbook fields) | AMM fee fields from the decoded account (`fees` struct), never a fixed 25 bps | On input, per program's swap logic | Curve floor | AMM account, authority, market accounts (bids/asks/queue/vaults), target orders | Executable for decoded snapshots; invalid/missing fee state is fail-closed |
| Orca Whirlpool | Tick-crossing sqrt-price curve (Q64.64) | Whirlpool sqrt_price + tick arrays | Fee on input per step (`fee_rate/1e6`); protocol fee is a configured share of earned fee; adaptive-fee state in WhirlpoolsConfigExt must be honored when present | Per-step from remaining input; protocol fee moved to config vault | Floor on out / next-sqrt per `swap_math.rs` | Whirlpool (245 B), config, both vaults, tick arrays (88-tick spacing blocks), oracle, token programs | Executable; `orca_sdk_parity` + mainnet differential suites PASS |
| Pump.fun | Bonding curve virtual reserves (constant product on virtual reserves) | Bonding curve account (virtual/real reserves, complete flag) | 125 bps fee (from pool's verified `fee_bps`, not hardcoded) on buy AND sell; mayhem/cashback modes unsupported ⇒ fail-closed | On input SOL and input token per direction | Curve ceil for cost | Bonding curve account, global/volume PDA config, event authority, mint | Executable for `complete=false`, non-mayhem, non-cashback curves |
| PumpSwap | Constant product post-graduation pool | Pool account reserves + coin/Pi vaults | Fee from live `Pool`/`GlobalConfig` fee ratios; missing ratio ⇒ fail-closed (no invented split); base-fee-only quote for generic `PoolState` | On input | Curve floor | Pool, global config, coin vault, Pi vault, mint(s) | Executable with verified fee config; generic PoolState quote carries total fee only |
| Kamino flash loans (klend) | Not a swap — liquidity borrow/repay | Reserve account `ReserveConfig.fees.flash_loan_fee_sf` (U68F60) | `fee_bits = max(amount × sf, 2^60)`, ties away from zero (klend `ReserveFees::calculate_fees`); `sf == u64::MAX` ⇒ flash loans disabled on that reserve | Repay transfer = principal + fee, exactly, in the borrowed mint | Ties-away on U68F60; BorrowTooSmall guard on RAW bits pre-rounding | Reserve, lending market, market authority, liquidity mint/vault, obligation/user ATA (token-program-aware), Instructions sysvar for borrow-index introspection | Executable; canonical implementation in `arb-math/src/economic.rs::calculate_kamino_flash_fee`; per-reserve sf extracted from live account, never a global % |
| Solana runtime (fees) | N/A | Transaction content + `ComputeBudget` config (V1: message config mask, not CU instructions) | Base fee = signatures × 5000 lamports (`SOLANA_BASE_FEE_LAMPORTS`); priority fee = CU-price × CU-limit (the transaction's own limit, not observed usage); V1 (SIMD-0385): priority fee as absolute lamports in message mask bits [0,1] | Deducted from signer lamports; encoded in compute-budget instructions (legacy) or message config (V1) | Integer lamports | Fee payer signer, blockhash, ComputeBudget/System config | Wired; `arb-tx/src/transport/adaptive.rs` V1 config-mask encoding |
| Jito | N/A — bundle auction | Tip floor market data / configured strategy (`tip_strategy`, TipOracle) | Tip = explicit `SystemTransfer` to a canonical tip account, inline (never ALT-loaded); amount from policy, not a fixed constant | Tip instruction inside the bundle; bundle is all-or-nothing, 1..=5 txs | Integer lamports | 8 canonical tip accounts (`arb_constants::JITO_TIP_ACCOUNTS`), block engine URL | Wired; tip accounts filtered out of ALTs (regression-tested) |
| Token-2022 transfer fees | N/A — mint extension | Mint `TransferFeeConfig` for the execution epoch | `transfer_fee_bps` + `maximum_fee` from the epoch's config; epoch selection uses Clock epoch, never wall-clock | Withheld on transfer in/out of the pool; represented in `transfer_fee_breakdown` of every `QuoteResult` | Protocol: ceil on fee, floor on net | Mint accounts (both), epoch, token program ids | Fail-closed in production routing: routes with T2x2 transfer fees or risky extensions are rejected unless a verified per-mint epoch schedule is supplied |

---

## 2. Upstream sources inspected (commits)

| Protocol | Repo | Commit inspected | Key files |
|---|---|---|---|
| Meteora DLMM | [MeteoraAg/dlmm-sdk](https://github.com/MeteoraAg/dlmm-sdk) | `1532e540db41ac405dbf4c5bb2ddfceaa1f98ff1` (head 2026-11-04) | `commons/src/quote.rs`, `ts-client/src/dlmm/helpers/fee.ts` |
| Orca Whirlpools | [orca-so/whirlpools](https://github.com/orca-so/whirlpools) | `cc683ec92acb5e7606caa3d8e514d9383e9c7c11` (head 2026-10-12) | `programs/whirlpool/src/math/swap_math.rs` |
| Raydium CPMM | [raydium-io/raydium-cp-swap](https://github.com/raydium-io/raydium-cp-swap) | `3d69122790e97cd6aff24e8ba95940793773f58c` (head 2026-11-01) | `programs/cp-swap/src/curve/constant_product.rs`, fee modules |
| Kamino klend | [Kamino-Finance/klend](https://github.com/Kamino-Finance/klend) | `686913ee421e0c5f3dc44c4c8344461ee0e4fb89` (release 1.28.0) | `handlers/handler_flash_borrow_reserve_liquidity.rs`, `state/reserve.rs` |
| Solana fees / V1 | [solana.com/docs](https://solana.com/docs/core/fees) + SIMD-0385 | docs as of 2026-11 | compute-budget, transfer-fee extension docs |
| Jito | [docs.jito.wtf/bundles](https://docs.jito.wtf/bundles/) | docs as of 2026-11 | bundle semantics, tip accounts |

The Raydium CLMM repo head was not re-pinned in this pass; the repo's CLMM
math is validated against its committed parity suites, and the live path is
fail-closed regardless (see row above). Raydium AMM V4's program is closed for
this audit; the repo quotes only from decoded on-chain AMM fee fields.

## 3. Repo vs. protocol deltas found in this pass

1. **No double fee deduction found in the canonical chain.** DLMM cross-pool
   sizing embeds DEX fees in exact bin-traversal outputs and adds only the
   flash fee + base fee + tip on top (`crates/arb-engine/src/cross_pool_dlmm.rs`
   `optimize_size` — "DEX fees are already embedded" is enforced by the
   `calculate_net_profit_raw_signed` call passing zero DEX-fee adders).
2. **Fee splits are never stacked over the total.** `FeesBreakdown` carries
   `total_fee` as authoritative; protocol/fund/creator are sub-allocations.
   CPMM route advancement subtracts non-LP fee buckets from the curve credit
   exactly once (`crates/arb-engine/src/adapters.rs::advance_pool_after_quote`).
3. **Legacy test-only constants exist and are quarantined to tests:**
   `arb_math::pump_fun_raydium::{DEFAULT_FLASH_LOAN_FEE_BPS=30,
   DEFAULT_RAYDIUM_FEE_BPS=25}` are referenced only from `#[cfg(test)]` code
   (`crates/arb-dex/src/pump_fun.rs` tests, `arb-math` tests). Production
   pricing never consumes them. Recorded in `HARDCODED_VALUES_AUDIT.md`.
4. **Raydium CLMM discovery fallback** still carries a structural
   `fee_bps: 25` projection for graph ranking (registered gap
   DEX-F-2026-11-22-N2); the operative quote path rejects CLMM entirely, so it
   cannot fabricate profit.
5. **DAMM v2 / DBC / LaunchLab** remain quote-only or unsupported in the
   dispatcher — fail-closed, matching the unified-parity audit's gates.

## 4. Not verified in this pass (honest gaps)

- No live-mainnet differential quotes were run (no RPC egress credentials in
  this environment; `.env` absent). All parity statements rest on committed
  fixtures and the repo's differential suites listed in
  `docs/active/audit/QUOTE_PARITY_REPORT.md`.
- Upstream heads move; the commits above were heads on 2026-11-26. A future
  audit must re-pin and re-diff before making new parity claims.
- Token-2022 transfer-hook account resolution is unimplemented upstream of the
  quote layer; such mints remain non-executable by policy.
- Jito V1 bundle support requires upstream verification (recorded in
  `CURRENT_STATUS.md`).
