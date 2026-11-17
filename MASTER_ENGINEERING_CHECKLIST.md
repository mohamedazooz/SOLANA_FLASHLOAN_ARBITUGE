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

> تدقيق فعلي منفَّذ على commit `72cc41c` (أحدث main وقت التدقيق).
> المنهج: قراءة الكود + تشغيل `cargo build --workspace` +
> `cargo test --workspace --all-targets` كاملة.

### التقدم الحالي

- **108 / 495** بند متحقق بدليل ومُعلَّم `[x]`.
- **387** بند متبقي: منها 12 فجوة مؤكدة (G1–G12 أعلاه) والباقي `NOT AUDITED` — يحتاج تدقيقاً فردياً أو تشغيلاً فعلياً (shadow/live) قبل التعليم.

### نتائج التشغيل

- `cargo build --workspace`: نجح (0 أخطاء).
- `cargo test --workspace --all-targets`: **1080 اختبار ناجح، 0 فشل** عبر 86 test binary.
- CI يفرض `cargo fmt --check` و `cargo clippy --workspace --all-targets -- -D warnings` و `cargo-deny` (rust-ci.yml).

### فجوات مؤكدة (CONFIRMED — لم تُعلَّم [x])

| # | الفجوة | الدليل |
|---|---|---|
| G1 | `ProviderCapabilityRegistry` + `validate_execution_prerequisites` غير مربوطة بالـ Supervisor — معرّفة ومُصدَّرة فقط | `capability.rs`, `arb-ingest/src/lib.rs:70` — صفر استدعاءات في `arb-bot` |
| G2 | `ProviderSupervisor` (health/circuit-breaker/failover selection) غير مستخدم في مسار التشغيل الحقيقي | صفر استدعاءات في `arb-bot` خارج re-export `lib.rs:24` |
| G3 | `build_quote_dependency_set` / `QuoteReadinessContext` / `SlotIntegrity` / `DependencyManifest` (بوابة fail-closed للـ quotes) غير مستدعاة خارج `arb-ingest` | grep على `arb-bot` = 0 نتائج |
| G4 | recurring HTTP market-data polling في hot path: `fetch_live_pool_states` تُستدعى دورياً عند كل slot notification + vault hydration عبر HTTP | `supervisor/run.rs:689`, `run.rs:705` |
| G5 | Solami Mirage parser لا يستخرج حقول protobuf حقيقية: يستخدم `latest_slot` محلي، `pubkey: None`, `write_version: None`، وغير موصّل بالبوت | `solami_mirage.rs:138-140` |
| G6 | أصل الاقتراض hardcoded WSOL فقط دون تقييم باقي reserves | `dlmm_flash_arbitrage.rs:306` |
| G7 | سعر SOL افتراضي $200 ما زال موجوداً (مسار التنبيهات) | `telegram.rs:63` |
| G8 | `slotSubscribe` يُرسل دائماً دون gating حسب provider capability | `helius_ws.rs:602` |
| G9 | لا يوجد cargo-fuzz harness منفصل — لكن proptest fuzzing suite موجود (500 cases، 4 crates) | `arb-math/tests/fuzzing_suite.rs` + proptest في arb-config/arb-dex/arb-tx |
| G10 | لا توجد خطوة Gitleaks في CI | `rust-ci.yml` فيه cargo-deny فقط |
| G11 | `damm_v2` / `launchlab` / `pump_fun_raydium` في `arb-math` بدون unit tests | grep `#[test]` = 0 لكل منها |
| G12 | لا يوجد دليل على 24-hour Shadow Soak (الأدوات موجودة: `run-shadow.sh`, `docker-compose.shadow.yml`؛ `reports/production/` فيه baseline + acceptance matrix فقط) | `reports/production/` |

### Acceptance Gates السابقة

- `reports/production/acceptance_matrix.json` يوثق Gates A–D بحالة PASS
  (repository integrity / protocol correctness / economic correctness /
  safety controls) — لكن على base_commit `45031bb` وليس أحدث main؛
  يحتاج إعادة تشغيل قبل الاعتماد النهائي (يرتبط بـ G12).

### ملاحظة منهجية

البند المتعلم `[x]` أدناه يعني: الكود موجود **ومُختبر** في crate الخاص به
بالدليل المذكور. البنود المتعلقة بالربط بالمسار الحقيقي (wiring) لم تُعلَّم
حتى يكتمل الربط الموثق في G1–G4.

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
- [x] إضافة quota accounting مركزي. `(rpc_pool.rs:128,761 — كشف 429/Retry-After/quota_exhausted لكل endpoint + telemetry)`
- [x] إضافة Provider Health Score. `(rpc_pool.rs:91-104 — EndpointStatus حسب latency/429/أخطاء)`
- [x] إضافة latency measurements لكل مزود. `(rpc_pool.rs:104,180 — total_latency_ms + p50/p95/p99)`
- [x] إضافة Provider Circuit Breaker. `(rpc_pool endpoint rotation + ProviderSupervisor OpenCircuit — الأخيرة غير مربوطة، انظر G2)`
- [x] إضافة exponential backoff. `(helius_ws.rs:490,771 + rpc_pool Retry-After)`
- [ ] إضافة jitter للـ retries. `(معلنة في ReconnectPolicy لكن تطبيقها الفعلي في helius_ws غير مؤكد)`
- [x] منع reconnect loops. `(backoff + cooldown rotation قبل إعادة الاتصال)`
- [x] منع retry غير المحدود. `(rpc_pool.rs:1042 — retry فقط عند وجود endpoint مرشح)`
- [x] إضافة provider cooldown. `(helius_ws.rs:768 — cooldown 2.5-3.0s عند التدوير)`
- [x] اختبار failover بين المزودين. `(provider_supervisor tests + rpc_telemetry_tests.rs)`
- [ ] اختبار فشل جميع المزودين.
- [ ] إيقاف إصدار الفرص عند فقد جميع مصادر البيانات.

## 2. Provider Capabilities وAdapters

- [ ] ربط ProviderCapabilityRegistry فعلياً بالـ Supervisor.
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
- [ ] عدم استهلاك الاشتراك الوحيد في slot subscription غير ضروري.
- [ ] جعل slot subscription اختيارياً حسب provider capability.
- [x] إصلاح single-endpoint backoff. `(record_connect_backoff + اختيار endpoint الذي ينتهي cooldown أولاً — helius_ws.rs:229-239)`
- [x] إصلاح resubscription بعد reconnect. `(إعادة الاشتراك من fresh state عند كل connect — helius_ws.rs:584-614)`
- [x] استعادة جميع الحسابات المطلوبة بعد reconnect. `(pending_subs تُبنى من tracked_order عند كل اتصال)`
- [ ] منع duplicate subscriptions.
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
- [ ] اختبار اتصال مزودين.
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
- [ ] تحديد Expected Owner لكل حساب. `(البنية موجودة — الربط بالمسار الحقيقي معلّق على G3)`
- [x] تحديد Required/Optional لكل dependency. `(AccountDependency مع role + required)`
- [ ] ربط dependency manifest بمسار الاشتراكات.
- [ ] ربط dependency manifest بمسار exact quote.
- [ ] ربط dependency manifest بمسار dispatch.
- [ ] رفض route عند فقد أي حساب مطلوب.
- [ ] رفض route عند وجود owner غير صحيح.
- [ ] رفض route عند عدم اكتمال account hydration.
- [ ] منع إنشاء manifest فارغ كأنه صالح.
- [x] إضافة dependency completeness tests. `(9 unit tests في dependency_manifest.rs)`
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
- [ ] منع استخدام سعر افتراضي مثل $200 كدليل اقتصادي.
- [ ] إصلاح اختلاف حدود min profit بين المسارات.
- [x] مراجعة MIN_NET_PROFIT_USD. `(config.rs:385-399 — configurable، >=0 validation، f64 محصور عند config boundary فقط)`
- [x] مراجعة SOL_USD_PRICE. `(مُراجع — الافتراضي $200 ما زال في مسار التنبيهات فقط: G7)`
- [ ] مراجعة min_net_profit_abs.
- [ ] إزالة بوابات gross spread الثابتة. `(CONFIRMED: min_spread_bps ما زال موجوداً — config.rs:285 + strategy.rs:217)`
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
- [x] التحقق من Transaction Serialization. `(V1 compiler مع wire-size check صريح — flash/compiler.rs:49)`
- [x] التحقق من Versioned Transactions. `(V1: 0x80/0x81 prefix، ≤4096 bytes، كل الحسابات inline — compiler.rs:5,37,758)`
- [x] التحقق من Address Lookup Tables. `(arb-tx/src/transport/master_alt.rs — 1,417 سطر؛ V1: 64 حساب inline + سقف 4096 bytes)`
- [ ] التحقق من حجم المعاملة 1232 bytes.
- [x] منع تقسيم atomic route إلى معاملات منفصلة. `(verify_atomic_integrity مستدعاة قبل التوقيع — dlmm_flash_arb.rs:36,496)`
- [ ] التحقق من Kamino repayment semantics.
- [ ] التحقق من Min-Out.
- [ ] التحقق من Min-Profit.
- [x] التحقق من Repayment Guard. `(BorrowTooSmall guard على RAW bits قبل التقريب — موثق في AGENTS.md invariants)`
- [ ] رفض المعاملة عند فشل أي شرط.
- [ ] اختبار builder باستخدام real account fixtures.
- [ ] اختبار instruction ordering.
- [ ] اختبار transaction size boundaries.

## 13. Jito وLanding

- [ ] التحقق من Jito endpoints.
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
- [ ] إضافة Replay Tests.
- [x] اختبار Integer Overflow. `(adversarial_tests.rs — extreme imbalance 1 lamport vs 10^18، deep pools 10^24)`
- [ ] اختبار Invalid Decimals.
- [ ] اختبار Malformed Account Data.
- [ ] اختبار Zero Liquidity.
- [ ] اختبار Missing Bins.
- [ ] اختبار Missing Ticks.
- [x] اختبار Negative Profit. `(test_negative_invalid_cross_currency_addition_rejected + loss-path في economic.rs:349-356)`
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
- [ ] مراجعة Gitleaks.
- [ ] مراجعة Docker Security.
- [ ] مراجعة RPC Endpoint Validation.
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
- [x] توحيد Configuration Source of Truth. `(arb-config: base.toml ← env TOML ← env vars ← CLI مع config.schema.json + validation)`
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
- [x] إصلاح Clippy Warnings. `(CI يفرض -D warnings — rust-ci.yml:50؛ build نظيف)`
- [x] إصلاح Formatting. `(CI يفرض cargo fmt --check — rust-ci.yml:41)`
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
