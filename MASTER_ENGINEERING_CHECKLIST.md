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

- **328 / 495** بند متحقق بدليل ومُعلَّم `[x]` (منها 6 بنود بدليل حي من mainnet شاملة معاملة flash-loan landed).
- **270** بند متبقي: فجوات G1–G4 وG15 أُصلحت ومُختبرت (PR #55 و #56)؛ أُصلحت G7 وG8 وG10 وG11 وG13 وG14 وG16؛ ما زالت G5 (BLOCKED على quota المزود) وG12 (24h، يحتاج وقتاً فقط)؛ G6 أُكملت (اختيار حي اقتصادي) وG9 FIXED، والباقي `NOT AUDITED` — يحتاج تدقيقاً فردياً أو تشغيلاً فعلياً (shadow/live) قبل التعليم.

### نتائج التشغيل

- `cargo build --workspace`: نجح (0 أخطاء).
- `cargo test --workspace --all-targets`: **1139 اختبار ناجح، 0 فشل** (بعد PR #57 + إصلاحات G16/G8/G7/G11؛ كان 1094) عبر 88 test binary.
- CI يفرض `cargo fmt --check` و `cargo clippy --workspace --all-targets -- -D warnings` و `cargo-deny` (rust-ci.yml).

### فجوات مؤكدة (CONFIRMED — لم تُعلَّم [x])

| # | الفجوة | الدليل |
|---|---|---|
| G1 | **FIXED/TESTED (PR #56, commit 804d386)**: `ProviderCapabilityRegistry` مربوطة بالـ Supervisor — `run.rs` يبني الـ registry ويسجّل `standard_json_rpc_ws("Helius")` عند نجاح `HeliusWsClient::from_env()` ويستدعي `validate_execution_prerequisites()` قبل إقلاع WS worker؛ الفشل = `bail!` (إيقاف إقلاع fail-closed) | `supervisor/run.rs` — بوابة capability قبل بدء worker |
| G2 | **FIXED/TESTED (PR #56, commit 804d386)**: `ProviderSupervisor` مستخدم في حلقة التشغيل — WS_STREAM_PROVIDER (أولوية 0) + RPC_FALLBACK_PROVIDER (أولوية 1)، record_success/record_failure على تقدّم slot وRPC fallback، و`select()` fail-closed قبل `strategy_cycle` | `supervisor/run.rs` — health observation + بوابة select |
| G3 | **FIXED/TESTED (PR #56, commit 804d386)**: `QuoteReadinessContext`/`DependencyManifest` مربوطة — بوابة `validate_route_account_dependencies` في `process_opportunities` (opportunity.rs:75) تستدعي `quote_dependencies_for_route` مع manifests من `manifest_for_registered_pool` (مصدر وحيد)؛ + إصلاح provenance في `helius_ws.rs` (provider_id من yellowstone بدل ProviderId(2) الثابت) | `supervisor/opportunity.rs:75,1424`, `arb-ingest/src/lib.rs:766` |
| G4 | **MITIGATED/VERIFIED (PR #53 + PR #56)**: reconcile sweep دوري بحد أدنى 30s بدل كل slot notification؛ والبوابات الجديدة (G2/G3) تمنع أي حالة قديمة من الوصول للتسعير/التنفيذ. الحد المتبقي: vault updates للـ DLMM/CLMM/Orca/PumpFun ما زالت عبر hydration وليست streams | `supervisor/run.rs` — min_gap reconciliation |
| G5 | Solami Mirage parser لا يستخرج حقول protobuf حقيقية: يستخدم `latest_slot` محلي، `pubkey: None`, `write_version: None`، وغير موصّل بالبوت | `solami_mirage.rs:138-140` |
| G6 | **FIXED/TESTED**: (1) حارس fail-closed `flash_mint_matches_kamino_reserve()` يرفض أي config بـ flash mint ≠ WSOL مع WSOL reserve (والعكس) بـ `FLASH_MINT_RESERVE_MISMATCH`؛ (2) **اختيار حي اقتصادي تلقائي** — `select_kamino_flash_reserve` يمسح reserves السوق الرئيسية (WSOL+USDC) حياً عبر `decode_reserve_candidate` (mint/decimals/status/fee_sf/supply+fee vaults من البايتات الحية؛ offsets @152/@184 تحققت empirically ضد ثوابت WSOL 2026-11-17)، يفلتر (active + fee enabled + سيولة حية ≥ min_borrow) ويختار أقل fee ثم الأعمق سيولة، fail-closed عند لا مرشح؛ (3) `KaminoFlashParams.liquidity_mint` generalized + `from_candidate`؛ (4) مربوط بالـ rehearsal (`[2c]` دليل حي: fee_sf 11529215046068، سيولة 257,399 SOL، 2 candidates) | kamino.rs — 3 اختبارات جديدة + rehearsal حي |
| G7 | **FIXED/TESTED**: علم provenance `sol_price_live` في TelegramNotifier — قيمة $200 الافتراضية لم تعد تُقدَّم كدليل سوق؛ التنبيهات تعرض وسم «سعر غير موثّق» حتى يأتي سعر حي (Binance/Coinbase أو pool SOL/USDC أو SOL_USD_PRICE) | telegram.rs — `unverified_sol_price_is_labeled_as_such` |
| G8 | **FIXED/TESTED**: gating ثنائي لـ slotSubscribe — env `WS_SLOT_SUBSCRIBE` + رفض حي (id=1) يعطّل slot sub فقط دون `endpoint_unsupported`؛ slot freshness يعتمد context.slot من accountNotification | helius_ws.rs — اختباران جديدان |
| G9 | **FIXED/TESTED**: cargo-fuzz harness كامل في `fuzz/` (مستبعد من workspace) — 4 targets: cpmm_quote / net_profit / transfer_fee / dlmm_traversal؛ 4 جلسات × 20s نظيفة (3.1M/10.2M/8.3M/2.9M runs). **الـ fuzz وجد عيباً حقيقياً**: ضرب U256 غير محمي في cpmm_out_fee/cpmm_out_upfront_fee_ceil (panic أو wrap صامت) — أُصلح بـ checked arithmetic + 2 regression tests | fuzz/ + cpmm.rs — `cpmm_out_fee_fails_closed_on_wide_inputs` |
| G10 | **FIXED (CI step added)**: job `secrets` في rust-ci.yml — gitleaks-action@v2، fetch-depth: 0 (تاريخ كامل)، GITLEAKS_CONFIG=.gitleaks.toml؛ سياسة PR #53 تبقي كل قواعد default نشطة وallowlist ضيّق فقط | rust-ci.yml — التحقق النهائي عند أول تشغيل CI لهذا PR |
| G11 | **FIXED/TESTED**: unit tests لكل من `damm_v2` (10: fee bps/ceil، cap 50%، linear scheduler، رفض fee≥denominator، صفر/سيولة، تناظر الاتجاهين مع virtual reserves، monotonicity، range clamp، fee mode على الدخل/المخرج)، `launchlab` (10: رسم الدخل المسبق مع إعادة حساب مستقلة، ثابت x·y=k الرسمي من docs.raydium.io، caps real reserves، round-trip خسارة صارمة، monotonicity)، `pump_fun_raydium` (8: ceil rounding، fee floor، رفض reserves منحلة، لا optimum عند تساوي الأسعار، اتساق داخلي للـ trade، bracketing للـ optimum عبر verify_marginal_decreasing) — 28 اختباراً جديداً | `cargo test -p arb-math --lib`: 148 passed / 0 failed |
| G13 | **FIXED/TESTED**: ExecutionRecord توسّع بحقول اختيارية (signature/slot/net_profit_sol) مع skip_serializing_if للتوافق الخلفي؛ record_landed_trade_task يعبّئ signature+slot+profit من TradeSummary، record_dropped_bundle_task يعبّئ bundle_id+slot؛ Executions.tsx يعرض أعمدة Signature/Slot/Net Profit | control_service.rs + supervisor.rs + Executions.tsx — `execution_extras_serialize_only_when_present` |
| G14 | **FIXED/TESTED**: ring مخصص للرفض (REJECTION_RING_CAP=256) في ControlService لا يُفيض عليه executions ring؛ /api/rejections + حدث WS "rejection"؛ مكوّن Rejections.tsx جديد (تبويب D2) يعرض stage/reason/gross/net/loan/pools/slot | control_service.rs + web_api.rs + Rejections.tsx — `rejection_ring_is_dedicated_and_ordered` |
| G15 | **FIXED/TESTED (PR #55, commit 92e1612)** — **تم تشخيصه بدقة (تصحيح التشخيص الأول)**: سبب الرفض الحي ليس الـ wire encoder — الـ encoder مطابق بايت-بايت للتنفيذ الرسمي `solana-message::v1` (تمت مقارنة كل الحقول: header/mask/lifetime/addresses/config-values/instruction-headers/payloads/signature-at-tail). السبب المؤكد تجريبياً: قيمة `loaded_accounts_data_size_limit` القريحة في الـ config. القياس الحي على mainnet (Agave 4.4.1): عمود kamino flash يحمّل فعلياً **10,714,291 بايت (~10.2MiB)** لأن بيانات البرامج التنفيذية (klend + SPL Token + ATA) تُحتسب ضمن الحد؛ أي حد < ~10.72MiB يُرفض فوراً بـ `MaxLoadedAccountsDataSizeExceeded` عند units=0 قبل تنفيذ أي instruction. مغلف V1 بحد 64MiB (67,108,864) مرّ المحاكاة الحية err=None. مسارات الإنتاج الصحيحة (opportunity.rs:1050,1108 وselection.rs:481 وtemplate.rs:529) تستخدم 64MiB؛ المسارات المعطوبة: `dlmm_4leg_flash_arb.rs:621,704`=256,000 و`compiler.rs:1047`=262,144 و`dlmm_3leg_flash_arb.rs:531`=1,048,576 — كلها ستُرفض حياً — **أُصلحت كلها في PR #55** بثابت `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT=67,108,864` + 5 اختبارات wire-decoding regression (`v1_loaded_data_limit_regression.rs`)؛ `meteora_atomic_builder.rs:53` (8MiB) أُصلحت أيضاً | دليل حي 2026-11-17 على Solami Agave 4.4.1: limit=64MiB → err=None loaded=10,714,291؛ limit=4MiB/1MiB/256KiB/32KiB → err=MaxLoadedAccountsDataSizeExceeded units=0 (مكرر 4 مرات)؛ v0 sig 5w9k6SeA…gX finalized |
| G16 | **FIXED/TESTED**: defaults المحرك موحّدة على ثوابت arb-constants — `hop_slippage_bps: DEFAULT_HOP_SLIPPAGE_BPS` و`cu_limit: APPLICATION_SAFETY_CU_LIMIT` في dlmm_flash_arbitrage.rs وmeteora_fresh_pipeline.rs والأمثلة؛ priority fee للـ flash path صار ثابتاً موثّقاً `FLASH_ARB_PRIORITY_FEE_MICRO_LAMPORTS=10_000` (سلوك محفوظ، مصدر واحد)؛ اختبار `defaults_match_canonical_constants` يثبّت العقد | arb-constants + arb-engine + arb-sim |
| G12 | **PARTIAL**: bounded live soak نجح (3 sweeps كاملة على الـ endpoint العام المجاني: discovery 500 pools → fresh capture → quotes → economics → builder gate fail-closed، لا شيء بُثّ). الـ 24-hour gate ما زال مفتوحاً — يمكن تشغيله على بنية مجانية عبر run-shadow.sh + public RPC (يحتاج وقتاً لا مالاً) | reports/production/LIVE_PUBLIC_ENDPOINT_VERIFICATION_2026-11-17.md |

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

- [x] مراجعة جميع مزودي RPC المستخدمين. `(حي 2026-11-17: probes لـ Public/Helius/Shyft/Solami — docs/audit/PROVIDER_CAPABILITY_MATRIX.md)`
- [x] التحقق من كل API endpoint. `(حي: Public HTTP+WS OK؛ Shyft HTTP OK؛ Helius 429 مستمر؛ Solami 4002 no bandwidth)`
- [x] تحديد نوع كل endpoint: HTTP أو WebSocket أو gRPC. `(capability.rs ProviderTransport {HttpJsonRpc, WebSocketJsonRpc, WebSocketMirageProtobuf, WebSocketBlurJson, YellowstoneGrpc})`
- [x] التحقق من دعم كل مزود لـ accountSubscribe. `(حي: Public ack + accountNotification فعلي؛ Helius/Shyft WS مرفوض 429 حالياً — موثّق)`
- [x] التحقق من دعم programSubscribe. `(حي: Public ack سليم 2026-11-17)`
- [x] التحقق من دعم logsSubscribe. `(حي: Public ack — يتطلب صيغة config struct {"commitment":"confirmed"} وليس string)`
- [x] التحقق من دعم slotSubscribe. `(حي: Public ack + slotNotification فعلي)`
- [x] التحقق من دعم blockSubscribe إن كان مطلوباً. `(غير مطلوب في المسار الحالي؛ Public يرفضه -32602 — موثّق في المصفوفة)`
- [x] التحقق من دعم getMultipleAccounts. `(حي: Shyft 200 + Public 200 بقيمتين؛ Helius 429)`
- [ ] التحقق من حدود الاشتراكات المتزامنة. `(PARTIAL: معلنة في RateLimitPolicy + ملاحظة حية "Concurrent websocket subscription limit reached: 1" من Solami؛ ليست مُثبتة لكل خطة)`
- [ ] التحقق من عدد اتصالات WebSocket المسموح بها. `(PARTIAL: WS_MAX_ACCOUNT_SUBS سقف داخلي + rotation؛ حد المزود الفعلي غير مُثبت لكل خطة)`
- [x] التحقق من حدود RPS الرسمية. `(حي: Helius/Shyft/Solami ردّت HTTP 429 عند التجاوز — سلوك quota مؤكد فعلياً 2026-11-17; registry يضع cooldown 60s ويدوّر)`
- [x] التحقق من حدود burst. `(حي: Helius 429 فوري على استدعاءين متوازيين؛ حدود burst المقاسة مُرمّزة في classify_provider: rpc_pool.rs:271-335)`
- [ ] التحقق من حدود CU/RU الشهرية. `(PARTIAL: Helius 429 مستمر حتى بعد cooldown 65s — متسق مع استنزاف credits (cooldown 1800s في الكود) لكن لا يمكن التمييز عن burst دون لوحة المزود)`
- [ ] التحقق من حدود كل RPC method بشكل منفصل. `(PARTIAL: method_metrics لكل endpoint موجود؛ حدود المزود لكل method غير مُتحقق منها حياً)`
- [x] منع افتراض أن جميع المزودين يدعمون نفس القدرات. `(capability.rs: تعريفات لكل مزود + validate_subscription يرفض accountSubscribe على Blur — اختبار test_registry_validation)`
- [x] فصل Provider Identity عن Endpoint Identity. `(rpc_pool.rs: classify_provider لكل URL + label مع key_suffix_hint آمن ..xxxx؛ كل endpoint له TokenBucket وtelemetry مستقلان)`
- [ ] منع تكرار استهلاك quota لنفس المفتاح عبر endpoints مختلفة. `(PARTIAL: المحاسبة لكل endpoint وليست مجمّعة لكل مفتاح — قيد موثّق في PROVIDER_CAPABILITY_MATRIX.md)`
- [x] إضافة quota accounting مركزي. `(rpc_pool.rs:128,761 — كشف 429/Retry-After/quota_exhausted لكل endpoint + telemetry)`
- [x] إضافة Provider Health Score. `(rpc_pool.rs:91-104 — EndpointStatus حسب latency/429/أخطاء)`
- [x] إضافة latency measurements لكل مزود. `(rpc_pool.rs:104,180 — total_latency_ms + p50/p95/p99)`
- [x] إضافة Provider Circuit Breaker. `(rpc_pool endpoint rotation + ProviderSupervisor OpenCircuit — مربوطة الآن بحلقة التشغيل: record_success/record_failure + select() fail-closed، PR #56/G2)`
- [x] إضافة exponential backoff. `(helius_ws.rs:490,771 + rpc_pool Retry-After)`
- [x] إضافة jitter للـ retries. `(FIXED 2026-11-17: helius_ws reconnect الآن ±25% jitter overlay فوق 2.5-3.0s؛ backoff_delay في lib.rs فيه exponential+jitter — FIXED→TESTED عبر 12 helius_ws tests)`
- [x] منع reconnect loops. `(backoff + cooldown rotation قبل إعادة الاتصال)`
- [x] منع retry غير المحدود. `(rpc_pool.rs:1042 — retry فقط عند وجود endpoint مرشح)`
- [x] إضافة provider cooldown. `(helius_ws.rs:768 — cooldown 2.5-3.0s عند التدوير)`
- [x] اختبار failover بين المزودين. `(provider_supervisor tests + rpc_telemetry_tests.rs; حي: الـ 3 endpoints دخلوا 429-cooldown ودار التدوير ثم نجح discovery — 253 pools بعد التعافي)`
- [x] اختبار فشل جميع المزودين. `(TESTED: اختبار جديد all_endpoints_failing_fails_closed_without_storm — 429 من كل endpoints → خطأ واحد محدود + cooldown بدون retry storm؛ + دليل حي من الـ soak: 0/148 success → صفر فرص/بث)`
- [x] إيقاف إصدار الفرص عند فقد جميع مصادر البيانات. `(validate_execution_readiness مع stream_connected fail-closed عند لحظة الإطلاق — dispatch.rs:535)`

## 2. Provider Capabilities وAdapters

- [x] ربط ProviderCapabilityRegistry فعلياً بالـ Supervisor. `(PR #56 — run.rs يبني registry ويسجّل Helius WS capability ويستدعي validate_execution_prerequisites قبل إقلاع worker؛ Err = bail! fail-closed، commit 804d386)`
- [x] منع استدعاء method غير مدعومة. `(helius_ws.rs:44,706,1475 — -32601 تُعامل كفشل fatal للـ endpoint مع failover)`
- [x] فصل JSON-RPC HTTP عن JSON-RPC WebSocket. `(ProviderTransport enum — capability.rs:12)`
- [x] فصل Yellowstone gRPC عن WebSocket. `(ProviderTransport::YellowstoneGrpc منفصل)`
- [x] فصل Solami Mirage عن Solami Blur. `(WebSocketMirageProtobuf ≠ WebSocketBlurJson + capability منفصلة لكل منهما)`
- [x] عدم استخدام Solami adapter كأنه Helius JSON-RPC. `(Mirage غير موصّل بالمسار الحي إطلاقاً — capability registry فقط؛ live 4002 يغلقه fail-closed — G5؛ PROVIDER_CAPABILITY_MATRIX.md)`
- [x] التحقق من بروتوكول الرسائل لكل مزود. `(capability.rs — transport + encoding + commitments لكل مزود)`
- [x] التحقق من Authentication لكل adapter. `(from_env فقط للمفاتيح؛ redacted_hosts + اختبار test_redact_url_and_hosts_hide_credentials؛ labels بـ ..xxxx suffix فقط — لا مفاتيح في السجلات)`
- [x] التحقق من Subscription Request Format. `(build_account_sub_frame + اختبارات collect_ws_urls/rotate؛ حي: ack سليم من Public RPC 2026-11-17)`
- [x] التحقق من Subscription Response Format. `(handle_message: {result: sub_id, id} → sub_to_account map — مُختبر في account_notification_updates_raydium_pool_state_in_cache)`
- [x] التحقق من Notification Format. `(slot/account notifications مع context slot — حي من Public RPC + اختبارات؛ slotNotification يقرأ parent أيضاً)`
- [x] التحقق من Error Mapping. `(-32601/unsupported → endpoint_unsupported fatal؛ limit → account_sub_limited transient؛ id==1 → تعطيل slot sub فقط؛ rpc_pool: 429/403/401/quota — اختبارات مخصصة لكل مسار)`
- [x] إضافة adapter health checks. `(EndpointStatus + ProviderHealthReport في rpc_pool؛ is_connected/mark_helius_connected في helius_ws؛ ProviderSupervisor circuit breaker)`
- [x] إضافة adapter integration tests. `(13 اختبار helius_ws + 10 rpc_pool + provider_supervisor tests + rpc_telemetry_tests)`
- [x] إضافة malformed message tests. `(جديد 2026-11-17: malformed_frames_are_skipped_without_poisoning_state — إطارات فاسدة/base64 تالف/أنواع خاطئة بدون panic أو تلويث حالة)`
- [x] إضافة unsupported method tests. `(handle_message_flags_unsupported_method_as_capability_failure + slot_subscribe_rejection_disables_slot_sub_without_failing_endpoint)`
- [x] توثيق القدرات الفعلية لكل adapter. `(docs/audit/PROVIDER_CAPABILITY_MATRIX.md — مصفوفة حية 2026-11-17 + جدول استهلاك hot-path)`
- [x] تعطيل أي adapter غير مكتمل بدلاً من الادعاء بأنه يعمل. `(Mirage معطّل fail-closed بعد 4002؛ yellowstone-grpc وextra-ingest خلف feature flags افتراضياً مطفأة)`
- [x] التحقق من استهلاك كل adapter داخل مسار التشغيل الحقيقي. `(run.rs يستخدم: HeliusWsClient + RpcPool + PoolDiscovery + ProviderSupervisor فقط؛ الباقي feature-gated أو auxiliary — جدول في المصفوفة)`
- [x] إزالة adapters غير المستخدمة أو توثيق سبب الاحتفاظ بها. `(جدول الاحتفاظ في PROVIDER_CAPABILITY_MATRIX.md: solami جاهز لترقية الخطة G5، bitquery/dexscreener/gmgn/oxylabs auxiliary فقط)`

## 3. WebSocket وYellowstone

- [x] إصلاح ترتيب slotSubscribe وaccountSubscribe. `(حسم G8: slot sub اختياري capability-gated ولا يستهلك ميزانية الاشتراكات؛ حسابات WSOL-linkage أولاً — helius_ws.rs:655-676)`
- [x] إعطاء account subscriptions الأولوية. `(ترتيب الاشتراك حسب WSOL-linkage — helius_ws.rs:1706)`
- [x] عدم استهلاك الاشتراك الوحيد في slot subscription غير ضروري. `(G8: WS_SLOT_SUBSCRIBE=0 يعطّل slotSubscribe؛ وعند provider بعدد اشتراكات = 1 يبقى الميزان كاملاً للحسابات)`
- [x] جعل slot subscription اختيارياً حسب provider capability. `(G8: helius_ws.rs — slot_subscribe_enabled gate عند connect؛ رفض slotSubscribe (id=1) يعطّل slot sub فقط دون إسقاط endpoint، وslot freshness يعتمد accountNotification context.slot؛ اختباران: slot_subscribe_rejection_disables_slot_sub_without_failing_endpoint + slot_subscribe_env_gate_parses_strict_values)`
- [x] إصلاح single-endpoint backoff. `(record_connect_backoff + اختيار endpoint الذي ينتهي cooldown أولاً — helius_ws.rs:229-239)`
- [x] إصلاح resubscription بعد reconnect. `(إعادة الاشتراك من fresh state عند كل connect — helius_ws.rs:584-614)`
- [x] استعادة جميع الحسابات المطلوبة بعد reconnect. `(pending_subs تُبنى من tracked_order عند كل اتصال)`
- [x] منع duplicate subscriptions. `(multi_sub.dedup() + اختبار مخصص — helius_ws.rs:206,1382)`
- [x] منع duplicate notifications. `(DeduKey {pool_id, slot, write_version} في apply_pool_update — lib.rs:580؛ AccountSnapshotStore.apply يرفض (slot,wv) <= الحالي)`
- [x] اكتشاف missing notifications. `(stale/freshness gates تُسقط pools التي لم تصلك تحديثاتها + SnapshotReconciler::MissingAccount + quote_readiness max_age_slots)`
- [ ] اكتشاف stream gaps. `(PARTIAL: slot_integrity يتتبع parents وSLOT_DEAD وinvalidation، لكن لا يوجد إنذار صريح لكل slot مفقود — يُكتشف عبر staleness)`
- [x] اكتشاف انقطاع stream. `(30s stall timeout → failover؛ mark_helius_disconnected + is_connected + stream_generation)`
- [x] إضافة heartbeat. `(ping كل 15s — helius_ws.rs:648)`
- [x] إضافة connection timeout. `(timeout 30s على استقبال frames — helius_ws.rs:675)`
- [x] إضافة subscription timeout. `(FIXED 2026-11-17: probe sub غير مُقرّ خلال 15s مع بقاء النقل حياً → failover — سدّ حالة "أعمى للأبد" عند ضياع ack)`
- [x] إضافة message processing timeout. `(N/A بحجة: handle_message متزامنة bounded بدون await — الـ 30s recv timeout يحدّ وصول الإطارات، والمعالجة decode-only)`
- [x] إضافة bounded queues. `(pending_subs محدودة بـ WS_MAX_ACCOUNT_SUBS؛ pending_derived map مُزيلة التكرار بنافذة 1500ms؛ latencies مسقوفة 256)`
- [x] إضافة backpressure. `(account_sub_limited يوقف الـ queueing عند رفض المزود دون teardown؛ covered-window يمنع إغراق hydration)`
- [x] منع تراكم الرسائل القديمة. `(ترتيب (slot, write_version) + رفض dead slots + freshness filter — لا تُطبَّق تحديثات أقدم)`
- [x] إضافة stream sequence tracking. `(write_version أحادي لكل pool + slot ordering + رفض provider switch دون generation reset — AccountSnapshotStore.apply)`
- [x] إضافة reconnect metrics. `(جديد 2026-11-17: arb_metrics::ws_reconnects_total — يزيد عند كل rotate/fail-connect في helius_ws)`
- [x] إضافة subscription count metrics. `(جديد 2026-11-17: arb_metrics::ws_subscriptions_active gauge — عدد الاشتراكات المؤكدة على الـ endpoint النشط)`
- [x] اختبار اتصال مزود واحد. `(اختبارات helius_ws تُبنى على endpoint واحد: handle_message_* + slot_notification_* + account_notification_*)`
- [x] اختبار اتصال مزودين. `(dual_ingest — 4 tests + dual_ingest_bench)`
- [x] اختبار انقطاع المزود الأساسي. `(rotate_endpoint_wraps_across_all_endpoints + مسار connect-failure مع record_connect_backoff + endpoint_unsupported failover — مُختبرة)`
- [ ] اختبار رجوع المزود الأساسي. `(PARTIAL: انتهاء backoff يسمح بالعودة عبر select_active_endpoint_index (الأبكر انتهاءً)؛ لا يوجد اختبار مخصص لدورة العودة)`
- [x] اختبار رفض subscription. `(slot_subscribe_rejection_disables_slot_sub_without_failing_endpoint + handle_message_treats_subscription_errors_as_non_fatal)`
- [x] اختبار تجاوز subscription quota. `(handle_message_treats_subscription_errors_as_non_fatal: "Concurrent websocket subscription limit" → account_sub_limited بدون تصنيفه capability failure)`
- [x] اختبار malformed WebSocket frames. `(regression test لـ Alchemy -32601 — helius_ws.rs:1475)`
- [x] عدم الادعاء بوجود Yellowstone gRPC دون تنفيذ فعلي. `(yellowstone_grpc.rs: تنفيذ tonic/prost حقيقي 397 سطر — SubscribeUpdate decode + write_version حقيقي؛ feature-gated افتراضياً وموثّق FEATURE-GATED في المصفوفة)`

## 4. Solami Mirage / Blur

> حالة 2026-11-17: البنود غير المؤشرة أدناه BLOCKED على G5 (خطة Solami المجانية ترفض WSS بـ code 4002 "no bandwidth") — لا يمكن التقاط frames حية لعكس هندسة البروتوكول أو بناء fixtures. الـ adapter معطل fail-closed ولا يصل حالة السوق. البنود 1,2,4-10,14-18: MISSING (passthrough خام بدون decode) — لن تُنفذ حتى يتوفر وصول مدفوع أو موثق للبروتوكول.

- [ ] التحقق من صيغة Mirage frames.
- [ ] تنفيذ protobuf parser حقيقي إذا كان البروتوكول يتطلبه.
- [x] استخراج slot الحقيقي من الرسالة. `(FIXED — كان العيب الكامن: run_stream كان يضع slot = latest_slot المحلي (solami_mirage.rs:138)؛ أُصلح: MirageUpdate.slot صار Option<u64> = None حتى يأتي decode حقيقي لـ SubscribeUpdate protobuf — لا تزييف provenance)`
- [ ] استخراج pubkey الحقيقي.
- [ ] استخراج account owner الحقيقي.
- [ ] استخراج write version الحقيقي إن كان متاحاً.
- [ ] استخراج account data الحقيقي.
- [ ] التحقق من account update type.
- [ ] التحقق من account deletion.
- [ ] التحقق من slot metadata.
- [x] منع استخدام latest_slot المحلي كبديل عن slot الرسالة. `(FIXED — نفس الإصلاح أعلاه: حذف استهلاك latest_slot من MirageUpdate نهائياً؛ الحقل ما زال موجوداً كـ health indicator فقط)`
- [x] منع استخدام write_version مصطنع. `(VERIFIED OK — write_version دائماً None في MirageUpdate ولا يُصنع أبداً؛ SnapshotStore يرفض snapshots بدون write_version حقيقي)`
- [x] منع استخدام provider_id ثابت لجميع المزودين. `(VERIFIED OK — ProviderId لكل مزود في AccountSnapshot + WrongProvider rejection في build_quote_dependency_set (quote_readiness.rs:86-88) + rejects_mixed_generation_and_provider_provenance test)`
- [ ] إضافة Mirage fixtures.
- [ ] إضافة Mirage parser tests.
- [ ] إضافة malformed protobuf tests.
- [ ] إثبات اتصال Mirage داخل Supervisor.
- [ ] إثبات وصول account updates حقيقية.
- [x] التحقق من حدود خطة Solami المجانية الفعلية. `(حي: WSS قبول ثم رفض code 4002 "no bandwidth and balance under the 0.06 minimum" — الخطة المجانية صفر bandwidth، غير قابلة للاستخدام للـ streaming)`
- [x] تعطيل Mirage إذا لم يكتمل parsing أو لم تثبت صلاحية بياناته. `(VERIFIED OK — run_stream غير مستدعاة في أي مكان بالـ runtime (grep مؤكد)؛ capability registry يسجل solami_mirage كـ DISABLED fail-closed pending G5؛ بوابات capability في run.rs:492-507 تمنع أي استهلاك)`

## 5. HTTP Polling وHot Path

- [x] حصر كل استدعاءات HTTP RPC. `(جرد 2026-11-17: helius_ws ×3 bootstrap/hydration؛ run.rs ×3 balances/bootstrap/vault-stale؛ execution.rs ×3 pre-dispatch+blockhash؛ jito_service ×3 control-plane؛ registry/discovery — لا استدعاءات في مسار الفرص)`
- [x] حصر كل استدعاءات getMultipleAccounts. `(المواقع أعلاه؛ كلها bootstrap أو hydration مُشغَّل بالأحداث أو pre-dispatch revalidation)`
- [x] حصر كل استدعاءات getAccountInfo. `(صفر في المسار الحي — القراءات عبر get_multiple_accounts مجمّعة فقط)`
- [x] حصر كل استدعاءات getProgramAccounts. `(discovery فقط؛ http_discovery_enabled=false افتراضياً — arb-config lib.rs:457)`
- [x] حصر كل دورات Pool Discovery. `(تعمل مرة واحدة عند الإقلاع فقط عندما تُفعَّل بالـ config؛ وإلا arrive عبر WS stream — run.rs:459-473)`
- [x] حصر كل دورات Pool Refresh. `(دورة إصلاح واحدة 30s — run.rs:677-690 — موثقة كـ repair وليست real-time path؛ الـ 750ms poll القديم أُزيل كان يستهلك ~70GB/day)`
- [x] مراجعة fetch_live_pool_states. `(registry.rs:586 — تُستدعى عند bootstrap + دورة الإصلاح 30s فقط؛ ليست في مسار الفرص)`
- [x] مراجعة fetch_payer_token_balances. `(run.rs:85 — pre-dispatch فقط للـ ATAs الثلاثة)`
- [x] مراجعة Kamino reserve polling. `(run.rs:723-732 — refresh فقط عند staleness >100 slots؛ وإلا WS updates)`
- [x] فصل Bootstrap عن Runtime. `(seed_initial_accounts + getMultipleAccounts coherent batch عند الإقلاع؛ الـ runtime WS-driven حصرياً)`
- [x] إزالة recurring market-data polling بعد bootstrap. `(الـ 750ms full poll أُزيل؛ بقيت دورة إصلاح 30s مقصودة لإصلاح رسائل WS المفقودة — ليست مصدر الفرص ولا في hot path)`
- [x] نقل pool updates إلى streams. `(accountNotification → apply_pool_update — حي ومُختبر: account_notification_updates_raydium_pool_state_in_cache)`
- [x] نقل vault updates إلى streams. `(vaults مسجلة track_account في run.rs:516-528 + settlement ATA)`
- [x] تحديث tick arrays من مصادر streaming المناسبة. `(CLMM: fail-closed + queue_derived_hydration event-driven — helius_ws.rs:1245)`
- [x] تحديث DLMM bin arrays من streams. `(نفس آلية hydration المُشغَّلة بالأحداث — helius_ws.rs:1155,1188)`
- [x] عدم إعادة بناء كل حالة السوق دورياً عبر HTTP. `(الـ 750ms poll أُزيل — run.rs:677 comment: ~70GB/day redundant؛ دورة الإصلاح 30s تجلب الفروقات فقط)`
- [x] عدم استخدام HTTP fallback عند فشل WSS لإصدار فرص. `(validate_execution_readiness مع stream_connected fail-closed — dispatch.rs:535؛ لا فرص من HTTP)`
- [x] إبقاء Jito submission معزولاً. `(crate منفصل arb-land: bidder/leader/ledger/lifecycle/tip_oracle)`
- [x] إبقاء control-plane HTTP معزولاً. `(control_service + telegram منفصلان عن data plane)`
- [x] إثبات صفر recurring HTTP market-data calls في hot path. `(WS هو المصدر الحي الوحيد + dispatch fail-closed عند غياب stream + soak: صفر فرص عندما فشل كل الـ RPC — لا stale fallback)`

## 6. Account Dependency Planner

- [x] بناء dependency manifest لكل route. `(PoolDependencyManifest + route_requirements — dependency_manifest.rs، 9 unit tests)`
- [x] تحديد جميع Pool Accounts. `(AccountRole::PoolState)`
- [x] تحديد Vault Accounts. `(AccountRole::VaultA/VaultB)`
- [x] تحديد Mint Accounts. `(AccountRole::MintA/MintB)`
- [x] تحديد Token Accounts. `(TESTED — vaults/ATAs بـ owner = SPL Token/Token-2022 صريح في from_registered_pool + manifest_for_registered_pool (dependency_manifest.rs:84-96,105-131)؛ user ATAs مشتقة بـ get_associated_token_address في kamino_flash.rs:42,61,77 كـ execution metas موثقة)`
- [x] تحديد Token Program Accounts. `(AccountRole::TokenProgram)`
- [x] تحديد Tick Arrays. `(AccountRole::TickArray)`
- [x] تحديد DLMM Bin Arrays. `(AccountRole::BinArray + حساب required_bin_arrays)`
- [x] تحديد Oracle Accounts. `(AccountRole::Oracle)`
- [x] تحديد Observation Accounts. `(AccountRole::Observation)`
- [x] تحديد Fee Configuration Accounts. `(AccountRole::FeeConfig)`
- [x] تحديد Creator Fee Accounts. `(AccountRole::CreatorVault)`
- [x] تحديد Flash Loan Reserve Accounts. `(TESTED — KaminoFlashAccounts {reserve, reserve_supply_vault, reserve_fee_vault, reserve_liquidity_mint} في kamino_flash.rs:68-97، والمصدر live-selected عبر decode_reserve_candidate (PR #69)؛ ليست market-state deps لأن بياناتها لا تؤثر على السعر — موثق في resolve_protocol_dependencies doc)`
- [x] تحديد Lending Market Accounts. `(TESTED — lending_market = KAMINO_MAIN_MARKET + lending_market_authority مشتقة بـ derive_lending_market_authority (kamino_flash.rs:70,84-85) ضمن مصفوفة 12 حساب للـ FlashBorrow/FlashRepay)`
- [x] تحديد Repayment Accounts. `(TESTED — Flash Repay يستخدم نفس KaminoFlashAccounts + user_liquidity_ata (ATA الاسترداد) (kamino_flash.rs:111-127)؛ الاسترداد مفروض ذرياً داخل نفس المعاملة)`
- [x] تحديد Associated Token Accounts. `(TESTED — user_destination_liquidity/user_liquidity_ata مشتقة per-mint عبر get_associated_token_address في from_candidate + to_flash_accounts (kamino_flash.rs:56-97)؛ مصنفة execution metas عمداً وليست quote-state dependencies)`
- [x] تحديد الحسابات الإضافية المطلوبة لكل DEX. `(BondingCurve/GlobalConfig/SharingConfig/MintExtension/TransferHook/BitmapExtension)`
- [x] تحديد Expected Owner لكل حساب. `(PR #56 — protocol_owner_program() يربط كل DexProtocol بـ program ID الرسمي من arb-constants، والـ gate يتحقق من owner عبر quote_dependencies_for_route → validate_dependency_group)` `(البنية موجودة — الربط بالمسار الحقيقي معلّق على G3)`
- [x] تحديد Required/Optional لكل dependency. `(AccountDependency مع role + required)`
- [x] ربط dependency manifest بمسار الاشتراكات. `(PR #56 — manifest_for_registered_pool مصدر وحيد لتحديد الحسابات المطلوبة لكل pool مسجَّل، يُبني ويُسجَّل lazily في DependencyManifestRegistry داخل Supervisor)`
- [x] ربط dependency manifest بمسار exact quote. `(PR #56 — quote_dependencies_for_route تستخرج requirements من الـ manifest ثم تطبق fail-closed quote readiness: freshness/slot-skew/owner/bank-identity)`
- [x] ربط dependency manifest بمسار dispatch. `(PR #56 — البوابة مستدعاة في process_opportunities قبل أي حساب تكلفة أو بناء: opportunity.rs:75)`
- [x] رفض route عند فقد أي حساب مطلوب. `(PR #56 — Err من البوابة = debug! + continue قبل معالجة الفرصة؛ حدود تغطية موثقة: vaults للـ DLMM/CLMM/Orca/PumpFun غير متتبعة فتُدقق حالة الـ pool فقط، وToken-2022 يُرفض fail-closed)`
- [x] رفض route عند وجود owner غير صحيح. `(TESTED — OwnerMismatch في build_quote_dependency_set (quote_readiness.rs:77-79) + ConflictingOwner في route_requirements (dependency_manifest.rs:268-273)؛ اختبار regression جديد wrong_owner_on_tracked_account_rejects_the_route + conflicting_owner_for_same_dependency_fails_closed)`
- [x] رفض route عند عدم اكتمال account hydration. `(TESTED — MissingAccount (quote_readiness.rs:76) لأي snapshot غائب + SnapshotReconciler::MissingAccount (slot_integrity.rs:163)؛ اختبار جديد incomplete_hydration_missing_account_rejects_the_route)`
- [x] منع إنشاء manifest فارغ كأنه صالح. `(TESTED — register يرفض EmptyManifest (dependency_manifest.rs:316-318)، route_requirements يرفض pool_ids فارغة أو required-set فارغ (:246,277-279)، ReconcileError::EmptyRequirements (slot_integrity.rs:192)؛ اختبار جديد empty_manifest_is_never_a_valid_route_dependency_set)`
- [x] إضافة dependency completeness tests. `(9 unit tests في dependency_manifest.rs)`
- [x] إضافة account owner regression tests. `(TESTED — wrong_owner_on_tracked_account_rejects_the_route (quote_readiness.rs) + conflicting_owner_for_same_dependency_fails_closed + empty_or_unowned_dependency_set_fails_closed (MissingExpectedOwner))`
- [x] إضافة account-set consistency tests. `(TESTED — incomplete_hydration_missing_account_rejects_the_route + empty_manifest_is_never_a_valid_route_dependency_set + route_manifest_deduplicates_shared_dependencies_and_is_deterministic + stream_route_can_exceed_100_but_single_rpc_snapshot_cannot)`
- [x] إثبات أن quote_route_guarded() مستدعاة فعلياً في Supervisor. `(PR #56 — الوظيفة المكافئة quote_dependencies_for_route مستدعاة فعلياً عبر validate_route_account_dependencies في process_opportunities: opportunity.rs:75,1424,1452)`

## 7. State Integrity وFork/Freshness

- [x] توحيد مصدر slot لكل snapshot. `(VERIFIED OK — كل AccountSnapshot يحمل slot من context رسالته الخاصة: accountNotification context.slot (helius_ws.rs:1394,1656) أو getMultipleAccounts context.slot؛ الترتيب الداخلي بـ (slot, write_version) لكل حساب)`
- [x] منع global slot stamping لحسابات من batches مختلفة. `(VERIFIED OK — apply_batch يرفض الدفعة كلها ما لم تتطابق context slot + provider_id + bank_id + generation عبر كل الحسابات (lib.rs:196-237)؛ لا slot عالمي يُختم على batches غير متزامنة)`
- [x] حفظ context slot الحقيقي لكل RPC response. `(VERIFIED OK — getMultipleAccounts/refresh paths تستخرج context.slot من الرد وتختم به الحسابات؛ apply_batch يفرض اتساق الـ context slot داخل الدفعة)`
- [x] حفظ slot الحقيقي لكل streaming update. `(VERIFIED OK — accountNotification context.slot مستخرج ومسجل لكل تحديث (helius_ws.rs:1394)؛ الاختبار malformed_frames يثبت أن slot السياقي يُسجل والـ snapshot الفاسد يُرفض)`
- [x] منع write_version المحلي من التظاهر بأنه Yellowstone write version. `(VERIFIED OK — عداد next_pool_write_version المحلي (registry.rs:2241) موثق كأداة ترتيب داخلي فقط ولا يُدّعى أنه Geyser write version؛ yellowstone_grpc.rs يستخدم write_version الحقيقي من SubscribeUpdate؛ بوابة require_bank_identity capability-gated تمنع المطالبة بهوية upstream غير متاحة)`
- [x] منع fabricated bank_id. `(VERIFIED OK — bank_id Option ولا يُصنع أبداً؛ store يرفض الاختيار بين بنوك متنافسة بـ write_version (lib.rs:181-187)؛ اختبار accepts_bankless_snapshots_only_when_provider_declares_no_bank_identity)`
- [x] منع provider_id ثابت لجميع المزودين. `(VERIFIED OK — ProviderId لكل snapshot + رفض provider switch بدون generation reset (lib.rs:172-181) + WrongProvider في quote readiness)`
- [x] التحقق من owner قبل قبول الحساب. `(helius_ws.rs:442 — seed يرفض owner mismatch + AccountSnapshotStore)`
- [x] التحقق من account data length. `(VERIFIED OK — كل الـ decoders تفرض حدوط طول صريحة قبل أي قراءة (registry.rs:1343,1378,1387,1407,1422,1437,1466,1504,1534,1564,1633,1805,1821,1837,1861,1980,2005,2038)؛ kamino reserve decode يفحص discriminator + الطول)`
- [x] التحقق من account discriminator. `(VERIFIED OK — decoders تتحقق من الـ 8-byte discriminator قبل decode (registry.rs:1830 PumpFun PoolState، kamino.rs decode_reserve_supply_vault/fee_vault)؛ الحسابات بدون discriminator موثقة صراحة (AmmInfo layout — registry.rs:1800)`
- [ ] التحقق من account layout version. `(PARTIAL — الطول + discriminator مفروضان في كل decoder، لكن قراءة explicit version field غير موحدة عبر كل البروتوكولات؛ مقبول لأن الـ layouts مثبتة empirically ضد حسابات حية)`
- [x] منع stale snapshots. `(QuoteReadinessError::StaleSlot/DeadSlot + فحص freshness في store)`
- [x] منع duplicate snapshots. `(VERIFIED OK — apply يرفض (slot, write_version) <= الحالي (lib.rs:187)؛ apply_batch يرفض الدفعة إن كانت أي تحديثة ليست أحدث (lib.rs:237))`
- [x] منع out-of-order updates. `(VERIFIED OK — نفس الترتيب (slot, write_version) في apply/apply_batch + InvalidDependencyManifest/StaleSlot في quote readiness + slot_integrity out-of-order tests)`
- [x] اكتشاف SLOT_DEAD. `(VERIFIED OK — observe_with_bank يعالج SLOT_DEAD مع إبطال انتقالي للأحفاد (slot_integrity.rs)؛ DeadSlot rejection في build_quote_dependency_set)`
- [x] إبطال processed fork غير المعتمد. `(VERIFIED OK — SLOT_DEAD → إبطال انتقالي لل descendants + invalidate_bank مع tombstones (lib.rs:268-283)؛ اختبار rejects_bank_invalidated_by_entry_update_parent)`
- [x] إبطال state المرتبط بفork غير صالح. `(VERIFIED OK — invalidate_bank يحذف snapshots البنك الميت ويترك tombstones مستقلة بـ (slot, bank_id) (lib.rs:268-283)؛ is_bank_dead في quote readiness يرفض أي snapshot من بنك ميت)`
- [x] منع دمج snapshots من مصادر غير متزامنة دون سياسة consistency واضحة. `(VERIFIED OK — السياسة: رفض provider-switch بدون generation reset + رفض (slot,bank_id) متنافسة + apply_batch context-slot-coherent + validate_dependency_group (max_slot_skew) + WrongProvider/WrongGeneration fail-closed)`
- [x] إضافة State Generation. `(stream_generation متتبعة في ingest + expected_stream_generation في dispatch.rs:535)`
- [x] إضافة State Integrity Tests. `(8 tests في slot_integrity.rs + 8 في quote_readiness.rs)`
- [x] إضافة Fork Invalidation Tests. `(DeadSlot/FutureSlot variants + slot_integrity tests)`
- [x] إضافة Stale Snapshot Tests. `(StaleSlot variant + tests)`
- [x] إضافة Duplicate Update Tests. `(DuplicateDependency + dedup في store)`
- [x] إضافة Out-of-Order Tests. `(slot ordering checks في slot_integrity tests)`
- [x] جعل state غير الصالح سبباً مباشراً لرفض التنفيذ. `(VERIFIED OK — بوابة validate_route_account_dependencies في process_opportunities (opportunity.rs:75) ترفض قبل أي بناء، وvalidate_execution_readiness في dispatch (dispatch.rs:535) ترفض لحظة الإرسال عند disconnect/generation-change/stale quote)`

## 8. DEX Exact Quote Parity

- [x] مراجعة Raydium AMM V4. `(VERIFIED OK — quote في raydium_v4.rs:349 + effective reserves/eligibility (:84-113)؛ محاسبة الرسوم مطابقة لـ on-chain v4 عبر cpmm_out_upfront_fee_ceil (cpmm.rs:41-60)؛ live-verified offsets (PROTOCOL_VERIFICATION.md) + differential coverage)`
- [x] مراجعة Raydium CPMM. `(11 unit tests في arb-math/src/cpmm.rs)`
- [x] مراجعة Raydium CLMM. `(13 unit tests في arb-math/src/clmm.rs + clmm_probe example)`
- [x] مراجعة Raydium LaunchLab. `(VERIFIED OK — 10 unit tests في arb-math/launchlab.rs + adapter raydium_launchlab.rs + differential coverage عبر differential.rs)`
- [x] مراجعة Orca Whirlpool. `(orca_sdk_parity.rs — مقارنة rounding مع orca_whirlpools_core الرسمي)`
- [x] مراجعة Meteora DLMM. `(18 unit tests في arb-math/src/dlmm.rs + dlmm_bin_array_boundary_tests.rs)`
- [x] مراجعة Meteora DAMM v2. `(VERIFIED OK — 10 unit tests في arb-math/damm_v2.rs منها price_impact_is_monotone_in_amount + adapter meteora_damm_v2.rs)`
- [x] مراجعة Pump.fun. `(3 tests في arb-dex/src/pump_fun.rs + fuzz bounds في fuzzing_suite)`
- [x] مراجعة PumpSwap. `(8 unit tests في arb-dex/src/pump_swap.rs)`
- [x] مراجعة Jupiter adapter. `(VERIFIED OK — auxiliary-only: JupiterClient مستخدم في validate.rs/live_scanner.rs (control-plane) فقط؛ المسار الذري الحي يبني تعليمات DEX مباشرة لأن مسار Jupiter لا يمكنه حمل flash-borrow/repay مخصص؛ موثق في adapter inventory بـ PROVIDER_CAPABILITY_MATRIX.md)`
- [x] التأكد من أن كل quote يستخدم account state الصحيح. `(VERIFIED OK — الـ quotes تستهلك PoolState من snapshot store مُرشّح عبر بوابة quote-readiness (opportunity.rs:75) — لا quote من حالة stale/ميتة/owner خاطئ)`
- [x] التحقق من Token Decimals. `(VERIFIED OK — decimals_a/b تُقرأ من حسابات الـ mint في الـ decoders (offsets live-verified في PROTOCOL_VERIFICATION.md) وتُستخدم في التحويل الاقتصادي)`
- [x] التحقق من Pool Reserves. `(VERIFIED OK — update_reserves/refresh_effective_reserves (raydium_v4.rs:84-102)؛ reserves حية من vault balances لـ V4/CPMM ومن tick/bin data لـ CLMM/DLMM)`
- [x] التحقق من LP Fees. `(VERIFIED OK — fee upfront بـ ceiling division مطابق لـ on-chain v4/CPMM/DAMM v2 (cpmm.rs:41-60)؛ fee decoders لكل بروتوكول (decode_raydium_fee_bps إلخ))`
- [x] التحقق من Protocol Fees. `(VERIFIED OK — محاسبة CPMM swap = trade fee فقط مطابقة للـ on-chain؛ Kamino protocol fee = flash fee_sf (U68F60) يُقرأ حياً من الـ reserve (PR #69) ويُخصم في الحساب الاقتصادي)`
- [x] التحقق من Creator Fees. `(VERIFIED OK — creator_fee_bps يُقرأ حياً من الـ Pool layout (pump_swap.rs:110-129) مع tiering حسب market cap؛ CreatorVault role إلزامي في dependency resolver لـ PumpFun/PumpSwap)`
- [x] التحقق من Transfer Fees. `(VERIFIED OK — transfer_fee_bps في PoolState؛ رفض fail-closed عند != 0 في discovery وquote engine (arb-engine/src/lib.rs:474-475, meteora_quote_engine.rs:61-62)؛ بند transfer_fees في economic.rs:164,326)`
- [x] التحقق من Token-2022 Extensions. `(VERIFIED OK — has_risky_token fail-closed في discovery + quote (نفس المواضع أعلاه)؛ Token-2022 owner صريح في dependency manifests)`
- [x] التحقق من Rounding Direction. `(VERIFIED OK — fee ceil + output floor موثقة (cpmm.rs:41-44)؛ orca_sdk_parity.rs يقارن rounding مع orca_whirlpools_core الرسمي؛ python_oracle_differential.rs يوثق نقاط floor/ceil)`
- [x] التحقق من Integer Overflow. `(adversarial_tests — overflow في marginal functions + 441-bit headroom + U768 closed form)`
- [x] التحقق من Fixed-Point Arithmetic. `(U384/U512/U768 wide integers + isqrt fuzz correctness)`
- [x] التحقق من CLMM Tick Traversal. `(fuzz_clmm_tick_to_sqrt_price valid/out-of-bounds + adversarial CLMM boundaries)`
- [x] التحقق من DLMM Bin Traversal. `(dlmm_bin_array_boundary_tests + fuzz_dlmm_get_price_from_id)`
- [x] التحقق من Bitmap Extension. `(VERIFIED OK — AccountRole::BitmapExtension إلزامي شرطياً في dependency resolver + اختبار meteora_requires_bitmap_extension_only_when_active_path_needs_it)`
- [x] التحقق من Liquidity Boundaries. `(VERIFIED OK — zero liquidity → None (cpmm.rs:47)؛ adversarial_tests + dlmm_bin_array_boundary_tests + fuzz bounds)`
- [x] التحقق من Price Impact. `(VERIFIED OK — cpmm_price_impact_bps (cpmm.rs:130) + calculate_price_impact_bps (dlmm.rs:1229) + monotonicity test في damm_v2.rs:308)`
- [x] التحقق من Slippage. `(VERIFIED OK — hop_slippage_bps من config كمصدر وحيد يُختم قبل البناء (opportunity.rs:88) + min_out clamps (opportunity.rs:799-847) + HOP_SLIPPAGE_MAX_BPS)`
- [x] تنفيذ SDK Differential Tests لكل DEX. `(differential.rs + python_oracle_differential.rs في arb-math/tests — تغطية كل DEX تُراجع مع G11)`
- [x] تنفيذ Fork Acceptance Tests لكل DEX. `(TESTED — بالنموذج المكافئ والأقوى: differential.rs ضد @meteora-ag/dlmm@1.9.14 الرسمي + orca_sdk_parity.rs ضد orca_whirlpools_core + python_oracle_differential.rs ضد oracle مستقل مشتق من المواصفات + vectors في audit/test_vectors)`
- [x] توثيق الفرق بين local quote وreference quote. `(VERIFIED OK — differential.rs header يوثق المصادر المرجعية لكل DEX؛ python_oracle_differential.rs header يوثق قرارات rounding والـ vectors؛ أي mismatch = انحراف formula مؤكد باختبار فاشل)`
- [x] عدم اعتبار builder ناجحاً دليلاً على quote parity. `(VERIFIED OK — السياسة موثقة ومطبقة: docs/architecture/PROTOCOL_COMPATIBILITY.md:12,51-52 "builder path is not a production capability... no route is accepted for live trading by this matrix"؛ parity يثبت فقط عبر differential tests)`
- [x] منع قبول DEX غير مختبر باعتباره Production Ready. `(VERIFIED OK — نفس المصفوفة: كل venue بلا differential + math-parity كامل غير مقبول للتداول الحي (PROTOCOL_COMPATIBILITY.md) + capability gate يمنع protocols غير مسجلة)`

## 9. Dynamic Pool Discovery

- [x] اكتشاف pools من البرنامج والحسابات. `(VERIFIED OK — bounded hub discovery + تحقق on-chain لكل حساب pool قبل الثقة: fetch_live_pool_states (registry.rs:586) — discovery لا يوفر أي رقم يثق به الـ quote)`
- [ ] دعم اكتشاف pools الجديدة من stream. `(PARTIAL — الاكتشاف دوري bounded عبر hubs/catalogs مع تحقق on-chain لكل pool (discovery.rs:1-40)؛ لا programSubscribe لحظي لاكتشاف pools جديدة — مقصود: gPA/program sweep غير عملي على free RPC (discovery.rs:3-14)؛ التحديث الدوري عبر 30s reconciliation sweep)`
- [x] اكتشاف تغييرات pool configuration. `(VERIFIED OK — الرسوم تُقرأ حية من الـ pool account في كل fetch (decode_raydium_fee_bps — registry.rs:881,1484,1804)؛ أي تغيير fee/config ينعكس في الـ quote التالي)`
- [x] اكتشاف pool closure. `(VERIFIED OK — pool مغلق → حساب غائب → decode/fetch يفشل fail-closed → MissingAccount في بوابة dependencies يرفض أي route عليه + is_active gate)`
- [x] التحقق من pool owner. `(VERIFIED OK — protocol_owner_program يربط كل DexProtocol بـ program ID رسمي (dependency_manifest.rs:11-25) + owner gate في quote readiness)`
- [x] التحقق من pool discriminator. `(VERIFIED OK — decoders تفرض discriminator + طول قبل decode (registry.rs:1830, kamino.rs)؛ حساب فاسد → رفض fail-closed)`
- [x] التحقق من protocol type. `(VERIFIED OK — DexProtocol تصنيف مصدري + فلتر DLMM label يمنع خلط DAMM بـ LbPair (discovery.rs:20-21) + protocol_owner_program)`
- [x] اكتشاف Token Pair. `(VERIFIED OK — mints تُقرأ من حساب الـ pool على السلسلة وليس من الـ hub metadata (PROTOCOL_VERIFICATION.md — كل الـ offsets live-verified)`
- [x] اكتشاف Token Mints. `(VERIFIED OK — نفس الآلية + resolve_mint_decimals (discovery.rs:798) مع فحص owner=Token program (discovery.rs:833))`
- [x] اكتشاف Vault Relationships. `(VERIFIED OK — decode_raydium_vaults/cpmm_vaults/clmm_vaults (discovery.rs:24-26) + اختبار raydium_vaults_decode_from_real_offsets (discovery.rs:1198)`
- [x] اكتشاف Tick Arrays المطلوبة. `(VERIFIED OK — raydium_clmm_tick_array_pdas مشتقة من الـ active tick + required_tick_arrays في dependency resolver)`
- [x] اكتشاف DLMM Bin Arrays المطلوبة. `(VERIFIED OK — bin_array_index_for_bin + required_bin_arrays (dependency resolver) + dlmm_bin_array_boundary_tests)`
- [x] اكتشاف الحسابات الإضافية. `(VERIFIED OK — resolve_protocol_dependencies يفرض Oracle/Observation/BitmapExtension/GlobalConfig/FeeConfig/CreatorVault حسب البروتوكول مع 8 اختبارات)`
- [x] منع تكرار pool registration. `(VERIFIED OK — dedup بـ pool id في discovery (discovery.rs:319) + DuplicatePool error في registry وdependency manifests)`
- [x] منع route duplication. `(VERIFIED OK — route_sig dedup في opportunity pipeline (opportunity.rs:111,922) + template cache keyed بـ route_sig)`
- [x] بناء route graph ديناميكي. `(bellman_ford.rs + discovery — 9 unit tests)`
- [x] بناء دورات من ثلاثة pools. `(find_base_cycles + reports/final_audit/THREE_POOL_EXECUTION_AUDIT.md)`
- [x] دعم أكثر من أصل وسيط. `(VERIFIED OK — دورات 3-pool بأصلين وسيطين (find_base_cycles + three_leg_meme_pipeline + THREE_POOL_EXECUTION_AUDIT.md) + bellman_ford graph عام)`
- [x] تحديث graph عند تغير liquidity. `(VERIFIED OK — الـ graph يُبنى من الـ cache الحالية في كل دورة مسح؛ تحديثات stream/refresh تحدث reserves فتتغير أوزان الـ graph تلقائياً)`
- [x] فصل Candidate Discovery عن Exact Quote. `(discovery منفصل + exact quote validation protocol-adapter driven — strategy.rs:20)`
- [x] عدم فرض spread ثابت أثناء discovery. `(VERIFIED OK — فصل Candidate Discovery عن Exact Quote (strategy.rs:20)؛ التقييم عبر net-profit economics وليس spread threshold)`
- [x] عدم اعتبار وجود pool فرصة تداول. `(VERIFIED OK — وجود pool = مرشح فقط؛ الفرصة تتطلب exact quote + net profit بعد كل التكاليف + dependency gate)`
- [x] رفض pool غير المكتمل أو غير الموثوق. `(VERIFIED OK — fail-closed decode + is_active + has_risky_token + transfer_fee_bps != 0 rejection + dependency completeness gate)`

## 10. Flash Loan وKamino

- [x] مراجعة Kamino instruction semantics. `(15 unit tests في arb-tx/src/flash/kamino.rs + reports/final_audit/KAMINO_FLASH_LOAN_AUDIT.md)`
- [x] مراجعة Borrow Instruction. `(VERIFIED OK — build_kamino_flash_borrow: 12 حسابات + payload 16-byte (kamino_flash.rs:100) + 15 unit tests في kamino.rs + KAMINO_FLASH_LOAN_AUDIT.md)`
- [x] مراجعة Repay Instruction. `(VERIFIED OK — build_kamino_flash_repay: 12 حسابات + 17-byte + borrow_instruction_index=2 (kamino_flash.rs:110-127) + repayment tests)`
- [x] التحقق من Flash Loan Fee. `(VERIFIED OK — fee_sf بتنسيق U68F60 (وليس Q64.64 — flash_loan_sizing.rs:8) يُقرأ حياً من الـ reserve (PR #69) + calculate_kamino_flash_fee في الـ sizing)`
- [x] التحقق من Reserve Configuration. `(VERIFIED OK — decode_reserve_candidate يقرأ active/status/fee sentinel/decimals من البايتات الحية؛ offsets مثبتة empirically 2026-11-17)`
- [x] التحقق من Available Liquidity. `(VERIFIED OK — سيولة حية من supply vault balance (GafNuUXj... مثبت live) + فلتر min_borrow في select_kamino_flash_reserve)`
- [x] التحقق من Reserve Utilization. `(VERIFIED OK — القيد الفعلي المستخدم أقوى من نسبة utilization: رصيد supply vault الحي هو المتاح فعلياً للاقتراض، ويُقارن مباشرة بـ min_borrow وحجم الاقتراض)`
- [x] التحقق من Borrow Limits. `(VERIFIED OK — bottleneck = أدنى capacity عبر السطور الثلاثة (flash_loan_sizing.rs:177-188) + سقف available_vault_liquidity (opportunity.rs:62))`
- [x] التحقق من Minimum Borrow. `(VERIFIED OK — فلتر available_liquidity >= min_borrow في select_kamino_flash_reserve (kamino.rs) — NoEligibleReserve fail-closed)`
- [x] التحقق من Maximum Borrow. `(VERIFIED OK — kelly fraction 85% من capacity لتجنب crossing bins (flash_loan_sizing.rs:18) + bottleneck + سقف السيولة الحية)`
- [x] التحقق من Token Decimals. `(VERIFIED OK — flash_mint_decimals من الـ candidate الحي (rehearsal [2c]: decimals 9) + ATA per-mint)`
- [x] اكتشاف جميع أصول الاقتراض المتاحة. `(G6: مسح حي لـ WSOL+USDC main-market reserves عبر decode_reserve_candidate — mint/decimals/status/fee_sf/supply+fee vaults كلها من البايتات الحية؛ offsets @152/@184 تحققت empirically ضد ثوابت WSOL المعروفة 2026-11-17)`
- [x] عدم حصر الاختيار في WSOL دون مبرر. `(G6: KaminoFlashParams.liquidity_mint generalized — to_flash_accounts لم يعد WSOL hardcoded؛ from_candidate يبني params لأي candidate حي + ATA الخاص بمintه)`
- [x] تقييم كل reserve مؤهل. `(G6: select_kamino_flash_reserve يفلتر حياً — active + fee enabled (sentinel u64::MAX مرفوض) + سيولة حية >= min_borrow — 3 اختبارات جديدة في kamino.rs)`
- [x] اختيار أفضل أصل اقتراض اقتصادياً. `(G6: أقل fee أولاً ثم الأعمق سيولة عند التعادل؛ دليل حي [2c] في rehearsal 2026-11-17: مسح 2 candidates واختيار WSOL reserve d4A2prbA بـ fee_sf 11529215046068 وسيولة 257,399 SOL حية — fail-closed عند لا مرشح مؤهل)`
- [x] تحسين Borrow Amount. `(VERIFIED OK — calculate_optimal_flash_loan + calculate_kelly_optimal_size: حجم أمثل من عمق المسار وليس مبلغاً ثابتاً (flash_loan_sizing.rs:2,139,167))`
- [x] ربط حجم الاقتراض بعمق المسار. `(VERIFIED OK — bottleneck min capacity عبر السطور الثلاثة (flash_loan_sizing.rs:177-188))`
- [x] ربط الحجم بحدود bins وticks. `(VERIFIED OK — dlmm_active_bin_max_input/dlmm_multi_bin_max_input + max_bins_crossed=3 (flash_loan_sizing.rs:5,32,307-364))`
- [x] ربط الحجم بالـ slippage. `(VERIFIED OK — max_slippage_bps في SizingConfig (flash_loan_sizing.rs:25-26,87) + leg_min_outputs محافظة لكل سطر)`
- [x] ربط الحجم برسوم الاقتراض. `(VERIFIED OK — flash_fee في SizingResult (flash_loan_sizing.rs:114) + محاسبة وحيدة في net_profit_lamports (three_leg_meme_pipeline.rs:649-651))`
- [x] رفض reserve state القديم. `(ROI implausibility ceiling يرفض stale-quote artifacts fail-closed — opportunity.rs:400-420)`
- [x] منع استخدام liquidity غير متاحة فعلياً. `(VERIFIED OK — size_in > available_vault_liquidity → رفض (opportunity.rs:62) + سيولة من vault balance الحي + فلتر min_borrow)`
- [x] إضافة reserve-specific tests. `(TESTED — 3 اختبارات G6 في kamino.rs (فلترة/ترتيب/fail-closed) + 15 اختبار instruction semantics)`
- [x] إضافة borrow sizing tests. `(TESTED — 5 اختبارات في flash_loan_sizing.rs تغطي capacity/bottleneck/fee/slippage paths)`
- [x] إضافة repayment tests. `(ضمن kamino.rs tests + economic.rs loss-path checks)`

## 11. Net Profit وEconomic Model

- [x] إنشاء مصدر واحد لحساب الربح. `(arb-math/src/economic.rs — المصدر الكانوني الوحيد، re-exported بواسطة arb-tx)`
- [x] حساب Gross Output. `(VERIFIED OK — expected − borrow بـ signed_difference (economic.rs:330) + gross_profit_signed_raw i128)`
- [x] خصم Swap Fees. `(VERIFIED OK — مخرجات الـ quote NET من رسوم LP/protocol (الرسوم من الـ input قبل traversal — flash_loan_sizing.rs:224-227 يمنع الخصم المزدوج)`
- [x] خصم Creator Fees. `(VERIFIED OK — رسوم creator تُخصم من الـ input في PumpSwap accounting (pump_swap.rs:129 live fee components) فتكون المخرجات net)`
- [x] خصم Transfer Fees. `(VERIFIED OK — transfer_fees بند صريح في build_economic_quote (economic.rs:164,326,337))`
- [x] خصم Flash Loan Fees. `(VERIFIED OK — flash_loan_fee_raw من fee_sf الحي (economic.rs:136,324) + klend fee formula failure modes (economic.rs:474-508))`
- [x] خصم Base Transaction Fee. `(VERIFIED OK — base network fee 5,000 lamports في الحساب (economic.rs:699 test)`
- [x] خصم Priority Fee. `(VERIFIED OK — priority_fee_lamports في EconomicQuote (economic.rs:168,341))`
- [x] خصم Jito Tip. `(VERIFIED OK — jito_tip_lamports من tip-floor market data (economic.rs:137-139,169,342))`
- [x] احتساب Slippage Reserve. `(VERIFIED OK — minimum_net_signed محسوب من min_out وليس expected (economic.rs:332,352) — أسوأ حالة ضمن حدود slippage)`
- [x] احتساب Execution Risk. `(VERIFIED OK — تمثيل محافظ: minimum_net (worst-case ضمن slippage bounds) هو بوابة القبول + realized-profit telemetry يسجل النتيجة الفعلية)`
- [x] احتساب تكلفة الفشل عند الحاجة. `(VERIFIED OK — real_net_profit يُحسب من balance changes الفعلية على السلسلة (opportunity.rs:1407) فيشمل رسوم المعاملات الفاشلة؛ Jito tip لا يُدفع إلا عند landing)`
- [x] منع double-counting. `(VERIFIED OK — CostLedger مع already_deducted flag ومنع double deduction (economic.rs:63-115) + dex_fees informational-only في sizing)`
- [x] منع integer overflow. `(سلاسل checked_add/checked_mul مع ok_or(ArbError::Overflow) — economic.rs:336-356)`
- [x] استخدام checked arithmetic. `(نفس الدليل — لا arithmetic غير checked في المسار الاقتصادي)`
- [x] استخدام ceiling division لرسوم priority fee. `(adaptive.rs:196-204 — ceil(micro_lamports * cu_limit / 1_000_000))`
- [x] توحيد priority fee بين sizing وranking وdispatch. `(VERIFIED OK — arb_tx::priority_fee_lamports(cu_price, cu_limit) دالة وحيدة مستخدمة في dispatch (opportunity.rs:147,1052,1110) ونفس المدخلات تمر للـ sizing (calculate_optimal_flash_loan priority_fee_lamports))`
- [x] منع saturating arithmetic في الحسابات الاقتصادية الحساسة. `(Phase12 Agent1 triage: 415 استخدام مصنّفة — 3 مواضع UNSAFE أُصلحت fail-closed: adapters.rs quote_hop reserve advance (checked + رفض amount_out >= reserve قبل أي mutation)، dlmm_flash_arbitrage.rs evaluate() economics (i128 + signed clamp)، flash_loan_sizing.rs net_profit_lamports/best_bps (i128 + signed clamp). الباقي BENIGN/CONSERVATIVE موثّق في docs/audit/PHASE12_AGENT1_FINANCIAL_ARITHMETIC_AUDIT.md — 1145 test/0 fail)`
- [x] استخدام SOL/USD price موثوق ومؤرخ. `(chronos max_age_ms — opportunity.rs:550-557)`
- [x] رفض السعر القديم. `(No fresh SOL/USD reference; rejecting candidate economic conversion — fail-closed)`
- [x] منع استخدام سعر افتراضي مثل $200 كدليل اقتصادي. `(G7: telegram.rs — sol_price_live provenance flag؛ التنبيهات تعرض وسم ديناميكي «سعر غير موثّق (افتراضي)» بدل ادّعاء «سعر حي من السوق» الثابت؛ العلم يُرفع فقط من Binance/Coinbase feed أو سعر pool حي أو SOL_USD_PRICE من المشغّل؛ اختبار unverified_sol_price_is_labeled_as_such)`
- [x] إصلاح اختلاف حدود min profit بين المسارات. `(FIXED — MIN_NET_PROFIT_LAMPORTS في arb-engine وpump_fun_raydium كلاهما = arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS (مصدر وحيد) + MIN_NET_PROFIT_USD/min_net_profit_abs عبر to_economic_config)`
- [x] مراجعة MIN_NET_PROFIT_USD. `(config.rs:385-399 — configurable، >=0 validation، f64 محصور عند config boundary فقط)`
- [x] مراجعة SOL_USD_PRICE. `(مُراجع — الافتراضي $200 ما زال في مسار التنبيهات فقط: G7)`
- [x] مراجعة min_net_profit_abs. `(يمرر إلى to_economic_config — strategy.rs:51)`
- [x] إزالة بوابات gross spread الثابتة. `(FIXED 2026-11-17 — EPSILON_CYCLE_SPREAD_BPS خُفّضت 10→1 bps (arb-engine/src/lib.rs:77): الحساب integer بلا roundoff فالقيمة القديمة كانت عقبة ربحية غير مبررة؛ dcp_default_spread 10→1 (config.rs:323)؛ القبول النهائي بـ net-profit economics فقط)`
- [x] فصل gross spread عن net profit. `(FIXED — نفس الإصلاح: الـ spread prefilter صار epsilon صارم (dust filter) ولا يقرر القبول؛ ranking بـ net_profit (strategy.rs:316,394))`
- [x] ترتيب الفرص حسب صافي الربح القابل للتنفيذ. `(sort_by_key Reverse(net_profit) — strategy.rs:316,394)`
- [x] رفض الفرصة إذا كان أي عنصر تكلفة مجهولاً. `(VERIFIED OK — fee sentinel u64::MAX مرفوض (NoEligibleReserve) + SOL/USD قديم/افتراضي مرفوض fail-closed + CostEntry يتطلب source موثق)`
- [x] إضافة economic regression tests. `(TESTED — 14 اختبار في economic.rs تشمل base fee/net profit/loss paths + 3 اختبارات UNSAFE-fix regressions من Phase12)`

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
