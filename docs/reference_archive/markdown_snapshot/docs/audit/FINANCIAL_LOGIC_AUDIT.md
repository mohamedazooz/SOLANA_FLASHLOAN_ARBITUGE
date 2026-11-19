# Financial Logic Audit — Kamino fee, sizing, net profit (2026-11-13)

Classification: **Verified** items ran in this campaign's test suite;
**Partially verified** items are code-inspected with partial test coverage.

## 1. Kamino flash-fee formula — Verified

Canonical owner: `crates/arb-math/src/economic.rs`
(`calculate_flash_fee_raw` / `calculate_flash_fee_checked`), re-exported by
`arb-tx::kamino`. Semantics (RAW bits, pre-rounding):

- `fee_bits = max(amount * origination_fee_sf, 2^60)` — the 2^60 floor is the
  Kamino minimum-fee semantic (1/2^60 fraction).
- `BorrowTooSmall` iff `fee_bits >= amount << 60` (fee would consume the
  principal) — checked variant, fail-closed.
- Rounding: `(fee_bits + 2^59) >> 60` — round-half-up on the RAW fraction.
- Disabled sentinel: `origination_fee_sf == u64::MAX` ⇒ fee disabled (0).
- Property test in arb-math enforces arb-tx/arb-math agreement (R3).

No hardcoded fee percentage exists anywhere in the execution path; the live
`origination_fee_sf` is read from the verified reserve snapshot.

## 2. Repayment enforcement — Verified

`supervisor/run.rs` (capture gate step 5):

```
total_required = size_in + flash_fee          (checked_add, overflow → reject)
expected_out >= total_required  else reject
```

The final on-chain guarantee is structural: Kamino `FlashRepay` is the last
instruction of the same atomic transaction; if the final balance cannot cover
repayment, the whole transaction reverts (no partial settlement).

## 3. Borrow sizing — Verified (bounded, non-monotonic-safe)

Owner: `arb-engine::three_leg_meme_pipeline` (3-leg) and
`arb-engine::MarketGraph::golden_section_size` (graph path).

- Candidate ladder built from `tuning.input_size` (×3 growth) bounded by
  `max_sizing` (risk-gate output) and `tuning.max_input`; web/Telegram
  overrides `min_borrow_lamports` / `max_borrow_lamports` clamp the ladder at
  the tuning boundary (`supervisor/run.rs`).
- Candidates ABOVE verified `kamino_available_flash_liquidity` are DROPPED,
  never clamped; if all candidates are dropped ⇒ `InsufficientFlashLiquidity`
  error (fail closed) —
  `three_leg_net_economics_tests::candidates_above_available_flash_liquidity_rejected`.
- Available liquidity source: actual supply-vault SPL balance
  (`decode_supply_vault_liquidity`), freshness-bounded (1500 slots), owner- and
  mint-checked (`supervisor/run.rs::kamino_available_flash_liquidity`).
- Objective is NET profit (gross − flash fee − network cost), never gross
  output. Non-monotonic safety: the 3-leg search evaluates an explicit discrete
  candidate set (bin transitions/dynamic fees make the quote function
  discontinuous; a blind binary search is not used there). The graph path uses
  golden-section search over a smooth CPMM-model curve, appropriate for that
  model.
- Network cost model (`three_leg_network_cost_lamports`): base fee + Jito tip +
  adaptive priority fee + WSOL settlement-ATA rent ONLY (intermediate meme
  ATAs are created and closed in-tx and reclaim rent). No double counting:
  DLMM fees are embedded in the curve quotes; the flash fee is added once.

## 4. Net-profit units — Verified

All amounts are raw integer token units in the flash-loan denomination (WSOL
lamports). USD conversion happens only at the reporting/override boundary via
the last verified live SOL price; a stale/zero price fails closed
(`min_profit_lamports` keeps its TOML floor).

## 5. Cross-pool route consistency — Verified

`quote_chain` passes each hop's actual output as the next hop's input (raw
integers); mint continuity is asserted by the graph DFS (from/to mint match)
and tested in `cross_dex_three_pool_live_loop.rs` (exact mint continuity +
per-protocol hops).

## 6. Partially verified / open

- Per-bin fee breakdown (base vs variable fee split per traversed bin) is
  modeled in `arb-math::dlmm` and differentially tested against SDK snapshots,
  but a live per-bin audit on a high-volatility pool has not been run in this
  environment (needs the ≥24 h shadow soak).
- Failure-related fees (failed-onchain tx cost) are counted in operational PnL
  via `risk_gate.record_loss`; a dedicated reconciliation of failed-tx fee
  totals against chain data is pending the soak.

## 7. Hardcoded values and live-vs-non-live pricing — exhaustive enumeration (2026-11-17, PR #70 batch 17)

### 7.1 Priced live from market/runtime state (لا يوجد هاردكود)
| القيمة | المصدر الحي | نقطة الفشل المغلق |
|---|---|---|
| رسوم flash-loan (`fee_sf`) | فكّ `ReserveConfig.fees.flash_loan_fee_sf` من حساب reserve الحي (`decode_reserve_flash_fee_sf` → `build_verified_inputs` — execution.rs) | قيمة غير قابلة للقراءة/حساب = `u64::MAX` → رفض (NoEligibleReserve) |
| اختيار reserve | مسح حي لمحمّلات WSOL+USDC بسيولة ≥ الحد الأدنى (G6) | لا مرشّح = رفض |
| رسوم كل hop (DEX) | حساب البروتوكول الحي: Pump.fun `fee_bps` من Fees/GlobalConfig (بحدود)، DLMM base+variable من params، DAMM v2/DBC/CLMM/Orca من حساباتها؛ Raydium CPMM 0.25% ثابت **بمقتضى مواصفة البروتوكول** (ليس سعر سوق) | fee ≥ denominator أو sentinel = رفض الصف |
| رسوم Token-2022 transfer | فكّ config من حساب المِنت + preflight tripwire عند الاقتناص (opportunity.rs:349) — أي hop غير مُتحقَّق منه = رفض المسار | `transfer_fees_verified_zero == false` → skip |
| Jito tip | `TipOracle` — REST+WSS من bundles.jito.wtf p50/p75، حد نضارة 5s؛ نضارة/فشل = لقطة **غير موجودة** (لا fallback صامت) | floor من verified tip فقط |
| عامِل المنافسة | `competition_factor()` من TipOracle الحي → يوسّع share bps بين base وmax ديناميكيًا | — |
| Priority fee | `adaptive_cu_price` = p75+25% من **أسعار CU للهبوطؤا المؤكَّدة** (نافذة مُقيَّدة 64 — batch 17 ربطت المصدر) ثم `cap_cu_price_by_profit` (سقف نسبة من الربح — المكان الوحيد للسقف) | ربح 0 → سعر 0 (لا bid) |
| سعر SOL/USD | `live_sol_price()` من تجمّع SOL-USDC الحي/feed؛ Chronos EV gate يرفض المرشّح كليًا بدون سعر حي (`opportunity.rs:551-558` — fail closed وليس ثابتًا مخمَّنًا) | None → skip |
| تنبيهات USD في Telegram | `sol_price_live` provenance flag (G7) — السعر غير المُوثَّق يُعرَض بوسم «سعر غير موثّق» بدل الادعاء الحي | — |
| سيولة الإقراض | رصيد supply vault الفعلي (فكّ SPL)، نضارة 1500 slots | قديم/صفر = رفض |

### 7.2 ثوابت نظامية ثابتة عمدًا (ليست أسعار سوق — ثوابت بروتوكول/رنتايم)
- `SOLANA_BASE_FEE_LAMPORTS = 5,000` — ثابت سلسلة.
- `MAX_COMPUTE_UNIT_LIMIT = 1,400,000`، `MAX_INSTRUCTION_STACK_DEPTH = 5/9`، `MAX_CALL_DEPTH = 64`، 64MiB — ثوابت Agave runtime (أُوكِّدت من المصدر 2026-11-17).
- `JITO_TIP_ACCOUNTS` (8) — المنشورة رسميًا من Jito على السلسلة.
- اشتقاق ATA (PDA بـ wallet‖token-program‖mint) — قاعدة برنامج SPL-ATA.

### 7.3 ثوابت كـموديل تصنيف/ترتيب (لا تدخل قرار التنفيذ النهائي — ومُقيَّدة بـ regression tests)
- `estimate_minimal_cu` (450 + 28,000 flash + 35,000/hop) و`CU_PER_BIN_ARRAY = 15,000` — موديل CU للترتيب/الـ preflight؛ يُستبدَل بـ measured CU لكل route_sig عند توفره (dispatch.rs Phase E) ومُقيَّد الآن بـ `cu_regression_benchmarks.rs` (item 531).
- `adaptive_cu_price` floor 100,000 µlam/CU عند نافذة فارغة (قبل أول هبوط) — السقف الحاكم `cap_cu_price_by_profit` يجعلها بلا أثر على الاقتصاد خارج الربح.
- `EXECUTOR_CPI_STACK_DEPTH_ESTIMATE = 4` — مثنّى بـ const-assert مقابل ثابت Agave (528).

### 7.4 قيم ثابتة في مسارات **غير إنتاجية** (معزولة/تدريب) — وثيقة انفصال
- `sol_usd_price: 180` (defaults في `meteora_fresh_pipeline.rs`، `meteora_size_optimizer.rs:115`) و`150.0` (defaults في arb-risk tests) — `meteora_fresh_pipeline`/replay/optimizer غير موصولة بالبوابة الحية؛ مسار الإنتاج يستخدم `live_sol_price()` fail-closed.
- `estimated_network_cost_lamports: 15_000` في كونفيغ optimizer نفسه؛ مسار الإنتاج (three-leg) يحسب `three_leg_network_cost_lamports(dynamic_tip_lamports)` من tip حي (run.rs:223).
- `$200` في telegram.rs:63 — بقايا افتراضية للعرض فقط، مع وسم provenance (G7) ومراجعة قائمة (item 451).

### 7.5 حساب ATA والتكلفة (تدقيق مطلوب صراحة)
- الاشتقاق PDA حتمي (`get_associated_token_address[_with_program_id]`).
- الإنشاء `CreateIdempotent` (opcode 1) — آمن على الموجود.
- إغلاق المتوسطات المؤقتة داخل نفس المعاملة (`should_close_intermediate_ata`) → الإيجار (~2,039,280 lamports) مسترد في-tx، وبالتالي `account_creation = 0` في نموذج الربح مسوَّغ.
- حارس `preexisting_intermediate_atas`: فحص RPC للـ ATAs الموجودة مسبقًا؛ عند فشل الفحص → تخطي كل الإغلاقات (الإيجار يبقى محجوزًا، لا revert، لا خسارة).
- `sync_native` بعد تمويل WSOL ATA (builder.rs:249,511؛ pump_fun_raydium.rs:275) — تغليف WSOL صحيح.
- تحت الحساب غير الموجود: الإنشاء يفشل في المحاكاة → رفض (لا خسارة؛ فشل المحاكاة لا يبث، والـ base fee لا يُدفع إلا على السلسلة).

## 8. Opportunity-capture chain — من التحديث الحي إلى القرار (تدقيق مطلوب صراحة)
1. تحديث ودّي بدليل slot/providers (state integrity gates G2/G3).
2. بناء routes على الودّة المُتحقَّق منها + manifest كامل الحسابات.
3. quotes لكل hop net-of-fee من حسابات البروتوكول الحية (لا حساب للاقتران إلا بعد نجاح كل الـ hops — `expected_out`).
4. `net_before_tip` fail-closed (`checked_sub` chain — لا saturation).
5. CU: measured لكل route_sig وإلا موديل مُقيَّد؛ `calculate_dynamic_cu_limit` + سقف 1.4M.
6. سعر priority: p75+25% من الهبطاات المؤكدة ثم `cap_cu_price_by_profit` على share من الربح الإجمالي (competition factor يوسّع السقف فقط بين الحدّين المُعدَّين).
7. tip الفعلي المُخطَّط هو نفسه داخل نموذج الربح (`opportunity.rs:359-375`).
8. نقل-رسوم tripwire + ROI implausibility gate (رفض ➜ artifacts quote قديمة) + Chronos EV gate بسعر SOL حي.
9. Jito leader gate + slot-window؛ بناء borrow/swap(s)/repay + ATA idempotent + close + tip آخر إرشاد.
10. fork sim (0-instruction — fail-closed 529) → profitability gate (`RealizedDeltaShortfall`, `NetProfitNegative` مستقل عن الـ hurdle).
11. dispatch (V0 أولاً ثم V1 بشرط canary)؛ الهبوط يُتتبَّع ويعلِم `recent_landed_cu` + `measured_cu` + realized-net من chain meta.
كل رفض يُسجَّل مع السبب في store/dashboard/Telegram (Telegram بوسم provenance للسعر).

## 9. مقارنة ممارسات مفتوحة المصدر (بحث 2026-11-17)
| أفضل ممارسة (المصدر) | حالة المستودع |
|---|---|
| اشتقاق priority fee من ظروف حية وليس ثابتًا (BoltTx guide؛ RPCFast fee-stack) | متجاوز — TipOracle حي + p75+25% من هبوطات مؤكدة (بعد batch 17) + سقف ربحي |
| سقف fee كنسبة من الـ edge (RPCFast) | مطابق — `cap_cu_price_by_profit` المكان الوحيد للسقف |
| التكلفة عند الفشل تُحسب في EV (solbotrank؛ devrels) | مطابق — chronos `failed_attempt_cost_lamports` + TELEMETRY للفاشلة |
| tip لـ Jito فقط عند ألزورة الذرّية (yavorovych/medium) | مطابق — الرسالة كلها مسار Jito bundle ذرّي |
| التحويل الفوري مقيد slippage/min-out (bybit wiki formula يحسبها في الربح) | مطابق — min-out لكل hop + on-chain guard + hop-slippage-once |
| تجنُّب حجم ثابت للحجم المحسوب من العمق (solana-arb-bot OSS) | مطابق — `calculate_optimal_flash_loan` + `calculate_kelly_optimal_size` من العمق الحي |
