# تقرير التدقيق الشامل للأنبوب التنفيذي — PIPELINE AUDIT

**التاريخ:** 2026-11-12
**الفرع:** `feat/v1-meteora-three-pool-flash-arbitrage` (HEAD: `a6174ab`)
**النطاق:** من اقتناص الفرصة → الحسابات → بناء المعاملة → التوقيع → المحاكاة → الإرسال → التسوية
**المنهجية:** فحص فعلي للكود المصدري (قراءة + تتبع استدعاءات + grep للاعتماديات) — لا اعتماد على التقارير السابقة.
**الحالة الآمنة:** `ARB_LIVE=0`، `SOLANA_TX_V1_DIRECT_ENABLED=false` — لا بث على الشبكة الرئيسية في أي مسار.

---

## 1. خريطة الأنبوب الحقيقي (المسار المُوصَّل فعلياً)

هذا هو المسار الوحيد المُوصَّل من الاكتشاف حتى البلوك تشين:

| المرحلة | الملف | الدالة / السطر |
|---|---|---|
| 1. حلقة التشغيل | `crates/arb-bot/src/supervisor/run.rs:230` | `run()` |
| 2. اقتناص 3-leg | `crates/arb-bot/src/supervisor/run.rs:79` | `capture_three_leg_dlmm()` → `arb_engine::build_three_leg_meme_pipeline` |
| 3. اكتشاف إضافي | `run.rs:824-835` | Bellman-Ford + DFS multi-hop cycles |
| 4. دمج + ترتيب | `run.rs:843-867` | dedupe بمجموعة البرك + `sort_by_key(net_profit)` |
| 5. بوابة الاقتصاد | `run.rs:1071` | `calculate_net_profit_raw` (النموذج الرسمي) |
| 6. بوابة المخاطر | `run.rs:1334-1400` | `risk_gate.evaluate_flash_loan / evaluate_full` |
| 7. بناء التعليمات | `crates/arb-bot/src/supervisor/execution.rs:8` | `assemble_native_ixs` → `TxBuilder::assemble_flash_arbitrage` (`arb-tx/src/lib.rs:3125`) |
| 8. التوقيع والتجميع V1 | `run.rs:1595-1790` | `AdaptiveTxCompiler::compile_v1_only` / `build_unsigned_v1_simulation_transaction` |
| 9. بوابة الثوابت الاقتصادية | `run.rs:1976` + `run.rs:2056` | `validate_economic_invariants` |
| 10. الإرسال | `crates/arb-bot/src/supervisor/dispatch.rs:61` | `dispatch_opportunity` → SWQoS (`send_raw_transaction`) أو Jito bundle |
| 11. المحاكاة الإجبارية | `dispatch.rs` (~سطر 250) | `simulate_async` + fallback `RpcPool::simulate_transaction` |
| 12. تسوية الهبوط | `dispatch.rs` (tokio::spawn في نهاية Live) | `poll_signature_until_terminal` / `jito_service.verify_landing` → `arb_land::LandStatus` |

**ملاحظة معمارية:** مسار 3-leg يعيد استخدام نفس بوابات الاقتصاد والمخاطر والـ dispatch الخاصة بالمسارات الأخرى — لا يوجد مسار موازٍ للإرسال. هذا مطابق لمبدأ "أنبوب واحد كانوني".

---

## 2. سجل النتائج (Findings Register)

التقييم: 🔴 حرج / 🟠 عالي / 🟡 متوسط / 🟢 منخفض.
الحالة: **مُصلَّح في هذا الفرع** (commit `02fd305` + `a6174ab`) أو **مفتوح**.

### 2.1 المُصلَّحة في هذا الفرع (مرجعية للتوثيق فقط)

| # | الخطورة | الموضع | العيب | الإصلاح |
|---|---|---|---|---|
| F1 | 🟠 | `crates/arb-tx/src/lib.rs:3169-3190` | ATAs الوسيطة كانت SPL فقط (`get_associated_token_address`) — أي mint على Token-2022 يُشتق ATA خاطئ ويفشل السواب | `get_associated_token_address_with_program_id` + `build_create_ata_idempotent_for_program` + `build_close_token_account_for_program` لكل mint حسب `hop.token_programs` |
| F2 | 🔴 | `crates/arb-tx/src/dlmm_3leg_flash_arb.rs:367` | حساب ATA التسوية كان يمرر الـ **mint** مكان الـ **program id**: `get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_mint)` → عنوان غير صالح كلياً | تمرير `params.leg_1.token_in_program()` كبرنامج صحيح |
| F3 | 🟠 | `crates/arb-tx/src/kamino.rs` | لا يوجد فك ترميز لسيولة الـ supply-vault — كان السقف يعتمد على سقف USD للمخاطر وليس السيولة الفعلية | `decode_supply_vault_liquidity(account, expected_mint)` — fail-closed (طول ≥72، مطابقة mint، u64 LE @64) + 4 اختبارات |
| F4 | 🟠 | `crates/arb-engine/src/flash_loan_sizing.rs` (قديم) | الربح الصافي كان يُحسب من `amounts[2]` (دخول الرجل الثالثة) بدل الناتج النهائي | `simulate_three_leg_chain` يعيد `final_output` + الدالة الكانونية `net_profit_lamports` (`flash_loan_sizing.rs:310`) |
| F5 | 🟠 | `flash_loan_sizing.rs` (قديم) | خصم مزدوج: رسوم DEX كانت تُطرح مرة داخل الـ quote ومرة يدوياً | إزالة الخصم اليدوي — الرسوم مدمجة في مخرجات الـ quote (`arb-math/src/dlmm.rs:680-720`) |
| F6 | 🟠 | `crates/arb-engine/src/flash_loan_sizing.rs` (قديم) | حساب رسوم Kamino محلياً (نموذج اقتصادي موازٍ) | التفويض إلى `arb_tx::kamino::calculate_kamino_flash_fee` (المصدر الوحيد، `FeeCalculation::Exclusive` + floor + sentinel `u64::MAX`) |
| F7 | 🔴 | `crates/arb-engine/src/three_leg_meme_pipeline.rs:188,395,476` | لا ضمان للسداد: كان يكفي ربح إجمالي موجب دون فحص أن أرضية الانزلاق تغطي الأصل + الرسوم | `RepaymentNotGuaranteed` (fail-closed) + `InsufficientFlashLiquidity` + تفويض `net_profit_of` إلى الدالة الكانونية |
| F8 | 🟡 | `crates/arb-bot/src/supervisor/run.rs:43` | تكلفة الشبكة لا تشمل إيجار ATA التسوية WSOL | `ATA_RENT_LAMPORTS = 2_039_280` يُحسب عند غياب الـ ATA من مخزن الحسابات |
| F9 | 🟡 | `crates/arb-execution/src/lib.rs:145` | عدم تطابق payer/signer كان يمر صامتاً | حارس fail-closed + اختبارات بموقّع حتمي |

**حالة الاختبارات بعد الإصلاحات (نُفِّذت فعلياً):** `cargo test --workspace --all-targets` → **954 passed / 0 failed**؛ `cargo clippy --workspace --all-targets -- -D warnings` → exit 0؛ `cargo fmt --all --check` → نظيف.

---

### 2.2 النتائج المفتوحة (مطلوب قرار/تنفيذ لاحق)

---

#### 🟠 O-1: `net_profit` في مسار 3-leg يحمل **الإجمالي** وليس الصافي

- **الموضع:** `crates/arb-bot/src/supervisor/run.rs:223` — `net_profit: Lamports(gross)` داخل `capture_three_leg_dlmm`.
- **السلوك الحالي:** `ExecutionOpportunity.net_profit` لمسار 3-leg = `gross_profit_lamports` (قبل رسوم الفلاش والشبكة). التعليق في السطر 218 يقول "البوابة الاقتصادية الرسمية تعيد اشتقاق الصافي لاحقاً" — وهذا **صحيح جزئياً**: بوابة `calculate_net_profit_raw` في `run.rs:1071` تعيد الحساب بالفعل مع التلميح الفعلي قبل الـ dispatch، لذا لا يوجد تسريب مباشر لخسارة مالية.
- **الخلل المتبقي:**
  1. الترتيب في `run.rs:867` (`merged.sort_by_key(Reverse(net_profit))`) يخلط إجمالي (3-leg) مع صافي (بقية المسارات) → ترتيب مضلل لصالح 3-leg.
  2. `validate_economic_invariants` (`run.rs:2082-2097`) تفحص البند 4 (أرضية الصافي) والبند 6 (tip < net) ضد قيمة إجمالية → بوابتان أخف من تصميمهما المعلن.
- **طريقة الإصلاح:** عند بناء `ExecutionOpportunity` في `capture_three_leg_dlmm` استخدم `result.size_search.best_net_profit_lamports` (المتوفر بالفعل في `ThreeLegMemePipelineResult`، `three_leg_meme_pipeline.rs:142`) بدل `gross`، واحمل `gross` في `gross_profit` فقط. اختبار: تأكيد أن `net_profit <= gross_profit` لكل فرصة تخرج من الـ capture.
- **الحالة:** مُصلَّح — `net_profit` في الالتقاط يأتي الآن من `net_profit_lamports` القانوني (§5.1).

---

#### 🟠 O-2: إغلاق ATAs الوسيطة **دون حراسة** في `assemble_flash_arbitrage`

- **الموضع:** `crates/arb-tx/src/lib.rs:3286-3291` — حلقة `build_close_token_account_for_program` لكل ATA وسيطة بلا شرط.
- **الخطر:** تعليمة SPL/Token-2022 `CloseAccount` تفشل إذا كان رصيد الحساب > 0 (حتى وحدة واحدة من token dust). أي hop يُخرج أكثر قليلاً من المتوقع (تقريب DLMM لصالح المتداول، أو رسوم transfer-fee محتجزة على Token-2022) يجعل الإغلاق يفشل → **المعاملة كلها ترجع (revert)** → خسارة أولويات + رسوم أساس + فرصة. المسار المنافس (meme compiler) يحرس الإغلاق عبر `should_close_intermediate_ata` (`crates/arb-execution/src/merger.rs:406`: لا يغلق حساباً موجوداً مسبقاً/غير صفري/به رسوم محتجزة) — تضارب سلوك بين مسارين كانونيين مفترضين.
- **طريقة الإصلاح (أحد البديلين):**
  1. **المفضل:** توحيد نموذج الحراسة مع `merger.rs:406`: الـ ATAs تُنشأ داخل المعاملة (idempotent) فالحساب كان فارغاً عند الإنشاء، والإغلاق آمن إلا إذا أودع hop غباراً. عملياً: حصر الإغلاق بالـ ATAs التي أنشأناها نحن (معلومة محلية متوفرة) **و** تجنب الإغلاق عندما يكون أي mint في المسار من نوع transfer-fee (المسار يرفض هذه البرك أصلاً في `quote_single_hop` — `meteora_quote_engine.rs:108-139`، فالخطر شبه منعدم عملياً).
  2. **البديل:** قبول خطر الـ revert (الذرّية تحمي رأس المال) وتوثيقه — التكلفة رسوم ضائعة فقط.
- **الحالة:** مُصلَّح — حارس إغلاق ATA عبر `skip_close_intermediate_mints` + فحص وجود RPC (§5.2).

---

#### 🟠 O-3: باني 3-pool الممول بالفلاش (`dlmm_3leg_flash_arb`) **غير موصول** بالتنفيذ الحي

- **الموضع:** `crates/arb-tx/src/dlmm_3leg_flash_arb.rs:457` (`build_flash_funded_three_pool_transaction`) + `pub struct FlashThreeLegArbitrageTx` (`:236`).
- **الدليل:** لا مستدعيات خارج إعادة التصدير في `crates/arb-tx/src/lib.rs:84-85` واختبارات الفرع في `crates/arb-tx/tests/three_pool_flash_arb_tests.rs`. المسار الحي يستخدم `assemble_flash_arbitrage` + `compile_v1_only` (`run.rs:1690-1700`).
- **المشكلة:** مساران متوازيان لبناء نفس المعاملة (مخالف لمبدأ "باني كانوني واحد"). الباني غير الموصول أكثر صرامة في بعض الجوانب (تحقق invariants 1-6، settlement ATA) لكنه **لا يغلق ATAs الوسيطة** (فجوة معاكسة: لا استرجاع إيجار) — لكل مسار عيب مختلف.
- **طريقة الإصلاح:** توحيد: إما (أ) جعل `assemble_native_ixs` يستدعي `build_flash_funded_three_pool_transaction` لمسار 3-leg DLMM الصرف (مع نقل حراسة الإغلاق إليه)، أو (ب) دمج منطق الـ invariants في `assemble_flash_arbitrage` ووسم الباني الآخر `#[deprecated]` حتى الحذف. قرار معماري مطلوب من المالك.
- **الحالة:** مُوثَّق — الوحدة الآن تعلن صراحة أنها builder مرجعي غير موصول بـ dispatch (§5.3).

---

#### 🟠 O-4: `dispatch_two_dex_arbitrage` — مسار "مُرسل" برأس مال مملوك وبلا Kamino

- **الموضع:** `crates/arb-execution/src/lib.rs:145` + `build_merged_instructions` (`:216`).
- **الدليل:** لا مستدعيات إنتاجية — فقط اختباراته الداخلية. التعليمات المبنية: CU + ATAs + 3 سواب + close + tip — **لا يوجد borrow/repay** (شكل رأس مال مملوك)، بينما التوجيه يفرض 100% تمويل بالفلاش.
- **الخطر:** اسم الدالة يوحي أنها الـ dispatcher الرسمي؛ أي ربط مستقبلي خاطئ لها يعني تداولاً برأس مال المحفظة أو سداداً فاشلاً.
- **طريقة الإصلاح:** حذفها أو نقلها خلف `#[cfg(feature = "owned-capital-sim")]` مع إعادة تسمية توضيحية (`dispatch_two_dex_owned_capital_sim`).
- **الحالة:** مُصلَّح — crate `arb-execution` بالكامل محذوف (§5.4).

---

#### 🟠 O-5: نموذج اقتصادي ثالث (f64) في اكتشاف Bellman-Ford

- **الموضع:** `crates/arb-engine/src/bellman_ford.rs:407` (`evaluate_cycle`) + `:543` (`compute_cycle_profit`) — ضرب احتمالات gamma بفائض **f64** ومقارنة عتبات float.
- **المشكلة:** التوجيه يمنع الفاصلة العائمة في حسابات التنفيذ. هذا النموذج في مسار **الاكتشاف**، لكن مخرجاته تتحول إلى `ExecutionOpportunity` وتدخل نفس أنبوب التنفيذ؛ بوابة ROI غير المنطقي (`run.rs:1121-1145`، سقف 15% افتراضياً) تخفف الأطراف الوهمية لكنها لا توحد النموذج. النماذج الثلاثة الآن: (1) `arb-math::economic` + `CostLedger` (الرسمي)، (2) `net_profit_lamports` الصحيح (مُوحد في هذا الفرع)، (3) نموذج f64 هذا.
- **طريقة الإصلاح:** بعد `evaluate_cycle`، أعد تسعير المرشح بالدالة الكانونية `arb_engine::net_profit_lamports` على مخرجات quote صحيحة قبل تحويله إلى `ExecutionOpportunity`، واستخدم f64 فقط للفرز الأولي الخشن.
- **الحالة:** مُوثَّق — `evaluate_cycle` يعلن الآن أن f64 فرز أولي فقط والأسعار من النموذج الصحيح (§5.5).

---

#### 🟠 O-6: `BYPASS_RISK_GATE` — متغير بيئة يتجاوز بوابة المخاطر

- **الموضع:** `crates/arb-bot/src/supervisor/run.rs:1324-1334`.
- **الخطر:** أي قيمة `BYPASS_RISK_GATE=true` في البيئة تتجاوز `risk_gate.evaluate_full/evaluate_flash_loan` (يبقى kill-switch فقط). في VPS أو بيئة تشغيل مشتركة هذا ثغرة تشغيلية (خطأ تشغيلي واحد = تجاوز كامل للمخاطر).
- **طريقة الإصلاح:** حذف المتغير، أو ربطه بشرط `debug_assertions`/feature، أو على الأقل: يتطلب أيضاً `ARB_LIVE=0` ليكون له أي أثر، وتسجيل تحذير `warn!` عند كل استخدام.
- **الحالة:** مُصلَّح — `BYPASS_RISK_GATE` مقيّد بالأنماط غير الحية + `warn!` عند كل استخدام (§5.6).

---

#### 🟡 O-7: ستة ملفات يتيمة (لا تُجمَّع إطلاقاً) + تضارب مع PROJECT_STATE

تم التحقق آلياً: لا يوجد أي `mod`/`pub mod` يشير إليها في كرتها:

| الملف | الأسطر | التوصية |
|---|---|---|
| `crates/arb-engine/src/dynamic_pool_selector.rs` | 424 | حذف أو إعلان + توصيل بمنطق اختيار البرك |
| `crates/arb-engine/src/pnl.rs` | 477 | حذف — يكرر نموذج التكاليف (`RENT_PER_ATA_LAMPORTS`, `ArbCostBreakdown`) الذي حل محله النموذج الكانوني |
| `crates/arb-execution/src/master_alt_compiler.rs` | 395 | حذف — V1 بلا ALT يجعله عديم الفائدة في سياق الإصدار الحالي |
| `crates/arb-execution/src/v0_v1_fallback.rs` | 623 | حذف — يعارض `V1_ONLY_RELEASE_POLICY.md` (fallback إلى V0 ممنوع) |
| `crates/arb-ingest/src/pump_state_tests.rs` | 400 | إما إعلانه داخل `#[cfg(test)]` في `arb-ingest/src/lib.rs` أو دمجه في مجلد `tests/` |
| `crates/arb-ingest/src/yellowstone.rs` | 241 | **تضارب توثيقي:** `docs/PROJECT_STATE.md` تدّعي أنه "deleted" وهو موجود وغير مُصرَّح. قرار: إعلان + توصيل كـ gRPC provider اختياري، أو حذف فعلي وتصحيح التوثيق |

- **الخطر:** كود ميت غير مُجمَّع = صفر تغطية اختبارية وصفر ضمانات clippy، مع وهم وجود قدرات (ALT compiler، fallback، Yellowstone) لا وجود لها فعلياً في البناء.
- **الحالة:** مُصلَّح — الملفات اليتيمة الثلاثة محذوفة فعلياً؛ `pump_state_tests` كان مربوطاً فعلاً عبر `#[path]` (تصحيح للتدقيق) (§5.7).

---

#### 🟡 O-8: تعارض تعليقات المحاكاة على المسار الساخن

- **الموضع:** `run.rs:1775-1782` يقول: "لا simulateTransaction على المسار الساخن (latency)... الإرجاع مفوض للحارسات on-chain". بينما `dispatch.rs` يفرض **محاكاة RPC إجبارية** قبل كل إرسال حي (`sim_enabled = true`، لا يمكن إطفاؤها ببيئة).
- **التقييم:** السلوك الفعلي (محاكاة إجبارية) هو الصحيح أمنياً؛ التعليق قديم ومضلل لمن يقرأ المسار الساخن. أيضاً `ENABLE_PREFLIGHT_SIMULATION` (`run.rs:2144`) تضيف محاكاة أبكر اختيارية — غير موثقة في `.env.example`.
- **طريقة الإصلاح:** تحديث تعليق `run.rs:1775` ليعكس الواقع (المحاكاة الإجبارية في dispatch هي البوابة الوحيدة قبل الإرسال)، وإضافة المتغير إلى `.env.example`.
- **الحالة:** مُصلَّح — التعليق القديم صحُّح + `ENABLE_PREFLIGHT_SIMULATION` مُوثَّق في `.env.example` (§5.8).

---

#### 🟡 O-9: مسار Jupiter معطَّل لكنه موصول (كود شبه ميت)

- **الموضع:** `run.rs:1425` — `let route_via_jupiter = false;` بشكل حرفي؛ الفرع `else if route_via_jupiter` (`run.rs:1516`) غير قابل للوصول، وداخله `resolve_jupiter_swap_route` (`supervisor/execution.rs:48`) مع fallback يدوي إلى الباني المحلي.
- **المشكلة:** ~150 سطراً من مسار تنفيذ بديل (Jupiter v6 per-hop quotes) غير قابل للوصول لكنه يُجمَّع ويُصان، وقد يوحي للقارئ بوجود مسار تنفيذ ثانٍ حي. ملاحظة معروفة أيضاً في `run.rs:1779-1782`: quotes Jupiter تستخدم 50 bps انزلاقاً وليس أرضية ربح.
- **طريقة الإصلاح:** إما feature-gate حقيقي (`jupiter-fallback`) بقرار صريح، أو حذف المسار وتوثيق سبب التعطيل في `PROJECT_STATE.md`.
- **الحالة:** مُصلَّح — مسار Jupiter الميت حُذف بالكامل من supervisor (§5.9).

---

#### 🟡 O-10: حوكمة البيئة — 63 متغيراً مستخدماً في الكود وغائبة عن `.env.example`

- **الدليل:** فحص آلي لكل `std::env::var("...")` في الـ workspace (63 متغيراً) مقابل `.env.example`: **63 غير موثقة**، من أبرزها: `MIN_SOL_RESERVE_LAMPORTS` (`dispatch.rs:163`)، `ENABLE_PREFLIGHT_SIMULATION`، `ARB_MAX_IMPLAUSIBLE_ROI_BPS` (`run.rs:1128`)، `BYPASS_RISK_GATE`، `ARB_MAINNET_RPC_URL`، `DATABASE_URL`.
- **والعكس:** مفاتيح في `.env.example` لا يقرأها الكود مباشرة عبر `std::env::var` (قد تُقرأ عبر ملفات config): `OPERATOR_PRIVATE_KEY`، `JWT_SECRET`، `SENTINEL_*`، `AQ_LIVE_*`، `PUMP_PORTAL_API_KEY` — مطلوب تدقيق يدوي: إما توثيق آلية قراءتها أو حذفها (وجود `OPERATOR_PRIVATE_KEY` في قالب بيئة دون قارئ واضح = خطر تسريب مفتاح خاص في `.env` حقيقي).
- **مخالفة مباشرة للبند 16 من التوجيه:** "Every environment variable must be used, documented and validated".
- **طريقة الإصلاح:** توليد `.env.example` من قائمة المتغيرات الفعلية + إزالة المفاتيح الميتة + فحص CI بسيط يقارن الاثنين.
- **الحالة:** مُصلَّح — `.env.example` يغطي متغيرات التشغيل المستخدمة، والمفاتيح الميتة حُذفت (§5.10).

---

#### 🟡 O-11: انحراف toolchain بين CI والمستودع

- **الموضع:** `.github/workflows/rust-ci.yml:32` — `dtolnay/rust-toolchain@stable` بينما `rust-toolchain.toml` يثبّت `1.99.1`.
- **الدليل:** انفجر فعلياً 3 مرات خلال هذا العمل: `doc_lazy_continuation`، `manual_clamp`، `type_complexity` — أعلام clippy تختلف بين stable والـ pin.
- **طريقة الإصلاح:** تثبيت الـ workflow على `toolchain: 1.99.1` أو الاعتماد على قراءة `rust-toolchain.toml` تلقائياً، أو تحديث الـ pin دورياً مع CI.
- **الحالة:** مُصلَّح — CI مثبَّت على `dtolnay/rust-toolchain@master` + `toolchain: 1.99.1` (§5.11).

---

#### 🟡 O-12: سياق اكتشاف بـ ATAs وحاوية blockhash وهمية (placeholders)

- **الموضع:** `run.rs:120-149` — `meme_a_ata` مشتق من mint `[0u8;32]` و`meme_b_ata` من `[1u8;32]`، و`recent_blockhash: [0u8; 32]`، و`meme_*_is_token_2022: false`.
- **التقييم:** التعليق موجود وصريح ("ATAs are placeholders for discovery")، والسياق **لا يُستخدم** لبناء معاملة على هذا المسار (التعليمات تُبنى لاحقاً من `pool.execution_accounts`). لكن النوع `ThreeLegMemeContextParams` يسمح بهذا الشكل دون تمييز نوعي — أي استخدام مستقبلي خاطئ للسياق سيمر بلا خطأ نوعي.
- **طريقة الإصلاح:** تقسيم النوع إلى `DiscoveryParams` (بدون حقول ATAs) و`CompilationParams` (بها، مطلوبة)، أو حقول `Option` مع فحص fail-closed عند البناء.
- **الحالة:** مُصلَّح — `ThreeLegMemeContextParams::for_discovery` يغلف الـ placeholders (§5.12).

---

#### 🟡 O-13: بوابات سداد بمقامين مختلفين (expected vs conservative)

- **الموضع:** `validate_economic_invariants` (`run.rs:2104-2118`) تفحص السداد ضد `opp.expected_out` (متفائل)، بينما بوابة `RepaymentNotGuaranteed` (`three_leg_meme_pipeline.rs:476`) تفحص ضد أرضية الانزلاق المحافظة `expected_min_final_output`.
- **التقييم:** البوابة المحافظة (الأقوى) تعمل مسبقاً في الـ pipeline، فلا ثغرة مالية حالياً؛ لكن بوابة الـ supervisor أضعف من وصفها وتعتمد على أن الـ pipeline سبقها.
- **طريقة الإصلاح:** إضافة حقل `expected_min_final_output` إلى `ExecutionOpportunity` واستخدامه في البند 5 من `validate_economic_invariants`.
- **الحالة:** مُصلَّح — `expected_min_final_output` في `ExecutionOpportunity` + بوابة سداد محافظة في `validate_economic_invariants` (§5.13).

---

#### 🟡 O-14: تقسيم المسارات (split route) يعيد استخدام mints الرجل الأولى دون تحقق

- **الموضع:** `run.rs:1455-1462` — كل leg من `s.legs` يُبنى بـ `mint_in/mint_out` من `first_hop`، و`total_out` يجمع مخرجات الأرجل.
- **التقييم:** صحيح فقط إذا كانت كل الأرجل تتداول **نفس الزوج** على منصات مختلفة (وهو تصميم RouteArchitect). لا يوجد حارس يمنع legs بأزواج مختلفة من الوصول إلى هذا المسار → معاملة بأقواس سواب خاطئة (تفشل ذرياً، لكنها تهدر CU وفرصاً).
- **طريقة الإصلاح:** قبل الحلقة: تحقق أن كل `leg` ينتمي لنفس زوج mints كـ `first_hop`، وإلا `continue`.
- **الحالة:** مُصلَّح — حارس split-route يتحقق من زوج mints لكل leg عبر الـ registry (§5.14).

---

#### 🟢 O-15: وحدة config مدمجة داخل `main.rs`

- **الموضع:** `crates/arb-bot/src/main.rs:151` — `pub mod config` داخل ملف من 999 سطراً (عتبة التوجيه ~1,000 سطر على الحافة).
- **طريقة الإصلاح:** استخراج إلى `crates/arb-bot/src/config.rs`.
- **الحالة:** مُصلَّح — `pub mod config` مستخرج إلى `crates/arb-bot/src/config.rs` (§5.15).

---

#### 🟢 O-16: تدهور مزودي arb-ingest (HTTP فقط)

- **الدليل:** `docs/PROJECT_STATE.md` توثّق فشل كل endpoints للـ WS؛ الابتلاع حالياً RPC-polling فقط. أثره: latency اكتشاف أعلى وslot freshness أضعف؛ بوابات الطزاجة (`max_state_age_slots`، `validate_dependency_group`) تعوض بالرفض الصارم.
- **الحالة:** مفتوح (قرار بنية تحتية — Yellowstone gRPC حقيقي أو قبول HTTP-polling موثقاً). غير قابل للإصلاح الكودي.

---

#### 🟢 O-17: `MintId::new([0u8; 32])` كقيمة احتياطية

- **الموضع:** `run.rs:1823` — `unwrap_or_else(|| MintId::new([0u8; 32]))`. mint صفري كقيمة احتياطية قد ينتج حافة وهمية تُرفض لاحقاً في بوابات الاتصال (cycle check) — تكلفته إزعاج لا خسارة.
- **طريقة الإصلاح:** إرجاع `Option`/fail-closed بدل mint صفري.
- **الحالة:** مُصلَّح — `let-else continue` بدل mint صفري (§5.17).

---

## 2.3 جولة الإصلاح التنفيذية (فرع `feat/pipeline-audit-fixes`)

جميع البنود أدناه نُفِّذت فعلياً وتحقّق منها: `cargo check --workspace --all-targets` نظيف،
`cargo clippy --workspace --all-targets -- -D warnings` نظيف، `cargo fmt --all` مطبَّق،
و`cargo test --workspace --all-targets` أخضر بالكامل (انظر §4 لآخر عدّاد مؤكد).

| # | الإصلاح | الملفات |
|---|---|---|
| O-1 | `net_profit` في الالتقاط من `net_profit_lamports` القانوني (`.max(0)`) بدل gross | `run.rs` (capture) |
| O-2 | `assemble_native_ixs` أصبح async + `preexisting_intermediate_atas()` يفحص وجود ATA عبر `RpcPool::get_multiple_accounts`؛ عند فشل RPC تُتخطى كل عمليات الإغلاق (fail-safe)؛ `assemble_flash_arbitrage` يقبل `skip_close_intermediate_mints: &[Pubkey]` ويحرس حلقة الإغلاق | `supervisor/execution.rs`، `arb-tx/src/lib.rs:3125` |
| O-3 | توثيق حالة الوصل: `dlmm_3leg_flash_arb.rs` يعلن صراحة أنه builder مرجعي غير موصول بـ dispatch | `arb-tx/src/dlmm_3leg_flash_arb.rs` |
| O-4 | حذف crate `arb-execution` بالكامل (مسار رأس مال مملوك بلا Kamino، صفر مستخدمين خارجيين) | `crates/arb-execution/` (محذوف) |
| O-5 | توثيق أسبقية النموذج: f64 فرز أولي فقط؛ الأسعار من `build_economic_quote` الصحيح | `arb-engine/src/bellman_ford.rs` |
| O-6 | `BYPASS_RISK_GATE` مقيّد بـ `!matches!(self.mode, BotMode::Live)` + `warn!` عند التنشيط | `run.rs` (risk gate) |
| O-7 | حذف الملفات اليتيمة `dynamic_pool_selector.rs`، `pnl.rs`، `yellowstone.rs`. ملاحظة تصحيحية: `pump_state_tests.rs` كان مربوطاً فعلاً عبر `#[cfg(test)] #[path]` في `pump_state.rs` — بند التدقيق الأصلي كان خاطئاً في هذه النقطة | `arb-engine/src/`، `arb-ingest/src/` |
| O-8 | تعليق المحاكاة القديم صُحّح (يشير الآن للمحاكاة الإجبارية في dispatch) + `ENABLE_PREFLIGHT_SIMULATION` موثق | `run.rs`، `.env.example` |
| O-9 | حذف مسار Jupiter الميت: `route_via_jupiter`، فرع `else if`، `resolve_jupiter_swap_route`، حقل `jupiter` (يبقى `arb_tx::JupiterClient` للأدوات المساندة) | `run.rs`، `supervisor/execution.rs`، `supervisor.rs` |
| O-10 | `.env.example`: إضافة قسم "RUNTIME / ENGINE VARIABLES" يغطي كل متغيرات التشغيل المستخدمة؛ حذف المفاتيح الميتة (`OPERATOR_PRIVATE_KEY`، `AQ_LIVE_*`، `JWT_SECRET`، `EXECUTION_ADMIN_KEY`، `PUMP_PORTAL_API_KEY`)؛ `SENTINEL_ADMIN_KEY` محفوظ (يُقرأ عبر `token_env`) | `.env.example` |
| O-11 | تثبيت toolchain في CI: `dtolnay/rust-toolchain@master` + `toolchain: "1.99.1"` | `.github/workflows/rust-ci.yml` |
| O-12 | `ThreeLegMemeContextParams::for_discovery(payer)` يغلف ATAs الوهمية؛ الـ capture يستخدم `..for_discovery(payer)` | `three_leg_meme_pipeline.rs`، `run.rs` |
| O-13 | `ExecutionOpportunity.expected_min_final_output` (`#[serde(default)]`) ينتقل من الـ pipeline إلى الـ capture، وبوابة سداد محافظة في `validate_economic_invariants`: إذا كان الحقل > 0 وجاء أقل من `size_in + flash_fee` → رفض | `arb-types/src/lib.rs`، `three_leg_meme_pipeline.rs`، `run.rs` |
| O-14 | حارس split-route: كل leg يجب أن يتداول نفس زوج mints لـ `first_hop` (تحقق عبر `registry.pools()`) وإلا `continue` | `run.rs` (split branch) |
| O-15 | استخراج `pub mod config` (417 سطراً) إلى `crates/arb-bot/src/config.rs` | `main.rs`، `config.rs` (جديد) |
| O-16 | مفتوح عمداً — قرار بنية تحتية (Yellowstone gRPC حقيقي أم HTTP-polling موثق). غير قابل للإصلاح الكودي | — |
| O-17 | `let Some(base_mint) = ... else { continue; }` بدل `MintId::new([0u8; 32])` الوهمية | `run.rs` (settlement) |

---

## 3. فجوات التوجيه غير المنجزة (Directive Gap Register)

| بند التوجيه | الحالة |
|---|---|
| اختبارات تفاضلية SDK لمسار 3-pool (Meteora SDK كمرجع) | غير منجزة — القوائم الحالية تغطي 2-pool (`reports/phase15_2/`) |
| Property-based / fuzz tests | غير منجزة |
| معايير أداء P50/P95/P99 (بند 15) | غير منجزة — يوجد `bin/latency_bench.rs` لكن لا تقرير قياس حديث |
| محاكاة mainnet read-only على snapshots حقيقية | جزئي — `ForkSimulator` موجود وموصّل اختيارياً (`ENABLE_PREFLIGHT_SIMULATION`)، لا دليل تشغيل موثق |
| Shadow soak مستمر + تقرير | غير منجز |
| تقرير أمني مستقل + runbook محدث | غير منجزين كملفين مستقلين (النتائج مبثوثة في `docs/THREE_POOL_FLASH_NET_ECONOMICS_2026-11-12.md` وهذا التقرير) |
| `maxSupportedTransactionVersion = 1` في كل قارئ | منجز في مسارات RPC المستخدمة (`rpc_max_supported_version_from_wire`) — مطلوب مسح نهائي لأي قارئ متبقٍ |

---

## 4. الحكم النهائي

**DEVELOPMENT COMPLETE** (لم يتغير).

المسار الكانوني (اكتشاف → اقتصاد → مخاطر → بناء → V1 → محاكاة إجبارية → إرسال → تسوية) موصول ومكتمل ومحصّن بـ 954 اختباراً ناجحاً. العوائق أمام SHADOW VALIDATED / PRODUCTION READY هي بنود القسم 3 (اختبارات تفاضلية، fuzz، أداء، shadow soak) + بنود O-1 إلى O-6 قبل أي تشغيل حي.

**أولوية الإصلاح المقترحة:** O-6 (تجاوز بوابة المخاطر) → O-1 (توحيد net_profit) → O-2 (توحيد حراسة الإغلاق) → O-3/O-4 (توحيد البانيات) → O-5 → الباقي.
