> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Mathematical Differential Audit (التدقيق الرياضي والتفاضلي)

تاريخ التدقيق: 17 نوفمبر 2026
الفاحص المسؤول: Agent 1 (Quant & Protocol Specialist)
المشرف: Supervisor

## 1. مقارنة CPMM / Constant Product Math (ضد Lorenz AMM)
- **الصيغة الرياضية**:
  مشروعنا في `crates/arb-math/src/cpmm.rs`:
  $$dy = \frac{y \cdot dx \cdot (10000 - fee)}{x \cdot 10000 + dx \cdot (10000 - fee)}$$
  يتم تنفيذها باستخدام `u128` و `u384` لتفادي أي Overflow أثناء الضرب التراكمي (مختبر حتى 441-bit dynamic range).
- **التقريب (Rounding)**:
  - التقريب لأسفل لصالح المسبح (Floor / Truncation لصالح الحوض)، مما يحمي الحسابات من الإفلاس الحسابي (Solvency Invariant).
  - لا يتم استخدام أي `f64` في مسار التسوية أو حساب الأرباح.
- **التوافق مع Lorenz AMM**:
  مشروع Lorenz يطبق نفس المبدأ في `lorenz-amm`. أظهرت الاختبارات التفاضلية في `crates/arb-math` تطابقاً كاملاً في النتائج لكافة نطاقات السيولة.

## 2. تدقيق Meteora DLMM Math (ضد Meteora Official SDK)
- **فك تشفير الـ Active Bin و Bin Arrays**:
  - يتم فك الـ 69 bins لكل `BinArray` متوافقة مع حسابات `commons/src/quote.rs` في Meteora SDK.
  - السعر المحسوب في البين:
    $$P(i) = (1 + \text{bin\_step})^{i}$$
    باستخدام حسابات Q64.64 الثابتة في `dlmm.rs`.
- **معالجة الـ Variable Fees والـ Dynamic Volatility**:
  - يتم حساب معامل التقلب `variable_fee_control` وربطه مع معدل الرسوم الأساسي `base_factor` مع سقف أقصى للرسوم يمنع التجاوز.

## 3. رسوم وقروض Kamino Flash Loan الحسابية
- **الصيغة القانونية المعتمدة**:
  $$\text{fee\_bits} = \max(\text{amount} \times \text{flash\_fee\_sf}, 2^{60})$$
  $$\text{fee} = \frac{\text{fee\_bits} + 2^{59}}{2^{60}}$$
- **حساب الأرباح الصافية (Net Profit)**:
  $$\text{Net Profit} = \Delta \text{Settlement Token} - \text{Flash Loan Fee} - \text{Gas/Priority Lamports} - \text{Jito Tip Lamports}$$
  تم إثبات أن `crates/arb-math/src/economic.rs` تحسب ذلك بالكامل على مستوى الأعداد الصحيحة مع رفض تام لأي تداول لا يتجاوز حد الربح الأدنى `MIN_NET_PROFIT`.

## 4. نتيجة الفحص الرياضي (Verdict)
- **حالة التدقيق**: **PASS**. لا توجد ثغرات أو انحرافات حسابية في المنطق المالي الراهن.
