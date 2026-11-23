# EXTERNAL_BEST_PRACTICES — Protocol Research Register

**Mission**: Atomic Profitability & Execution Engineering
**Research date**: 2026-11-21
**Method**: Every finding below was checked against the cited URL during this session. Only findings actually observed in the cited source are recorded; items not inspected are explicitly marked `NOT INSPECTED`. No open-source implementation was copied into production.

---

## 1. Solana Transaction V1 (SIMD-0385 / SIMD-0296)

| Item | Finding | Checked |
|---|---|---|
| Spec status | **Activated** | [proposals/0385-transaction-v1.md](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md) (raw fetched 2026-11-21) |
| Activation | Mainnet epoch 1035, activation slot 447,120,000, ~2026-09-15T01:00 UTC | [solana.com/upgrades/larger-transaction-sizes](https://solana.com/upgrades/larger-transaction-sites) via [solana.com upgrade page](https://solana.com/upgrades/larger-transaction-sizes), 2026-11-21 |
| Wire format | VersionByte `129` (0x81); LegacyHeader (3 bytes); TransactionConfigMask (u32 LE); LifetimeSpecifier (32 B); NumInstructions (u8); NumAddresses (u8); Addresses (inline 32 B each); ConfigValues (4 B per set bit); InstructionHeaders (4 B each: program index u8, account count u8, data len u16 LE); contiguous payloads; **trailing signatures** | SIMD-0385 spec text |
| Priority fee | Config mask bits **[0,1] both set**, value = 8-byte LE u64 = **absolute total in lamports** (NOT micro-lamports/CU). If only one bit is set the transaction is invalid. | SIMD-0385 |
| Unset resource requests | Priority fee 0; **CU limit 0 (transaction fails immediately)**; loaded-accounts-data limit 0 (cost model reserves 32 KiB page); heap 32 KiB | SIMD-0385 |
| Heap size | MUST be a 1 KiB multiple in [32 KiB, 256 KiB]; otherwise sanitization failure | SIMD-0385 |
| Limits | 4,096-byte size, 12 signatures, **64 accounts**, **64 instructions**, 255 accounts/ix | SIMD-0385 constraints table |
| ALTs | **Not supported in v1** — all addresses inline (32 B each) | SIMD-0385 §Detailed Design |
| ComputeBudget instructions | Ignored for configuration in TxV1 (processed as successful no-ops that still consume 150 CU) → must be stripped from V1 messages and any instruction-index-dependent CPI data (e.g. klend `borrow_instruction_index`) rewritten against the executed list | SIMD-0385 + [solana.com v1 announcement](https://solana.com/news/transaction-v1-and-the-alt-trade-off) |
| Account headroom | 64-account limit "may remain the binding constraint" for multi-pool routes even inside the 4,096-byte envelope; dense ALT users gain the least | ALT trade-off analysis (Dune sample, 2026-08-17) |
| Draft SIMD-0596 | Would raise the account limit to 96 — **not activated**; do not code against it | solana.com upgrade page (2026-11-21) |

**Implementation decision**: The repository's `crates/arb-tx/src/transport/adaptive.rs` V1 compiler matches every point above (verified line-by-line this session). No change made. The `jitodontfront` MEV-protection marker (rebates nothing; blocks sand win) was noted from Jito docs but is deliberately **not** adopted without an explicit operator decision.

## 2. Kamino (Klend) Flash Loans

| Item | Finding | Checked |
|---|---|---|
| Program ID | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (mainnet) | [Kamino architecture docs](https://www.mintlify.com/kamino-finance/klend/concepts/architecture), 2026-11-21; matches `crates/arb-constants/src/lib.rs:463` |
| Flash-loan flow | Build `flash_borrow` + custom instructions + `flash_repay` (kamino.com docs tutorial); `borrow_instruction_index` **specifies the position of the flash borrow instruction in the final transaction** → any V1 ComputeBudget stripping must rewrite it | [kamino.com/docs/build/tutorials/borrow/flash-loans](https://kamino.com/docs/build/tutorials/borrow/flash-loans), 2026-11-21 |
| Rust client | `klend-interface = "0.6.0"` with `solana-pubkey = "2.1"`, `solana-sdk = "~2.3"`; zero-copy account deserialization; `instructions_sysvar` is required (unchecked account validated by address to `solana_program::sysvar::instructions::ID`) | [docs.rs/klend-interface](https://docs.rs/klend-interface/latest/klend_interface/), 2026-11-21 |
| Fee semantics | Sample community measurement: USDC reserve flash fee 0.001% (rate stored as U68F60 sf) — repo's `max(amount·sf, 2^60)` + ties-away rounding preserved | community tooling (30daysmoney PR #14) — classification: PARTIALLY VERIFIED, repo oracle tests are the binding check |
| Restrictions / sysvar scan | Flash-repay scans **top-level** instructions for the borrow CPI; CPI-nested borrows can evade the scan — repository must never rely on this as a feature (exploit research, not a pattern to copy) | [adpthegreat/bypassing-kamino-restrictions](https://github.com/adpthegreat/bypassing-kamino-restrictions), 2026-11-21 |

**Implementation decision**: Keep the existing canonical kamino fee implementation (`arb-math/src/economic.rs`) and the borrow-index rewrite in `compile_v1_message`. No protocol defect found; Kamino integration preserved per mission rule 7.

## 3. Jito Block Engine

Source: [docs.jito.wtf/lowlatencytxnsend](https://docs.jito.wtf/lowlatencytxnsend/) (fetched 2026-11-21; page dated 2026-11-06).

| Item | Finding |
|---|---|
| Bundles | Max **5 transactions**, executed sequentially & atomically in one slot; all-or-nothing |
| Auction | Parallel auctions at **50 ms ticks**; bundles prioritized by tip/CU efficiency; highest paying combination submitted to validator |
| Minimum tip | **1,000 lamports** (block-engine enforced). The repo default floor (10,000 lamports, `arb-constants`) is stricter — safe direction. Comment now clarified. |
| Tip accounts | 8 accounts, constant (see docs page). Repo's `JITO_TIP_ACCOUNTS` matches all 8 exactly |
| Tip placement | Tip = any instruction transferring SOL to one of the 8 tip accounts (top-level or CPI); random account selection recommended to reduce contention. Tip account must remain inline, not ALT-loaded. **Second-pass fix (2026-11-23):** default ALT no longer provisions tip accounts, provisioning APIs filter them, and compiler forces inline even for legacy ALTs containing old entries. Existing on-chain ALTs must be replaced to remove stored tip addresses before reuse as a clean manifest. |
| Status lifecycle | `getInflightBundleStatuses` (≤5 ids, 5-minute lookback): Failed / Pending / Landed / Invalid; `getBundleStatuses` returns slot + confirmation_status; idempotency via signature-status checks before any resubmit |
| Rate limits | Default 1 request/s/IP/region; 429 on exceed → repo's bounded retry policy is aligned (wait ≥1 s, single retry surface) |
| Regions |<mainnet block engines: global, amsterdam, dublin, frankfurt, london, ny, slc, singapore (shred receivers + NTP hosts per region) |
| `sendTransaction` path | Proxies directly to validator with MEV protection, `skip_preflight=true` always — pre-flight simulation must therefore be done by the bot itself (matches repo's mandatory arb-sim gate) |
| Sandwich mitigation | `jitodontfront`-prefixed read-only account accepted as bundle index-0 marker (not adopted) |

**Implementation decision**: Dynamic tip bounded by `ExpectedNetProfitAfterTip >= RequiredProfitFloor`; no fixed gross-percentage tip change made this session (existing share caps 70/85 bps-of-edge defaults retained and explicitly documented as defaults, not enforced constants).

## 4. Meteora DLMM

| Item | Finding | Checked |
|---|---|---|
| Official TS SDK | `@meteora-ag/dlmm` latest npm version **1.9.14** (published ~2026-07) | [npm](https://www.npmjs.com/package/@meteora-ag/dlmm), 2026-11-21 |
| Differential oracle pin | The repo's differential harness pins exactly `@meteora-ag/dlmm@1.9.14` over a captured mainnet snapshot — the pin is still current | `scripts/differential-harness` + npm, 2026-11-21 |
| DLMM Pro | Separate `dlmm-pro-sdk` (TS + Rust commons) exists — the repo's `dlmm_pro.rs` adapter tracks that protocol; parity testing for DLMM Pro is **NOT INSPECTED** this session | [github.com/MeteoraAg](https://github.com/MeteoraAg), 2026-11-21 |

**Implementation decision**: The layer-match source (`LayerFill`/`BinFillResult`, `Rounding::Up` full-layer input / `Rounding::Down` partial output) in `crates/arb-math/src/dlmm/swap.rs` mirrors the official SDK `FillResult` semantics and remains the parity reference. No change.

### 4.1 OnlyY (`collect_fee_mode = 1`) + ConcreteFunctionType semantics (VERIFIED 2026-11-21)

| Item | Finding | Checked |
|---|---|---|
| OnlyY fee semantics | `CollectFeeMode.OnlyY (1)` collects fees in token Y; X→Y swap → fee on output (Y); Y→X swap → fee on input (Y) | [docs.meteora.ag — DLMM Collect Fee Mode](https://docs.meteora.ag/core-products/dlmm/collect-fee-mode), fetched 2026-11-21 |
| SDK `getFeeMode` | `InputOnly(0) → feeOnInput = true; OnlyY(1) → feeOnInput = !swapForY` — byte-matches `arb-math/src/dlmm/error.rs::dlmm_fee_on_input` | `@meteora-ag/dlmm@1.9.14` `dist/index.js` (installed dependency, verified in-script), 2026-11-21 |
| SDK `isSupportLimitOrder` | `function_type 2 (LimitOrder) → true; 1 (LiquidityMining) → false; 0 (Undetermined) → true iff every reward mint is PublicKey.default; default → throw` — the old "Spot/Concentrated/Percentile" meaning was from an older IDL generation and is WRONG | same SDK source, 2026-11-21 |
| Mainnet OnlyY population | 8,160 deployed LbPair accounts with `collect_fee_mode=1` (discriminator-filtered `getProgramAccounts`); function_type histogram `{1: 595, 2: 7565}`, **zero** ft=0 pools — OnlyY is a launch/farm-pool feature | on-chain scan, `scripts/differential-harness/discover_onlyy_pools.js`, executed 2026-11-21 |
| Policy decision | {0,1} accepted everywhere (1 is quote-equivalent to 0: `getLimitOrderLiquidity` returns {0,0} when `!supportLimitOrder`); ≥2 rejected fail-closed — LimitOrder fill paths exist in `arb-math` but have **no differential fixture** (7,565 ft=2 pools discovered, none captured) | `arb-dex/src/dlmm_validation.rs` + engine gate alignment, 2026-11-21 |
| Transfer-fee interaction | First OnlyY capture candidate was refused: mint is Token-2022 with TransferFeeConfig (300 bps); the SDK wrapper folds transfer-fee inclusion into totals while our engine refuses transfer-fee mints fail-closed — correct behavior, not a parity miss | runner gate + `@solana/spl-token` mint decode, 2026-11-21 |

## 5. Raydium / Orca

### 5.1 Orca Whirlpool (VERIFIED 2026-11-21)

| Item | Finding | Checked |
|---|---|---|
| Program ID | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`; classic config `2LecshUwdy9xi7meFgHtFJQNSKk4KdTrcpvaB56dP2NQ` | [whirlpools-sdk@0.22.0 `dist/types/public/constants.js`](https://github.com/orca-so/whirlpools), 2026-11-21 |
| Official quote oracle | `@orca-so/whirlpools-core` **v3.1.1** wasm: `swapQuoteByInputToken` → `ExactInSwapQuote{tokenIn, tokenEstOut, tokenMinOut, tradeFee}`, `swapQuoteByOutputToken` → `ExactOutSwapQuote{tokenOut, tokenEstIn, tokenMaxIn, tradeFee}`; also exports `getTickArrayStartTickIndex`, `getPrev/NextInitializableTickIndex`, `tickIndexToSqrtPrice`, `sqrtPriceToTickIndex`, `tryGetAmountDeltaA/B`, `tryGetNextSqrtPriceFromA/B`, `tryApplySwapFee` | [npm @orca-so/whirlpools-core](https://www.npmjs.com/package/@orca-so/whirlpools-core), 2026-11-21 |
| Exact-in token flag | `swap_quote_by_input_token(..., a_to_b)` bool = **input token is A**; `swap_quote_by_output_token(..., a_to_b)` bool = **OUTPUT token is A** (inverted). Harness must invert the flag for exact-out | [rust-sdk/core/src/quote/swap.rs](https://github.com/orca-so/whirlpools/tree/main/rust-sdk/core/src/quote/swap.rs), 2026-11-21 |
| Traversal semantics | `compute_swap` targets the next/prev **initialized tick over the whole loaded `TickArraySequence`** (boundary = sequence start for a_to_b / end for b_to_a when no initialized tick survives); inner step: `calc = remaining·(D−fee)/D` (floor), reach target iff `initial ≤ calc`, `unfixed_out = Δ(cur→next, round_down)`, `fee = ceil(inter·fee/(D−fee))` at initialization boundaries else `remaining − inter`; liquidity update only when landing exactly on the boundary tick | same source, 2026-11-21 |
| Fee scope | Full trade fee charged on the INPUT token each step; SDK `tradeFee` maps 1:1 to engine `fee_paid` | same source, 2026-11-21 |
| TickArray PDA | Seed = `Buffer.from(startTickIndex.toString())` — **ASCII decimal string**, NOT i32 LE (classic silent-corruption trap) | [whirlpools-sdk `pda-utils.js`](https://github.com/orca-so/whirlpools), 2026-11-21 |
| Adaptive-fee detection | `feeTierIndexSeed` = 2 raw account bytes @43; `feeTierIndex = seed[0] + seed[1]·256`; pool is **classic iff `seed == tick_spacing`** — adaptive-fee pools are fail-closed-rejected in the capture harness | whirlpools-sdk 0.22.0 IDL `artifacts/whirlpool.json` + core facade rules, 2026-11-21 |
| Account layouts | Whirlpool: 8-byte discriminator + 645 B (`tick_spacing`@41 u16, `fee_tier_index_seed`@43, `fee_rate`@45, `protocol_fee_rate`@47, `liquidity`@49 u128 LE, `sqrt_price`@65 u128 LE, `tick_current_index`@81 i32 LE). TickArray: 8 B disc + `start_tick_index` i32@8 + 88 ticks @ `12 + i·113` (`initialized` bool@0, `liquidity_net` i128@1, `liquidity_gross` u128@17), total 9,988 B. Facade rules: exactly 3 `rewardInfos`, each tick `rewardGrowthsOutside` length 3 | whirlpools-sdk@0.22.0 IDL artifacts, 2026-11-21 |
| Step math source | `get_next_sqrt_price_from_a_round_up` / `_from_b_round_down` verified in `programs/whirlpool/src/math/swap_math.rs` + `token_math.rs` | [orca-so/whirlpools `programs/whirlpool/src/math`](https://github.com/orca-so/whirlpools), 2026-11-21 |

**Implementation decision**: Defect found by differential capture (array-edge-by-edge traversal vs official sequence-wide targeting) fixed via new `arb_math::clmm::compute_whirlpool_swap` — a verbatim checked-arithmetic port of the official traversal over the repo's proven CLMM primitives; the Whirlpool adapter routes to it, Raydium CLMM keeps the previous shared engine. No code copied from any third-party arbitrage implementation.

### 5.2 Raydium CLMM (VERIFIED 2026-11-21)

| Item | Finding | Checked |
|---|---|---|
| Official traversal | `swap_internal` searches for the next initialized tick **inside the current tick array first**; only when none survives it advances to the next tick array (bitmap/extension decides the index) and uses `first_initialized_tick(direction)`. Array-edge stepping is therefore official Raydium semantics — unlike Orca's single-sequence view (§5.1) | [programs/amm/src/instructions/swap.rs](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs), 2026-11-21 |
| Fee recomputation | `state.get_total_fee_rate()` runs **every inner step**: base + dynamic (quadratic in volatility accumulator, ceiling division, `MAX_FEE_RATE` clamp); fee side via `is_fee_on_input(zero_for_one)` (0=FromInput, 1=Token0Only, 2=Token1Only) | same source, 2026-11-21 |
| Tick advance + limit orders | zero_for_one: `tick = next−1` (no limits) but `tick = next` with unfilled limit orders; mirrored for !zero_for_one. Unfilled limit-order fills execute at the boundary price before advancing | same source, 2026-11-21 |
| Spec-derived defect fixed | Both general-engine consumers (`quote_pool` CLMM branch and `quote_clmm_hop` Raydium branch) previously forced the Whirlpool shape (88-tick span, Orca tick rounding). Raydium arrays start on 60·spacing multiples, so any hydrated Raydium CLMM hop whose active tick sits in the second 60-spaced array aligned down to a never-hydrated array start and failed closed (`MissingTickArrayCoverage`) — Raydium CLMM liquidity was invisible to route discovery. Both consumers now render the 60-tick span + `ClmmTickMath::Raydium` (the shape already verified inside `arb_dex::raydium_clmm`) | repo diff vs official source, 2026-11-21 |

| Deployed account layout | After the 2026-09-30 CLMM upgrade the `PoolState` is **1544 bytes**; live-mainnet field offsets verified by byte-search + official `PoolInfoLayout.decode` agreement: `liquidity` u128 @237, `sqrt_price_x64` @253, `tick_current` @269, `status` @389, `fee_on` @390, `dynamic_fee_info` @1096..1176. Token programs are NOT fields — determined by fetching the mints' owners | live mainnet `getAccountInfo` probes + `@raydium-io/raydium-sdk-v2@0.2.85-alpha` layouts, 2026-11-21 |
| Fee gross-up formula | `swapMath.ts::computeSwap` (isBaseInput + isFeeOnInput): the fee is NOT a plain upfront deduction — full step: `fee = ceil(amount_in_max × rate / (1e6 − rate))` with `amount_in = amount_in_max + fee`; partial step: `fee = amount_remaining − recomputed_amount_in`. A 100% rate is rejected (`feeRate must be less than the fee rate denominator`) | [src/raydium/clmm/libraries/swapMath.ts](https://github.com/raydium-io/raydium-sdk-v2), 0.2.85-alpha, 2026-11-21 |
| Empty-span traversal | The official walk (`swapSimulator.ts::swapInternal`) advances from the current array to the **next array in the caller-ordered list** when no initialized tick survives in the current one; it never traverses empty spans of closed/unallocated tick arrays and never needs their accounts. Adopted verbatim in `find_next_tick_boundary_with_size` (nearest loaded array in direction) — contiguous lists keep identical adjacency behavior | [libraries/swapSimulator.ts](https://github.com/raydium-io/raydium-sdk-v2), 0.2.85-alpha, 2026-11-21 |
| Wide step bound | The SDK survives `MaxTokenOverflow` from `calculateAmountInRange` (returns `null`) and completes a partial step from the price move alone; the engine mirrors this with a wide (u128) reachability pre-check instead of failing a high-liquidity quote on a u64 gap bound | same source, 2026-11-21 |

- Step-level parity against the official `raydium-clmm` `compute_swap` swap-math remains green (`crates/arb-math/tests/raydium_clmm_swap_math_parity_tests.rs`); repo oracle vectors still the binding check for fee/rounding.
- RESOLVED 2026-11-21: multi-array end-to-end parity executed against `@raydium-io/raydium-sdk-v2` `PoolUtils.getOutputAmountAndRemainAccounts` over 6 live-mainnet captured pools (tick spacings 1/60/120, `fee_on` 0/1, both directions, small + deep probes incl. an empty-span crossing); exact integer parity everywhere. See `QUOTE_PARITY_REPORT.md` §4.3.

## 6. Testing Infrastructure

| Item | Finding | Checked |
|---|---|---|
| LiteSVM | In-process SVM used by the repo's sim strategy (arb-sim integration) — NOT INSPECTED at upstream this session | — |
| Mollusk | Anza test framework alternative — NOT INSPECTED | — |
| Differential harness | Repo's own JS harness captures byte-identical mainnet snapshots and SDK quotes → used as the SDK oracle for DLMM parity | repo source, this session |

## 7. Anti-hallucination ledger

Assertions in code comments/docs that could NOT be independently re-verified this session and remain classified `PARTIALLY VERIFIED`:

1. `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 64 MiB` justification cites a 2026-11-17 live Agave 4.4.1 measurement (10,714,291-byte klend spine) — plausible and conservative, but not re-measured here (no network).
2. Oracle test vectors in `audit/test_vectors/` are generated by the in-repo Python oracle — that oracle derives from protocol specs, not from deployed program execution. DLMM parity against the **official SDK** is covered only by the captured-snapshot differential; a broader parameter sweep against the deployed programs is a future capture task.

---

# Session 2 — Open-Source Sweep Applied to `arb-executor` Tip Configuration (2026-11-21)

Scope: evaluate (vs. in-repo native implementation) the frameworks and payment-flow
conventions used by production on-chain MEV programs before implementing the verified
best-practice change: an on-chain conditional Jito tip instruction
(`execute_three_leg_with_tip`, new variant `[124, 34, 60, 60, 78, 94, 133, 177]`).

## S2.1 Program frameworks (`anza-xyz/pinocchio`)

| Item | Finding | Checked |
|---|---|---|
| Project | `pinocchio` v0.11–0.12, maintained under `anza-xyz`; "no external dependencies", zero-copy, "optimized in terms of both compute units consumption and binary size"; `no_std`; `lazy_program_entrypoint!` for on-demand input parsing; companion crates `pinocchio-system`/`pinocchio-token` 0.5.0; upstream BPF target needs `solana-define-syscall` with `unstable-static-syscalls` | [raw README](https://raw.githubusercontent.com/anza-xyz/pinocchio/master/README.md), 2026-11-21 |
| Applicability | Migrating `arb-executor` would change program ID/wire-independent behavior for an already-met goal. The repo already implements pinocchio-class patterns natively on the Agave 3.x split crates (`solana-address`/`solana-account-info`/`solana-cpi` with `default-features = false`, zero-copy `64..72` balance reads, no Anchor). A migration buys no measured CU/binary win and adds supply-chain + feature-flag risk | repo diff + pinocchio docs, 2026-11-21 |

**Implementation decision**: NO framework migration. `arb-executor` stays native
zero-copy on split Agave crates (same patterns pinocchio packages), preserving the
verified program identity and wire formats.

## S2.2 Jito conditional-tip flow (open-source searcher references)

| Item | Finding | Checked |
|---|---|---|
| Official examples | `jito-labs/searcher-examples` builds the tip as a plain `system_instruction::transfer` to one of the canonical tip accounts and sends bundles via `send_bundle_with_confirmation` | [repo](https://github.com/jito-labs/searcher-examples), [cli/src/main.rs](https://github.com/jito-labs/searcher-examples/blob/master/cli/src/main.rs), 2026-11-21 |
| Tip placement | The tip must be present in the **last transaction of the bundle**; SOL transfer without ALT indirection; min 1,000 lamports (block-engine enforced) | [QuickNode guide](https://www.quicknode.com/guides/solana-development/transactions/jito-bundles), [dev.to bundle guide](https://dev.to/mohammedalabd/atom-transactions-using-jito-bundles-1m78), Jito docs (Session 1), 2026-11-21 |
| Min-tip reality check | Practitioner reports: "Bundle must tip at least 1000 lamports" — confirmed as block-engine validation, below-auction tips simply do not land | [zhe-t gist](https://gist.github.com/zhe-t/60938c69e29276b7a9f098e1b0672c79), 2026-11-21 |

**Implementation decision**: Conditional tip implemented **on-chain** (not off-chain
guessing): `execute_three_leg_with_tip` transfers `floor(gross_edge * tip_bps / 10_000)`
lamports from the signing user to a canonical tip account only after the canonical
three-leg body succeeds, guarded by (a) canonical tip-account validation,
(b) WSOL-mint dimensional guard (WSOL 1:1 lamport peg), (c) retained-edge floor
`gross_edge - tip >= min_final_wsol_delta - leg1_amount_in` so the tip can never
consume the flash-loan cost headroom or the operator's stipulated minimum profit.
Because it is the final instruction in the arbitrage transaction and Jito bundles are
all-or-nothing, a losing or under-profit execution never pays a tip.

## S2.3 Meteora DLMM CPI reference (unadopted, corroborating)

| Item | Finding | Checked |
|---|---|---|
| Official CPI guide | `docs.meteora.ag/developer-guides/dlmm/rust-integration/cpi.md`: call `lb_clmm` from Rust programs with generated bindings, event-CPI accounts, remaining accounts (bin arrays), and validation | [docs.meteora.ag llms.txt index](https://docs.meteora.ag/llms.txt), 2026-11-21 |
| `swap2` discriminator | third-party `solroute` pins `Swap2IxDiscm = [65, 75, 63, 76, 235, 91, 91, 136]` — byte-identical to the repo's `METEORA_SWAP2_DISCRIMINATOR` in `arb-executor/cpi/meteora_dlmm.rs` | [solroute meteora pkg](https://pkg.go.dev/github.com/solana-zh/solroute/pkg/pool/meteora), 2026-11-21 |

**Implementation decision**: No change to the swap2 CPI layer (already matches).

## S2.4 Anti-hallucination ledger (Session 2 additions)

1. Pinocchio's CU/binary advantages are VERIFIED claims from its docs for **programs
   that use it**; the assertion that `arb-executor` gains nothing from migration is an
   ENGINEERING JUDGMENT (PARTIALLY VERIFIED) — a CUs-delta would need a `cargo
   build-sbf` + on-chain/CU-measurement harness, which this sandbox cannot run
   (no SBF toolchain guarantee; command: `cargo build-sbf -p arb-executor`).
2. The `execute_three_leg_with_tip` tip economics assume `min_final_wsol_delta >=
   repay (principal + fee)` as documented in `FLASHLOAN_EXECUTION_REPORT.md`; the
   on-chain retained-edge floor uses exactly that operator stipulation.

## S2.5 LiteSVM end-to-end borrow→swap→repay harness — UNBLOCKED (toolchain + blobs + milestone-A harness) — updated 2026-11-21

**Status update (2026-11-21)**: the artifact blockers are RESOLVED in the sandbox and the
milestone-A harness is implemented (`crates/arb-sim/tests/litesvm_arb_executor_e2e.rs`). The full
economic borrow→swap→repay proof still requires a consistent mainnet account snapshot (Kamino
reserve/market + DLMM pool/bin arrays + CPMM pool + vaults at one slot) and remains tracked as a
later milestone.

Unblock evidence (all VERIFIED in this session):

1. Agave toolchain installed via `agave-install-init stable --no-modify-path -d /workspace/agave-toolchain`
   (release.anza.xyz/stable, cargo-build-sbf 4.5.0, platform-tools v1.57, sbf rust 1.95.0, solana CLI).
   `programs/arb-executor` then needed `[lib] crate-type = ["cdylib", "lib"]` in `programs/arb-executor/Cargo.toml`
   — without it `cargo build-sbf` produced only an rlib and no deploy artifact.
2. `arb_executor.so` built: sha256 `faf2b7e6ffd9aa8d748406bd10c4d39e3fc3c0fac3d55e725d9eecdac76e31b8`
   (64,728 bytes, `target/deploy/arb_executor.so`). Note: `cargo build-sbf -p arb-executor` is invalid for this
   workspace; run `cargo build-sbf` from `programs/arb-executor` (SBF output dir: `target/sbpf-solana-solana/`).
3. Mainnet program blobs fetched via RPC `getAccountInfo` (base64 → bytes, following the
   BPFLoaderUpgradeable programdata pointers). Correct wire layout: `[4B enum tag][8B slot][1B Option tag][32B
   upgrade authority]` = **45-byte header**; ELF slice at data offset 45 (base64 payload minus 36 when only slot-ish
   tail is present after a 9-byte strip — always locate `\x7fELF` and slice)
   - `klend.so` (program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`, programdata
     `9uSbGW1y9H5Av6H5TKxQ1wnFApSq2t3oEpfF2YfjDQGA`): ELF slice 10,485,715 bytes,
     sha256 `6482390f659a6774b059768313b6f701b14819110f9e10db8fa651a311e9c97b`.
   - `dlmm.so` (program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`, programdata
     `HZcJwcJ2njPDxZtpPoKnF8v2w9QAx2rS7TdJPSRkbEhu`): ELF slice 2,229,776 bytes,
     sha256 `d296c6771cec945601027613ca637c1be6721044c5859009d41c453477844c1f`.
4. LiteSVM 0.18.2 milestone-A harness — all 4 tests passed locally:
   `cargo test -p arb-sim --features litesvm --test litesvm_arb_executor_e2e`
   (`arb_executor_sbf_elf_loads_and_dispatches_typed_error`: real SBF artifact deployed at
   `ArbExec111…` and dispatcher returned typed `Custom(6000)`; short-data guard test; system-transfer VM boot;
   mainnet klend + dlmm blobs deploy). Artifacts resolve from `ARB_EXECUTOR_SO` / `LITESVM_PROGRAM_BLOBS`
   (`target/deploy/program-blobs/{klend,dlmm}.so`) and tests SKIP with an explicit message when absent — never a
   fabricated pass.
5. Deterministic-replay capture for ISSUE-P0-04 delivered: `crates/arb-sim/src/forensic.rs`
   (`ForensicBundle`/`HopProvenance`, sha256-pinned wire, bounded logs, emitting-program hint,
   `replay_locally` via the fail-closed `litesvm_sim::simulate_locally`), wired into the supervisor's
   pre-flight failure paths (`crates/arb-bot/src/supervisor/opportunity.rs`, writes `reports/forensics/`,
   override `FORENSIC_CAPTURE_DIR`).

A full LiteSVM harness (repo already pins `litesvm = 0.18.2`, and `arb-sim/src/litesvm_sim.rs`
is fail-closed: every invoked program must be explicitly deployed in the VM or the result
is classified unverified) requires three SBF `.so` blobs:

1. `arb-executor` itself — needs `cargo-build-sbf` + Agave `platform-tools`
   (RESOLVED: installed 2026-11-21, see status update above).
2. Kamino `klend` mainnet program blob (RESOLVED: fetched + hash-pinned, see above).
3. Meteora DLMM `lb_clmm` mainnet program blob (RESOLVED: fetched + hash-pinned, see above).

**Blocked because (historical)**: the SBF toolchain and program blobs are large external artifacts and
the blobs must be provenance-verified (mainnet `getAccountInfo` raw bytes or official
release builds) before any test that uses them may be cited as evidence. Partial value
delivered instead: byte-level wire-parity tests lock the off-chain builder to the on-chain
argument structs for BOTH instruction variants
(`cargo test -p arb-tx -- wire_parity`).

**Exact unblock commands (networked host)**:

```bash
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"   # cargo-build-sbf + platform-tools
cargo build-sbf -p arb-executor                                  # target/sbf-solana-solana/arb-executor.so

# mainnet program blobs (or official release builds from
# github.com/Kamino-Finance/klend + MeteoraAg program release assets):
solana account KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD --output json | \
  jq -r .data[0] | base64 -d > klend.so
solana account LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo --output json | \
  jq -r .data[0] | base64 -d > dlmm.so

# then: litesvm.with_program(ARB_EXECUTOR_ID, include_bytes!(...so)) and run
# borrow -> swap x3 -> repay -> conditional tip inside one composed tx
# (release blobs must be hash-verified against the deployed on-chain program hash)
```

---

# Session 3 — Cross-Verification Sweep (2026-11-22)

Scope: re-verify Session 1/2 claims against live web sources in a fresh session and
deliver the Phase 1 research dossier under `docs/active/research/`.

| Item | Outcome | Evidence |
|---|---|---|
| SIMD-0385 | Re-verified from the **raw proposal**: frontmatter `status: Activated`; wire layout, config mask bits [0,1]/[2]/[3]/[4], 4096/12/64/64/255 constraints, no ALTs, CB-instruction no-op semantics — all match the repo V1 compiler | raw fetch 2026-11-22 → `SOLANA_BEST_PRACTICES.md` |
| V1 mainnet activation | epoch 1035, slot 447,120,000, ~2026-09-15; Agave ≥4.2.2 / Jito-Solana ≥4.2.2 required to avoid silent v1→v0 downgrade | solanacompass + xroot gate tracker, 2026-11-22 |
| Jito | 8 tip accounts (getTipAccounts), min tip 1,000 lamports (engine-enforced), 50 ms auction ticks, acceptance ≠ landing, getInflightBundleStatuses ≤5 ids / 5-min lookback | docs.jito.wtf, mev-protos, Alchemy, 2026-11-22 |
| Kamino | Program ID, borrowIxIndex semantics, klend-interface 0.6.0, U68F60 fee sf with 2^60 floor, wallet-balance repayment subsidy risk | kamino.com docs, docs.rs/klend-interface, 2026-11-22 |
| LiteSVM/Mollusk | LiteSVM 0.18.x current (0.18 adds loaded_accounts_data_size field + SIMD-0437/0438 rent gates), Mollusk 0.16.x, bankrun deprecated in favor of LiteSVM — **repo pin 0.18.2 is current** (Session 1 "NOT INSPECTED" item resolved) | chainstack + dev-skill testing refs, 2026-11-22 |
| Yellowstone | Rust client ≥13.1 built-in auto-reconnect + from_slot replay overlap dedup; repo pins 17.0.0 behind feature flag | docs.rs CHANGELOG + rpcedge, 2026-11-22 |
| Tokio | bounded-channel backpressure + runtime isolation guidance re-confirmed | dial9 principles (upd. 2026-09), 2026-11-22 |
| Supply chain | 2026 consensus: cargo-audit hard gate + cargo-deny policy; gitleaks Go binary remains MIT (only the action-wrapper relicensed) — repo posture compliant | pistack + codevetter, 2026-11-22 |

Deliverables added this session:
- `docs/active/research/README.md` + 7 domain documents (Solana, Jito, Yellowstone,
  DEX, Flash Loan, Rust HFT, Security), each with facts vs REC separation and
  required regression tests.
