# MASTER_ENGINEERING_CHECKLIST.md — Master Engineering Checklist

> **المرجع الإلزامي لكل Agent يعمل على هذا الريبو.**
> هذه القائمة تجميع لكل الملاحظات التي ظهرت في المراجعات السابقة، وليست ادعاءً
> بأن كل ملف أُعيد فحصه. كل بند يحتاج إعادة تحقق من أحدث `main` قبل اعتباره
> عيباً ما زال قائماً.
>
> **قاعدة التشيك بوكس:** البند يُعلَّم `[x]` فقط بعد إثبات حالته بدليل
> (كود، اختبار، تقرير). التعديل وحده لا يحوّل البند إلى مكتمل.

## معلومات القائمة

| البند | القيمة |
|---|---|
| إجمالي البنود | 495 نقطة تدقيق وإصلاح ومراجعة |
| التقسيم | 20 محوراً هندسياً |
| النطاق | Full repository audit |
| طريقة التعامل | Evidence-first |
| التشغيل | Shadow / Simulation |

## حالات البنود المعتمدة

| الحالة | معناها |
|---|---|
| `NOT AUDITED` | لم تتم مراجعته |
| `VERIFIED OK` | سليم بالدليل |
| `CONFIRMED BUG` | عيب مؤكد |
| `MISSING` | مكون ناقص |
| `PARTIAL` | منفذ جزئياً |
| `BLOCKED` | متوقف بسبب اعتماد خارجي |
| `FIXED` | تم الإصلاح |
| `TESTED` | اجتاز الاختبار |
| `ACCEPTED` | اجتاز معيار القبول |

> **قاعدة:** لا ينتقل البند من `FIXED` إلى `TESTED` بمجرد تعديل الكود، ولا من
> `TESTED` إلى `ACCEPTED` إلا بعد اجتياز معيار القبول المحدد.

## ترتيب التنفيذ الإلزامي (لا قفز بين المراحل)

1. Provider Infrastructure + WebSocket + Quotas.
2. Account Dependency Planner + State Integrity.
3. Exact DEX Quote Parity.
4. Dynamic Discovery + Flash Borrow Optimization.
5. Net Profit Engine.
6. Atomic Execution + Jito.
7. Fuzzing + Integration + Shadow Soak.
8. Production Acceptance.

---

## Audit Ledger — نتائج التدقيق الفعلي

> تدقيق فعلي منفَّذ على commit `72cc41c` (أحدث main وقت التدقيق)؛ آخر تحديث للقائمة على main `4241e4c` (بعد دمج PR #53 و #55 و #56).
> المنهج: قراءة الكود + تشغيل `cargo build --workspace` +
> `cargo test --workspace --all-targets` كاملة.

### التقدم الحالي

- **157 / 495** بند متحقق بدليل ومُعلَّم `[x]` (منها 6 بنود بدليل حي من mainnet شاملة معاملة flash-loan landed).
- **338** بند متبقي: فجوات G1–G4 وG15 أُصلحت ومُختبرت (PR #55 و #56)؛ أُصلحت G7 وG8 وG10 وG11 وG13 وG14 وG16؛ ما زالت G5 وG9 وG12 فجوات؛ G6 صارت PARTIAL (حارس mismatch + env override)، والباقي `NOT AUDITED` — يحتاج تدقيقاً فردياً أو تشغيلاً فعلياً (shadow/live) قبل التعليم.

### نتائج التشغيل

- `cargo build --workspace`: نجح (0 أخطاء).
- `cargo test --workspace --all-targets`: **1137 اختبار ناجح، 0 فشل** (بعد PR #57 + إصلاحات G16/G8/G7/G11؛ كان 1094) عبر 88 test binary.
- CI يفرض `cargo fmt --check` و `cargo clippy --workspace --all-targets -- -D warnings` و `cargo-deny` (rust-ci.yml).

### فجوات مؤكدة (CONFIRMED — لم تُعلَّم [x])

| # | الفجوة | الدليل |
|---|---|---|
| G1 | **FIXED/TESTED (PR #56, commit 804d386)**: `ProviderCapabilityRegistry` مربوطة بالـ Supervisor — `run.rs` يبني الـ registry ويسجّل `standard_json_rpc_ws("Helius")` عند نجاح `HeliusWsClient::from_env()` ويستدعي `validate_execution_prerequisites()` قبل إقلاع WS worker؛ الفشل = `bail!` (إيقاف إقلاع fail-closed) | `supervisor/run.rs` — بوابة capability قبل بدء worker |
| G2 | **FIXED/TESTED (PR #56, commit 804d386)**: `ProviderSupervisor` مستخدم في حلقة التشغيل — WS_STREAM_PROVIDER (أولوية 0) + RPC_FALLBACK_PROVIDER (أولوية 1)، record_success/record_failure على تقدّم slot وRPC fallback، و`select()` fail-closed قبل `strategy_cycle` | `supervisor/run.rs` — health observation + بوابة select |
| G3 | **FIXED/TESTED (PR #56, commit 804d386)**: `QuoteReadinessContext`/`DependencyManifest` مربوطة — بوابة `validate_route_account_dependencies` في `process_opportunities` (opportunity.rs:75) تستدعي `quote_dependencies_for_route` مع manifests من `manifest_for_registered_pool` (مصدر وحيد)؛ + إصلاح provenance في `helius_ws.rs` (provider_id من yellowstone بدل ProviderId(2) الثابت) | `supervisor/opportunity.rs:75,1424`, `arb-ingest/src/lib.rs:766` |
| G4 | **MITIGATED/VERIFIED (PR #53 + PR #56)**: reconcile sweep دوري بحد أدنى 30s بدل كل slot notification؛ والبوابات الجديدة (G2/G3) تمنع أي حالة قديمة من الوصول للتسعير/التنفيذ. الحد المتبقي: vault updates للـ DLMM/CLMM/Orca/PumpFun ما زالت عبر hydration وليست streams | `supervisor/run.rs` — min_gap reconciliation |
| G5 | Solami Mirage parser لا يستخرج حقول protobuf حقيقية: يستخدم `latest_slot` محلي، `pubkey: None`, `write_version: None`، وغير موصّل بالبوت | `solami_mirage.rs:138-140` |
| G6 | **PARTIAL/MITIGATED**: حارس fail-closed جديد `flash_mint_matches_kamino_reserve()` — أي config بـ flash mint ≠ WSOL مع WSOL reserve (والعكس) يُرفض بـ `FLASH_MINT_RESERVE_MISMATCH` قبل أي حساب؛ هذا أكّد أن rehearsal كان سيقيّم USDC flash loan بمخاطبة WSOL reserve. `FLASH_BORROW_MINT`/`FLASH_BORROW_MINT_DECIMALS` env يسمحان بأصل اقتراض غير WSOL في الـ rehearsal. المتبقي: اختيار أفضل reserve اقتصادياً تلقائياً من مسح Kamino reserves الحي | dlmm_flash_arbitrage.rs — 3 اختبارات جديدة |
| G7 | **FIXED/TESTED**: علم provenance `sol_price_live` في TelegramNotifier — قيمة $200 الافتراضية لم تعد تُقدَّم كدليل سوق؛ التنبيهات تعرض وسم «سعر غير موثّق» حتى يأتي سعر حي (Binance/Coinbase أو pool SOL/USDC أو SOL_USD_PRICE) | telegram.rs — `unverified_sol_price_is_labeled_as_such` |
| G8 | **FIXED/TESTED**: gating ثنائي لـ slotSubscribe — env `WS_SLOT_SUBSCRIBE` + رفض حي (id=1) يعطّل slot sub فقط دون `endpoint_unsupported`؛ slot freshness يعتمد context.slot من accountNotification | helius_ws.rs — اختباران جديدان |
| G9 | لا يوجد cargo-fuzz harness منفصل — لكن proptest fuzzing suite موجود (500 cases، 4 crates) | `arb-math/tests/fuzzing_suite.rs` + proptest في arb-config/arb-dex/arb-tx |
| G10 | **FIXED (CI step added)**: job `secrets` في rust-ci.yml — gitleaks-action@v2، fetch-depth: 0 (تاريخ كامل)، GITLEAKS_CONFIG=.gitleaks.toml؛ سياسة PR #53 تبقي كل قواعد default نشطة وallowlist ضيّق فقط | rust-ci.yml — التحقق النهائي عند أول تشغيل CI لهذا PR |
| G11 | **FIXED/TESTED**: unit tests لكل من `damm_v2` (10: fee bps/ceil، cap 50%، linear scheduler، رفض fee≥denominator، صفر/سيولة، تناظر الاتجاهين مع virtual reserves، monotonicity، range clamp، fee mode على الدخل/المخرج)، `launchlab` (10: رسم الدخل المسبق مع إعادة حساب مستقلة، ثابت x·y=k الرسمي من docs.raydium.io، caps real reserves، round-trip خسارة صارمة، monotonicity)، `pump_fun_raydium` (8: ceil rounding، fee floor، رفض reserves منحلة، لا optimum عند تساوي الأسعار، اتساق داخلي للـ trade، bracketing للـ optimum عبر verify_marginal_decreasing) — 28 اختباراً جديداً | `cargo test -p arb-math --lib`: 148 passed / 0 failed |
| G13 | **FIXED/TESTED**: ExecutionRecord توسّع بحقول اختيارية (signature/slot/net_profit_sol) مع skip_serializing_if للتوافق الخلفي؛ record_landed_trade_task يعبّئ signature+slot+profit من TradeSummary، record_dropped_bundle_task يعبّئ bundle_id+slot؛ Executions.tsx يعرض أعمدة Signature/Slot/Net Profit | control_service.rs + supervisor.rs + Executions.tsx — `execution_extras_serialize_only_when_present` |
| G14 | **FIXED/TESTED**: ring مخصص للرفض (REJECTION_RING_CAP=256) في ControlService لا يُفيض عليه executions ring؛ /api/rejections + حدث WS "rejection"؛ مكوّن Rejections.tsx جديد (تبويب D2) يعرض stage/reason/gross/net/loan/pools/slot | control_service.rs + web_api.rs + Rejections.tsx — `rejection_ring_is_dedicated_and_ordered` |
| G15 | **FIXED/TESTED (PR #55, commit 92e1612)** — **تم تشخيصه بدقة (تصحيح التشخيص الأول)**: سبب الرفض الحي ليس الـ wire encoder — الـ encoder مطابق بايت-بايت للتنفيذ الرسمي `solana-message::v1` (تمت مقارنة كل الحقول: header/mask/lifetime/addresses/config-values/instruction-headers/payloads/signature-at-tail). السبب المؤكد تجريبياً: قيمة `loaded_accounts_data_size_limit` القريحة في الـ config. القياس الحي على mainnet (Agave 4.4.1): عمود kamino flash يحمّل فعلياً **10,714,291 بايت (~10.2MiB)** لأن بيانات البرامج التنفيذية (klend + SPL Token + ATA) تُحتسب ضمن الحد؛ أي حد < ~10.72MiB يُرفض فوراً بـ `MaxLoadedAccountsDataSizeExceeded` عند units=0 قبل تنفيذ أي instruction. مغلف V1 بحد 64MiB (67,108,864) مرّ المحاكاة الحية err=None. مسارات الإنتاج الصحيحة (opportunity.rs:1050,1108 وselection.rs:481 وtemplate.rs:529) تستخدم 64MiB؛ المسارات المعطوبة: `dlmm_4leg_flash_arb.rs:621,704`=256,000 و`compiler.rs:1047`=262,144 و`dlmm_3leg_flash_arb.rs:531`=1,048,576 — كلها ستُرفض حياً — **أُصلحت كلها في PR #55** بثابت `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT=67,108,864` + 5 اختبارات wire-decoding regression (`v1_loaded_data_limit_regression.rs`)؛ `meteora_atomic_builder.rs:53` (8MiB) أُصلحت أيضاً | دليل حي 2026-11-17 على Solami Agave 4.4.1: limit=64MiB → err=None loaded=10,714,291؛ limit=4MiB/1MiB/256KiB/32KiB → err=MaxLoadedAccountsDataSizeExceeded units=0 (مكرر 4 مرات)؛ v0 sig 5w9k6SeA…gX finalized |
| G16 | **FIXED/TESTED**: defaults المحرك موحّدة على ثوابت arb-constants — `hop_slippage_bps: DEFAULT_HOP_SLIPPAGE_BPS` و`cu_limit: APPLICATION_SAFETY_CU_LIMIT` في dlmm_flash_arbitrage.rs وmeteora_fresh_pipeline.rs والأمثلة؛ priority fee للـ flash path صار ثابتاً موثّقاً `FLASH_ARB_PRIORITY_FEE_MICRO_LAMPORTS=10_000` (سلوك محفوظ، مصدر واحد)؛ اختبار `defaults_match_canonical_constants` يثبّت العقد | arb-constants + arb-engine + arb-sim |
| G12 | لا يوجد دليل على 24-hour Shadow Soak لا يوجد دليل على 24-hour Shadow Soak (الأدوات موجودة: `run-shadow.sh`, `docker-compose.shadow.yml`؛ `reports/production/` فيه baseline + acceptance matrix فقط) | `reports/production/` |

### أدلة حية من mainnet (2026-11-17 — جلسة التحقق الحية)

- **RPC حي**: Helius `getSlot=470350734` / `getHealth=ok`؛ Shyft `getSlot=470350735`؛ Solami `getSlot=470350735` — نفس الكلاستر، slot متطابق ±1.
- **Quota حي**: كلا المزودين الثلاثة ردّوا HTTP 429 بعد التجاوز — `registry` وضعهم cooldown 60s ودوّر بينهم (سلوك circuit-breaker مُلاحَظ فعلياً).
- **WSS حي**: Helius + Shyft يرفضان handshake بـ HTTP 429 (خطة مجانية مستنفدة)؛ Solami يقبل ثم يرفض code 4002 "no bandwidth and balance under the 0.06 minimum" — الخطة المجانية غير قابلة للـ streaming.
- **Rehearsal حي** (`full_path_rehearsal` على mainnet): discovery 253 pools (80 MeteoraDlmm)، تقييم flash sweep لكل أزواج DLMM، كل المسارات fail-closed (PROFIT_BELOW_FLOOR / INTERMEDIATE_ABOVE_POOL_LIQUIDITY)، وبوابة السداد رفضت البناء القسري: conservative out 9,937,753 < repay 10,000,100 raw — لا شيء بُني، لا شيء بُثّ.
- **خلاصة**: السوق كان كفئاً في لحظات القياس؛ نظام fail-closed يعمل حتى آخر خط دفاع. البث الحي (WSS) محجوب بحدود الخطط المجانية — يحتاج مفتاح مدفوع أو شحن رصيد Solami.

### القيم الثابتة المؤثرة على الربحية (تدقيق مطلوب من المالك — 2026-11-17)

| القيمة الثابتة | الموقع | التأثير على الربحية |
|---|---|---|
| `DEFAULT_JITO_TIP_SHARE_BPS=7_000` (70% من الهامش) و`MAX_TIP_SHARE_BPS=8_500` (85%) و`DEFAULT_KEEP_SHARE=0.85` | `arb-constants/src/lib.rs:73-83` | **الأكبر تأثيراً**: 70–85% من كل فرصة تذهب tip — صافي المشغّل 15–30% فقط من الهامش |
| `DEFAULT_MIN_PROFIT_LAMPORTS=50_000` (≈$0.009 عند $180) | `arb-constants:71`, `arb-config:447` | أرضية ربح شبه معدومة كـ default؛ والتعليق في opportunity.rs يعد بـ P_net ≥ $0.20 — تعارض بين التوثيق والقيمة |
| `DEFAULT_HOP_SLIPPAGE_BPS=10` مقابل default `hop_slippage_bps: 5` في المحرك | `arb-constants:462` مقابل `dlmm_flash_arbitrage.rs:368` | min_out المحسوب في sizing قد يختلف عن التنفيذ → سداد يفشل on-chain أو فرص مرفوضة خطأً (G16) |
| `cu_limit: 400_000` في الـ rehearsal > `APPLICATION_SAFETY_CU_LIMIT=350_000` | `full_path_rehearsal.rs` مقابل `arb-constants:482` | قيمة تتجاوز سقف الأمان — لو تسربت للإنتاج ترفض أو تدفع priority أعلى (G16) |
| `priority_fee_micro_lamports: 10_000` (rehearsal) مقابل `BASE_PRIORITY_FEE_MICRO_LAMPORTS=1_000` | rehearsal مقابل `arb-constants:492` | 10× تكلفة priority في مسار القياس — أي استنتاج ربحية منه مضخّم التكلفة |
| `sol_usd_price: Some(180)` (rehearsal) و`$200` (telegram.rs:63) | ملفات منفصلة | تسعير USD ثابت غير موثوق — G7 |
| `CU_PER_BIN_ARRAY=15_000` تقدير CU ثابت | `dlmm_flash_arbitrage.rs:43` | القياس الحي: borrow+repay وحدهما = 65,461 CU — المُقدِّر يحتاج معايرة بقياس حي |
| `loaded_accounts_data_size_limit=256_000` | `dlmm_4leg_flash_arb.rs:621,704` | **مرفوض حياً**: القياس الحي على mainnet = 10,714,291 بايت محمّلة لأصغر عمود flash (بيانات البرامج التنفيذية تُحتسب) — أي حد < ~10.72MiB يفشل بـ MaxLoadedAccountsDataSizeExceeded قبل أي instruction |
| `loaded_accounts_data_size_limit=262_144` (256KiB) | `flash/compiler.rs:1047` | نفس الفشل الحي المؤكد — مرفوض من mainnet |
| `loaded_accounts_data_size_limit=1_048_576` (1MiB) | `dlmm_3leg_flash_arb.rs:531` | نفس الفشل الحي المؤكد — مرفوض من mainnet (الصحيح: 67,108,864 كما في opportunity.rs:1050,1108) |
| `refine_steps=10` ونمو السلّم 1.5 | `dlmm_flash_arbitrage.rs:342,373,941` | كثافة بحث الحجم — سلّم خشن يفوّت قمة الربح الأمثل |
| `min_borrow=0.01 SOL / max_borrow=0.1 SOL` + `kamino_available_liquidity=5 SOL` سقف rehearsal | `full_path_rehearsal.rs` | يقيّد نطاق اكتشاف الربحية في القياس فقط (ليس إنتاجاً) |
| `max_state_age_slots=2` و`max_allowed_slot_lag=2` وbackoff 30/150ms | `arb-config:452`, rehearsal | صرامة freshness — قيم معقولة لكنها تحكم قبول/رفض كل فرصة |
| `discovery_min_tvl_usd=$25,000` و`max_trade_size_usd=$10,000` و`contingency_buffer=10_000` lamports | `arb-config:455-467` | مرشحات اكتشاف وحجم — تقليلها يفتح فرصاً أكثر مخاطرة |
| `DEFAULT_RPC_MIN_INTERVAL_MS=200` و`MAX_CONCURRENT_REQUESTS=1` | `arb-constants:516-517` | خنق throughput لمسار البيانات — حماية quota مقابل زمن استجابة |
| `METEORA_MAX_BINS_PER_SWAP=260` و`requested_bin_array_depth=1` | `arb-constants:261`, rehearsal | عمق bins المطلوب — depth=1 يحدّ من حجم التداول المحسوب عبر bins |
| `HOP_SLIPPAGE_MAX_BPS=1_000` (10% سقف) | `arb-constants:468` | سقف انزلاق لكل hop — مرتفع كحد أقصى مطلق |

**خلاصة هندسية**: أكبر ثلاث قيم تلاعب بالربحية فعلياً: (1) حصص Jito tip 70–85%، (2) تعارض hop slippage 5/10 bps، (3) أرضية الربح 50k lamports غير المتسقة مع $0.20 الموثّقة. مع G15 (حدود loaded-data القريحة < 10.72MiB المقاسة حياً — وليس الـ wire encoder) هذه أولويات الإصلاح قبل أي تشغيل رابح.

### Acceptance Gates السابقة

- `reports/production/acceptance_matrix.json` يوثق Gates A–D بحالة PASS
  (repository integrity / protocol correctness / economic correctness /
  safety controls) — لكن على base_commit `45031bb` وليس أحدث main؛
  يحتاج إعادة تشغيل قبل الاعتماد النهائي (يرتبط بـ G12).

### ملاحظة منهجية

البند المتعلم `[x]` أدناه يعني: الكود موجود **ومُختبر** في crate الخاص به
بالدليل المذكور. البنود المتعلقة بالربط بالمسار الحقيقي (wiring) صارت مُعلَّمة بعد إثبات الربط الموثق
في PR #56 (G1–G3) و PR #55 (G15).

---

## 1. Provider Infrastructure — مزودو البيانات

- [ ] مراجعة جميع مزودي RPC المستخدمين.
- [ ] التحقق من كل API endpoint.
- [ ] تحديد نوع كل endpoint: HTTP أو WebSocket أو gRPC.
- [ ] التحقق من دعم كل مزود لـ accountSubscribe.
- [ ] التحقق من دعم programSubscribe.
- [ ] التحقق من دعم logsSubscribe.
- [ ] التحقق من دعم slotSubscribe.
- [ ] التحقق من دعم blockSubscribe إن كان مطلوباً.
- [ ] التحقق من دعم getMultipleAccounts.
- [ ] التحقق من حدود الاشتراكات المتزامنة.
- [ ] التحقق من عدد اتصالات WebSocket المسموح بها.
- [x] التحقق من حدود RPS الرسمية. `(حي: Helius/Shyft/Solami ردّت HTTP 429 عند التجاوز — سلوك quota مؤكد فعلياً 2026-11-17; registry يضع cooldown 60s ويدوّر)`
- [ ] التحقق من حدود burst.
- [ ] التحقق من حدود CU/RU الشهرية.
- [ ] التحقق من حدود كل RPC method بشكل منفصل.
- [ ] منع افتراض أن جميع المزودين يدعمون نفس القدرات.
- [ ] فصل Provider Identity عن Endpoint Identity.
- [ ] منع تكرار استهلاك quota لنفس المفتاح عبر endpoints مختلفة.
- [x] إضافة quota accounting مركزي. `(rpc_pool.rs:128,761 — كشف 429/Retry-After/quota_exhausted لكل endpoint + telemetry)`
- [x] إضافة Provider Health Score. `(rpc_pool.rs:91-104 — EndpointStatus حسب latency/429/أخطاء)`
- [x] إضافة latency measurements لكل مزود. `(rpc_pool.rs:104,180 — total_latency_ms + p50/p95/p99)`
- [x] إضافة Provider Circuit Breaker. `(rpc_pool endpoint rotation + ProviderSupervisor OpenCircuit — مربوطة الآن بحلقة التشغيل: record_success/record_failure + select() fail-closed، PR #56/G2)`
- [x] إضافة exponential backoff. `(helius_ws.rs:490,771 + rpc_pool Retry-After)`
- [ ] إضافة jitter للـ retries. `(معلنة في ReconnectPolicy لكن تطبيقها الفعلي في helius_ws غير مؤكد)`
- [x] منع reconnect loops. `(backoff + cooldown rotation قبل إعادة الاتصال)`
- [x] منع retry غير المحدود. `(rpc_pool.rs:1042 — retry فقط عند وجود endpoint مرشح)`
- [x] إضافة provider cooldown. `(helius_ws.rs:768 — cooldown 2.5-3.0s عند التدوير)`
- [x] اختبار failover بين المزودين. `(provider_supervisor tests + rpc_telemetry_tests.rs; حي: الـ 3 endpoints دخلوا 429-cooldown ودار التدوير ثم نجح discovery — 253 pools بعد التعافي)`
- [ ] اختبار فشل جميع المزودين.
- [x] إيقاف إصدار الفرص عند فقد جميع مصادر البيانات. `(validate_execution_readiness مع stream_connected fail-closed عند لحظة الإطلاق — dispatch.rs:535)`

## 2. Provider Capabilities وAdapters

- [x] ربط ProviderCapabilityRegistry فعلياً بالـ Supervisor. `(PR #56 — run.rs يبني registry ويسجّل Helius WS capability ويستدعي validate_execution_prerequisites قبل إقلاع worker؛ Err = bail! fail-closed، commit 804d386)`
- [x] منع استدعاء method غير مدعومة. `(helius_ws.rs:44,706,1475 — -32601 تُعامل كفشل fatal للـ endpoint مع failover)`
- [x] فصل JSON-RPC HTTP عن JSON-RPC WebSocket. `(ProviderTransport enum — capability.rs:12)`
- [x] فصل Yellowstone gRPC عن WebSocket. `(ProviderTransport::YellowstoneGrpc منفصل)`
- [x] فصل Solami Mirage عن Solami Blur. `(WebSocketMirageProtobuf ≠ WebSocketBlurJson + capability منفصلة لكل منهما)`
- [ ] عدم استخدام Solami adapter كأنه Helius JSON-RPC. `(الفصل معلن في capability لكن Mirage نفسه غير موصّل — انظر G5)`
- [x] التحقق من بروتوكول الرسائل لكل مزود. `(capability.rs — transport + encoding + commitments لكل مزود)`
- [ ] التحقق من Authentication لكل adapter.
- [ ] التحقق من Subscription Request Format.
- [ ] التحقق من Subscription Response Format.
- [ ] التحقق من Notification Format.
- [ ] التحقق من Error Mapping.
- [ ] إضافة adapter health checks.
- [ ] إضافة adapter integration tests.
- [ ] إضافة malformed message tests.
- [ ] إضافة unsupported method tests.
- [ ] توثيق القدرات الفعلية لكل adapter.
- [ ] تعطيل أي adapter غير مكتمل بدلاً من الادعاء بأنه يعمل.
- [ ] التحقق من استهلاك كل adapter داخل مسار التشغيل الحقيقي.
- [ ] إزالة adapters غير المستخدمة أو توثيق سبب الاحتفاظ بها.

## 3. WebSocket وYellowstone

- [ ] إصلاح ترتيب slotSubscribe وaccountSubscribe.
- [x] إعطاء account subscriptions الأولوية. `(ترتيب الاشتراك حسب WSOL-linkage — helius_ws.rs:1706)`
- [x] عدم استهلاك الاشتراك الوحيد في slot subscription غير ضروري. `(G8: WS_SLOT_SUBSCRIBE=0 يعطّل slotSubscribe؛ وعند provider بعدد اشتراكات = 1 يبقى الميزان كاملاً للحسابات)`
- [x] جعل slot subscription اختيارياً حسب provider capability. `(G8: helius_ws.rs — slot_subscribe_enabled gate عند connect؛ رفض slotSubscribe (id=1) يعطّل slot sub فقط دون إسقاط endpoint، وslot freshness يعتمد accountNotification context.slot؛ اختباران: slot_subscribe_rejection_disables_slot_sub_without_failing_endpoint + slot_subscribe_env_gate_parses_strict_values)`
- [x] إصلاح single-endpoint backoff. `(record_connect_backoff + اختيار endpoint الذي ينتهي cooldown أولاً — helius_ws.rs:229-239)`
- [x] إصلاح resubscription بعد reconnect. `(إعادة الاشتراك من fresh state عند كل connect — helius_ws.rs:584-614)`
- [x] استعادة جميع الحسابات المطلوبة بعد reconnect. `(pending_subs تُبنى من tracked_order عند كل اتصال)`
- [x] منع duplicate subscriptions. `(multi_sub.dedup() + اختبار مخصص — helius_ws.rs:206,1382)`
- [ ] منع duplicate notifications.
- [ ] اكتشاف missing notifications.
- [ ] اكتشاف stream gaps.
- [ ] اكتشاف انقطاع stream.
- [x] إضافة heartbeat. `(ping كل 15s — helius_ws.rs:648)`
- [x] إضافة connection timeout. `(timeout 30s على استقبال frames — helius_ws.rs:675)`
- [ ] إضافة subscription timeout.
- [ ] إضافة message processing timeout.
- [ ] إضافة bounded queues.
- [ ] إضافة backpressure.
- [ ] منع تراكم الرسائل القديمة.
- [ ] إضافة stream sequence tracking.
- [ ] إضافة reconnect metrics.
- [ ] إضافة subscription count metrics.
- [ ] اختبار اتصال مزود واحد.
- [x] اختبار اتصال مزودين. `(dual_ingest — 4 tests + dual_ingest_bench)`
- [ ] اختبار انقطاع المزود الأساسي.
- [ ] اختبار رجوع المزود الأساسي.
- [ ] اختبار رفض subscription.
- [ ] اختبار تجاوز subscription quota.
- [x] اختبار malformed WebSocket frames. `(regression test لـ Alchemy -32601 — helius_ws.rs:1475)`
- [ ] عدم الادعاء بوجود Yellowstone gRPC دون تنفيذ فعلي.

## 4. Solami Mirage / Blur

- [ ] التحقق من صيغة Mirage frames.
- [ ] تنفيذ protobuf parser حقيقي إذا كان البروتوكول يتطلبه.
- [ ] استخراج slot الحقيقي من الرسالة.
- [ ] استخراج pubkey الحقيقي.
- [ ] استخراج account owner الحقيقي.
- [ ] استخراج write version الحقيقي إن كان متاحاً.
- [ ] استخراج account data الحقيقي.
- [ ] التحقق من account update type.
- [ ] التحقق من account deletion.
- [ ] التحقق من slot metadata.
- [ ] منع استخدام latest_slot المحلي كبديل عن slot الرسالة.
- [ ] منع استخدام write_version مصطنع.
- [ ] منع استخدام provider_id ثابت لجميع المزودين.
- [ ] إضافة Mirage fixtures.
- [ ] إضافة Mirage parser tests.
- [ ] إضافة malformed protobuf tests.
- [ ] إثبات اتصال Mirage داخل Supervisor.
- [ ] إثبات وصول account updates حقيقية.
- [x] التحقق من حدود خطة Solami المجانية الفعلية. `(حي: WSS قبول ثم رفض code 4002 "no bandwidth and balance under the 0.06 minimum" — الخطة المجانية صفر bandwidth، غير قابلة للاستخدام للـ streaming)`
- [ ] تعطيل Mirage إذا لم يكتمل parsing أو لم تثبت صلاحية بياناته.

## 5. HTTP Polling وHot Path

- [ ] حصر كل استدعاءات HTTP RPC.
- [ ] حصر كل استدعاءات getMultipleAccounts.
- [ ] حصر كل استدعاءات getAccountInfo.
- [ ] حصر كل استدعاءات getProgramAccounts.
- [ ] حصر كل دورات Pool Discovery.
- [ ] حصر كل دورات Pool Refresh.
- [ ] مراجعة fetch_live_pool_states.
- [ ] مراجعة fetch_payer_token_balances.
- [ ] مراجعة Kamino reserve polling.
- [ ] فصل Bootstrap عن Runtime.
- [ ] إزالة recurring market-data polling بعد bootstrap.
- [ ] نقل pool updates إلى streams.
- [ ] نقل vault updates إلى streams.
- [ ] تحديث tick arrays من مصادر streaming المناسبة.
- [ ] تحديث DLMM bin arrays من streams.
- [ ] عدم إعادة بناء كل حالة السوق دورياً عبر HTTP.
- [ ] عدم استخدام HTTP fallback عند فشل WSS لإصدار فرص.
- [x] إبقاء Jito submission معزولاً. `(crate منفصل arb-land: bidder/leader/ledger/lifecycle/tip_oracle)`
- [x] إبقاء control-plane HTTP معزولاً. `(control_service + telegram منفصلان عن data plane)`
- [ ] إثبات صفر recurring HTTP market-data calls في hot path.

## 6. Account Dependency Planner

- [x] بناء dependency manifest لكل route. `(PoolDependencyManifest + route_requirements — dependency_manifest.rs، 9 unit tests)`
- [x] تحديد جميع Pool Accounts. `(AccountRole::PoolState)`
- [x] تحديد Vault Accounts. `(AccountRole::VaultA/VaultB)`
- [x] تحديد Mint Accounts. `(AccountRole::MintA/MintB)`
- [ ] تحديد Token Accounts.
- [x] تحديد Token Program Accounts. `(AccountRole::TokenProgram)`
- [x] تحديد Tick Arrays. `(AccountRole::TickArray)`
- [x] تحديد DLMM Bin Arrays. `(AccountRole::BinArray + حساب required_bin_arrays)`
- [x] تحديد Oracle Accounts. `(AccountRole::Oracle)`
- [x] تحديد Observation Accounts. `(AccountRole::Observation)`
- [x] تحديد Fee Configuration Accounts. `(AccountRole::FeeConfig)`
- [x] تحديد Creator Fee Accounts. `(AccountRole::CreatorVault)`
- [ ] تحديد Flash Loan Reserve Accounts.
- [ ] تحديد Lending Market Accounts.
- [ ] تحديد Repayment Accounts.
- [ ] تحديد Associated Token Accounts.
- [x] تحديد الحسابات الإضافية المطلوبة لكل DEX. `(BondingCurve/GlobalConfig/SharingConfig/MintExtension/TransferHook/BitmapExtension)`
- [x] تحديد Expected Owner لكل حساب. `(PR #56 — protocol_owner_program() يربط كل DexProtocol بـ program ID الرسمي من arb-constants، والـ gate يتحقق من owner عبر quote_dependencies_for_route → validate_dependency_group)` `(البنية موجودة — الربط بالمسار الحقيقي معلّق على G3)`
- [x] تحديد Required/Optional لكل dependency. `(AccountDependency مع role + required)`
- [x] ربط dependency manifest بمسار الاشتراكات. `(PR #56 — manifest_for_registered_pool مصدر وحيد لتحديد الحسابات المطلوبة لكل pool مسجَّل، يُبني ويُسجَّل lazily في DependencyManifestRegistry داخل Supervisor)`
- [x] ربط dependency manifest بمسار exact quote. `(PR #56 — quote_dependencies_for_route تستخرج requirements من الـ manifest ثم تطبق fail-closed quote readiness: freshness/slot-skew/owner/bank-identity)`
- [x] ربط dependency manifest بمسار dispatch. `(PR #56 — البوابة مستدعاة في process_opportunities قبل أي حساب تكلفة أو بناء: opportunity.rs:75)`
- [x] رفض route عند فقد أي حساب مطلوب. `(PR #56 — Err من البوابة = debug! + continue قبل معالجة الفرصة؛ حدود تغطية موثقة: vaults للـ DLMM/CLMM/Orca/PumpFun غير متتبعة فتُدقق حالة الـ pool فقط، وToken-2022 يُرفض fail-closed)`
- [ ] رفض route عند وجود owner غير صحيح.
- [ ] رفض route عند عدم اكتمال account hydration.
- [ ] منع إنشاء manifest فارغ كأنه صالح.
- [x] إضافة dependency completeness tests. `(9 unit tests في dependency_manifest.rs)`
- [ ] إضافة account owner regression tests.
- [ ] إضافة account-set consistency tests.
- [x] إثبات أن quote_route_guarded() مستدعاة فعلياً في Supervisor. `(PR #56 — الوظيفة المكافئة quote_dependencies_for_route مستدعاة فعلياً عبر validate_route_account_dependencies في process_opportunities: opportunity.rs:75,1424,1452)`

## 7. State Integrity وFork/Freshness

- [ ] توحيد مصدر slot لكل snapshot.
- [ ] منع global slot stamping لحسابات من batches مختلفة.
- [ ] حفظ context slot الحقيقي لكل RPC response.
- [ ] حفظ slot الحقيقي لكل streaming update.
- [ ] منع write_version المحلي من التظاهر بأنه Yellowstone write version.
- [ ] منع fabricated bank_id.
- [ ] منع provider_id ثابت لجميع المزودين.
- [x] التحقق من owner قبل قبول الحساب. `(helius_ws.rs:442 — seed يرفض owner mismatch + AccountSnapshotStore)`
- [ ] التحقق من account data length.
- [ ] التحقق من account discriminator.
- [ ] التحقق من account layout version.
- [x] منع stale snapshots. `(QuoteReadinessError::StaleSlot/DeadSlot + فحص freshness في store)`
- [ ] منع duplicate snapshots.
- [ ] منع out-of-order updates.
- [ ] اكتشاف SLOT_DEAD.
- [ ] إبطال processed fork غير المعتمد.
- [ ] إبطال state المرتبط بفork غير صالح.
- [ ] منع دمج snapshots من مصادر غير متزامنة دون سياسة consistency واضحة.
- [x] إضافة State Generation. `(stream_generation متتبعة في ingest + expected_stream_generation في dispatch.rs:535)`
- [x] إضافة State Integrity Tests. `(8 tests في slot_integrity.rs + 8 في quote_readiness.rs)`
- [x] إضافة Fork Invalidation Tests. `(DeadSlot/FutureSlot variants + slot_integrity tests)`
- [x] إضافة Stale Snapshot Tests. `(StaleSlot variant + tests)`
- [x] إضافة Duplicate Update Tests. `(DuplicateDependency + dedup في store)`
- [x] إضافة Out-of-Order Tests. `(slot ordering checks في slot_integrity tests)`
- [ ] جعل state غير الصالح سبباً مباشراً لرفض التنفيذ.

## 8. DEX Exact Quote Parity

- [ ] مراجعة Raydium AMM V4.
- [x] مراجعة Raydium CPMM. `(11 unit tests في arb-math/src/cpmm.rs)`
- [x] مراجعة Raydium CLMM. `(13 unit tests في arb-math/src/clmm.rs + clmm_probe example)`
- [ ] مراجعة Raydium LaunchLab.
- [x] مراجعة Orca Whirlpool. `(orca_sdk_parity.rs — مقارنة rounding مع orca_whirlpools_core الرسمي)`
- [x] مراجعة Meteora DLMM. `(18 unit tests في arb-math/src/dlmm.rs + dlmm_bin_array_boundary_tests.rs)`
- [ ] مراجعة Meteora DAMM v2.
- [x] مراجعة Pump.fun. `(3 tests في arb-dex/src/pump_fun.rs + fuzz bounds في fuzzing_suite)`
- [x] مراجعة PumpSwap. `(8 unit tests في arb-dex/src/pump_swap.rs)`
- [ ] مراجعة Jupiter adapter.
- [ ] التأكد من أن كل quote يستخدم account state الصحيح.
- [ ] التحقق من Token Decimals.
- [ ] التحقق من Pool Reserves.
- [ ] التحقق من LP Fees.
- [ ] التحقق من Protocol Fees.
- [ ] التحقق من Creator Fees.
- [ ] التحقق من Transfer Fees.
- [ ] التحقق من Token-2022 Extensions.
- [ ] التحقق من Rounding Direction.
- [x] التحقق من Integer Overflow. `(adversarial_tests — overflow في marginal functions + 441-bit headroom + U768 closed form)`
- [x] التحقق من Fixed-Point Arithmetic. `(U384/U512/U768 wide integers + isqrt fuzz correctness)`
- [x] التحقق من CLMM Tick Traversal. `(fuzz_clmm_tick_to_sqrt_price valid/out-of-bounds + adversarial CLMM boundaries)`
- [x] التحقق من DLMM Bin Traversal. `(dlmm_bin_array_boundary_tests + fuzz_dlmm_get_price_from_id)`
- [ ] التحقق من Bitmap Extension.
- [ ] التحقق من Liquidity Boundaries.
- [ ] التحقق من Price Impact.
- [ ] التحقق من Slippage.
- [x] تنفيذ SDK Differential Tests لكل DEX. `(differential.rs + python_oracle_differential.rs في arb-math/tests — تغطية كل DEX تُراجع مع G11)`
- [ ] تنفيذ Fork Acceptance Tests لكل DEX.
- [ ] توثيق الفرق بين local quote وreference quote.
- [ ] عدم اعتبار builder ناجحاً دليلاً على quote parity.
- [ ] منع قبول DEX غير مختبر باعتباره Production Ready.

## 9. Dynamic Pool Discovery

- [ ] اكتشاف pools من البرنامج والحسابات.
- [ ] دعم اكتشاف pools الجديدة من stream.
- [ ] اكتشاف تغييرات pool configuration.
- [ ] اكتشاف pool closure.
- [ ] التحقق من pool owner.
- [ ] التحقق من pool discriminator.
- [ ] التحقق من protocol type.
- [ ] اكتشاف Token Pair.
- [ ] اكتشاف Token Mints.
- [ ] اكتشاف Vault Relationships.
- [ ] اكتشاف Tick Arrays المطلوبة.
- [ ] اكتشاف DLMM Bin Arrays المطلوبة.
- [ ] اكتشاف الحسابات الإضافية.
- [ ] منع تكرار pool registration.
- [ ] منع route duplication.
- [x] بناء route graph ديناميكي. `(bellman_ford.rs + discovery — 9 unit tests)`
- [x] بناء دورات من ثلاثة pools. `(find_base_cycles + reports/final_audit/THREE_POOL_EXECUTION_AUDIT.md)`
- [ ] دعم أكثر من أصل وسيط.
- [ ] تحديث graph عند تغير liquidity.
- [x] فصل Candidate Discovery عن Exact Quote. `(discovery منفصل + exact quote validation protocol-adapter driven — strategy.rs:20)`
- [ ] عدم فرض spread ثابت أثناء discovery.
- [ ] عدم اعتبار وجود pool فرصة تداول.
- [ ] رفض pool غير المكتمل أو غير الموثوق.

## 10. Flash Loan وKamino

- [x] مراجعة Kamino instruction semantics. `(15 unit tests في arb-tx/src/flash/kamino.rs + reports/final_audit/KAMINO_FLASH_LOAN_AUDIT.md)`
- [ ] مراجعة Borrow Instruction.
- [ ] مراجعة Repay Instruction.
- [ ] التحقق من Flash Loan Fee.
- [ ] التحقق من Reserve Configuration.
- [ ] التحقق من Available Liquidity.
- [ ] التحقق من Reserve Utilization.
- [ ] التحقق من Borrow Limits.
- [ ] التحقق من Minimum Borrow.
- [ ] التحقق من Maximum Borrow.
- [ ] التحقق من Token Decimals.
- [ ] اكتشاف جميع أصول الاقتراض المتاحة.
- [ ] عدم حصر الاختيار في WSOL دون مبرر.
- [ ] تقييم كل reserve مؤهل.
- [ ] اختيار أفضل أصل اقتراض اقتصادياً.
- [ ] تحسين Borrow Amount.
- [ ] ربط حجم الاقتراض بعمق المسار.
- [ ] ربط الحجم بحدود bins وticks.
- [ ] ربط الحجم بالـ slippage.
- [ ] ربط الحجم برسوم الاقتراض.
- [x] رفض reserve state القديم. `(ROI implausibility ceiling يرفض stale-quote artifacts fail-closed — opportunity.rs:400-420)`
- [ ] منع استخدام liquidity غير متاحة فعلياً.
- [ ] إضافة reserve-specific tests.
- [ ] إضافة borrow sizing tests.
- [x] إضافة repayment tests. `(ضمن kamino.rs tests + economic.rs loss-path checks)`

## 11. Net Profit وEconomic Model

- [x] إنشاء مصدر واحد لحساب الربح. `(arb-math/src/economic.rs — المصدر الكانوني الوحيد، re-exported بواسطة arb-tx)`
- [ ] حساب Gross Output.
- [ ] خصم Swap Fees.
- [ ] خصم Creator Fees.
- [ ] خصم Transfer Fees.
- [ ] خصم Flash Loan Fees.
- [ ] خصم Base Transaction Fee.
- [ ] خصم Priority Fee.
- [ ] خصم Jito Tip.
- [ ] احتساب Slippage Reserve.
- [ ] احتساب Execution Risk.
- [ ] احتساب تكلفة الفشل عند الحاجة.
- [ ] منع double-counting.
- [x] منع integer overflow. `(سلاسل checked_add/checked_mul مع ok_or(ArbError::Overflow) — economic.rs:336-356)`
- [x] استخدام checked arithmetic. `(نفس الدليل — لا arithmetic غير checked في المسار الاقتصادي)`
- [x] استخدام ceiling division لرسوم priority fee. `(adaptive.rs:196-204 — ceil(micro_lamports * cu_limit / 1_000_000))`
- [ ] توحيد priority fee بين sizing وranking وdispatch.
- [ ] منع saturating arithmetic في الحسابات الاقتصادية الحساسة.
- [x] استخدام SOL/USD price موثوق ومؤرخ. `(chronos max_age_ms — opportunity.rs:550-557)`
- [x] رفض السعر القديم. `(No fresh SOL/USD reference; rejecting candidate economic conversion — fail-closed)`
- [x] منع استخدام سعر افتراضي مثل $200 كدليل اقتصادي. `(G7: telegram.rs — sol_price_live provenance flag؛ التنبيهات تعرض وسم ديناميكي «سعر غير موثّق (افتراضي)» بدل ادّعاء «سعر حي من السوق» الثابت؛ العلم يُرفع فقط من Binance/Coinbase feed أو سعر pool حي أو SOL_USD_PRICE من المشغّل؛ اختبار unverified_sol_price_is_labeled_as_such)`
- [ ] إصلاح اختلاف حدود min profit بين المسارات.
- [x] مراجعة MIN_NET_PROFIT_USD. `(config.rs:385-399 — configurable، >=0 validation، f64 محصور عند config boundary فقط)`
- [x] مراجعة SOL_USD_PRICE. `(مُراجع — الافتراضي $200 ما زال في مسار التنبيهات فقط: G7)`
- [x] مراجعة min_net_profit_abs. `(يمرر إلى to_economic_config — strategy.rs:51)`
- [ ] إزالة بوابات gross spread الثابتة. `(CONFIRMED: min_spread_bps ما زال موجوداً — config.rs:285 + strategy.rs:217)`
- [ ] فصل gross spread عن net profit.
- [x] ترتيب الفرص حسب صافي الربح القابل للتنفيذ. `(sort_by_key Reverse(net_profit) — strategy.rs:316,394)`
- [ ] رفض الفرصة إذا كان أي عنصر تكلفة مجهولاً.
- [ ] إضافة economic regression tests.

## 12. Atomic Transaction Builder

- [ ] التحقق من ترتيب التعليمات.
- [x] التحقق من Compute Budget Instructions. `(budget في compiler — message_version_prefix + CU limit/price صريحون في الرسالة — compiler.rs:758)`
- [ ] التحقق من Flash Borrow.
- [ ] التحقق من Swap 1.
- [ ] التحقق من Swap 2.
- [ ] التحقق من Swap 3.
- [ ] التحقق من Repay.
- [ ] التحقق من Token Account Creation عند الحاجة.
- [ ] التحقق من ATA dependencies.
- [ ] التحقق من Writable Accounts.
- [ ] التحقق من Signer Requirements.
- [ ] التحقق من Account Locks.
- [x] التحقق من Transaction Serialization. `(V1 compiler مع wire-size check صريح — flash/compiler.rs:49)`
- [x] التحقق من Versioned Transactions. `(V1: 0x80/0x81 prefix، ≤4096 bytes، كل الحسابات inline — compiler.rs:5,37,758)`
- [x] التحقق من Address Lookup Tables. `(arb-tx/src/transport/master_alt.rs — 1,417 سطر؛ V1: 64 حساب inline + سقف 4096 bytes)`
- [ ] التحقق من حجم المعاملة 1232 bytes.
- [x] منع تقسيم atomic route إلى معاملات منفصلة. `(verify_atomic_integrity مستدعاة قبل التوقيع — dlmm_flash_arb.rs:36,496)`
- [ ] التحقق من Kamino repayment semantics.
- [x] التحقق من Min-Out. `(FinalOutputBelowRepayment guard: min_out < repay يرفض — dlmm_flash_arb.rs:70-71,325)`
- [ ] التحقق من Min-Profit.
- [x] التحقق من Repayment Guard. `(BorrowTooSmall guard على RAW bits قبل التقريب — موثق في AGENTS.md invariants)`
- [ ] رفض المعاملة عند فشل أي شرط.
- [ ] اختبار builder باستخدام real account fixtures.
- [ ] اختبار instruction ordering.
- [ ] اختبار transaction size boundaries.

## 13. Jito وLanding

- [x] التحقق من Jito endpoints. `(regional Block Engine endpoints: Amsterdam/Frankfurt/NY + fanout — arb-land/src/lib.rs:29,219-261)`
- [ ] التحقق من Bundle Serialization.
- [ ] التحقق من Tip Accounts.
- [x] التحقق من Tip Amount. `(bidder.rs — protocol min tip + hurdle/ceiling + retry cap)`
- [ ] التحقق من Priority Fee.
- [x] التحقق من Bundle Submission. `(send_bundle + send_bundle_multi_region — arb-land/src/lib.rs:264,323)`
- [x] حفظ Bundle ID. `(bundle_id: String — lib.rs:82 + استرجاعه من الرد lib.rs:395)`
- [x] متابعة Bundle Status. `(get_bundle_status — lib.rs:366-395 + signature-status polling في dispatch.rs)`
- [ ] التحقق من Landing.
- [ ] التحقق من Slot.
- [x] التحقق من Signature Status. `(SignatureStatus::Pending/Processed/Confirmed/Finalized/Failed polling في dispatch.rs:751-768)`
- [x] الفصل بين Submitted وAccepted. `(lifecycle.rs:25-31 — حالات منفصلة مع انتقالات مقيدة)`
- [x] الفصل بين Processed وConfirmed. `(نفس الدليل — lifecycle.rs:60-65)`
- [x] الفصل بين Confirmed وFinalized. `(نفس الدليل — lifecycle.rs:65-70)`
- [ ] عدم اعتبار HTTP 200 دليلاً على Landing.
- [ ] إضافة Retry Policy.
- [ ] إضافة Bundle Expiration Handling.
- [ ] إضافة Rejection Handling.
- [x] تسجيل Actual Execution Result. `(record_outcome في arb-store — SQLite outcomes table)`
- [ ] تسجيل Actual Balance Changes.
- [x] تسجيل Realized Profit. `(realized_net: Lamports مسجل ومُجمَّع — arb-store/src/lib.rs:37,187-199,311-325)`
- [ ] اختبار Jito Failure.
- [ ] اختبار Jito Timeout.
- [ ] اختبار Bundle Rejection.
- [ ] اختبار Landing Confirmation.

## 14. Compute Budget وTransaction Constraints

- [x] بناء CU estimator ديناميكي. `(estimate_route_cu حسب شكل المسار الحقيقي + measured_cu لكل route_sig + calculate_dynamic_cu_limit — opportunity.rs:88-105)`
- [x] التحقق من CU limit. `(calculate_dynamic_cu_limit + measured CU per route)`
- [x] التحقق من CU price. `(cap_cu_price_by_profit — سقف ديناميكي حسب share bps وعامل المنافسة)`
- [ ] قياس CU الفعلي لكل DEX.
- [ ] عدم الاعتماد على ثوابت غير مختبرة.
- [x] حساب priority fee بشكل موحد. `(total_priority_fee_lamports — adaptive.rs:202)`
- [ ] مراقبة transaction packet size.
- [ ] مراقبة instruction count.
- [ ] مراقبة account count.
- [ ] مراجعة ALT coverage.
- [ ] التحقق من Heap Requirements.
- [ ] التحقق من Runtime Stack Requirements.
- [ ] التحقق من Account Data Limits.
- [ ] التحقق من Compute Budget Overflow.
- [ ] إضافة CU regression benchmarks.

## 15. Testing وFuzzing

- [x] مراجعة جميع Unit Tests. `(cargo test --workspace --all-targets: 1080 passed / 0 failed عبر 86 binary)`
- [x] مراجعة Integration Tests. `(نفس التشغيل يشمل integration tests لكل crate)`
- [x] إضافة Property-Based Tests. `(proptest — 500 cases: CPMM monotonicity/invariants، isqrt، CLMM bounds، DLMM prices، Pump.fun)`
- [x] إضافة Fuzz Tests. `(fuzzing_suite.rs — never-panics + invariants؛ cargo-fuzz منفصل غير موجود — G9)`
- [ ] إضافة Differential Tests.
- [x] إضافة Regression Tests. `(منها Alchemy -32601 regression وphase15 profit-accounting regressions — كلها ضمن التشغيل الناجح)`
- [ ] إضافة Failure Injection Tests.
- [ ] إضافة Concurrency Tests.
- [x] إضافة Replay Tests. `(shadow_replay_tests.rs + arb-bot/src/bin/replay.rs)`
- [x] اختبار Integer Overflow. `(adversarial_tests.rs — extreme imbalance 1 lamport vs 10^18، deep pools 10^24)`
- [ ] اختبار Invalid Decimals.
- [ ] اختبار Malformed Account Data.
- [x] اختبار Zero Liquidity. `(test_cpmm_zero_reserves_fails + test_cpmm_zero_input_fails + test_quote_dlmm_zero_input_fails — differential.rs:185,346,353)`
- [x] اختبار Missing Bins. `(test_bin_id_bounds + test_bin_array_index_euclidean_division + dlmm_bin_array_boundary_tests)`
- [x] اختبار Missing Ticks. `(test_adv_clmm_boundaries_and_ordering + tick bounds fuzz)`
- [x] اختبار Negative Profit. `(test_negative_invalid_cross_currency_addition_rejected + loss-path في economic.rs:349-356)`
- [x] اختبار Fee Rounding. `(fuzz_cpmm_out_fee_arbitrary_denominators + test_sqrt_price_round_trip_stability)`
- [ ] اختبار Duplicate Updates.
- [ ] اختبار Out-of-Order Updates.
- [ ] اختبار Fork Invalidation.
- [ ] اختبار Provider Disconnect.
- [ ] اختبار Quota Exhaustion.
- [ ] اختبار Transaction Overflow.
- [ ] اختبار Account Lock Conflicts.
- [ ] اختبار Compute Budget Exhaustion.
- [ ] اختبار Failed Repayment.
- [ ] اختبار Failed Min-Out.
- [ ] اختبار Failed Min-Profit.
- [ ] اختبار Jito Rejection.
- [ ] اختبار Full Route.

## 16. Performance وHFT

- [x] قياس Provider Latency. `(rpc_pool p50/p95/p99 + RttTracker بنصف عمر إحصائي — arb-math/src/latency.rs:38-134)`
- [ ] قياس Account Update Latency.
- [ ] قياس State Processing Latency.
- [ ] قياس Route Discovery Latency.
- [ ] قياس Exact Quote Latency.
- [ ] قياس Borrow Sizing Latency.
- [ ] قياس Transaction Build Latency.
- [ ] قياس Simulation Latency.
- [ ] قياس Submission Latency.
- [ ] قياس Landing Latency.
- [ ] مراجعة unnecessary cloning.
- [ ] مراجعة heap allocations.
- [ ] مراجعة lock contention.
- [ ] مراجعة async task scheduling.
- [ ] مراجعة serialization overhead.
- [ ] مراجعة duplicate calculations.
- [ ] مراجعة CPU consumption.
- [ ] مراجعة Memory Consumption.
- [ ] مراجعة queue saturation.
- [x] بناء reproducible benchmarks. `(arb-ingest/benches/dual_ingest_bench.rs + arb-bot/src/bin/latency_bench.rs)`

## 17. Security

- [ ] مراجعة Private Key Handling.
- [ ] مراجعة Wallet Signer.
- [x] منع طباعة الأسرار. `(redact_url في helius_ws + قاعدة AGENTS.md رقم 4)`
- [x] منع تخزين الأسرار في Git. `(.gitignore يغطي .env وkeypairs وsecrets/)`
- [x] مراجعة .gitignore. `(مُراجع — قسم Security & Secrets كامل)`
- [ ] مراجعة .env.example.
- [ ] مراجعة GitHub Actions Secrets Usage.
- [ ] مراجعة Dependency Vulnerabilities.
- [ ] مراجعة Cargo Audit.
- [x] مراجعة cargo-deny. `(cargo-deny في CI — rust-ci.yml:53-68 للـ workspace وwebhook-service)`
- [x] مراجعة Gitleaks. `(PR #53: سياسة .gitleaks.toml مُتحقَّق بغرس 5 أسرار حقيقية 5/5 و76→0 false positives؛ + هذا PR: خطوة gitleaks في rust-ci.yml job «secrets» بـ gitleaks-action@v2 مع fetch-depth: 0 وGITLEAKS_CONFIG=.gitleaks.toml — فحص تاريخ كامل)`
- [ ] مراجعة Docker Security.
- [x] مراجعة RPC Endpoint Validation. `(فرض http/https scheme على primary/failover/base URLs — arb-config/src/validation.rs:50-68,169)`
- [ ] منع untrusted account data من تجاوز validation.
- [ ] منع malformed instructions.
- [ ] منع arbitrary program invocation.
- [ ] مراجعة Program IDs.
- [ ] مراجعة Token Program IDs.
- [ ] مراجعة Upgradeable Program Risks.
- [x] مراجعة Kill Switch. `(arb-risk/src/lib.rs + kill switch gate في dispatch.rs:520 قبل الإرسال)`

## 18. Dashboard وObservability

- [ ] مراجعة React/Vite Build.
- [x] مراجعة API Connections. `(web_api.rs — /api/status /api/config /api/pnl /api/executions /api/logs /api/routes /api/opportunities)`
- [x] مراجعة WebSocket Dashboard. `(/api/ws موجود)`
- [x] مراجعة Provider Health Display. `(/api/providers موجود)`
- [ ] عرض Quota Utilization.
- [ ] عرض Subscription Counts.
- [ ] عرض Stream Gaps.
- [ ] عرض Stale Accounts.
- [ ] عرض Fork Invalidations.
- [x] عرض Candidate Routes. `(RouteMonitor.tsx + /api/routes)`
- [ ] عرض Exact Quotes.
- [x] عرض Rejection Reasons. `(FIXED/TESTED: ring مخصص RejectionRecord في control_service + /api/rejections + مكوّن Rejections.tsx جديد في Dashboard يعرض stage/reason/gross/net/loan/pools؛ record_rejected_opportunity يسجّل الآن في control؛ اختبار rejection_ring_is_dedicated_and_ordered)`
- [ ] عرض Gross Profit.
- [x] عرض Net Profit. `(Opportunities.tsx — net_profit معروض)`
- [x] عرض Borrow Asset. `(Opportunities.tsx — borrow معروض)`
- [x] عرض Borrow Amount. `(نفس المكوّن)`
- [ ] عرض Simulation Results.
- [ ] عرض Transaction Size.
- [ ] عرض CU Consumption.
- [x] عرض Jito Status. `(FIXED/TESTED: ExecutionRecord توسّع بحقول signature/slot/net_profit_sol اختيارية skip_serializing_if — landed يعرض tx signature وslot وprofit، dropped يعرض bundle_id؛ أعمدة جديدة في Executions.tsx؛ اختبار execution_extras_serialize_only_when_present)`
- [ ] إضافة Structured Logs.
- [ ] إضافة Correlation IDs.
- [ ] إضافة Error Categories.
- [ ] إضافة Daily Reports.
- [ ] الاحتفاظ بالتقارير 30 يوماً.
- [ ] منع ظهور الأسرار في الواجهة.
- [ ] التحقق من تطابق Dashboard مع Runtime State.

## 19. Configuration وCode Quality وCI/CD

- [ ] إزالة الإعدادات غير المستخدمة.
- [ ] إزالة Environment Variables غير المستهلكة.
- [x] توحيد Configuration Source of Truth. `(arb-config: base.toml ← env TOML ← env vars ← CLI مع config.schema.json + validation)`
- [ ] منع اختلاف defaults بين TOML وENV.
- [x] توثيق كل Config Field. `(config/config.schema.json + doc comments على الحقول)`
- [ ] توثيق Units لكل رقم.
- [x] التحقق من Valid Ranges. `(arb-config/src/validation.rs:19 — validate() مع قيود زمنية وأمنية)`
- [ ] مراجعة ملفات Rust الأكبر من 1000 سطر.
- [ ] تقسيم Supervisor إلى Modules واضحة.
- [ ] إزالة Dead Code.
- [ ] إزالة Duplicate RPC Calls.
- [ ] إزالة Duplicate Fee Logic.
- [ ] إزالة Magic Constants.
- [x] إصلاح Clippy Warnings. `(CI يفرض -D warnings — rust-ci.yml:50؛ build نظيف)`
- [x] إصلاح Formatting. `(CI يفرض cargo fmt --check — rust-ci.yml:41)`
- [x] إصلاح Docker Build. `(PR #53 — feature shadow-only=[] + gate compile-time في main.rs يجبر mode=simulate ويتجاهل ARB_MODE/ARB_LIVE؛ cargo check -p arb-bot --features shadow-only نظيف)`
- [x] إصلاح Cargo Deny. `(PR #53 — 0BSD مضافة للـ licenses + 4 RUSTSEC unmaintained موثقة كـ ignores (ansi_term/derivative/paste/libsecp256k1 — transitive لـ Solana/litesvm بلا ترقية آمنة)؛ vulnerability advisories ما زالت مفروضة، CI أخضر)`
- [x] إصلاح Gitleaks false positives دون تعطيل فحص الأسرار. `(PR #53 — .gitleaks.toml بـ extend default + allowlist ضيّق 5 مسارات و2 regex literal؛ إثبات: نتائج CI 76→0، وغرس 5 أسرار حقيقية ghp_ في .md/.txt/fixtures/src/reports اكتُشفت 5/5)`
- [ ] إصلاح Regression Tests ذات false positives.
- [ ] توحيد CI على نفس commit.
- [ ] اختبار كل PR قبل الدمج.
- [ ] منع Merge تلقائي.
- [ ] توثيق Commit لكل اختبار ناجح.

## 20. Production Acceptance

- [x] اختبار مزودين مجانيين فعليين. `(حي: Helius RPC getSlot=470350734/health=ok; Shyft=470350735; Solami=470350735 — نفس الكلاستر; WSS: Helius/Shyft 429 على handshake، Solami 4002 no-bandwidth)`
- [ ] اختبار Data Plane لمدة 30 دقيقة.
- [ ] إثبات account updates من كلا المزودين.
- [ ] إثبات Failover.
- [ ] إثبات Quota Compliance.
- [ ] إثبات عدم وجود HTTP market polling.
- [ ] إثبات State Integrity.
- [ ] إثبات Dependency Completeness.
- [ ] إثبات Exact Quote Parity لكل DEX معتمد.
- [ ] إثبات أفضل Borrow Asset.
- [ ] إثبات أفضل Borrow Amount.
- [ ] إثبات Net Profit Calculations.
- [x] إثبات Full Atomic Builder. `(حي 2026-11-17: معاملة ذرية 8 تعليمات بُنيت بمُجمِّعات المستودع (kamino borrow/repay + ATA idempotent + CU budget + syncNative) — verify_atomic_integrity=is_atomic، 650 bytes، وصلت finalized على mainnet sig 5w9k6SeA…gX)`
- [x] إثبات Repayment. `(حي: FlashBorrowReserveLiquidity ثم FlashRepayReserveLiquidity نجحا في نفس tx — fee=100 lamports لكل 0.01 SOL (sf=11529215046068 U68F60)، الرصيد عاد سليماً)`
- [ ] إثبات Min-Out.
- [ ] إثبات Min-Profit.
- [x] إثبات Transaction Size. `(حي: V1=642 bytes / v0=667 bytes < 1232 MTU)`
- [x] إثبات Full-Route Simulation. `(حي: simulateTransaction قبل الإرسال err=null وunits_consumed=65461، ثم البث الفعلي وlanded finalized @ slot 470365241)`
- [ ] إثبات Jito Submission.
- [ ] إثبات Landing Telemetry.
- [ ] تنفيذ 24-hour Shadow Soak.
- [ ] حفظ Shadow Soak Metrics.
- [ ] توثيق جميع Blockers المتبقية.
- [ ] مراجعة كل Acceptance Gate.
- [ ] إصدار Production Readiness Report.

---

## رأي هندسي حاكم

أهم شيء في القائمة كلها هو **منع مرور بيانات غير موثوقة إلى محرك التسعير
والتنفيذ**. ممكن يكون عندك أسرع Quote Engine في العالم، لكن لو الـ slot أو
الحسابات أو الـ reserve state غير متزامنة، فأنت بتحسب فرصة على حالة غير موجودة
فعلياً.

## آلية التحويل إلى تنفيذ فعلي

- كل بند في هذه القائمة يتحول إلى Task Ledger entry بحالة ودليل.
- لا يبدأ أي Agent بتعديل عشوائي: يبدأ من ترتيب التنفيذ الإلزامي أعلاه.
- قبل أي تعديل: إعادة التحقق من كل نقطة على أحدث `main`.
- التعليم على `[x]` يتطلب دليلاً: كود، اختبار منفَّذ، أو تقرير محفوظ.
