> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Upstream Comparison Matrix (مقارنة المراجع بالمشروع الحالي)

تاريخ التدقيق: 17 نوفمبر 2026
المرجع: `audit/external-reference-audit`
الملف المقفل: `UPSTREAM_LOCK.json`

## مصفوفة المقارنة الفنية

| المرجع الخارجي | المكون الذي تم فحصه | حالة المرجع الخارجي | الكود الحالي في مشروعنا | أوجه القوة في مشروعنا | الفجوة المكتشفة والفرصة الهندسية |
|---|---|---|---|---|---|
| **Cetipoo on-chain arbitrage** | `src/route.rs`, `src/v10.rs`, `cost.rs` | كود مرجعي تجريبي (MIT)، يستخدم V10 executor خاص ومصفوفات حسابات محددة مسبقاً. | `crates/arb-tx` و `programs/arb-executor` مع دعم Kamino Flash Loan و V1 transaction wire ceiling. | مشروعنا ينفذ قروض Kamino الخاطفة الذرية الحقيقية مع التحقق الكامل من `Supply Vault`، بينما Cetipoo يعتمد على Vault داخلي خاص به. | الاستفادة من فحص أدوار الـ Mints (Role validation: Borrow -> Intermediate -> Repay) لضمان عدم وجود حلقة غير مغلقة في الـ Compiler. |
| **Lorenz Protocol** | `lorenz-amm`, `lorenz-graph`, `risk.rs`, `docs/INVARIANTS.md` | مشروع تجريبي متقدم في الرياضيات الصحيحة، ولكن Geyser stream فيه يحتوي على `NotImplemented`. | `crates/arb-math` (تحتوي على 120 اختبار وحدة دقيقة للأعداد الصحيحة) و `arb-risk`. | مشروعنا لا يحتوي على Floating point في الحسابات النقدية مطلقاً، ومختبر تفاضلياً ضد Raydium و Meteora و Orca. | اعتماد وثيقة Invariants الصارمة المماثلة لـ Lorenz لإثبات عدم إمكانية رفع حدود المخاطرة (Tighten-only Risk). |
| **FnZero sol-trade-sdk** | `LOW_LATENCY_BOTS.md`, `GAS_FEE_STRATEGY.md` | إرشادات متقدمة في تقليل Latency في Hot Path واستراتيجيات الـ Dynamic Gas/Priority Fees. | `crates/arb-bot/src/supervisor/run.rs` و `arb-land`. | كود Hot-path في مشروعنا معزول ولا يستدعي RPC blockhash داخل الحلقة السريعة، مع استخدام Pre-warmed templates. | تدقيق ضبط الـ Priority Fee ليكون مطابقاً لنماذج التسعير المبنية على سرعة الـ Slots والـ Jito Tip Tiers. |
| **FnZero sol-parser-sdk & streamer** | `grpc_raw_cache.rs`, `yellowstone_grpc.rs` | إدارة اشتراكات Yellowstone gRPC والتعامل مع out-of-order slots والـ Tombstones. | `crates/arb-ingest` و `arb-engine/src/state_guard.rs`. | `StateGuard` في مشروعنا يتحقق من `write_version` و `slot` ويرفض أي تحديث متأخر أو قديم. | تدقيق lifecycle إعادة الاتصال (Reconnect loop) عند انقطاع الـ gRPC للتأكد من تفريغ أي State غير مؤكد والبدء بـ Clean Snapshot جديد فوراً. |
| **Meteora DLMM SDK** | `commons/src/quote.rs`, `price_math.rs`, `idls/dlmm.json` | SDK و IDL الرسمي لبرنامج Meteora DLMM (`LBUZ...`). | `crates/arb-dex/src/meteora.rs`, `crates/arb-math/src/dlmm.rs`. | متطابق بنسبة 100% مع أحدث معادلات Bin Array traversal وتوزيع الرسوم الديناميكية. | مطابقة تفاضلية لـ Discriminator وأمر الحسابات في `swap2` عبر اختبارات Differential tests مستقلة لضمان عدم وجود أي انحراف عن الـ Deployed Program ID. |
