MASTER EXECUTION PROMPT — SOLANA ATOMIC ARBITRAGE
> **الدور:** Principal Rust/Solana Engineer + Quant Execution Engineer + Security Reviewer + SRE.   > **المهمة:** استلام أحدث نسخة حقيقية من مستودع GitHub، تدقيقها مقابل سجل المشروع وقراراته، ثم إغلاق الفجوات حتى Phase 15 وفق أدلة قابلة لإعادة التشغيل. لا تعتبر أي مرحلة مكتملة لمجرد وجود ملفات أو TODO أو تعليقات.
￼
0) تعليمات حاكمة — اقرأها قبل لمس الكود
أنت تعمل على مشروع Solana Atomic Arbitrage ممول عبر Flash Loans. لا يوجد رأس مال تداول مملوك داخل الاستراتيجية؛ الرصيد التشغيلي من SOL مخصص فقط للرسوم وJito tips والحسابات وتكاليف التشغيل والخسائر التشغيلية المعتمدة. لا تفترض وجود رأس مال إضافي.
ابدأ من أحدث GitHub repository/branch/commit متاح فعليًا للمستخدم، والمستودع المعروف من سياق العمل هو:
	●	Repository: mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE
	●	Branch المتوقع: main (تحقق ولا تفترض)
	●	آخر commit ذُكر سابقًا: c42c38777b23080a67630d4ca730d5df0a7913f1 — هذا مرجع تاريخي فقط، وليس إثباتًا أنه HEAD الحالي.
	●	نقطة العمل المنطقية المعتمدة في النقاش: Phase 12.12.
	●	المطلوب النهائي: إكمال النواقص وصولًا إلى Phase 15، لكن لا تفعّل Live Trading تلقائيًا. الوصول إلى Phase 15 يعني اكتمال بنية shadow/controlled-live والاختبارات والحوكمة؛ لا يعني تصريحًا غير مشروط بالتداول الحقيقي.
قواعد المصدر والحقيقة
	1.	اتصل بـGitHub واقرأ أحدث commit وbranch وworking tree وPRs/tags إن أمكن. سجّل SHA ووقت الجلب.
	2.	افحص git status, git log, git diff, submodules، tags، CI، workflows، releases، والـbranches. لا تستبدل تغييرات المستخدم ولا تعمل force push.
	3.	افحص كل ملفات المشروع والوثائق والاختبارات والـconfig وCargo manifests وlockfiles. لا تعتمد على README وحده.
	4.	ملفات ZIP الموجودة في سياق العمل هي مراجع تاريخية/مساعدة وليست مرجعًا أحدث من GitHub تلقائيًا:
	●	solana-arb-bot.zip
	●	solana-arb-remediation-v1(1).zip
	●	solana-arbitrage-f2-2-pump-quote-supervisor-shadow-2026-11-07(2).zip لا تدمجها فوق أحدث GitHub عميانيًا. قارن الملفات والـhashes والـcommits، وانتقِ فقط التعديلات التي تثبت أنها غير موجودة في HEAD الحالي ولا تتعارض معه.
	5.	لا تدّعِ أنك قرأت GitHub أو بنيت المشروع أو شغّلت الاختبارات إلا إذا نفذت ذلك فعليًا وأرفقت المخرجات.
	6.	أي finding قديم يجب إعادة التحقق منه على HEAD الحالي. صنّفه: OPEN / PARTIAL / FIXED / NOT_REPRODUCED / BLOCKED مع دليل ومسار وسطر واختبار.
	7.	لا تستخدم افتراضات أو أرقام fees أو decimals أو PDA seeds أو account layouts من الذاكرة. استخدم مصادر البروتوكول الرسمية/SDK/IDL المنشورة المتوافقة مع deployment والـcluster المستهدف. دوّن رابط المصدر وإصداره/commit وتاريخ التحقق.
	8.	لا تستخدم Ponytail أو تعتمد عليه. لا تضف framework لمجرد التفضيل الشخصي.
	9.	لا تغيّر الاستراتيجية أو البروتوكولات أو مزودي البيانات أو بنية التشغيل إلا بمبرر موثق وقرار توافق واضح.
	10.	لا تتوقف عند كتابة تقرير. المطلوب إصلاح فعلي، اختبارات، توثيق، وملفات تسليم. إذا تعذر شيء، اتركه BLOCKED ولا تزعم إغلاقه.
مبدأ الأمان الاقتصادي
Fail closed. لا توجد صفقة «مضمونة الربح» قبل الإدراج. لا توقّع ولا ترسل مع غياب بيانات موثوقة أو quote غير متطابق أو simulation غير ناجحة أو حسابات ناقصة أو رسوم مجهولة أو تجاوز حدود المخاطر. الربح المتوقع ليس ربحًا محققًا. لا توجد ضمانة landing من RPC أو Jito.
￼
1) المطلوب منك أولًا: Baseline وRepository Forensics
أنشئ docs/audit/BASELINE.md ويحتوي:
	●	repo URL، branch، HEAD SHA، commit date، dirty/clean state.
	●	Rust toolchain، Cargo version، OS/target، Solana/Anchor/SDK versions، enabled features.
	●	قائمة crates/binaries/features والـentrypoints الفعلية.
	●	topology حقيقية: Yellowstone gRPC، WebSocket providers، RPC، account cache، quote engine، route search، risk gate، simulator، tx builder، sender/Jito، confirmation/reconciliation، DB/metrics/Telegram/dashboard.
	●	ما هو implemented ومربوط في production path وما هو موجود كملف فقط وما هو disabled/no-op.
	●	جميع env vars مع المصدر والمالك والـrequired/optional وdefault الآمن؛ لا تعرض secrets.
	●	جرد TODO/FIXME/XXX/allow(dead_code)/unwrap/expect/panic/saturating/default values/fallbacks/feature gates.
	●	جرد endpoints/providers وtimeouts/retry/backoff/health checks/reconnect behavior.
	●	جرد state machines والـstatus transitions والـpersistence.
	●	جرد test coverage: unit/property/fuzz/differential/integration/replay/shadow/fault-injection.
قبل التعديل شغّل baseline commands وسجّل النجاح والفشل:
rustc --version
cargo --version
rustup show
cargo metadata --no-deps --format-version 1
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo audit
cargo deny check
نفّذ الأوامر المتوافقة مع المشروع فقط؛ لا تفعّل كل features عشوائيًا إذا كانت تتطلب credentials أو network أو feature غير production. سجّل كل feature matrix منفصلة. شغّل Docker build/compose validation وsecret scanning وdependency scanning عند توفر الأدوات. إذا فشل build أصلح السبب أو اشرح blocker بدقة.
￼
2) سجل المراحل المعتمد — لا ترجع إلى Phase 09 كخط أساس
Phase 12.1–12.10
اعتبرها مراحل سابقة مطلوب إعادة تدقيقها لا إعادة تنفيذها عميانيًا. يجب التحقق من أن تغطيتها الفعلية تشمل:
	●	slot/state integrity، fork awareness، processed-slot rollback وSLOT_DEAD invalidation.
	●	RPC/stream consistency، provider health، reconnect/gap handling، generation/versioning.
	●	account freshness وrequired-account completeness وwrite-version/slot provenance.
	●	quote readiness وعدم تمرير snapshot غير مكتمل أو stale.
	●	execution lifecycle وdispatch guards وعدم الخلط بين discovery وexecution.
	●	bank/state coherence ومنع خلط حسابات من forks أو slots غير متوافقة.
Phase 12.11–12.12 — يجب الحفاظ على ما أضيف وعدم كسره
المتوقع من هذه النقطة حسب سجل العمل:
	●	Protocol dependency manifests.
	●	تصحيح حدود/اختلاف Yellowstone مقابل HTTP/RPC limits.
	●	Validated snapshot decoding.
	●	GuardedRouteQuote.
	●	slot/protocol/version checks.
	●	revalidate_before_dispatch.
تحقق من التنفيذ الفعلي والـcall graph. لا تعتبر وجود النوع أو الدالة دليلًا على فرضها في كل executor. لا تحذف أو تضعف هذه الحواجز أثناء refactor.
Phase 12.13 — Decoder parity (إلزامي)
	●	أكمل decoders لكل البروتوكولات المطلوبة، مع parity موثقة مع official SDK/IDL/on-chain layout.
	●	راجع account discriminator، owner program، PDA derivation/seeds/bump، version، lengths، endian، alignment، optional fields، token program/Token-2022، mint decimals، vaults، fee config، authority، status flags.
	●	لا تستخدم decoder عام أو heuristic كمرجع تنفيذي.
	●	لكل decoder: golden fixtures، malformed/truncated/oversized inputs، wrong owner/discriminator، unknown version، missing dependency، stale slot، wrong fork.
	●	Differential tests مقابل official SDK أو fixtures مستخرجة من chain عند نفس version. وثّق أي اختلاف rounding.
	●	كل decoded account يحمل provenance: pubkey، owner، slot، write_version إن توفر، commitment/fork generation، source provider، decode version، observed_at.
	●	أي account غير مفهوم أو غير مكتمل يرفض promotion ولا ينتج quote قابل للتنفيذ.
Phase 12.14 — Executor enforcement (إلزامي)
	●	اجعل GuardedRouteQuote أو contract مكافئًا هو المدخل الوحيد لكل مسار transaction builder/executor.
	●	لا تسمح ببناء أو dispatch عبر legacy/alternate path يتجاوز quote guard.
	●	اربط hash موحدًا للـroute + quote + snapshot versions + fee assumptions + min-out + borrow amount. يجب أن يتحقق RiskGate وbuilder وpre-dispatch من نفس hash.
	●	أعد التحقق مباشرة قبل dispatch من slot freshness، fork generation، pool state versions، mint metadata، fee config، dependency completeness، quote expiry، blockhash validity، balance/ATA readiness.
	●	إذا تغير أي dependency أو quote hash، أعد quote/risk/simulation؛ وإلا ارفض.
	●	simulation Disabled/NotRun/Failed/Timeout/Unknown = رفض، لا fallback ولا نجاح افتراضي.
	●	اختبر جميع executors والـbinaries والـfeature-gated paths، لا المسار الرئيسي فقط.
Phase 12.15–12.16 — E2E / Simulation / Acceptance
	●	اختبارات من account snapshot إلى decoder إلى quote إلى route sizing إلى economics إلى risk إلى transaction build إلى simulation إلى dispatch mock إلى status reconciliation.
	●	replay على snapshots حقيقية محفوظة، مع forks/reorgs/gaps/out-of-order/duplicate updates.
	●	fault injection: provider down، lag spike، reconnect storm، queue overflow، RPC timeout، stale blockhash، account missing، simulation error، Jito timeout، dropped bundle، unknown status.
	●	لا تستخدم fake execution أو mocked success على أنه دليل نجاح حقيقي. mocks مسموحة لاختبار state transitions فقط ويجب وسمها بوضوح.
￼
3) Phase 13 — Quote, economics, fees, optimal borrow sizing
13.1 Canonical units and token accounting
	●	عرّف أنواعًا صريحة غير قابلة للخلط مثل Lamports, TokenAmount<Mint>, UsdValue, Bps, Slot, ComputeUnits؛ لا تستخدم u64 عام لكل شيء بلا type boundary.
	●	كل amount يحمل mint/decimals/unit. لا تخزن raw token units داخل Lamports.
	●	امنع التحويل الضمني بين WSOL وlamports وبين USDC raw وUSD.
	●	لا تعتمد decimals=9 ثابتة لأي token؛ استخرجها من mint account موثوق ومحدّث.
	●	استخدم checked arithmetic. لا unwrap_or(0), unwrap_or(MAX), saturating arithmetic أو default values في economic/quote paths. Overflow/underflow/zero denominator = typed error ورفض.
	●	وحّد rounding بحسب protocol: floor/ceil/input-side/output-side، وبيّن موضعه واختبره.
13.2 Exact protocol quotes
طبّق exact-in/exact-out حسب ما يتطلبه كل protocol، مع state traversal الحقيقي لا approximations:
	●	Raydium CPMM / AMM حسب program/version الفعلي.
	●	Raydium CLMM: tick arrays، sqrt price، swap steps، crossing، liquidity net/gross، direction، fee rounding.
	●	Orca Whirlpools: tick arrays، adaptive fees إن انطبقت، protocol fee، transfer fees، token extensions.
	●	Meteora DLMM: bin arrays، active bin، volatility/reference state، fee parameters، bin traversal، exact integer math.
	●	Pump.fun bonding curve: Global + FeeConfig PDA بالـseeds الصحيحة بما فيها seed الإضافي إن كان مطلوبًا، BondingCurve state، fee recipient، creator fee، token/mint metadata، graduation state.
	●	PumpSwap: Pool/vault/mint/GlobalConfig/FeeConfig coherence، canonical PDA/bump وcanonical vault ATAs، fee direction flags، Mayhem/Holder Rewards، custom-pair creator-fee override، fee recipient/sweep accounting.
	●	Jupiter: لا تعتبر quote API وحده ضمانًا لحالة on-chain أو قابلية تنفيذ؛ تحقّق من route instructions/accounts/expiry/slippage/fees، ولا تسمح بتعارض quote authority مع native adapters.
لكل protocol وثّق: program ID، deployed version، SDK/IDL commit، account dependencies، formula، fee order، rounding، transfer-fee treatment، min-out، tests.
13.3 Pump.fun → PumpSwap migration and cross-venue routing
	●	لا تفترض أن Pump.fun وPumpSwap نفس المنحنى أو نفس الرسوم أو نفس الحسابات.
	●	لا تستخدم static fee fallback مثل 125 bps أو 5 bps أو أي رقم ثابت دون مصدر موثوق مرتبط بالـslot/version.
	●	لا تعمل synthetic migration-straddle quote إلا إذا تم إثبات transition state والحسابات والـfees والـliquidity على جانبي migration، مع model موثق واختبارات differential.
	●	لا تعتبر DexScreener USD liquidity أو Bitquery/GMGN price discovery أو بيانات off-chain reserves أساسًا executable quote. يمكن استعمالها discovery فقط.
	●	لا تنفذ route لمجرد وجود سعرين مختلفين؛ احسب depth الفعلي والـfees والـtransfer fees والـprice impact والـaccount locks وCU والـpriority fee وJito tip والـflash fee والـrent/ATA costs عند الحاجة.
13.4 Split routing and multi-leg correctness
أعد فحص findings القديمة التالية على HEAD الحالي:
	●	L-01: تقسيم price impact على عدد legs heuristic غير صحيح؛ احسب aggregate impact من exact re-quote.
	●	L-02: لا تنسخ mint_in/mint_out/fee_bps من first hop إلى كل legs؛ metadata والرسوم لكل pool مستقل.
	●	L-03: economics gate يجب أن يستعمل نفس split route والـquote الذي سيُبنى منه التنفيذ.
	●	L-04: لا تعامل CLMM/DLMM كأنها CPMM في split allocation.
	●	L-05: لا fallback إلى stale pools عند قلة pools fresh.
	●	L-06: لا تستخدم مضاعفًا اعتباطيًا لعمر graph؛ عرّف freshness policy مستندة إلى slot/latency والـprotocol.
	●	أعد quote للـreturn hop باستخدام مجموع intermediate outputs الحقيقي وحالة pool الفعلية، لا haircut تقريبي.
	●	امنع duplicate pools، cyclic route غير مغلق، intermediate mint mismatch، أو استخدام pool نفسه بما يسبب تضارب state/account locks.
	●	احسب slippage per hop/protocol، ولا تستبدل إعداد 10 bps بقيمة 50 bps في split path دون policy صريحة.
	●	أزل التباين بين net_before_tip وrisk gate (مثل اختلاف 10k مقابل 5k lamports) عبر canonical cost ledger واحد.
13.5 EconomicQuote — single source of truth
استخدم canonical EconomicQuote أو بنية مكافئة واحدة مشتركة بين discovery/route ranking/risk/builder/reconciliation:
	●	gross output، input/borrow principal، flash fee، DEX fees، Token-2022 transfer fees، slippage/min-out، network/base fee، priority fee، Jito tip، ATA/rent/cleanup costs، other verified costs.
	●	لا تخصم أي cost مرتين ولا تسقط أي cost.
	●	افصل estimated، conservative bound، simulated، realized.
	●	net_profit_lamports يجب أن يكون وحدة صحيحة؛ إذا كان base mint غير WSOL، لا تحوّل raw token amount إلى lamports دون سعر تحويل موثق.
	●	حسم contract الخاص بـmin_profit_raw: هل هو base mint raw أم lamports؟ أصلح API والـcallers والـdocs معًا. لا تنفذ one-line fix مثل حذف conversion قبل حسم units.
	●	عالج M-01 double conversion، FIN-02 ad-hoc economics، FIN-04 hardcoded decimals، FIN-05 default fee عند denominator صفر، FIN-06 raw units stored as Lamports، وTX-02 inconsistent fee subtraction.
	●	fee denominator صفر أو missing fee data = رفض quote، لا default 25 bps.
	●	Kamino fee لا يقرأ من hardcoded 5 bps. اقرأ flashLoanFeeSf/البيانات الصحيحة من reserve snapshot المتحقق، وطبّق rounding الدقيق واتجاهه.
	●	راجع كل public builder والـlegacy wrapper؛ وجود verified fee في supervisor لا يكفي إذا كان مسار آخر يمرر fee_sf: 0.
13.6 Optimal borrow / trade sizing
	●	ابحث عن أفضل حجم اقتراض/تنفيذ على أساس exact discrete quotes وليس continuous approximation فقط.
	●	الهدف: maximize conservative net profit بعد كل التكاليف، مع قيود liquidity، slippage، CU، account locks، borrow cap، min/max trade، lender semantics، risk budgets.
	●	لا تستخدم حجمًا أمثل محسوبًا على سعر marginal أو reserve estimates فقط.
	●	استخدم search مناسبًا لطبيعة الدالة (discrete/non-smooth CLMM/DLMM/bin crossing)، مع bounds وtermination guarantees.
	●	قارن optimizer بنتائج brute force على أحجام صغيرة وadversarial fixtures. أبلغ optimum وrunner-up وحساسية الربح للحجم.
	●	إن لم يكن optimum مستقرًا أمام state/fee uncertainty، قلّص الحجم أو ارفض.
13.7 Tip and fee strategy
	●	لا تستخدم tip كـ50%/55% من الربح بشكل أعمى.
	●	احسب break-even bid، opportunity value، CU price، base/priority fees، Jito tip floor/percentile إن كانت بيانات موثوقة، expected landing probability بحذر، وتكلفة الفشل.
	●	لا تخلط priority fee مع Jito tip؛ لكل منهما وجهة وآلية محاسبة.
	●	حد أقصى للـtip والـpriority fee وdaily budget وper-slot budget؛ enforce قبل build وقبل send.
	●	إذا بيانات auction/tip غير متاحة أو stale، استخدم policy محافظة محددة أو لا ترسل؛ لا تختلق floor.
￼
4) Phase 14 — Atomic Flash Loan + Transaction Construction + Dispatch
14.1 Atomicity and lender semantics
	●	Solana ليست EVM: لا تفترض وجود EVM-style flash loan أو أن Jito bundle يساوي flash loan.
	●	تحقق من Kamino أو أي lender مستهدف من المصدر الرسمي والبرنامج المنشور: instruction sequence، reserve state، borrow/repay accounts، fee calculation، token program، CPI constraints، same-transaction repayment requirement، borrow limits، reserve liquidity.
	●	إذا lender يشترط borrow وrepay في transaction واحدة، اجمع borrow → swaps → repay → on-chain profit guard في نفس Solana transaction.
	●	Jito bundle transport ليس بديلًا عن atomicity داخل transaction. Bundle قد يضم عدة transactions لكنه لا يغير semantics الخاصة بالقرض.
	●	لا تدعم أكثر من flash loan أو lender إلا بعد إثبات compatibility، account constraints، order، repayment، aggregate fees، CU، locks، وfailure atomicity. ارفض composition غير مثبت.
14.2 Instruction builder correctness
	●	كل builder يستخدم protocol-specific verified accounts/instructions، لا templates ثابتة أو hardcoded account indexes.
	●	تحقق من account metas (signer/writable/readonly)، program IDs، PDA derivation، token program IDs، ATAs، associated token program، system program، sysvars، remaining accounts، instruction discriminators/serialization.
	●	لا تستخدم ALT helpers غير مثبتة أو disabled؛ إذا ALT مستخدم، تحقق من authority/deactivation/last_extended_slot/index bounds وaddress resolution وlookup table freshness.
	●	لا تخلط legacy/v0 message semantics.
	●	تأكد من compute budget instructions وعدم تكرار/تعارض CU limit أو CU price.
	●	لا تتجاوز transaction size/packet limits وaccount lock limits وCU budget. احسبها قبل التوقيع.
	●	لا تستخدم unwrap في transaction construction. أي missing account أو serialization error يوقف العملية.
14.3 Min-out and on-chain profit protection
	●	احسب min-out لكل hop بناء على quote exact ومنهج protocol الصحيح.
	●	لا يكفي min-out للـroute النهائي إذا كانت هناك repayment obligations أو transfer fees.
	●	أضف/تحقق من on-chain guard يضمن repayment الكامل وتحقيق الحد الأدنى للربح المطلوب، بحسب architecture والـprograms الفعلية. إذا لا يوجد guard on-chain قابل للتحقق، لا تدّعِ حماية ذرية للربح.
	●	الشرط النهائي يجب أن يغطي: principal + flash fee + all mandatory costs + min profit policy، مع وحدات صحيحة.
	●	تأكد أن repayment token/mint والـamount صحيحان تمامًا، ولا تعتمد على رصيد قديم أو expected balance غير مثبت.
14.4 Simulation and preflight
	●	simulation against coherent recent bank/account state قبل الإرسال.
	●	Disabled, NotRun, Failed, Timeout, Inconclusive كلها رفض.
	●	لا تخلط local LiteSVM no-op مع نجاح. إذا feature litesvm disabled، caller يجب أن يرفض التنفيذ بوضوح.
	●	اختبر RPC simulation والـlocal simulation parity وحدود الاختلاف. لا تجعل simulation نجاحًا مضمونًا للـlanding.
	●	revalidate quote hash وslot/fork/account versions والـfees والـblockhash قبل signing/dispatch.
	●	تحقق من wallet/fee-payer balance كافٍ للـnetwork fees/tip/rent فقط؛ لا تعتبره trade capital.
14.5 Jito / SWQoS / sender lifecycle
	●	ميّز بدقة بين bundle_id وtransaction signature. Bundle ID ليس signature ولا landing proof.
	●	صمّم durable state machine على الأقل: PLANNED → BUILT → SIMULATED → SIGNED → SUBMITTED → ACKNOWLEDGED → PENDING → LANDED/CONFIRMED/FINALIZED أو REJECTED/FAILED/DROPPED/EXPIRED/UNKNOWN حسب confirmations المتاحة.
	●	تعامل مع ambiguous timeout: قد تكون الصفقة أرسلت بالفعل. لا تعاود الإرسال عشوائيًا؛ reconcile باستخدام signatures/idempotency.
	●	خزّن bundle ID، tx signatures، blockhash، lastValidBlockHeight، send provider، timestamps، attempt count، status observations، errors، slot/commitment، transaction metadata.
	●	poll bundle status وsignature status وtransaction metadata بصورة مستقلة، ثم reconcile نهائيًا مع finalized transaction.
	●	retry policy لا تعيد إرسال صفقة قد تكون landed، وتمنع duplicate execution.
	●	failover بين providers يجب أن يكون health-scored مع resubscribe/refresh، لا مجرد تبديل URL.
￼
5) Phase 15 — Shadow → Controlled Live Readiness (دون تفعيل تلقائي)
15.1 Shadow mode
	●	استقبل بيانات حقيقية، فكّها، أنشئ quotes/routes، سجّل القرار، واحسب simulated outcomes دون توقيع أو إرسال معاملات.
	●	لا تسجل EXECUTED أو LANDED لأي simulation أو mock. استخدم statuses واضحة: SHADOW_CANDIDATE, SIMULATED, WOULD_DISPATCH.
	●	سجّل أسباب الرفض بدقة وعدادات لكل gate.
	●	اجمع predicted vs realized market movement حيث يمكن التحقق؛ لا تنسب simulated profit إلى PnL فعلي.
15.2 Controlled-live gates
اجعل كل Gate صريحًا، persistent، fail-closed، وموقعًا/مصرحًا من operator. لا تسمح config أو env وحدهما بتجاوز KillSwitch أو approval.
قبل السماح بأي live path، يجب إثبات:
	●	Security: secrets rotated، no secrets in repo/history/images/logs/artifacts.
	●	Data integrity: fork-aware state، no stale/missing/mixed-slot dependencies، gap reconciliation.
	●	Protocol correctness: decoder/quote differential tests لكل protocol المستعمل.
	●	Economics: canonical quote and exact all-in costs، no unknown fees.
	●	Execution: same quote hash، correct borrow/swap/repay/min-profit guard، transaction limits.
	●	Simulation: successful and recent simulation; disabled/timeout reject.
	●	Reliability: sender lifecycle + confirmation reconciliation + durable journal.
	●	Risk: persisted kill switch، operator auth، per-slot/per-day loss/tip/attempt budgets، max CU، max exposure/borrow، cooldown، circuit breakers.
	●	Monitoring: metrics, alerts, dashboard, audit log, restart recovery.
	●	Acceptance: reproducible CI and signed release checklist.
حتى بعد نجاح الاختبارات، اترك LIVE_EXECUTION_ENABLED=false وKillSwitch في وضع الإيقاف افتراضيًا. لا تشغل live أو توقع/ترسل transaction إلا بتفويض صريح منفصل من المستخدم بعد عرض gates ونتائج الاختبارات.
15.3 Persistent risk controls
	●	KillSwitch persisted in DB/durable store and survives restart; fail-safe on storage failure.
	●	Operator authentication/authorization؛ لا تعتمد على مجرد وجود Telegram command.
	●	Daily loss budget، daily tip budget، per-slot tip budget، max attempts/slot، max retries، max borrow، min wallet SOL، max CU، max stale age، max consecutive failures.
	●	Define UTC/day-boundary semantics and local display timezone explicitly.
	●	Rate limits and bounded queues; protect against memory growth/cooldown map growth.
	●	Manual emergency stop and restart procedure; require explicit re-arming after critical fault.
	●	Test crash/restart between every execution state transition.
￼
6) Findings قديمة يجب إعادة اختبارها وإغلاقها
لا تعتبر الأرقام أو المواضع أدناه مؤكدة على HEAD الجديد؛ استخدمها كقائمة بحث. أعد اكتشافها وتحديث line references.
Security / secrets / fake execution
	●	SEC-01: credential ظاهر في config.toml في snapshot تاريخي؛ تعامل معه كأنه مكشوف إذا كان credential حقيقيًا. ألغِه ودوّره من provider، نظّف tracked files والتاريخ وفق سياسة repo، افحص logs/images/CI/artifacts. لا تضع قيمة حقيقية في .env.example.
	●	webhook auth token ثابت في .env.example في snapshot قديم؛ استبدله placeholder واضحًا، وارفض placeholder/default في production.
	●	LIVE-01/LIVE-02: تحقق من عدم وجود fabricated execution/landing status أو canary logs توحي بنجاح لم يحدث.
	●	SEC-09: رسائل error لا تحتوي أرقام profit floor hardcoded مضللة.
	●	لا تسجل API keys، private keys، seed phrases، signed transactions الحساسة، أو authorization headers.
	●	لا تحفظ private keys داخل config أو source أو Docker layer. استخدم secret manager أو env injection مناسبًا، مع عدم طباعة القيم.
Logic / math / routing
	●	L-01 إلى L-07: split impact، metadata per leg، split economics، CLMM/DLMM allocation، stale pool fallback، arbitrary graph age، cooldown map growth.
	●	FIN-01: Kamino flash fee hardcoded/legacy path أو fee_sf: 0; افحص جميع builders والـbinaries.
	●	FIN-02: ad-hoc economics بدل EconomicQuote.
	●	FIN-03: tip fraction ثابت من الربح.
	●	FIN-04: decimals hardcoded.
	●	FIN-05: default 25 bps عند denominator صفر.
	●	FIN-06: raw token units في Lamports.
	●	TX-01: split slippage لا يطابق config.
	●	TX-02: اختلاف fee subtraction بين net profit وrisk gate.
	●	CPMM-01: saturation قد ينتج 0/MAX ويخلق phantom liquidity.
	●	DATA-01: DexScreener USD-liquidity-derived reserves وdecimals=0 لا تستخدم كـexecution state.
	●	M-01: min_profit_raw double conversion/unit contract inconsistency.
	●	M-02: marginal ceiling guard unreachable.
	●	M-03/M-04: fee truncation/rounding.
	●	M-06: CLMM tick stall/no progress.
	●	M-07: DLMM phantom prices/fallback arithmetic.
	●	راجع كل findings أخرى في docs/reports/FINDINGS.md وgm.md/تقارير التدقيق إن كانت موجودة، وأعد ترقيم القائمة دون إسقاط أي finding.
Dead code / wiring — لا تحذف عشوائيًا
	●	راجع arb-math::chronos (كان 865 LOC بلا production callers) مقابل arb-risk::ChronosEvInput المستخدم. لا توصل module إضافيًا للـhot path دون benchmark/validation؛ لا تحذفه قبل call-graph proof.
	●	LiteSVM feature disabled/no-op: لا تسمح باعتبار no-op نجاحًا. فعّلها فقط إذا toolchain/dependencies/fixtures صالحة، وإلا fail closed.
	●	arb-dex decoders مقابل decoders المكررة في arb-ingest::registry: عيّن canonical owner واحدًا وامنع drift.
	●	arb-metrics helpers الميتة: إما wiring صحيح أو حذف موثق؛ راجع cardinality.
	●	arb-mcp standalone: احتفظ به أو وثّق سبب عدم استخدامه؛ لا تحذفه لمجرد أنه غير spawned.
	●	recent_landed_cu غير متغير، http_client غير مستخدم، route_worker no-op، verified_config::is_fresh، unused public methods/config fields: صنّف كل عنصر KEEP/WIRE/REMOVE/ARCHIVE بدليل.
	●	راجع canary, GrpcConfig, JupiterConfig, AdminConfig.token_env, RiskConfig.max_attempts_per_slot, WalletConfig.min_sol_balance_lamports, JitoConfig.tip_floor_percentile: إما اربطها فعليًا بالـruntime أو أزلها/وثّقها؛ لا تترك config مضللًا.
	●	صحّح تضارب تعداد dead-code: تقرير قديم يقول D-01…D-24 بينما جدوله يصل D-28. أعد العد من findings الفعلية.
￼
7) Data pipeline, providers, StateIntegrity
التصميم المطلوب منطقيًا:
Yellowstone gRPC + configured WS feeds
  -> provider health / slot & fork tracker
  -> account cache with provenance and freshness
  -> protocol dependency planner + canonical decoders
  -> exact protocol quote adapters
  -> candidate graph (discovery only)
  -> exact route sizing + unified signed economics
  -> risk gate + operator policy + persistent kill switch
  -> transaction planner (borrow + swaps + repay + profit guard)
  -> simulation / CU & fee estimator
  -> Jito bundle or explicitly configured sender
  -> bundle + signature status tracker
  -> finalized metadata + PnL reconciliation
  -> metrics / dashboard / Telegram / durable audit journal
Provider rules
	●	المستخدم سبق أن اعتمد VPS الحالي وحساب Solami/API key الحالي بميزانية بيانات إضافية منخفضة؛ لا تنشئ local Agave Validator أو full RPC node.
	●	استخدم Yellowstone gRPC كمصدر streaming الأساسي إن كان هو configured source، مع WebSocket/RPC كقنوات fallback/verification حسب السياسة. لا تفترض أن fallback فعّال قبل اختبار failover.
	●	لا تستخدم local full validator أو full RPC node.
	●	لا تستخدم stale cache أو synthetic fallback عند provider failure.
	●	لكل provider: health score، slot lag، reconnect count، message rate، subscription state، last successful update، endpoint identity.
	●	عند reconnect: اكتشف فجوة البيانات، invalidate affected cache، resubscribe، resnapshot dependencies، ثم أعد promotion فقط بعد coherence.
	●	duplicate/out-of-order writes لا تفسد state؛ استخدم slot + write_version + fork/generation ordering المناسب.
	●	SLOT_DEAD أو rollback يبطل state المتأثرة والـquotes المشتقة والـpending plans.
	●	لا تخلط processed fork state مع confirmed/finalized state دون policy صريحة.
	●	لا تستخدم Yellowstone account stream وHTTP RPC كأن limits/provenance/semantics متطابقة؛ احترم حدود كل provider.
	●	كل queues bounded مع backpressure؛ لا memory leak تحت burst/reconnect.
StateIntegrity / StateGuard acceptance
	●	duplicate updates، out-of-order updates، missing required account، stale account، mismatched slot، mismatched fork، write-version regression، reconnect gap، slot dead، provider disagreement.
	●	dependency snapshot immutable/versioned؛ quote references exact snapshot identity.
	●	invalidation cascades إلى routes/quotes/simulations/plans التابعة.
	●	لا تعتمد على تشغيل Validator محلي لإثبات fork correctness؛ المطلوب in-process integrity tracker متوافق مع مزود البيانات الحالي.
￼
8) Protocol dependency manifests and readiness
لكل route leg، أنشئ dependency manifest يحدد كل الحسابات المطلوبة بدقة، ومنها عند الاقتضاء:
	●	pool/state account
	●	vault A/B
	●	mint A/B + token program
	●	fee config/global config
	●	tick arrays/bin arrays
	●	oracle/observation accounts
	●	authority/config/creator/fee recipient
	●	lender reserve/market/obligation accounts
	●	ATA/token accounts اللازمة للـpayer/borrower
قواعد:
	1.	manifest مرتبط بالـprotocol + version + pool type + direction + route.
	2.	required vs optional accounts محددة؛ missing required = not ready.
	3.	لا تستخدم account من cache لمجرد أن pubkey موجود؛ تحقق من owner/layout/slot/fork/freshness.
	4.	manifest يتحقق قبل quote، قبل build، وقبل dispatch.
	5.	account fetch fallback لا يغيّر provenance بصمت.
	6.	account limit/transaction size/CU implications تحسب قبل اختيار route.
￼
9) Dashboard / observability / operator UX
سلّم Dashboard حقيقية متصلة بالـruntime، لا واجهة mock فقط. يجب أن تعرض على الأقل:
	●	Mode: SHADOW / PAPER / CONTROLLED_LIVE / DISABLED، مع بيان أن live disabled افتراضيًا.
	●	KillSwitch state وسبب آخر إيقاف ومن قام بالتغيير ووقت التغيير.
	●	Provider health: Yellowstone/WS/RPC status، slot lag، reconnects، gaps، freshness.
	●	Account cache: valid/stale/missing/fork-invalid counts.
	●	Protocol readiness لكل venue: decoder version، quote status، dependencies.
	●	Candidates: route، pools، mints، input size، outputs، gross/net breakdown، quote age، snapshot slot/hash، rejection reason.
	●	Risk gates: pass/fail لكل شرط، لا مجرد status أخضر إجمالي.
	●	Simulation: status، logs/errors، CU consumed، units، snapshot identity.
	●	Execution lifecycle: plan ID، bundle ID، signatures، states، retries، confirmation/finality، transaction metadata.
	●	PnL: estimated vs simulated vs realized، ولا تسمِّ الأولين realized.
	●	Costs: base fee، priority fee، Jito tip، flash fee، DEX fees، transfer fees، rent/ATA.
	●	Daily budgets: loss/tips/attempts، current/remaining.
	●	Latency histogram p50/p95/p99 لكل مرحلة.
	●	alerts وaudit log وoperator actions.
أمان Dashboard:
	●	Admin authentication/authorization حقيقية، لا token غير مستخدم أو secret في frontend.
	●	لا تتيح dashboard تجاوز KillSwitch أو approval gates دون policy مصرح بها.
	●	لا تكشف keys أو internal endpoints أو signed payloads.
	●	كل action حساس مسجل audit trail.
￼
10) Configuration and .env.example
	●	سلّم .env.example كاملًا ومحدثًا، placeholders فقط، دون أي credentials حقيقية أو token يبدو قابلًا للاستخدام.
	●	لكل متغير: اسم، وصف، required/optional، default آمن، validation، secret/non-secret، المكونات التي تقرأه.
	●	لا تضع secret في config.toml أو Dockerfile أو compose أو CI.
	●	افشل startup عند required secret مفقود أو placeholder في وضع يتطلبه.
	●	LIVE_EXECUTION_ENABLED=false افتراضيًا ولا يمكنه تجاوز KillSwitch أو approval.
	●	افصل config للـdev/test/shadow/controlled-live مع منع خلطها.
	●	تحقق من URL schemes وTLS، timeouts، numeric bounds، URLs الفارغة، duplicate providers، invalid program IDs.
	●	لا تطبع env values في logs؛ اطبع اسم المتغير وحالة توفره فقط.
	●	راجع Docker secrets، mounts، user permissions، read-only filesystem حيث ممكن، healthcheck، graceful shutdown، resource limits.
￼
11) Code quality and architecture rules
	●	Rust stable/toolchain المحدد في rust-toolchain.toml، idiomatic safe Rust أولًا؛ unsafe فقط مع justification، invariants، tests، audit comment.
	●	لا تستخدم include! لتقسيم ملفات ضخمة.
	●	الملفات الإنتاجية التي يزيد طولها على 1,000 سطر يجب تفكيكها حسب المسؤولية، مع الحفاظ على public API عبر pub use facade. لا تقسم حسب عدد أسطر عشوائيًا.
	●	التقسيم المتوقع تاريخيًا إذا ظل مطابقًا للـHEAD:
	●	arb-tx/src/lib.rs (كان ~3,372 LOC): compute budget، Jito tip، token accounts، Kamino instructions، protocol builders، route dispatch، builder.
	●	arb-ingest/src/registry.rs (كان ~1,419): pool decode، mint metadata، account fetch، fee decode، CLMM/DLMM decode.
	●	arb-engine/src/lib.rs (كان ~1,342): graph، quote، cycle search، sizing، opportunity.
	●	arb-types/src/lib.rs (كان ~1,264): amounts، IDs، protocol، pool، route، execution، statistics، errors.
	●	arb-math/src/dlmm.rs (~1,115) وclmm.rs (~1,095): فصل state/math/quote/traversal. هذه أرقام قديمة؛ أعد القياس على HEAD الحالي.
	●	لا تكرر quote/fee/decoder/transaction logic في crates متعددة. Owner واحد لكل مفهوم.
	●	pure math/decoding لا يحتوي network I/O؛ ingest يملك feeds؛ execution يملك signing/sending؛ reconciliation يملك post-submit truth.
	●	استخدم typed context structs بدل positional args الطويلة.
	●	لا تكتم الأخطاء بـlet _ =, broad unwrap_or_default, silent fallback أو log-and-continue في critical path.
	●	error types محددة وقابلة للتصنيف، مع context آمن.
	●	لا تحذف dead code إلا بعد call-graph/feature reference/test proof.
	●	لا تخلط production code مع probes/diagnostic binaries؛ صرّح عن bins التي تدخل release.
	●	حافظ على backward compatibility أو وثّق migration.
￼
12) Tests — الحد الأدنى المطلوب
Math / accounting
	●	zero، one، max integer، overflow/underflow، denominator zero، decimals 0/6/9/غير قياسية.
	●	property tests: monotonicity حيث تنطبق، conservation، bounds، no negative/phantom liquidity، deterministic rounding.
	●	differential fixtures ضد official SDK/IDL لكل protocol version.
	●	token-2022 transfer fee / withheld amounts / extensions إن كانت مدعومة؛ وإلا رفض صريح.
Routing
	●	two pools with different fees/mints/decimals.
	●	split route uneven allocation.
	●	aggregate output changes return-hop price.
	●	stale leg among fresh legs.
	●	duplicate pool.
	●	intermediate mint mismatch/non-closed cycle.
	●	CLMM tick boundary/no progress.
	●	DLMM bin traversal and missing bin array.
	●	Pump.fun migration boundary before/after/ambiguous.
Data integrity
	●	duplicate/out-of-order account update.
	●	slot rollback/SLOT_DEAD.
	●	provider reconnect gap and resnapshot.
	●	required dependency missing.
	●	account owner/discriminator/version mismatch.
	●	same account pubkey but incompatible slot/fork.
Execution
	●	simulation disabled/timeout/failure/success.
	●	missing Kamino fee snapshot.
	●	quote hash differs between risk and builder.
	●	quote stale before dispatch.
	●	min-out below repayment requirement.
	●	insufficient fee payer SOL.
	●	packet/compute/account limit exceeded.
	●	landed/failed/dropped/expired/unknown confirmation.
	●	ambiguous send timeout without duplicate execution.
	●	crash/restart during every state transition.
Security / operations
	●	secret scanner on repo/history/image/CI/artifacts.
	●	webhook auth rejects placeholder/default.
	●	live mode cannot override KillSwitch/missing approval.
	●	bounded queues under burst/reconnect.
	●	metrics cardinality and cooldown map memory bound.
	●	authorization tests for every admin/Telegram/dashboard action.
￼
13) Performance, reliability and profitability evidence
Performance
استخدم نفس captured snapshots ونفس candidate set للمقارنة. قِس p50/p95/p99 وCPU/allocations/queue depth لكل مرحلة:
	●	ingest/decode
	●	dependency validation
	●	quote per protocol
	●	route search
	●	optimal sizing
	●	split re-quote
	●	economics/risk
	●	simulation
	●	transaction build/serialize
	●	send acknowledgement
	●	landing/finality
لا تعتمد على average latency فقط. سجّل stale rejection rate وprovider lag وdrop rate وreconnect recovery وlanded success rate. لا تحسّن latency على حساب correctness.
Reliability
	●	soak test، restart test، provider failover، DB outage، disk full، clock skew، RPC rate limits، Jito outage.
	●	لا تفقد execution journal أو kill switch عند restart.
	●	لا يحدث duplicate trade بسبب retries.
	●	graceful shutdown يوقف dispatch ويترك reconciliation مستمرًا أو يستأنف بأمان.
Profitability
	●	Out-of-sample replay ثم shadow/canary observations.
	●	افصل gross estimate، all-in predicted net، simulated result، actual balance-delta PnL.
	●	اعتمد transaction metadata/balance deltas بعد finality، لا logs أو quote output.
	●	أظهر landed rate، failed/reverted/dropped/expired rates، fees/tips، realized PnL distribution، confidence intervals، sample size، regime/time breakdown.
	●	لا تعتبر backtest أو simulation أو quote موجبًا دليل ربح مستقبلي.
	●	لا تعلن profitability ما لم تكن positive net realized results مثبتة على عينة كافية وبعد كل التكاليف، مع عدم ضمان استمرارها.
￼
14) CI/CD, release and packaging
	●	أنشئ/أصلح CI لتشغيل fmt، check، tests، clippy، audit/deny، secret scan، config validation، Docker build، integration tests المتاحة.
	●	pin dependencies/toolchain قدر الإمكان، راجع advisories وlicense policy.
	●	لا تنشر secret أو artifact يحتوي secret.
	●	release manifest يسجل commit SHA، toolchain، features، test report، SBOM إن أمكن، config schema version.
	●	Docker image non-root، healthcheck، graceful shutdown، minimal runtime image، لا build secrets في layers.
	●	لا ترفع commit/PR دون فحص diff وtests. لا force push ولا rewrite history إلا بتوجيه صريح.
	●	إن كانت لديك صلاحية PR، أنشئ branch واضحًا وPR مع summary/findings/tests/known blockers؛ لا تدمج تلقائيًا.
￼
15) Acceptance gates — لا تعلن اكتمال Phase 15 إلا إذا تحققت
Gate A — Repository integrity
	￼	أحدث GitHub SHA موثق؛ التعديلات لا تمحو user changes.
	￼	لا توجد ملفات قديمة مدمجة فوق HEAD دون provenance.
	￼	كل findings مصنفة ومربوطة بأدلة واختبارات.
Gate B — Build and quality
	￼	cargo fmt --all -- --check ناجح.
	￼	cargo check --workspace --all-targets ناجح.
	￼	cargo test --workspace --all-targets ناجح.
	￼	cargo clippy --workspace --all-targets --all-features -- -D warnings ناجح أو exceptions موثقة ومحدودة.
	￼	audit/deny/secret scans ناجحة أو findings مصنفة.
	￼	Docker build/compose validation ناجح.
Gate C — Data and quote correctness
	￼	StateIntegrity/fork/gap/reconnect tests ناجحة.
	￼	protocol decoders parity مثبتة لكل protocol مستخدم.
	￼	no stale/missing/mixed-fork account can produce executable quote.
	￼	all amounts typed and decimals verified.
	￼	no overflow/saturating/default fee fallback in critical paths.
	￼	split and return hops exactly re-quoted.
	￼	one canonical EconomicQuote and cost ledger used end-to-end.
Gate D — Atomic execution
	￼	Kamino fee sourced from verified state on all builder paths.
	￼	borrow→swap(s)→repay and profit guard semantics proven for exact lender path.
	￼	same guarded quote hash reaches risk, builder, pre-dispatch.
	￼	simulation Disabled/NotRun/Failed/Timeout always rejects.
	￼	min-out/repayment/transaction limits verified.
	￼	Jito bundle ID never treated as signature or landing proof.
	￼	durable execution state machine and idempotent reconciliation tested.
Gate E — Risk, security and operations
	￼	exposed credentials revoked/rotated; no secrets in source/history/artifacts.
	￼	persistent KillSwitch, operator authorization, daily/per-slot budgets tested.
	￼	dashboard shows truthful mode/status and complete cost/PnL separation.
	￼	alerts, metrics, audit journal, restart recovery tested.
	￼	shadow soak and fault-injection report attached.
	￼	live remains disabled by default and no live tx sent without explicit authorization.
Gate F — Evidence and release
	￼	docs/audit/FINDINGS_REGISTER.md كامل.
	￼	docs/PHASE_STATUS.md يوضح 12.1–15 status مع evidence.
	￼	docs/ARCHITECTURE.md محدث ويطابق runtime.
	￼	docs/PROTOCOL_COMPATIBILITY.md لكل protocol/version.
	￼	docs/OPERATIONS_RUNBOOK.md للاستضافة/restart/kill switch/provider outage.
	￼	docs/SECURITY.md وsecret rotation procedure.
	￼	.env.example placeholders فقط ومحدث.
	￼	Dashboard متصلة فعلًا وليست mock.
	￼	release notes + reproducible test logs + commit SHA.
	￼	ZIP نهائي لا يحتوي .env أو مفاتيح أو target/build artifacts غير لازمة.
أي checkbox غير مثبت يبقى مفتوحًا. لا تضع علامة pass بناءً على code inspection فقط إذا كان المطلوب runtime proof.
￼
16) ملفات التوثيق والتسليم المطلوبة
أنشئ أو حدّث على الأقل:
docs/
  audit/
    BASELINE.md
    FINDINGS_REGISTER.md
    SECURITY_AUDIT.md
    LOGIC_AND_MATH_AUDIT.md
    DATA_INTEGRITY_AUDIT.md
    EXECUTION_AUDIT.md
  PHASE_STATUS.md
  ARCHITECTURE.md
  PROTOCOL_COMPATIBILITY.md
  ECONOMICS_AND_SIZING.md
  EXECUTION_LIFECYCLE.md
  OPERATIONS_RUNBOOK.md
  RELEASE_READINESS.md
  TEST_EVIDENCE.md
.env.example
قد تختلف أسماء/هيكل المشروع؛ حافظ على conventions القائمة ولا تنشئ نسخًا مكررة من نفس الوثيقة.
Findings register schema
لكل finding:
	●	ID ثابت.
	●	Severity: Critical/High/Medium/Low.
	●	Status: OPEN/PARTIAL/FIXED/NOT_REPRODUCED/BLOCKED.
	●	Source commit + file:line.
	●	Reproduction/evidence.
	●	Root cause.
	●	Risk/impact.
	●	Fix commit/file.
	●	Regression test name and result.
	●	Remaining caveat.
Final response from IDE agent
في نهاية المهمة أعطني تقريرًا صريحًا بهذه الأقسام:
	1.	Source of truth: repository/branch/HEAD SHA الذي عملت عليه.
	2.	What changed: كل ملف رئيسي وما الذي تغير ولماذا.
	3.	Phase matrix: 12.1–12.16 و13.x و14.x و15.x؛ status مع دليل.
	4.	Findings closure: العدد المفتوح/المغلق/الجزئي/المتعذر، مع أخطر 10 findings.
	5.	Build/test evidence: الأوامر الحرفية وexit codes والنتائج. لا تكتب “passed” إذا لم تشغّل.
	6.	Security: credential rotation status، secret scan، أي blockers.
	7.	Runtime truth: ما يعمل فعلًا، ما هو shadow-only، ما هو disabled، ما لم يتم اختباره.
	8.	Performance/profit evidence: أرقام مقاسة فقط؛ لا توقعات مختلقة.
	9.	Remaining blockers: ما الذي يمنع production أو live.
	10.	Delivery: branch/PR/commit، ZIP path، dashboard، .env.example، release docs.
￼
17) طريقة التنفيذ — ترتيب العمل الإلزامي
	1.	Inspect only: اجلب GitHub، اقرأ HEAD، أنشئ baseline، لا تعدّل شيئًا بعد.
	2.	Threat + correctness audit: secrets، fake execution، state integrity، unit/economics، protocol decoders، transaction lifecycle.
	3.	Build baseline: شغّل toolchain والاختبارات الحالية، وثّق failures قبل التعديل.
	4.	Fix critical security/data blockers: rotate secrets، fail-closed simulation، state/fork guards، remove executable fallback.
	5.	Complete 12.13 decoder parity.
	6.	Complete 12.14 universal executor enforcement.
	7.	Complete 12.15–12.16 integration/replay/fault tests.
	8.	Complete Phase 13 canonical quote/economics/sizing/fees.
	9.	Complete Phase 14 atomic borrow/swap/repay/build/simulation/dispatch/reconciliation.
	10.	Complete Phase 15 shadow, controls, dashboard, monitoring, runbooks, acceptance.
	11.	Refactor large modules one responsibility at a time, only after correctness and with tests.
	12.	Run full validation after each logical crate change and again from clean checkout.
	13.	Package release candidate, scan archive for secrets and verify ZIP integrity.
	14.	Do not enable live or submit transactions. توقف عند controlled-live readiness، واعرض تقرير gates للمستخدم لقرار مستقل.
قواعد التعامل مع التعارضات
	●	أحدث HEAD هو baseline، لكن لا تتجاهل قرارًا سابقًا مثبتًا في المحادثات إذا كان ما زال متوافقًا. دوّن أي تعارض في docs/audit/DECISIONS_AND_CONFLICTS.md.
	●	إذا تعارضت وثيقة قديمة مع كود أحدث، لا تعتبر أيًا منهما صحيحًا تلقائيًا: ارجع إلى on-chain program/IDL/SDK والاختبارات والـcommit history.
	●	إذا تعارضت عدة implementations، اختر canonical path واحدًا بعد إثبات parity، ثم deprecate/remove الآخرين تدريجيًا.
	●	لا تُخفِ اختلافات protocol versions؛ اجعلها صريحة في types/config.
	●	لا ترفع limits أو تخفف guards لإجبار الاختبارات على النجاح.
	●	لا تغيّر fee/decimal/slippage/tip defaults لتجميل النتائج.
	●	لا تسجل فرصة على أنها مربحة إلا بعد احتساب كل التكاليف وبالوحدات الصحيحة.
ابدأ الآن بالخطوة 1 فقط: جلب أحدث GitHub والتحقق من baseline. بعد ذلك نفّذ الخطة كاملة بالترتيب، ولا تسأل المستخدم أسئلة يمكن حسمها من المستودع أو المصادر الرسمية. اسأل فقط إذا واجهت قرارًا تشغيليًا/ماليًا لا يمكن استنتاجه بأمان.