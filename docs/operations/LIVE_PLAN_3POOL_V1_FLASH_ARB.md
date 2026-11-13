# خطة تشغيل Live: Meteora DLMM 3-Pool Flash-Loan Arbitrage على Transaction V1

التاريخ: 2026-11-12
الحالة: **خطة — غير مصرح بالتشغيل Live بعد**. أي تنفيذ قبل استيفاء بوابات القبول ممنوع.
المصادر: SIMD-0385 (Activated)، docs.jito.wtf، bam.dev (DBI)، Helius Sender docs، توثيق المشروع (PROJECT_STATE / audit_reports / V1_LIVE_EXECUTION_PATH)، وتحقق مباشر من mainnet RPC.

---

## 0. ملخص بحث الإنترنت (Best Practices مثبتة)

### 0.1 Solana Transaction V1 (SIMD-0385) — رسمي
- **الحالة: `Activated`**. الميزة `enable_tx_v1` **فعّالة على mainnet منذ slot 447,120,000 (epoch 1035)**.
- **تحقق حي (هذا اليوم 2026-11-12 عبر `api.mainnet-beta.solana.com`): epoch = 1084، absoluteSlot = 468,547,732** → V1 ينزل فعلياً على الشبكة الرئيسية.
- قيود الـ sanitization (كسر أي منها = المعاملة غير صالحة):
  - الحجم ≤ **4,096 بايت**، ≤ **64 حسابات**، ≤ **64 تعليمات**، ≤ **12 توقيع**، بدون ALTs نهائياً، بدون duplicate addresses.
  - بايت أولى `0x81` (129). `num_readonly_signed < num_required_signatures`.
  - `ComputeBudgetProgram` instructions **تُتجاهل للإعداد** لكنها ما زالت تستهلك CU → **يجب إزالتها من الـ v1** (الـ compiler عندنا بيعمل ده).
  - **Priority fee = إجمالي lamports (u64 LE، البتّان [0,1] معاً وإلا المعاملة غير صالحة)** — مش micro-lamports لكل CU. CU limit = bit 2 (u32). Loaded-accounts-data-limit = bit 3 (**لو 0 → تُحسب صفحة واحدة 32KiB للـ cost model**). Heap = bit 4 (مضاعف 1KiB في [32KiB, 256KiB]).
  - لو bit 3 غير محدد → loaded limit = 0 → 1 صفحة للحسابات (انتبه: حساب كبير محمّل بالكامل لازم يحددها صراحة).
- **لا يوجد ALTs في v1** — كل الحسابات تُذكر صراحة → ميزانية الـ 64 عنوان هي القيد الأقسى للمسار 3-pool.

المصدر: `solana-improvement-documents/proposals/0385-transaction-v1.md` (raw على GitHub).

### 0.2 Jito / BAM — رسمي
- **BAM Direct Bundle Ingest (DBI) يقبل صراحة legacy وv0 وv1 (SIMD-0385)**، والحزمة الواحدة قد تمزجها. العقدة تقرأ الـ version من بايتات المعاملة نفسها — لا يوجد خيار request للنسخة أو الحجم:
  - v1: حد أقصى **4,096 بايت**، بايت أولى `0x81`، لا ALTs، ≤64/64/12 (accounts/instructions/signatures).
  - **لازم ضبط كل من CU limit و loaded-accounts-data-size-limit في ترويسة الرسالة. غير المحدد = 0.**
  - Priority fee هو **إجمالي lamports** وليس micro-lamports/CU.
  - **v1 تُنزل فقط حيث `enable_tx_v1` مفعلة. على mainnet مفعلة منذ slot 447,120,000 (epoch 1035). BAM يقبل v1 على أي حال، فلو كانت الميزة غير مفعلة على cluster آخر تُقبل الحزمة لكنها لن تُنزل.**
  - DBI allowlisted (form.typeform.com/to/tQDIpkYh)، JSON-RPC UUID أو QUIC keypair — اعتمادية منفصلة لكل transport.
- **Block Engine (`sendBundle`)**: التوثيق الرسمي لا يعطي ضمان صريح بقبول 4096-byte v1 على ingress؛ نفس تحفظات `RESEARCH_AND_UNIFIED_ROUTE_INTEGRATION_2026-11-11.md`: *لا تفترض أن sendBundle أو كل ingress إقليمي يقبل v1؛ اشترط canary موقّع >1232 بايت على مسار غير مالي + فك ترميز المعاملة النازلة قبل التفعيل*.
  - Minimum tip = **1,000 lamports** للـ Block Engine (قد لا تكفي في المنافسة).
  - عند `sendBundle`: **الـ tip وحده يهم**. عند `sendTransaction`: توصية 70/30 (priority fee / tip).
  - **الـ tip داخل المعاملة الرئيسية نفسها** (مش معاملة منفصلة)، و**لا تستخدم ALT لحساب الـ tip** (inفعّال على غير Jito-Solana leaders).
  - خلّي المعاملة في نفس الحزمة أول index، أو استخدم `jitodontfront…` (اختياري، ضد الـ sandwich، read-only account).
  - تحقق من النزول عبر `getInflightBundleStatuses` / `getSignatureStatuses` على RPC الخاص بك (DBI لا تظهر في bundle explorer).
  - الحدود الافتراضية: 1 req/s لكل IP لكل region (رفعها عبر ticket).
  - حلول "uncle bandit": tip داخل نفس المعاملة + فحوصات state قبل/بعد — موجودة في التصميم عندنا (repay على السلسلة + min_out).

المصدر: docs.jito.wtf (bundles, lowlatencytxnsend)، bam.dev/docs/bam/dbi.

### 0.3 Helius Sender — رسمي
- كل Sender transaction **لازم tip + priority fee**:
  - `sender-max`: حد أدنى tip **0.001 SOL** للدخول في priority tip buffer؛ `swqos-only`: **0.000005 SOL** كأقل تكلفة.
  - ملاحظة Sender يتطلب tip إلى **حسابات Helius الخاصة** (مختلفة عن حسابات Jito الثمانية!) + priority fee عبر `ComputeBudgetProgram::setComputeUnitPrice`.
  - **v1 لا تحمل ComputeBudget instructions** (تُتجاهل للإعداد) → لا يوجد تأكيد رسمي أن Sender Max يقبل v1 حالياً. **يُعتبر غير مؤكد حتى يُختبر canary صغير — وإلا اعتمد v0/legacy عبر هذا الـ transport.**
- حدود: 50 req/s لكل API key لكل region؛ بدون مفتاح 1 req/s لكل IP. يوجد `?mev-protect=true` وربط الإقليمي الأقرب + connection warming عبر `/ping`.

المصدر: helius.dev/docs/sending-transactions/sender.

### 0.4 Kamino Flash Loan (klend) — مرجع تنفيذي (إلزامية)
- `flash_borrow_reserve_liquidity` في بداية الـ tx و`flash_repay_reserve_liquidity` في نهايتها بنفس الـ tx — **الذرّية مضمونة بترتيب على السلسلة + فحوصات klend** (`liquidity_amount == borrow amount`؛ الرسوم تُخصم فوق الأصل).
- الـ repay لازم يساوي رأس المال بالضبط (principal)، والرسوم Q64.64 تُقرأ من الـ reserve (عندنا: `verified.flash_loan_fee_sf` من snapshots حية — صحيح وفش-closed).
- **WSOL فقط** عبر Kamino Main Market WSOL Reserve → المسار يجب أن يفتح ويُغلق بـ WSOL (يتوافق مع WSOL→A→B→WSOL).

### 0.5 Meteora DLMM — مثبت داخلياً
- التسعير مطابق **حرفياً** لـ `@meteora-ag/dlmm@1.9.14` (16 حالة، diff = 0). Bin arrays بالتجاوز الحقيقي، bitmap extension، Token-2022 fail-closed. لا تغيير مطلوب في المنطق — المطلوب ربطه وإثباته.

### 0.6 قواعد التشغيل Production (تجميع من المصادر أعلاه)
1. Tip داخل المعاملة الرئيسية دائماً؛ account عشوائي من 8 (Block Engine) أو 10 (Helius)؛ لا ALT للـ tip.
2. Priority fee إجمالي lamports لـ v1 (وليس micro-lamports/CU) وحسابها بـ ceil: `(micro × CU + 999,999) / 1,000,000`.
3. ابدأ بـ non-broadcast simulation دائماً؛ skip_preflight فقط بعد soak مثبت.
4. تأكد من `maxSupportedTransactionVersion: 2` في كل استدعاء `simulateTransaction/getTransaction/getBlock` (تم إصلاحه 2026-11-12 — G2).
5. Connection warming + regional endpoint الأقرب للـ infra.
6. راقب النزول عبر `getSignatureStatuses` على RPC خاص — لا تثق باستجابة "accepted".
7. Kill-switch drill قبل أي canary حي؛ canary بمبلغ تافه ثم تدرّج.
8. احتفظ بسجلّات (ledger) منفصلة للـ simulated vs realized PnL.

---

## 1. الوضع الحالي (Baseline) — ولماذا "Live" ممنوع الآن

| البند | الحالة | الدليل |
|---|---|---|
| بناء 3-leg flash-loan (Kamino borrow → 3× swap2 DLMM → repay + tip) | موجود كوحدة مكتبة + unit tests | `crates/arb-tx/src/dlmm_3leg_flash_arb.rs` |
| ربطه بالـ supervisor | **غير مربوط** — السوبرفايزر يستخدم المسار العام `assemble_flash_arbitrage` + `build_swap_hop_instruction` | لا توجد أي إشارة له خارج arb-tx |
| Bug في v1 path للـ 3-leg builder | `priority_fee_lamports = micro / 1_000_000` بدل `(micro × CU + 999_999) / 1_000_000`، و`FormatSelectionReason::LargePayloadV1 { accounts_count: 0, wire_size: 0 }` — placeholder وهمي | dlmm_3leg_flash_arb.rs ~L420–436 |
| V1 compiler | سليم وموثّق بتفاضل vs solana-message v4.6.0؛ السوبرفايزر يحسب الـ priority fee صح بالـ ceil | run.rs ~L1610؛ adaptive.rs |
| Route حقيقي WSOL→MemeA→MemeB→WSOL | **لا يوجد** موثق | PROJECT_STATE: *"repository contains no verified meme DLMM pool addresses/bin arrays"* |
| Simulation على mainnet للمسار 3-leg بenvelope v1 | **لا يوجد** | لا artifact في reports/ ولا docs |
| Transport يقبل v1 | DBI مؤكد نظرياً (allowlisted)؛ Sender Max غير مؤكد؛ Block Engine غير مضمون | bam.dev / helius.dev / docs.jito.wtf |
| ميزانية الـ accounts/bytes الحقيقية للمسار | **غير مقاسة** على route حقيقي (خطر تجاوز 64 حساباً أو 4096 بايت) | — |
| Audit | `18_FINAL_AUDIT_VERDICT` = **BLOCKED – CRITICAL** (8 CRITICAL)؛ بعضها عولج لاحقاً لكن **لا re-audit ولا sign-off** | audit_reports/ |
| Phase 15.2 | **BLOCKED – CORRECTNESS ISSUE** (ROI anomaly ~965%؛ الإصلاح كُتب لكن لم يُعاد التحقق) | PROJECT_STATE |
| `.env` | `ARB_LIVE=1`, `LIVE=true`, `EXECUTE_LIVE=true` — **غير مصرح**، تُفرض بـ override في scripts فقط | residual risk مسجل |
| Arming الافتراضي | `mode = "shadow"`، `ARB_LIVE=0`، canary = simulated only | config.toml / EXECUTION_LIFECYCLE |

---

## 2. الخطة الشاملة (Phases / Gates)

> قاعدة ذهبية: **لا تنتقل من Phase إلى التالية إلا ببوابة قبول موقّعة (evidence + توقيع operator).** أي تخطّي = توقف.

### Phase 0 — نزع السلاح والأمان (يوم 0)
- [ ] `.env`: `ARB_LIVE=0`, `LIVE=false`, `EXECUTE_LIVE=false`, `DRY_RUN=true`, `SOLANA_TX_V1_*=false`. حذف/تدوير أي keys live مستخدمة بالخطأ.
- [ ] Kill-switch drill: تأكد `KILL_SWITCH_PATH` يعمل فعلياً على نفس النسخة (إيقاف/تشغيل).
- [ ] فصل محفظة canary بحد أقصى للرصيد (مثلاً ≤ 0.2 SOL + 10 USDC) — مفتاح `CANARY_KEYPAIR_PATH` منفصل، لا تلمس أي key إنتاجي.
- [ ] تأكيد `getEpochInfo` ≥ 1035 على كل RPC مُعدّ (عندنا: epoch 1084 ✅ للـ public endpoint؛ تحقق من endpoints الإنتاج من هنا نفسك).
- [ ] تسجيل الحالة: commit للـ .env example فقط؛ الـ .env الحقيقي لا يُرفع (git status نظيف).
- **بوابة 0:** صفر متغيرات live = 1 بالقيم؛ توقيع operator.

### Phase 1 — إغلاق عيوب الكود المعروفة (يوم 1–2)
1. **إصلاح dlmm_3leg_flash_arb.rs**:
   - `priority_fee_lamports = (cu_price as u128 * cu_limit as u128 + 999_999) / 1_000_000` مع fail-closed على تجاوز u64.
   - تمرير `accounts_count`/`wire_size` الحقيقيين في `FormatSelectionReason::LargePayloadV1` (قيساً بعد التجميع، مش placeholder).
   - توحيد `loaded_accounts_data_size_limit` (262_144 v1 config في two-pool vs 67_108_864 في 3-leg — حدّد قيمة معلنة ومُختبرة، 256KiB تكفي عادة؛ 0 خطأ ممنوع).
   - إزالة V1 tip-strip / أي dead fallback (تحقق من v1_fallback).
2. **قرار معماري واحد ومكتوب:** إما (أ) ربط dlmm_3leg_flash_arb بالمسار `capture_three_leg_dlmm` بدل المسار العام، أو (ب) اعتماد المسار العام + AdaptiveTxCompiler (الأنضج حالياً). **يُفضل (ب) للـ canary**، ويُبقى (أ) كوحدة tests/تحسين لاحق — لا تشغّل مسارين في الإنتاج معاً.
3. إعادة تشغيل: `cargo fmt --check`، `cargo check --workspace --all-targets`، `cargo clippy --workspace --all-targets -- -D warnings`، `cargo test --workspace --all-targets --no-fail-fast` — كلها خضراء + حفظ الـ logs.
4. إصلاح `net_profit = gross` في capture (اشتق الـ net من الـ gate الحقيقي، لا تمرير gross كنهائي).
5. التأكد أن `maxSupportedTransactionVersion: 2` مُمرر في كل simulate (موجود — G2؛ أضف regression test للمسار 3-leg).
- **بوابة 1:** قائمة 16_COMPLETE_BUG_REGISTER.csv محدثة، الـ CRITICAL المفتوحة الملامسة للمسار= صفر، توقيع.

### Phase 2 — Discovery وإثبات الـ Route (يوم 2–4)
- [ ] شغّل shadow على الـ feed الحقيقي (`run_shadow.ps1` فقط) واجمع كل 3-leg candidate لفترة ≥ 72 ساعة.
- [ ] لكل candidate: `pool addresses + bin arrays resolved + bitmap extension + mints + token programs + active bin slot + reserve/oracle + execution accounts complete`.
- [ ] **قياس فعلي**: عدد العناوين distinct و حجم wire لكل envelope (legacy / v0+ALT / v1). القيد الصارم: **≤ 64 عنوان و ≤ 4096 بايت لـ v1**. لو تجاوزت: قلّص bin arrays بعناية (exact traversal، لا أكثر)، أو v0 + ALT (منشور وموثّق + التوقيع)، أو تخلّص من leg.
- [ ] Sim gate على الـ candidate: `result.gross_profit_lamports > 0` **و** net بعد كل التكاليف > `min_net_profit_abs` **و** نسبة ROI منطقية (< مثلاً 50% — رفض أي شيء أهم)، وstale rejection سليم.
- [ ] توثيق triple معتمد (CSV/JSON) + paper journal (OpportunityHistoryRecorder) بمدّة ≥ أسبوع.
- **بوابة 2:** ≥ 1 triple بـ ≥ 5 محاولات profitable منطقية مسجلة بأرقام فعلية (slots, sizes, gross, net).

### Phase 3 — Simulation دليل (يوم 4–5)
- [ ] بناء الـ TX كامل موقّع (canary keypair) ومحاكاته على **mainnet-beta غير-broadcast** مع `maxSupportedTransactionVersion: 2`:
  - `simulateTransaction` يعيد `unitsConsumed > 0`، ولا `InstructionError`، مع reconciliation رصيد WSOL قبل/بعد + **repay نجح** (logs من [5] و[9]) وmints محفوظة.
  - حفظ الـ evidence JSON (النمط الموجود: `reports/phase15_2/mainnet_simulation_evidence.json`) + slot basis.
- [ ] اختبار إجهاض مقصود: غيّر min_out ليفشل → يجب أن تفشل المحاكاة → على السلسلة ستفشل → **لا رسوم سوى base fee ولا tip يُدفع**.
- [ ] فحص CU المستهلك مقابل `estimate_route_cu`/`cu_limit` — عاير `COMPUTE_BUDGET_LIMIT_3HOP` (120_000 default مش موثوق لـ 3 swaps).
- [ ] **v1-specific assertions:** بايت 0 = `0x81`، mask صحيح (bits 0+1 لـ priority fee = lamports إجمالي، bit 2 CU، bit 3 loaded، bit 4 heap)، لا `ComputeBudget` instructions داخل الـ payload، لا ALT، ≤12 توقيع.
- **بوابة 3:** artifact JSON موقّع + diff نظيف بين quote وsim logs (صفر drift).

### Phase 4 — Transport Bring-up (يوم 5–7)
ترتيب المخاطر من الأدنى للأعلى:
1. **SWQoS مباشر (Staked validator connection)** — الأسرع، لا يحتاج BAM allowlist، يفضّل أن يكون أول canary v1. المتطلبات: حدّد الـ staked endpoints، tip داخل نفس الـ tx، sender يرسل v1 (تأكد من إصدار العميل: Agave ≥ الإصدار الداعم لـ SIMD-0385 على الشبكة).
2. **BAM DBI** — مدعوم رسمياً لـ v1 (JSON-RPC `sendBundle` أو QUIC). يتطلب **allowlisting + JSON-RPC UUID أو QUIC keypair** + regional endpoints (`<region>.mainnet.bam.jito.wtf`). مثالي للمسار الإنتاجي (atomic + revert protection + خصوصية قبل التنفيذ).
3. **Jito Block Engine** — فقط بعد "signed >1232-byte canary على مسار غير مالي + landed decode" (شرط `RESEARCH_…2026-11-11.md`). الافتراض الحالي: ingress لا يضمن v1/4096.
4. **Helius Sender** — غير مؤكد لـ v1 (يتطلب priority fee كـ ComputeBudget instruction ودمج tip بحسابات Helius ≠ حسابات Jito). **لا تفعّله لـ v1 حتى ردّ رسمي أو اختبار canary**. يمكن استخدامه للمقارنة بـ v0/legacy لاحقاً.
5. Fallback للـ v1 في حال فشل transport: **v0 + ALT منشور** (operator-signed، عنوان موثّق، rotation plan) أو إلغاء الـ leg الزائد.

خطوات:
- [ ] اختر transport واحد للـ canary (الافتراضي الموصى به: SWQoS؛ الأفضل إنتاجياً: DBI بعد الموافقة).
- [ ] تدرّب على canary بمبلغ تافه: 1–3 محاولات retry-free، landing عبر `getSignatureStatuses` على RPC خاص (وليس "accepted" من الـ ingress).
- [ ] Logging كامل: slot sent / slot landed / tip paid / actual outcome vs simulated.
- **بوابة 4:** ≥ 1 canary نازل مثبت بأرقام (slot, tip, realized delta) و≥ 0 tip مدفوع في أي معاملة فاشلة.

### Phase 5 — Shadow Soak + Re-audit (أسبوع 1)
- [ ] Shadow soak مستمر ≥ 7 أيام على الـ candidates الحقيقية (لا synthetic): نسبة القبول/الرفض، سبب الرفض، freshness، latency p50/p95/p99.
- [ ] Replay ≥ 1,000 route عبر المسار المصلح — sim-pass rate، CU calibration، wire sizes.
- [ ] Re-audit كامل ضد `16_COMPLETE_BUG_REGISTER.csv` (يفضّل مراجع خارجي + مستقل): صفر CRITICAL/HIGH مفتوح.
- [ ] Re-validation لشذوذ ROI (Phase 15.2): نسبة ROI معقولة على كل candidate مقبول؛ صفر implausible ROI.
- [ ] تحقق عملي من kill-switch و Telegram alerts أثناء الـ soak.
- **بوابة 5 (Sign-off للـ Live):** توقيع operator مكتوب + تحديث PROJECT_STATE + tag في git (`v1-live-canary-approved`).

### Phase 6 — Live Canary الإنتاج (أسبوع 2)
- [ ] canary keypair بحدود: `max_trade_size_usd` صغير (مثلاً ≤ $25)، `max_tip_lamports` ≤ 50,000، `max_orders_per_hour` ≤ 10، min reserve محفوظ.
- [ ] 1 معاملة واحدة في المرة؛ لا توازي؛ `EXECUTE_LIVE=true` **لهذه الجلسة فقط** وبموافقة صريحة.
- [ ] مراقبة: lands، RPC cross-check للـ balance delta، alert فوري عند أي fail غير متوقع؛ auto-disarm عند: fail ≥ 2 متتاليتين، أو tip paid بلا ربح، أو drift بين sim وrealized.
- [ ] Backout الكامل: `ARB_LIVE=0` + kill-switch + إيقاف العملية — ≤ 60 ثانية.
- **بوابة 6:** ≥ 5 canaries ناجحة متتالية بدون خرق حدود، PnL موجب أو محايد.

### Phase 7 — Scale-up (تدريجي، فقط بعد بوابة 6)
- رفع الحدود تدريجياً (25%→50%→100%)؛ تفعيل DBI كـ primary وBlock Engine كـ fallback؛ تفعيل multi-region؛ ALT إذا لزم (v1 لا يحتاجها — الأفضل الاستمرار بدون ALT)؛ توسيع الـ triples المسموحة (قائمة بيضاء موقّعة فقط).
- هل ننتقل لـ Continuous Live؟ فقط بمراجعة أسبوعية + تقرير PnL حقيقي موقّع.

---

## 3. Risk Register القصير (أعلى 8)

| # | الخطر | الاحتمال | الأثر | التخفيف |
|---|---|---|---|---|
| R1 | تجاوز 64 عنوان / 4096 بايت في المسار الحقيقي | متوسط | المعاملة غير صالحة فوراً | قياس Phase 2؛ تقليل bin arrays؛ v0+ALT fallback موقّع |
| R2 | Transport لا يقبل v1 (ingress/version) | متوسط–عالٍ | dropped / acknowledged-but-never-lands | DBI أو SWQoS؛ signed canary غير مالي قبل أي transport؛ رفض fail-closed (موجود) |
| R3 | Tip مدفوع على معاملة فاشلة | منخفض | خسارة مباشرة | Tip داخل نفس الـ tx (موجود) + canary صغير + abort gates |
| R4 | Route stale بين quote وbuild | متوسط | revert / خسارة | max_state_age_slots fail-closed (موجود) + CU limit محكم + blockhash freshness |
| R5 | Priority fee v1 محسوب غلط (micro vs total lamports) | منخفض بعد Phase 1 | عدم نزول أو دفع زائد | assert mask bits + diff test للـ header (موجود) + مراجعة يدوية للـ artifact |
| R6 | Kamino repay أقل من المطلوب | منخفض | revert دائماً | snapshot حي للـ flash_loan_fee_sf (موجود) + assert repay == borrow على مستوى الكود والحسابات |
| R7 | Sandwich/MEV على route معروف | متوسط | أسوأ سعر تنفيذ | mev-protect (SWQOS/Sender)، jitodontfront، slippage محكم، tip داخل الـ tx |
| R8 | `.env` live flags مُفعّلة بالخطأ خارج canary | منخفض | تنفيذ غير مصرح به | Phase 0 disarm + canary keypair منفصل + git hook/CI check |

---

## 4. Acceptance Criteria للـ "Live" النهائي
1. بوابات 0–6 موقّعة بالترتيب، بلا تخطٍ.
2. ≥ 1 canary نازل + ≥ 5 canaries إنتاجية ناجحة.
3. PnL realized محسوب بدقة (RPC cross-check) ومطابق للـ simulated ضمن tolerance مُعلنة.
4. صفر معاملة فاشلة دفعت tip؛ صفر breach للحدود؛ kill-switch يعمل وقت الحاجة.
5. PROJECT_STATE محدّث + tag + audit register نظيف.

## 5. مصادر البحث المستخدمة
- SIMD-0385 raw (Activated) + القيود حرفياً: github.com/solana-foundation/solana-improvement-documents/blob/master/proposals/0385-transaction-v1.md
- BAM DBI — دعم v1 صراحة، شروط النزول، epoch 1035، 447,120,000: bam.dev/docs/bam/dbi
- Jito Bundles + sendBundle/sendTransaction/70-30/minimum tip/jitodontfront/getInflightBundleStatuses: docs.jito.wtf (bundles, lowlatencytxnsend)
- Helius Sender — إلزامية tip + priority fee، حدود 50 rps، حسابات tip مختلفة: helius.dev/docs/sending-transactions/sender
- Mainnet الحي (2026-11-12 عبر api.mainnet-beta.solana.com): epoch=1084، slot=468,547,732 → enable_tx_v1 مفعلة.
- داخلي: ../reference/V1_LIVE_EXECUTION_PATH_2026-11-12.md، ../reference/METEORA_TWO_POOL_V1_2026-11-12.md، ../PROJECT_STATE.md، audit_reports/18_FINAL_AUDIT_VERDICT.md، audit_reports/17_REMEDIATION_PLAN.md، RESEARCH_AND_UNIFIED_ROUTE_INTEGRATION_2026-11-11.md.

## 6. ما لا يُفعل أبداً (من التدقيق)
- لا تُزل Kill-switch أو ARB_LIVE "com لتسهيل التشغيل" — نشرة 2026-11-09 القديمة غير معتمدة.
- لا تشغّل على production keys أثناء الإعداد؛ canary keypair منفصل فقط.
- لا تفعّل DBI بدون موافقة allowlist رسمية؛ لا تفترض Block Engine يقبل v1.
- لا تغيّر حد الـ slippage أو min-profit لإنجاح route خاسر.
- لا تخلط ledger الـ simulated مع الـ realized.
