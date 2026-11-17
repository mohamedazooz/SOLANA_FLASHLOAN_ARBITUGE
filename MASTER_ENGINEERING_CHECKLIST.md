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
- [ ] التحقق من حدود RPS الرسمية.
- [ ] التحقق من حدود burst.
- [ ] التحقق من حدود CU/RU الشهرية.
- [ ] التحقق من حدود كل RPC method بشكل منفصل.
- [ ] منع افتراض أن جميع المزودين يدعمون نفس القدرات.
- [ ] فصل Provider Identity عن Endpoint Identity.
- [ ] منع تكرار استهلاك quota لنفس المفتاح عبر endpoints مختلفة.
- [ ] إضافة quota accounting مركزي.
- [ ] إضافة Provider Health Score.
- [ ] إضافة latency measurements لكل مزود.
- [ ] إضافة Provider Circuit Breaker.
- [ ] إضافة exponential backoff.
- [ ] إضافة jitter للـ retries.
- [ ] منع reconnect loops.
- [ ] منع retry غير المحدود.
- [ ] إضافة provider cooldown.
- [ ] اختبار failover بين المزودين.
- [ ] اختبار فشل جميع المزودين.
- [ ] إيقاف إصدار الفرص عند فقد جميع مصادر البيانات.

## 2. Provider Capabilities وAdapters

- [ ] ربط ProviderCapabilityRegistry فعلياً بالـ Supervisor.
- [ ] منع استدعاء method غير مدعومة.
- [ ] فصل JSON-RPC HTTP عن JSON-RPC WebSocket.
- [ ] فصل Yellowstone gRPC عن WebSocket.
- [ ] فصل Solami Mirage عن Solami Blur.
- [ ] عدم استخدام Solami adapter كأنه Helius JSON-RPC.
- [ ] التحقق من بروتوكول الرسائل لكل مزود.
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
- [ ] إعطاء account subscriptions الأولوية.
- [ ] عدم استهلاك الاشتراك الوحيد في slot subscription غير ضروري.
- [ ] جعل slot subscription اختيارياً حسب provider capability.
- [ ] إصلاح single-endpoint backoff.
- [ ] إصلاح resubscription بعد reconnect.
- [ ] استعادة جميع الحسابات المطلوبة بعد reconnect.
- [ ] منع duplicate subscriptions.
- [ ] منع duplicate notifications.
- [ ] اكتشاف missing notifications.
- [ ] اكتشاف stream gaps.
- [ ] اكتشاف انقطاع stream.
- [ ] إضافة heartbeat.
- [ ] إضافة connection timeout.
- [ ] إضافة subscription timeout.
- [ ] إضافة message processing timeout.
- [ ] إضافة bounded queues.
- [ ] إضافة backpressure.
- [ ] منع تراكم الرسائل القديمة.
- [ ] إضافة stream sequence tracking.
- [ ] إضافة reconnect metrics.
- [ ] إضافة subscription count metrics.
- [ ] اختبار اتصال مزود واحد.
- [ ] اختبار اتصال مزودين.
- [ ] اختبار انقطاع المزود الأساسي.
- [ ] اختبار رجوع المزود الأساسي.
- [ ] اختبار رفض subscription.
- [ ] اختبار تجاوز subscription quota.
- [ ] اختبار malformed WebSocket frames.
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
- [ ] التحقق من حدود خطة Solami المجانية الفعلية.
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
- [ ] إبقاء Jito submission معزولاً.
- [ ] إبقاء control-plane HTTP معزولاً.
- [ ] إثبات صفر recurring HTTP market-data calls في hot path.

## 6. Account Dependency Planner

- [ ] بناء dependency manifest لكل route.
- [ ] تحديد جميع Pool Accounts.
- [ ] تحديد Vault Accounts.
- [ ] تحديد Mint Accounts.
- [ ] تحديد Token Accounts.
- [ ] تحديد Token Program Accounts.
- [ ] تحديد Tick Arrays.
- [ ] تحديد DLMM Bin Arrays.
- [ ] تحديد Oracle Accounts.
- [ ] تحديد Observation Accounts.
- [ ] تحديد Fee Configuration Accounts.
- [ ] تحديد Creator Fee Accounts.
- [ ] تحديد Flash Loan Reserve Accounts.
- [ ] تحديد Lending Market Accounts.
- [ ] تحديد Repayment Accounts.
- [ ] تحديد Associated Token Accounts.
- [ ] تحديد الحسابات الإضافية المطلوبة لكل DEX.
- [ ] تحديد Expected Owner لكل حساب.
- [ ] تحديد Required/Optional لكل dependency.
- [ ] ربط dependency manifest بمسار الاشتراكات.
- [ ] ربط dependency manifest بمسار exact quote.
- [ ] ربط dependency manifest بمسار dispatch.
- [ ] رفض route عند فقد أي حساب مطلوب.
- [ ] رفض route عند وجود owner غير صحيح.
- [ ] رفض route عند عدم اكتمال account hydration.
- [ ] منع إنشاء manifest فارغ كأنه صالح.
- [ ] إضافة dependency completeness tests.
- [ ] إضافة account owner regression tests.
- [ ] إضافة account-set consistency tests.
- [ ] إثبات أن quote_route_guarded() مستدعاة فعلياً في Supervisor.

## 7. State Integrity وFork/Freshness

- [ ] توحيد مصدر slot لكل snapshot.
- [ ] منع global slot stamping لحسابات من batches مختلفة.
- [ ] حفظ context slot الحقيقي لكل RPC response.
- [ ] حفظ slot الحقيقي لكل streaming update.
- [ ] منع write_version المحلي من التظاهر بأنه Yellowstone write version.
- [ ] منع fabricated bank_id.
- [ ] منع provider_id ثابت لجميع المزودين.
- [ ] التحقق من owner قبل قبول الحساب.
- [ ] التحقق من account data length.
- [ ] التحقق من account discriminator.
- [ ] التحقق من account layout version.
- [ ] منع stale snapshots.
- [ ] منع duplicate snapshots.
- [ ] منع out-of-order updates.
- [ ] اكتشاف SLOT_DEAD.
- [ ] إبطال processed fork غير المعتمد.
- [ ] إبطال state المرتبط بفork غير صالح.
- [ ] منع دمج snapshots من مصادر غير متزامنة دون سياسة consistency واضحة.
- [ ] إضافة State Generation.
- [ ] إضافة State Integrity Tests.
- [ ] إضافة Fork Invalidation Tests.
- [ ] إضافة Stale Snapshot Tests.
- [ ] إضافة Duplicate Update Tests.
- [ ] إضافة Out-of-Order Tests.
- [ ] جعل state غير الصالح سبباً مباشراً لرفض التنفيذ.

## 8. DEX Exact Quote Parity

- [ ] مراجعة Raydium AMM V4.
- [ ] مراجعة Raydium CPMM.
- [ ] مراجعة Raydium CLMM.
- [ ] مراجعة Raydium LaunchLab.
- [ ] مراجعة Orca Whirlpool.
- [ ] مراجعة Meteora DLMM.
- [ ] مراجعة Meteora DAMM v2.
- [ ] مراجعة Pump.fun.
- [ ] مراجعة PumpSwap.
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
- [ ] التحقق من Integer Overflow.
- [ ] التحقق من Fixed-Point Arithmetic.
- [ ] التحقق من CLMM Tick Traversal.
- [ ] التحقق من DLMM Bin Traversal.
- [ ] التحقق من Bitmap Extension.
- [ ] التحقق من Liquidity Boundaries.
- [ ] التحقق من Price Impact.
- [ ] التحقق من Slippage.
- [ ] تنفيذ SDK Differential Tests لكل DEX.
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
- [ ] بناء route graph ديناميكي.
- [ ] بناء دورات من ثلاثة pools.
- [ ] دعم أكثر من أصل وسيط.
- [ ] تحديث graph عند تغير liquidity.
- [ ] فصل Candidate Discovery عن Exact Quote.
- [ ] عدم فرض spread ثابت أثناء discovery.
- [ ] عدم اعتبار وجود pool فرصة تداول.
- [ ] رفض pool غير المكتمل أو غير الموثوق.

## 10. Flash Loan وKamino

- [ ] مراجعة Kamino instruction semantics.
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
- [ ] رفض reserve state القديم.
- [ ] منع استخدام liquidity غير متاحة فعلياً.
- [ ] إضافة reserve-specific tests.
- [ ] إضافة borrow sizing tests.
- [ ] إضافة repayment tests.

## 11. Net Profit وEconomic Model

- [ ] إنشاء مصدر واحد لحساب الربح.
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
- [ ] منع integer overflow.
- [ ] استخدام checked arithmetic.
- [ ] استخدام ceiling division لرسوم priority fee.
- [ ] توحيد priority fee بين sizing وranking وdispatch.
- [ ] منع saturating arithmetic في الحسابات الاقتصادية الحساسة.
- [ ] استخدام SOL/USD price موثوق ومؤرخ.
- [ ] رفض السعر القديم.
- [ ] منع استخدام سعر افتراضي مثل $200 كدليل اقتصادي.
- [ ] إصلاح اختلاف حدود min profit بين المسارات.
- [ ] مراجعة MIN_NET_PROFIT_USD.
- [ ] مراجعة SOL_USD_PRICE.
- [ ] مراجعة min_net_profit_abs.
- [ ] إزالة بوابات gross spread الثابتة.
- [ ] فصل gross spread عن net profit.
- [ ] ترتيب الفرص حسب صافي الربح القابل للتنفيذ.
- [ ] رفض الفرصة إذا كان أي عنصر تكلفة مجهولاً.
- [ ] إضافة economic regression tests.

## 12. Atomic Transaction Builder

- [ ] التحقق من ترتيب التعليمات.
- [ ] التحقق من Compute Budget Instructions.
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
- [ ] التحقق من Transaction Serialization.
- [ ] التحقق من Versioned Transactions.
- [ ] التحقق من Address Lookup Tables.
- [ ] التحقق من حجم المعاملة 1232 bytes.
- [ ] منع تقسيم atomic route إلى معاملات منفصلة.
- [ ] التحقق من Kamino repayment semantics.
- [ ] التحقق من Min-Out.
- [ ] التحقق من Min-Profit.
- [ ] التحقق من Repayment Guard.
- [ ] رفض المعاملة عند فشل أي شرط.
- [ ] اختبار builder باستخدام real account fixtures.
- [ ] اختبار instruction ordering.
- [ ] اختبار transaction size boundaries.

## 13. Jito وLanding

- [ ] التحقق من Jito endpoints.
- [ ] التحقق من Bundle Serialization.
- [ ] التحقق من Tip Accounts.
- [ ] التحقق من Tip Amount.
- [ ] التحقق من Priority Fee.
- [ ] التحقق من Bundle Submission.
- [ ] حفظ Bundle ID.
- [ ] متابعة Bundle Status.
- [ ] التحقق من Landing.
- [ ] التحقق من Slot.
- [ ] التحقق من Signature Status.
- [ ] الفصل بين Submitted وAccepted.
- [ ] الفصل بين Processed وConfirmed.
- [ ] الفصل بين Confirmed وFinalized.
- [ ] عدم اعتبار HTTP 200 دليلاً على Landing.
- [ ] إضافة Retry Policy.
- [ ] إضافة Bundle Expiration Handling.
- [ ] إضافة Rejection Handling.
- [ ] تسجيل Actual Execution Result.
- [ ] تسجيل Actual Balance Changes.
- [ ] تسجيل Realized Profit.
- [ ] اختبار Jito Failure.
- [ ] اختبار Jito Timeout.
- [ ] اختبار Bundle Rejection.
- [ ] اختبار Landing Confirmation.

## 14. Compute Budget وTransaction Constraints

- [ ] بناء CU estimator ديناميكي.
- [ ] التحقق من CU limit.
- [ ] التحقق من CU price.
- [ ] قياس CU الفعلي لكل DEX.
- [ ] عدم الاعتماد على ثوابت غير مختبرة.
- [ ] حساب priority fee بشكل موحد.
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

- [ ] مراجعة جميع Unit Tests.
- [ ] مراجعة Integration Tests.
- [ ] إضافة Property-Based Tests.
- [ ] إضافة Fuzz Tests.
- [ ] إضافة Differential Tests.
- [ ] إضافة Regression Tests.
- [ ] إضافة Failure Injection Tests.
- [ ] إضافة Concurrency Tests.
- [ ] إضافة Replay Tests.
- [ ] اختبار Integer Overflow.
- [ ] اختبار Invalid Decimals.
- [ ] اختبار Malformed Account Data.
- [ ] اختبار Zero Liquidity.
- [ ] اختبار Missing Bins.
- [ ] اختبار Missing Ticks.
- [ ] اختبار Negative Profit.
- [ ] اختبار Fee Rounding.
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

- [ ] قياس Provider Latency.
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
- [ ] بناء reproducible benchmarks.

## 17. Security

- [ ] مراجعة Private Key Handling.
- [ ] مراجعة Wallet Signer.
- [ ] منع طباعة الأسرار.
- [ ] منع تخزين الأسرار في Git.
- [ ] مراجعة .gitignore.
- [ ] مراجعة .env.example.
- [ ] مراجعة GitHub Actions Secrets Usage.
- [ ] مراجعة Dependency Vulnerabilities.
- [ ] مراجعة Cargo Audit.
- [ ] مراجعة cargo-deny.
- [ ] مراجعة Gitleaks.
- [ ] مراجعة Docker Security.
- [ ] مراجعة RPC Endpoint Validation.
- [ ] منع untrusted account data من تجاوز validation.
- [ ] منع malformed instructions.
- [ ] منع arbitrary program invocation.
- [ ] مراجعة Program IDs.
- [ ] مراجعة Token Program IDs.
- [ ] مراجعة Upgradeable Program Risks.
- [ ] مراجعة Kill Switch.

## 18. Dashboard وObservability

- [ ] مراجعة React/Vite Build.
- [ ] مراجعة API Connections.
- [ ] مراجعة WebSocket Dashboard.
- [ ] مراجعة Provider Health Display.
- [ ] عرض Quota Utilization.
- [ ] عرض Subscription Counts.
- [ ] عرض Stream Gaps.
- [ ] عرض Stale Accounts.
- [ ] عرض Fork Invalidations.
- [ ] عرض Candidate Routes.
- [ ] عرض Exact Quotes.
- [ ] عرض Rejection Reasons.
- [ ] عرض Gross Profit.
- [ ] عرض Net Profit.
- [ ] عرض Borrow Asset.
- [ ] عرض Borrow Amount.
- [ ] عرض Simulation Results.
- [ ] عرض Transaction Size.
- [ ] عرض CU Consumption.
- [ ] عرض Jito Status.
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
- [ ] توحيد Configuration Source of Truth.
- [ ] منع اختلاف defaults بين TOML وENV.
- [ ] توثيق كل Config Field.
- [ ] توثيق Units لكل رقم.
- [ ] التحقق من Valid Ranges.
- [ ] مراجعة ملفات Rust الأكبر من 1000 سطر.
- [ ] تقسيم Supervisor إلى Modules واضحة.
- [ ] إزالة Dead Code.
- [ ] إزالة Duplicate RPC Calls.
- [ ] إزالة Duplicate Fee Logic.
- [ ] إزالة Magic Constants.
- [ ] إصلاح Clippy Warnings.
- [ ] إصلاح Formatting.
- [ ] إصلاح Docker Build.
- [ ] إصلاح Cargo Deny.
- [ ] إصلاح Gitleaks false positives دون تعطيل فحص الأسرار.
- [ ] إصلاح Regression Tests ذات false positives.
- [ ] توحيد CI على نفس commit.
- [ ] اختبار كل PR قبل الدمج.
- [ ] منع Merge تلقائي.
- [ ] توثيق Commit لكل اختبار ناجح.

## 20. Production Acceptance

- [ ] اختبار مزودين مجانيين فعليين.
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
- [ ] إثبات Full Atomic Builder.
- [ ] إثبات Repayment.
- [ ] إثبات Min-Out.
- [ ] إثبات Min-Profit.
- [ ] إثبات Transaction Size.
- [ ] إثبات Full-Route Simulation.
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
