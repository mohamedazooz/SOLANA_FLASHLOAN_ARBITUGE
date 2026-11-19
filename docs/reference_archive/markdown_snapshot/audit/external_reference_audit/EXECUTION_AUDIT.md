# Execution & Transaction Landing Audit (تدقيق التنفيذ وإرسال المعاملات)

تاريخ التدقيق: 17 نوفمبر 2026
الفاحص المسؤول: Agent 2 (HFT & Execution Specialist)
المشرف: Supervisor

## 1. محددات حجم المعاملات (V1 Transaction Packaging)
بالمقارنة مع نموذج `Cetipoo cost.rs` ونموذج `sol-trade-sdk`:
- **معاملات V1**:
  - تمييز المعاملة بـ `0x81 Discriminator`.
  - سقف حجم المعاملة على الشبكة لا يتجاوز 4096 بايت.
  - دعم ما يصل إلى 64 inline accounts دون الحاجة الإجبارية لاستخدام Address Lookup Tables (ALTs) في المسارات البسيطة، مع دعم كامل للـ ALTs في المسارات المركبة (Cross-Pool 3-Hop).
  - تحديد دقيق لوحدات الحساب `Compute Unit Limit` ورسوم الأولوية `Compute Unit Price` بشكل صريح ومسبق.

## 2. إرسال المعاملات وحزم MEV (Jito Bundles & SWQoS)
- **مسار الإرسال**:
  - إرسال متزامن عبر قنوات Jito Block Engine المعتمدة لضمان عدم التعرض للـ Front-running أو الساندوتش.
  - تضمين إكرامية Jito Tip في تعليمة مخصصة تذهب لأحد الحسابات الثمانية الرسمية المعلنة من Jito.
  - إمكانية الإرسال عبر SWQoS (Stake-Weighted Quality of Service) كمسار احتياطي أو تكميلي عند الضرورة.

## 3. مطابقة المعاملات والتعافي (Reconciliation & Recovery)
- يتم تتبع التوقيع (Signature) عبر `supervisor/recovery.rs`.
- في حال تأخر الاستجابة من الشبكة (Timeout)، لا يُعاد إرسال المعاملة بتوقيع جديد أو حسابات مختلفة تجنباً لتكرار السحب، بل يتم فحص حالة التوقيع الأصلي ومطابقة الـ Slot Progression (صلاحية الـ Blockhash لمدة 150 سولت تقريباً) للتأكد النهائي من نجاحها أو سقوطها قبل تحرير الموارد.
