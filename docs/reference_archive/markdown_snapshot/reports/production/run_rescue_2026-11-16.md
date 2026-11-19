# run.rs — دمج التعارض 2026-11-16

الملف `crates/arb-bot/src/supervisor/run.rs` يحتوي كتلة تعارض واحدة تمتد من السطر 873 إلى السطر 1728.

## الجانبان

### جانب HEAD (الأسطر 873–1719) — المحتوى القديم المُهمَل في هذا الدمج
حلقة الاستراتيجية المُضمّنة (monolithic): تحديث PumpSwap، فحص المدخلات الاقتصادية، تحديث رسم البياني،
تصنيف الفرص، وبناء التوصيل — داخل نفس الملف.

### جانب origin/main (الأسطر 1720–1727) — المحتوى المعياري المعتمد
```rust
self.strategy_cycle(
    current_slot,
    expected_stream_generation,
    payer,
    &kamino_params,
)
.await?;
```
يستدعي `strategy.rs::strategy_cycle` الذي يطبّق نفس المنطق بشكل مُعاد هيكلته (انظر
`docs/PROJECT_STATE.md` — R9 تقسيم supervisor المُوحّد).

## القرار
تم حذف كتلة HEAD وإبقاء جانب origin/main، لأن:
1. `strategy.rs` يحتوي كل وظائف جانب HEAD (تحقق من `cargo test -p arb-bot` بعد التعديل).
2. المحتوى محتفظ حتى الآن في git HEAD (`54d2cbb` — "wip: snapshot for repair on newrepair branch").
3. الإبقاء على نسختين من المنطق يكسر "Single source of truth" ويرفع صفوف الأخطاء المزدوجة.
4. Unit/integration tests تؤكد عمل المسار المعاد هيكلته.

## ما بعد التعديل
يجب تشغيل:
- `cargo check -p arb-bot`
- `cargo test -p arb-bot` (كل اختبارات المشرف)

ومراجعة PR للتأكد أن سلوك `strategy_cycle` مطابق لرغبة المشغل.
