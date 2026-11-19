# دليل المحاكاة الآمنة (Safe Simulation Runbook)

- **الغرض:** تشغيل النظام في وضع محاكاة/تدقيق دون أي خطر مالي.
- **آخر تحديث:** 2026-11-19

> ⚠️ هذا الدليل **لا** يفعّل التداول. يبقى `ARB_LIVE=0` و`DRY_RUN=true` في كل خطوة.

## 0. الشروط المسبقة

| البند | المطلوب |
|---|---|
| نظام التشغيل | Linux (yellowstone gRPC يعتمد عليه) |
| Toolchain | `rustc 1.99.1` / `cargo 1.99.1` |
| `.env` | موجود، git-ignored، يحتوي مفاتيح RPC للقراءة فقط |
| الشبكة | وصول RPC (mainnet) للقراءة |

> على Windows يمكن تشغيل `cargo test`/`cargo check`، لكن تدفّق Yellowstone وبعض اختبارات الشبكة تتطلب Linux.

## 1. فحص البيئة قبل أي شيء

```bash
# يجب أن تكون هذه القيم صفراً/false
grep -E '^(ARB_LIVE|DRY_RUN)=' .env
# المتوقع:
# ARB_LIVE=0
# DRY_RUN=true
```

إن كانت `ARB_LIVE=1` → **توقف**. لا تكمل.

## 2. التحقق من البناء

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
```

- لا تتابع إذا فشل `cargo check`.
- `cargo clippy --workspace --all-targets -- -D warnings` قد يفشل بسبب تحذيرات سابقة (P3) — راجع `docs/audits/FINAL_VERIFICATION_REPORT.md`.

## 3. تسجيل الأدلة

احفظ مخرجات الاختبارات:

```bash
mkdir -p artifacts/forensics
cargo test --workspace --all-targets 2>&1 | tee artifacts/forensics/workspace_test.log
```

## 4. محاكاة معاملة (بدون إرسال)

> **ممنوع:** `sendTransaction` أو `sendBundle` في هذه المرحلة.

استخدم `simulateTransaction` فقط، على معاملة مبنية من حالة السوق الحالية.

```bash
set -a; . ./.env; set +a
# مثال: شغّل الـbinary في وضع shadow فقط إن كان يدعمه، وإلا استخدم أداة محاكاة منفصلة
```

الشروط:
1. `simulateTransaction` مع `sigVerify: false` لا يكفي للتحقق من التوقيع — استخدم `sigVerify: true` عند توفر المفتاح.
2. تحقق أن `logs` لا تحتوي `Program failed` أو `insufficient`.
3. تحقق من `unitsConsumed` مقابل `computeUnitLimit`.
4. تحقق أن `postTokenBalances` تعكس الدلتا المتوقعة.

## 5. التحقق من مسار Orca تحديداً

قبل أي محاكاة لمسار Orca:

- تأكد أن `execution_accounts` تحتوي 6 PDAs (3 لـ`a_to_b` و3 لـ`b_to_a`).
- تأكد أن `select_tick_arrays` اختارت النصف الصحيح حسب اتجاه الرحلة.
- تأكد أن `sqrt_price_limit` هو `MIN`/`MAX` الرسمي، وليس نسبة.

## 6. عند الفشل

| العَرَض | السبب المحتمل | الإجراء |
|---|---|---|
| `InvalidTickArraySequence` | نافذة tick arrays خاطئة | راجع `orca_tick_array_start_indexes` |
| `SqrtPriceLimitBelowPrice` | `sqrt_price_limit` خاطئ | تحقق من المظروف |
| `MissingTickArrayCoverage` | مصفوفة الـtick الحالي غائبة | fail-closed مقصود؛ لا تُضعفه |
| `BorrowTooSmall` | مبلغ الاقتراض أصغر من الرسوم | ارفع المبلغ أو ارفض الفرصة |

## 7. التقارير

بعد كل جلسة محاكاة، سجّل في `artifacts/forensics/`:
- `simulate_<timestamp>.json`
- `workspace_test.log`
- أي فرق في `reports/` (ثم `git checkout -- reports/` إن كان churn زمنياً فقط).

## 8. قائمة منع صريحة

- ❌ `ARB_LIVE=1`
- ❌ `sendTransaction` / `sendBundle`
- ❌ تعديل `.env` أو المفاتيح
- ❌ إعادة تشغيل `pm2` / services
- ❌ نشر برنامج على السلسلة
- ❌ حذف بيانات أو `git reset --hard`
