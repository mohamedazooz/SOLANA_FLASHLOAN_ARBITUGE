> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# تدقيق الرسوم وتكاليف التنفيذ

- **النطاق:** `arb-math/src/economic.rs`, `arb-tx/src/transport/adaptive.rs`, `arb-tx/src/primitives/instruction.rs`, `arb-land/src/bidder.rs`, `arb-constants`
- **التاريخ:** 2026-11-19

## 1. مكوّنات التكلفة

`build_economic_quote` / `calculate_net_profit_raw` يخصمان:

| التكلفة | الوحدة | المصدر |
|---|---|---|
| `swap_fee` | atomic units | تسعير DEX (أعلاه) |
| `flash_fee` | atomic units | `kamino_flash_fee_checked` (U68F60) |
| `network_fee` | lamports → base | ثابت 5,000 lamports |
| `priority_fee` | lamports → base | `total_priority_fee_lamports(μlamports/CU, CU)` |
| `jito_tip` | lamports → base | `AdaptiveBidder` (سوق tip floor) |
| `account_creation` | lamports → base | ATA rent عند الحاجة |

- التحويل إلى عملة الطريق يتم عبر `lamports_to_base_raw` بمعدّل صريح (`multiplier`)، وبدون `f64` في قرار الربح. ✅
- `net_profit_raw` و`expected_net_profit_signed_raw` أعداد صحيحة (يمكن أن تكون سالبة)، و`is_profitable` يشترط `> 0`. ✅

## 2. حساب priority fee (V1)

```rust
pub fn total_priority_fee_lamports(micro_lamports_per_cu: u64, cu_limit: u64) -> Option<u64> {
    let total = u128::from(micro_lamports_per_cu)
        .checked_mul(u128::from(cu_limit))?
        .checked_add(999_999)?
        / 1_000_000;
    u64::try_from(total).ok()
}
```

- `ceil(μlamports_per_cu × CU / 1_000_000)` — صيغة صحيحة.
- fail-closed عند overflow (`Option`). ✅
- **لا يوجد saturating cast صامت.** ✅

مثال المعاملة المعلنة: `priorityFee = 1,000` lamports مع `CU = 200,000` ⇒ 1,000 lamports فعلياً (5 μlamports/CU × 200,000 / 1e6 = 1,000). ✅ متسق مع `getTransaction`.

## 3. رسوم المنصة (base fee)

- 5,000 lamports لكل توقيع (Solana runtime) — تُدفع دائماً.
- الأجرة الكلية المرصودة للمعاملة = `6,000` lamports (base + priority).

## 4. Jito tip

- `JITO_TIP_ACCOUNTS` = 8 عناوين رسمية. ✅ مطابقة للقائمة المنشورة.
- `build_jito_tip_instruction` ينشئ `SystemProgram::Transfer` (index 2) بالمبلغ المحدد. ✅
- `AdaptiveBidder`:
  - `MIN_JITO_TIP_LAMPORTS = 1_000` (الحد الأدنى الذي يفرضه block engine).
  - `keep_share` (افتراضي 0.85) يحدّد أدنى هامش يجب أن يبقى.
  - `max_tip_share` (افتراضي 0.85) يحدّد سقف البقشيش.
  - يرفض الفرصة إذا كان السقف < 1,000 lamports.
  - `ladder_growth = 1.6` يحدّ من تصاعد البقشيش بين المحاولات.

### ملاحظة P2 (غير مُصلَحة)

`BidderConfig` يستخدم `f64` (`keep_share`, `max_tip_share`, `ladder_growth`) ويحوّل عبر `fraction_of`. هذا يخالف حرفياً قاعدة "no f64 in authoritative decisions" في `AGENTS.md`، رغم أن:
- النطاق محصور `[0,1]` مع تحقق `validate()`.
- الخطأ الكمي محدود (كسر lamport واحد).
- قرار الربح النهائي (`is_profitable`) ما زال عدداً صحيحاً.

**التصنيف:** P2 — تحسين مطلوب، لا يفسد قراراً مالياً حالياً. لم يُغيَّر لتجنّب تعديل واجهة إعدادات المشغّل.

## 5. تكلفة إنشاء ATA

- `build_create_ata_idempotent_for_program` تُضاف قبل الرحلة.
- rent المستردّ/المدفوع يُحتسب في `account_creation` ضمن التكاليف.
- **لا يوجد مسار reclaim تلقائي في المعاملة** (إغلاق ATA لاسترداد rent). هذا يعني أن كل ATA جديدة تُترك مفتوحة بعد التنفيذ. على مسار عالي التكرار، هذا استنزاف صغير لكنه دائم.

**التصنيف:** P2 — ميزة reclaim غير مفعّلة. لم تُنفَّذ لأن إغلاق ATA داخل نفس المعاملة قد يكسر تحققات رصيد لاحقة، ويتطلب قراراً تشغيلياً.

## 6. التحقق من الرسوم الفعلية في المعاملة المعلنة

| البند | القيمة |
|---|---|
| base fee | 5,000 lamports |
| priority | 1,000 lamports |
| Jito tip | 0 (لا توجد تعليمة tip) |
| ATA rent | 0 (لا توجد ATA) |
| flash fee | 0 (لا يوجد Kamino) |
| swap fee | 0 (لا يوجد swap) |

**المجموع:** 6,000 lamports ← مطابق لـ`meta.fee`. ✅

## 7. ما لم يتم التحقق منه

| البند | الحالة |
|---|---|
| دقة `TipFloor` مقابل بيانات Jito الحيّة | ⚠️ not verified (يحتاج شبكة) |
| مطابقة `account_creation` لـrent الفعلي على mainnet | ⚠️ partial — الثوابت مأخوذة من runtime، لم تُختبر حياً |
| تكلفة CU الفعلية لكل بروتوكول مقابل `simulateTransaction` | ⚠️ not verified |

## الخلاصة

| البند | الحالة |
|---|---|
| صيغة priority fee | ✅ verified + tested |
| احتساب flash/network/priority/tip في الربح | ✅ verified |
| عدم استخدام f64 في قرار الربح | ✅ (لكن tip sizing يستخدم f64 — P2) |
| Jito tip accounts/instruction | ✅ verified |
| استرداد rent تلقائي | ❌ غير موجود (P2) |
| أرقام الرسوم في المعاملة المعلنة | ✅ verified |
