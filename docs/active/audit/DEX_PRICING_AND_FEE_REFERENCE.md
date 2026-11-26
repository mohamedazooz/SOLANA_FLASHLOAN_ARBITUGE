# DEX Pricing & Fee Reference — Forensic Audit

**Audit date:** 2026-11-26
**Branch:** `feat/forensic-pricing-fee-unification`
**Scope:** Every DEX adapter and quote engine in `crates/arb-dex`, `crates/arb-math`, and `crates/arb-engine`.
**Method:** Static code inspection of the current tree (no reliance on prior reports). Source code is the authority; documentation is descriptive only.

> **Honesty note (revised 2026-11-26, audit branch `vibe/forensic-pricing-fee-audit-1b099c`).**
> Upstream sources were re-fetched and **pinned** via the GitHub API in the
> follow-up session. No live Mainnet differential against a running RPC/SDK
> was performed; the Kamino fee path is the one area now covered by an
> in-repo differential test against a from-scratch replication of the pinned
> upstream algorithm (`crates/arb-math/tests/kamino_klend_differential.rs`).
> The remaining unpinned items are listed in §8.
>
> Pinned upstream SHAs verified in-session:
>
> | Source | Commit (pinned) |
> |---|---|
> | `MeteoraAg/dlmm-sdk` | `1532e540db41ac405dbf4c5bb2ddfceaa1f98ff1` |
> | `orca-so/whirlpools` | `cc683ec92acb5e7606caa3d8e514d9383e9c7c11` |
> | `raydium-io/raydium-cp-swap` | `3d69122790e97cd6aff24e8ba95940793773f58c` |
> | `Kamino-Finance/klend` | `686913ee421e0c5f3dc44c4c8344461ee0e4fb89` (release 1.28.0) |

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

**Verified against `Kamino-Finance/klend` @ `686913ee` —
`state/reserve.rs::ReserveFees::calculate_fees`,
`state/lending_market/lending_operations.rs::flash_borrow_reserve_liquidity`
/`flash_repay_reserve_liquidity`,
`instructions/handler_flash_borrow_reserve_liquidity.rs`,
`instructions/handler_flash_repay_reserve_liquidity.rs`,
`utils/fraction.rs`.**

- `flash_loan_fee_sf` is a **U68F60 scaled fraction** read from the Kamino `Reserve` state, NOT a fixed bps. Mainnet WSOL market: `sf = 11_529_215_046_068` (= 1e-5 = 0.1 bps).
- Upstream formula (Exclusive): `fee_bits = (amount << 60) * sf >> 60` (truncating wide multiply — exactly `amount * sf`); `fee_f = max(fee_bits, 2^60)` (minimum fee = 1 atomic unit, applied BEFORE rounding); `BorrowTooSmall` iff `fee_f >= amount << 60` (raw bits, pre-rounding); `fee = round_half_away(fee_f)`.
- **Referral split is INTERIOR to the one fee, never additive:** upstream computes `(protocol_fee, referral_fee) = calculate_flash_loan_fees(amount, lending_market.referral_fee_bps, referrer_token_state.is_some())` with `referral_fee = floor(fee_f × bps/10^4)` (full fee when bps = 10^4) and `protocol_fee = fee − referral_fee`. With `has_referrer = false` the referral share is structurally **zero**.
- **Repayment handler makes TWO transfers:** `liquidity_amount` (+ `referral_fee` when present) → reserve destination, then `protocol_fee` → `reserve_liquidity_fee_receiver` (fee vault). Total user deduction = principal + THE ONE fee. `flash_repay_checks` enforces `liquidity_amount == borrow_liquidity_amount` and validates the borrow instruction index via the Instructions sysvar — so the repay instruction's `amount` MUST be the principal, never `principal + fee`.
- Repo conformance: every production builder passes `referrer_account`/`referrer_token_state` = `None` (`arb-tx/src/flash/kamino.rs`, `kamino_flash.rs`, `provider.rs`), so `has_referrer = false` upstream and the repo's single-fee charge is exact. Repay builders pass the borrow principal (`arb-tx/src/primitives/builder.rs` `repay_principal = d_star`; `arb-tx/src/flash/dlmm_flash_arb.rs` `params.borrow_amount`).
- Flash loans disabled: `flash_loan_fee_sf == u64::MAX` → upstream `FlashLoansDisabled` revert before fee math; repo maps to `KaminoFeeError::FlashLoanDisabled` (fail closed).
- **Differential test:** `crates/arb-math/tests/kamino_klend_differential.rs` re-implements the upstream algorithm from scratch (wide-integer U68F60 math, min-fee floor, pre-round guard, ties-away rounding, referral split) and asserts bit-exact equality over ~110k+ (amount, fee_sf) combinations including all boundary classes, plus the split-interiority and total-deduction identities. No divergence found.
- **Referral fee accounting:** because all production builders wire `None`, no separate referral-fee ledger entry exists. Should a referrer ever be wired, the referral share MUST be added as a split of the existing single flash fee (never a second deduction) and the `has_referrer` flag must flow into `kamino_flash_fee_*`'s accounting — tracked as an open follow-up.
- Production fee source: `flash_loan_fee_sf` reaches pricing via `VerifiedValue<u64>` decoded from the reserve account (`arb-bot/src/verified_config.rs`), consumed at `supervisor/strategy.rs` and `supervisor/opportunity.rs` with slot-age freshness checks. No fixed bps fallback exists in any production path.
- Different reserves (WSOL vs USDC) have different configs; `select_kamino_flash_reserve` must match flash mint to reserve mint.

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
6. **Opportunity stages are evidence-gated** (`arb-types/src/opportunity_stage.rs`): `QUOTE_VALID` requires quote completeness + freshness + same-slot consistency; `ECONOMICALLY_POSITIVE` requires strictly positive integer net profit ON TOP of valid-quote evidence (a stale/slot-inconsistent/incomplete quote can never rank above `DISCOVERED`, and a successful simulation of a LOSING trade tops out at `QUOTE_VALID`); `EXECUTABLE` requires an explicit builder-success proof (`CandidateOpportunityRecord::builder_succeeded`); `SIMULATED` requires simulation SUCCESS (`SimulationOutcome::Succeeded`), never a mere attempt — `REJECTED_SIM_FAIL` maps to `DISCOVERED`, `STALE_QUOTE` to `DISCOVERED`, `SIMULATED_SUCCESS_NEGATIVE_EDGE` to `QUOTE_VALID`. `SHADOW_VALIDATED`/`MAINNET_VERIFIED` require dedicated proof structures (`ShadowContinuityProof`, `MainnetSettlementProof`) that record fields cannot supply — they are unreachable until shadow-continuity replay and settlement reconciliation produce real proofs.

## 8. Unverified items

- Upstream SHAs above are pinned and statically verified; a live Mainnet
  differential (RPC + deployed program versions + real account fixtures) was
  NOT performed and remains queued (DEX-F-2026-11-25-N2).
- Mainnet differential vectors for Orca adaptive fee, Raydium CLMM dynamic fee, Meteora DLMM limit-order non-zero layers, and PumpSwap creator-fee attribution are queued (see OPEN_ISSUES).
- Live pool decode coverage gaps (DATA-COVERAGE-2026-11-25-N1).
- Referral-fee wiring: verified structurally zero (all builders pass `None`); a split-aware accounting entry is only needed if a referrer is ever wired.
