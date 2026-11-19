# خطة مهام رفع جاهزية Solana Arbitrage

> **الملف المقترح بعد اعتماد التنفيذ:** `C:\Users\3aZzO0OzZ\Downloads\SOOOOLLL\solana-arbitrage-FINAL\tasks.md`  
> **المرجع الفني الأساسي:** `solana-arbitrage-FINAL\REPORT.md` وملفات `docs\` في النسخة الموحدة.  
> **الحالة الحالية:** Prototype؛ التداول الحي غير جاهز ويجب أن يظل مقفولًا. هذه الخطة لا تعد بالربحية ولا تفترض أن دعم كل DEX ممكن أو مطلوب.

## قواعد التنفيذ والقبول العامة

- [ ] لا تغيّر `GUARDED_DISPATCH_INTEGRATED` ولا تتجاوز أي live lock قبل اجتياز بوابات الإطلاق أدناه.
- [ ] لا تُعلن دعم venue أو mint أو pool إلا بعد اجتياز اختبارات البيانات والتسعير وبناء المعاملة والمحاكاة الخاصة به؛ الرفض الصريح للحالات غير المدعومة هو السلوك الصحيح.
- [ ] اجعل القرار المالي النهائي بوحدات raw integer وعمليات checked؛ يجوز استخدام `f64` لترتيب مرشحين فقط، لا لتحديد تنفيذ أو ربح.
- [ ] كل نتيجة اختبار أو benchmark تسجل الأمر، البيئة، commit، fixtures، النتيجة، وحدود الاستنتاج في `docs/TEST_EVIDENCE.md`.
- [ ] لا تعتبر قبول RPC/Jito دليلًا على landing، ولا landing دليلًا على ربح محقق؛ افصل accepted / landed / finalized / realized PnL.
- [ ] اجعل كل تغيير قابلًا للتراجع، ولا تضع أسرارًا أو مفاتيح تداول أو معاملات حقيقية في الاختبارات أو ملفات المشروع.
- [ ] **تعريف Done لكل مهمة:** تغييرات مراجعة، اختبارات مستهدفة ناجحة، توثيق محدث، عدم وجود مسار جانبي يتجاوز الحراسة، ونتيجة موثقة في evidence log.

## المرحلة 0 — تثبيت النطاق والحالة الحقيقية

### S0.1 توحيد مصدر الحقيقة
- [ ] قارن التقرير الجذري `REPORT.md` بتقرير النسخة الموحدة؛ صحح أو أرشف الادعاءات غير المثبتة (إحصاءات السوق والعوائد، معرفات البرامج، رسوم/قروض غير موثقة).
- [ ] أنشئ جدول حقيقة لكل venue: إصدار البرنامج/IDL أو SDK المثبت، أنواع pools المدعومة، mint/extensions المقبولة، حالة التنفيذ (مصدر فقط / unit / parity / fork / mainnet-observed)، وأسباب الرفض.
- [ ] أصلح تناقضات `RELEASE_READINESS.md` مع وجود `dashboard/index.html`، وأضف/صحح `docs/TEST_EVIDENCE.md` والإحالات إليه.
- [ ] صنّف كل بند في `PHASE_STATUS.md` و`REMAINING_ISSUES.md` حسب مانع إطلاق / مطلوب للإنتاج / تحسين لاحق، واربطه بمعيار قبول قابل للتحقق.

### S0.2 نطاق النسخة الأولى
- [ ] اعتمد allowlist ضيقًا ومعلنًا لأنواع pools وtokens؛ لا تستخدم عبارة «كل DEX» كمعيار قبول.
- [ ] اختر مسارًا ذريًا واحدًا داخل Solana كبداية، وقرر صراحةً مصدر رأس المال (رأس مال bot أو flash borrow موثق). لا تعتبر Flash Loan شرطًا إذا كانت كلفته أو مخاطره غير مجدية.
- [ ] أخرج cross-chain من نطاق الإطلاق الأول؛ وثّق أنه ليس ذريًا عبر سلسلتين، وأنه منتج منفصل بمخزون ومخاطر تسوية.
- [ ] عرّف أهداف latency وavailability وmaximum exposure بعد قياس baseline على بيئة تشغيل محددة؛ لا تضع أرقام HFT تسويقية قبل القياس.

**بوابة G0:** لا يبدأ أي تمكين تنفيذ حي قبل اعتماد النطاق والـvenue matrix، وتأكيد بقاء live lock مغلقًا.

## المرحلة 1 — بناء قابل للتكرار وأساس اختبارات موثوق

### S1.1 تثبيت بيئة التطوير وCI
- [ ] ثبّت toolchain وإصدارات Rust/dependencies ومصدر Linux المرجعي، ووفّر CI نظيفًا لـ`fmt --check` و`cargo check --workspace --locked` و`cargo test --workspace --locked` و`clippy` و`cargo deny` حسب إعدادات المشروع.
- [ ] عالج اختلافات `rustfmt` الموثقة، ثم سجّل نتيجة baseline قبل التغييرات الوظيفية.
- [ ] أضف فحصًا لاختبارات/أمثلة لا تستخدم مفاتيح أو RPC mainnet حقيقيًا، وفحصًا يمنع تضمين `.env` أو أسرار في artifact.
- [ ] أنشئ سجل نتائج CI/evidence مع commit ونسخة compiler وبيئة التشغيل؛ لا تنقل PASS من ملف أو نسخة مختلفة.

### S1.2 عقود البيانات والاختبارات المرجعية
- [ ] أنشئ fixtures ثابتة وآمنة لحسابات كل venue (bytes، owner، slot/version، مصدرها وترخيصها/مرجعها)، مع decoder tests للحالات الصحيحة والتالفة والقديمة.
- [ ] أضف property tests وfuzz tests للـdecoders والرياضيات والتحويلات، مع حدود overflow/underflow وmalformed account وfee config غير صالح.
- [ ] اجعل اختبارات replay قابلة للتكرار بمدخلات محفوظة، ولا تستخدم بيانات حية متغيرة كشرط وحيد لنجاح CI.

**بوابة G1:** كل commit قابل للبناء والاختبار على بيئة Linux المرجعية، وفشل CI يمنع الدمج.

## المرحلة 2 — تدفق بيانات متماسك وحديث

### S2.1 سلامة Yellowstone / RPC
- [ ] حدّد مالكًا موحدًا لحالة الحسابات يتتبع provider، generation/reconnect، slot/bank/fork، write-version، commitment، وعمر البيانات.
- [ ] نفّذ اكتشاف فجوات stream وإعادة مزامنة snapshot موثوق قبل إعادة quotes؛ أبطل كل quote عند reconnect أو fork ambiguity أو state regression.
- [ ] اجمع بيانات كل pool ومكوّناته التابعة (vaults، config، oracle، tick/bin arrays، mint/token program) من سياق حالة متسق؛ ارفض المزج بين providers/banks/slots غير المتوافقة.
- [ ] أضف سياسة freshness قابلة للقياس حسب نوع الحساب والـvenue، مع رفض fail-closed للحساب المفقود أو owner/layout/slot غير المتوقع.
- [ ] اختبر إعادة الاتصال، الترتيب المتأخر، التكرار، slot rollback/fork، البيانات الناقصة، وانقطاع RPC وامتلاء القنوات.

### S2.2 `VerifiedPoolContext`
- [ ] عرّف سياقًا موثقًا ومكتملًا للـpool يشمل program/version، PDA، mint ordering، vaults ومالكيها، token programs، authority/config، رسوم الحالة، accounts المطلوبة للـswap، وslot/write-version/generation.
- [ ] تحقق من PDA/owner/mint/token-account authority وعلاقات الحسابات قبل نشر الحالة للـquote workers.
- [ ] اجعل عدم وجود أي حقل/حساب ضروري سببًا لرفض pool أو quote؛ امنع defaults مثل tick/bin صفر أو افتراض WSOL لتحديد الاتجاه.

**بوابة G2:** quote واحد لا يمكن أن يعتمد على أجزاء حالة من fork أو provider أو slot غير متسق، وكل انقطاع/تغيير غير مفهوم يبطل المرشحين.

## المرحلة 3 — تسعير صحيح ومثبت لكل venue

### S3.1 نموذج quote والاقتصاد
- [ ] وحّد واجهة exact-in quote مع تعريف واضح لوحدات raw amounts، اتجاه التحويل، تقريب البروتوكول، ورسوم كل hop.
- [ ] نفّذ `CostLedger` موحدًا يمنع الخصم المكرر ويغطي رسوم DEX/protocol/creator، transfer-fee، flash-loan premium، base/priority fee، Jito tip، والتكاليف التشغيلية المثبتة.
- [ ] افصل expected quote وminimum output عن صافي الربح المتوقع وعن PnL المحقق من balances/metadata بعد التسوية.
- [ ] استعمل optimizer لتوليد مرشحين فقط؛ أعد تسعير كل حجم مرشح بحسابات venue الدقيقة وبـinteger rounding وحدود السيولة/الاقتراض قبل القبول.
- [ ] أضف اختبارات للـfees المتغيرة، tier boundaries، حدود ticks/bins، caps، decimals، Token-2022 transfer-fee epochs، والـcreator fees.

### S3.2 Adapter parity — بوابة مستقلة لكل عائلة
- [ ] **Raydium AMM v4:** اشتق كل حسابات market/vault/authority/config من حالة pool الفعلية؛ لا تستبدل عنوان pool داخل قالب SOL/USDC ثابت.
- [ ] **Raydium CPMM:** تحقق من `AmmConfig` ورسومه وvaults وtoken programs، وطبّق مصفوفة دعم Token-2022 الفعلية.
- [ ] **Raydium CLMM:** طابق swap-step وsqrt-price/liquidity/tick math ومصفوفات ticks واتجاهها مع SDK/IDL مثبتين.
- [ ] **Orca Whirlpools:** اشتق mint ordering وtick spacing/current tick وoracle/vaults/tick arrays من الحالة الفعلية، وطابق quote وaccount metas مع SDK/program المنشور.
- [ ] **Meteora DLMM:** طبّق active bin/bin step وdynamic fees وbin arrays واتجاه X/Y من حالة `LbPair`؛ ارفض arrays غير المكتملة أو القيم الافتراضية.
- [ ] **Pump bonding curve:** طابق إصدارات تعليمات V3 وFeeConfig/tiers/creator mode والـreal/virtual reserves وcomplete/synthetic migration؛ ارفض حالة ما بعد اكتمال المنحنى إذا لم يدعمها المسار.
- [ ] **PumpSwap:** ثبّت fixtures حديثة لحقل signed `i128` بما فيها القيم السالبة، وطابق effective reserves والرسوم وcanonical pool وV2 account lists/multi-hop مع docs/IDL/SDK.
- [ ] لكل adapter: قارن integer quote والتقريب والحسابات المطلوبة مع SDK/كود البرنامج على fixtures؛ نفّذ اختبارات fork/local-validator مستقلة؛ لا تفعّل `execution_capability` قبل نجاحها.
- [ ] **Token-2022:** استخدم token program الصحيح، احسب transfer-fees الفعلية، واقبل extensions المدعومة فقط؛ ارفض transfer hook أو extension غير ممثل في quote/builder.
- [ ] انشر supported/unsupported matrix داخل `PROTOCOL_COMPATIBILITY.md`، وفعّل venue واحدًا في كل مرة بعد بوابته.

**بوابة G3:** كل مسار منفّذ له parity tests ومجموعة حسابات صحيحة ومحاكاة ناجحة؛ أي فرق غير مفسر أو إصدار مجهول يؤدي إلى الرفض.

## المرحلة 4 — quote محروس واكتشاف فرص غير مضلل

### S4.1 مسار quote موحد
- [ ] عرّف `GuardedQuote` غير قابل للتعديل يحمل route، amount، الحسابات/الحالة التي بُني عليها، الرسوم، slot/bank/generation، expiry/validity، وسبب/نتيجة التحقق.
- [ ] أنشئ canonical fingerprint يغطي كل الحسابات التابعة والمؤثرة، mint/extension/config/fee state، الاتجاه والحجم، ونسخة adapter/IDL.
- [ ] وصّل المسار من ingestion إلى decode → quote → economics/risk → builder → dispatch باستخدام نفس quote؛ امنع واجهات تنفيذ بديلة تقبل quote غير محروس.
- [ ] راجع جميع call sites بحيث `quote_route_guarded` و`revalidate_before_dispatch` يطبقان قبل البناء، وقبل التوقيع/الإرسال، على كل dispatch path.
- [ ] ألغِ quote إذا تغير fingerprint أو generation أو bank/slot/write-version، أو تجاوز عمره، أو نقصت أي بيانات؛ لا تستخدم success-shaped fallback.

### S4.2 اكتشاف وربحية المرشح
- [ ] اجعل رسم الأسواق event-driven مع تحديث incremental للحواف/المسارات المتأثرة، واحتفظ بخوارزمية اكتشاف الدورات كمولّد مرشحين لا كدليل ربح.
- [ ] افصل حسابات الترتيب التقريبية عن exact quote؛ ارفض fee denominator صفرًا أو fee config غير صالح بدل استبداله برسوم افتراضية.
- [ ] أضف حدًا أدنى صافٍ بعد كل التكاليف وهوامش مخاطر/احتمال landing، مع حدود حجم/تعرّض ومكافحة تكرار الفرصة.
- [ ] اختبر عدم الربحية، الربح الحدّي، stale state، السيولة غير الكافية، overflow، والرسوم المحسوبة مرتين.

**بوابة G4 — مانع إطلاق:** لا يوجد أي مسار حقيقي من source إلى signer/relay يستطيع تجاوز quote المحروس أو إعادة التحقق النهائي.

## المرحلة 5 — بناء المعاملة والمحاكاة والتنفيذ الآمن

### S5.1 Transaction builders
- [ ] أعد بناء كل builder من `VerifiedPoolContext` لا من `pool_id` أو ترتيب hop وحده؛ افحص كل account meta وPDA وwritable/signer وtoken program.
- [ ] تحقّق من حدود حجم transaction، compute units، address lookup tables، account locks، ترتيب التعليمات، ووجود جميع حسابات tick/bin اللازمة.
- [ ] اجمع borrow → swaps → repay داخل **معاملة Solana واحدة** عندما يكون مصدر التمويل flash borrow؛ تحقق من المبلغ المقترض والسداد والـminimum net output ذريًا.
- [ ] وثّق مسار رأس المال الذاتي كبديل منفصل إذا لم يثبت مصدر flash loan المتاح ورسومه وشروطه؛ لا تنسب دعمًا لمزوّد غير موصول/غير مثبت.
- [ ] افصل signer محدود الرصيد عن treasury/withdrawal signer؛ أزل تحويلات cold wallet من hot path، واجعل السحب إجراءً منفصلًا بموافقة وحدود.

### S5.2 المحاكاة والتواقيع والإرسال
- [ ] حاكِ نفس transaction bytes والتعليمات والحسابات والرسوم التي ستوقّع وترسل، مع سياق commitment/bank وblockhash واضحين؛ لا تعتبر `replaceRecentBlockhash` برهانًا على تطابق حالة التنفيذ.
- [ ] افشل مغلقًا عند simulation timeout/error/log mismatch أو CU margin غير كافٍ أو اختلاف quote/hash أو انتهاء صلاحية blockhash.
- [ ] طبّق دورة blockhash/last-valid-height؛ لا تعاود sign/submit لفرصة واحدة بعد timeout غامض حتى تحسم signature القديم.
- [ ] افصل base fee وpriority fee وJito tip؛ حدّث tip من بيانات/auction ملائمة، ولا ترسل tip عند عدم وجود مبرر أو schedule غير موثوق.
- [ ] افصل حالات relay accepted عن landed؛ تعامل مع Jito bundle constraints والـfallback وفق سياسة مكتوبة لا بإرسال متكرر عشوائي.
- [ ] اختبر builders والمحاكاة محليًا/fork لكل route مسموح، مع حالات فشل على كل hop؛ لا تستخدم أموال أو مفاتيح mainnet في الاختبارات.

**بوابة G5:** transaction كاملة من borrow/own capital إلى swaps والربح الصافي/السداد تمر بمحاكاة متطابقة، وكل تغيير في state يبطلها قبل الإرسال.

## المرحلة 6 — دورة حياة دائمة وعمليات ومراقبة

### S6.1 Execution state machine والمصالحة
- [ ] عرّف حالات دائمة/idempotent مثل prepared, simulated, signed, submitted, accepted, landed, failed, expired, finalized، مع signature وbundle ID وquote fingerprint وblockhash.
- [ ] عند restart، أعد مصالحة العمليات غير المحسومة مع RPC/Jito قبل أي retry؛ لا تعتبر timeout فشلًا مؤكدًا ولا ترسل trade مكررًا.
- [ ] سجّل balances قبل/بعد، رسومًا مدفوعة، token deltas، realized PnL والسبب الدقيق للفشل؛ امنع ازدواج الكتابة أو احتساب الربح مرتين.
- [ ] أضف اختبارات crash/restart وtimeouts/reorg/expired blockhash/replacement والردود المتأخرة أو المكررة.

### S6.2 Observability وKill controls
- [ ] سجّل histograms p50/p95/p99 لكل من ingest age، decode/snapshot، route scan، exact quote، risk، compile، sign، simulation، relay RTT، inclusion، finality، وصافي PnL.
- [ ] أعد كتابة `latency_bench` ليقيس pipeline حقيقيًا وreplay قابلًا للتكرار؛ لا يستخدم seed/keypair ثابتًا للتداول ولا يرسل معاملات حقيقية، ولا يعرض أرقام pipeline ثابتة كقياس.
- [ ] أضف dashboard/alerts لحالة providers والبيانات القديمة/fork gaps، اختلاف parity، المحاكاة، الرسوم، landed rate، PnL المحقق، حدود رأس المال، وأسباب kill switch.
- [ ] افصل metrics/HTTP/Telegram/DB والمهام البطيئة عن hot path؛ أضف backpressure وحدود queues ووسائل تنبيه قابلة للاختبار.
- [ ] اجعل kill switch دائمًا وفوريًا عبر كل workers/relays، مع حد يومي للخسارة، وحد أقصى للتعرّض/المعاملات والتواقيع، وإجراء إيقاف/تشغيل موثق ومختبر.
- [ ] أضف runbooks للحوادث، provider outage، استنزاف SOL للرسوم، keys rotation، restart، reconciliation، وrollback.

**بوابة G6:** restart أو outage لا ينتج عنه إرسال مكرر أو فقد حالة مالية؛ يمكن للمشغل رؤية كل محاولة وسبب قرارها وإيقاف النظام فورًا.

## المرحلة 7 — الأداء والـHFT بعد إثبات الصحة

- [ ] أزل الانتظار الثابت 300ms من hot path بعد ضمان event ordering/freshness، واستبدله بتدفق updates مع coalescing/backpressure دون إسقاط تغييرات لازمة.
- [ ] افصل ingest، quote workers، builder/signer، relay، reconciliation، وtelemetry مع snapshots immutable وتقليل lock contention.
- [ ] قس end-to-end من وصول update إلى landed/finalized على target hardware/provider؛ انشر p50/p95/p99 وtimeout/loss وCU وcost، مع حجم عينات كافٍ.
- [ ] قارن الأداء baseline/بعد التحسين على replay وfork دون التضحية بسلامة الحسابات أو زيادة stale trades؛ لا تضع ادعاء HFT إلا إذا أثبتته نتائج قابلة لإعادة الإنتاج.
- [ ] حسّن pool selection والحسابات writable والـALTs وcompute budget فقط بقياس فعلي وباختبارات parity، لا بتقليل فحوص الصحة.
- [ ] اختبر الضغط والازدحام، provider jitter، منافسة account locks، tip auctions، drop/reconnect، والـtail latency.

**بوابة G7:** أرقام latency/reliability موثقة على بيئة تشغيل محددة، والـtail latency ضمن ميزانيات متفق عليها دون تراجع في freshness أو correctness.

## المرحلة 8 — إطلاق مرحلي آمن

- [ ] **Offline/replay:** كل اختبارات الوحدة/parity/property/fuzz وCI ناجحة، ولا يوجد known critical/high blocker.
- [ ] **Local validator/fork:** جميع routes والـfailure cases ناجحة على transaction كاملة مع الأدلة.
- [ ] **Shadow mode:** بيانات حية للقراءة فقط، حساب quotes/قرارات افتراضية، مقارنة مع state on-chain وقياس فرص فائتة/خاطئة؛ لا توقيع ولا إرسال.
- [ ] **Canary مقيّد يدويًا:** لا يبدأ إلا بعد مراجعة أمنية وتشغيلية، موافقة صريحة، signer محدود، حد رأس مال صغير محدد، kill switch مجرب، ومشغل يراقب؛ التنفيذ الحي خارج نطاق هذه الخطة التلقائية.
- [ ] ارفع حدود التعرّض فقط عبر مراجعة موثقة لمعدل landing، الربح المحقق بعد كل التكاليف، الفشل، latency، وحوادث التشغيل؛ لا ترفعها اعتمادًا على backtest أو paper PnL.
- [ ] انشر `RELEASE_READINESS.md` جديدًا يذكر ما اجتاز كل بوابة، والأدلة وروابطها، والمخاطر/التغطية غير المدعومة، وصاحب قرار الإطلاق.

**بوابة G8 — Go/No-Go:** لا انتقال إلى أموال حقيقية قبل اجتياز G0–G7 ومراجعة مستقلة؛ أي venue أو extension غير مثبتة تبقى معطلة.

## المرحلة 9 — Cross-chain (اختيارية، مشروع منفصل)

- [ ] صمّمها كسوق مخزون ممول مسبقًا على كل سلسلة مع تسوية/تحوط لاحق، لا كمعاملة ذرية مشتركة ولا كجسر داخل hot path.
- [ ] اختر جسرًا/تسوية بعد تحليل الثقة والنهائية والـlatency والرسوم وحدود السحب؛ عالج timeout والرسائل المتأخرة والـreorg والـdepeg والتجميد.
- [ ] أنشئ state machine متعدد السلاسل مع idempotency ومصالحة وlimits ومحاسبة منفصلة لكل chain، ومفاتيح/سياسات خزينة مستقلة.
- [ ] اختبر الفشل والـreorg والتسوية في بيئات اختبار كل سلسلة؛ ابدأ بإشارات/محاكاة فقط، ولا تضفها إلى بوابات إطلاق Solana حتى تثبت استقلالها.

## ترتيب الاعتماديات المختصر

`G0 Scope & Truth` → `G1 Reproducible CI` → `G2 Coherent State` → `G3 Venue Parity` → `G4 Guarded Quote` → `G5 Transaction/Simulation` → `G6 Durable Operations` → `G7 Performance Evidence` → `G8 Controlled Release`

يمكن تنفيذ توحيد الوثائق وCI وبعض fixtures بالتوازي. لا يجوز تجاوز بوابة اعتمادها السابقة، ولا يعوض أداء أسرع عن نقص صحة أو أمان.
