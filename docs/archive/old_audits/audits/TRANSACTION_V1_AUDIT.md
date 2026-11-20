> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# تدقيق معاملات V1 (SIMD-0385)

- **النطاق:** `crates/arb-tx/src/transport/adaptive.rs`, `transport/selection.rs`, `flash/compiler.rs`
- **التاريخ:** 2026-11-19

## 1. الحدود والثوابت

| الثابت | القيمة | المعنى |
|---|---|---|
| `V1_VERSION_DISCRIMINATOR` | `0x81` | البايت الأول (version 1 \| 0x80) |
| `SOLANA_PACKET_V1_MTU` | `4,096` | سقف حجم الحزمة |
| `MAX_V1_INSTRUCTIONS` | `64` | سقف التعليمات |
| `MAX_V1_ADDRESSES` | `64` | سقف العناوين المضمّنة |
| `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT` | `67,108,864` (64 MiB) | سقف بيانات الحسابات المحمّلة |

كل الحدود مفروضة صراحةً في `compile_v1_message` (رفض، لا اقتطاع صامت). ✅

## 2. تخطيط الرسالة (Message layout)

```
[0]                     : 0x81
[1..3]                  : num_required_signatures, num_readonly_signed, num_readonly_unsigned
[4..8]                  : TransactionConfigMask (u32 LE)
[8..40]                 : recent blockhash
[40]                    : numInstructions
[41]                    : numAddresses
[42..42+32*N]           : inline addresses
[...]                   : config values (لكل بت مفعّل في mask)
[...]                   : instruction headers (4 بايت لكل تعليمة: program_idx u8, account_count u8, data_len u16)
[...]                   : instruction payloads متصلة
[tail]                  : signatures (64 بايت لكل توقيع)
```

- **تم التحقق من هذا التخطيط مقابل معاملة mainnet حقيقية** (انظر `MAINNET_TRANSACTION_FORENSICS.md`):
  - `[40] = 0x01` (تعليمة واحدة) ✅
  - `[41] = 0x02` (حسابان) ✅
  - `[4..8] = 0x0F` (mask) ✅
  - حجم الرسالة 171 بايت + توقيع 64 = 235 ✅
- parser في `selection.rs` يقرأ العدد من `[40]` و`[41]` بدقة. ✅

## 3. التعامل مع `ComputeBudget` داخل V1

V1 يحمل `computeUnitLimit` و`priorityFee` و`loadedAccountsDataSizeLimit` داخل الرسالة نفسها، لذا تعليمات `ComputeBudget` تصبح no-op.

الكود:
1. يزيل تعليمات ComputeBudget.
2. **يعيد كتابة `borrow_instruction_index`** في تعليمة Kamino `flash_repay` لأن الفهرس يجب أن يشير إلى ترتيب التعليمات **بعد الإزالة** (klend يتحقق عبر sysvar instructions).

```rust
let executed_index = index_remap.get(borrow_ix_index)...;
ix.data[16] = u8::try_from(executed_index)?;
```

- إن كان الفهرس يشير إلى تعليمة ComputeBudget مُزالة → `SanitizationError` (fail closed). ✅
- هذا إصلاح دقيق لثغرة كانت ستسبب `InvalidFlashRepay` عند بناء V1 من بُنّائين يُصدرون ComputeBudget في البداية. ✅ **verified logic + tested**.

## 4. ترتيب الحسابات وإزالة التكرار

- `BTreeMap<Pubkey, (is_signer, is_writable)>` يجمع الصلاحيات (OR) ويزيل التكرار.
- الترتيب: writable signers → readonly signers → writable non-signers → readonly non-signers.
- الـpayer دائماً index 0 وقابل للكتابة والتوقيع.
- إعادة تعيين الفهارس في التعليمات تلقائياً.

**ملاحظة (P3):** `BTreeMap` يفرز حسابياً، لذا ترتيب الحسابات داخل كل فئة ليس ترتيب الظهور. هذا صالح على السلسلة لكنه يجعل حجم ALT/الضغط أقل كفاءة قليلاً؛ لا يؤثر على الصحة.

## 5. عدم استخدام ALT

- V1 يعتمد على عناوين مضمّنة (inline) حتى 64 حساباً — لا ALT.
- `compile_v1_message` يرفض تجاوز 64 عنواناً بدلاً من التحويل الصامت إلى V0. ✅
- `Never silently downgrade V1→V0 or strip instructions to fit` — محفوظ: يرفض بدلاً من التعديل.

## 6. ميزانية الحجم في `flash/compiler.rs`

- `header_bytes = 7` (version + 3 header + mask + numInstructions + numAddresses).
- يحسب الحجم المتوقع لكل تعليمة + config values + العناوين + التوقيعات، ويرفض إذا تجاوز 4,096. ✅
- لا توجد أي `saturating_sub` تخفي تجاوزاً.

## 7. النقل (transport)

- `getTransaction` يُطلب بـ`maxSupportedTransactionVersion: 1` و`encoding: base64`.
- `to_base64()` يستخدم `STANDARD` base64. ✅
- استخراج التوقيع يدعم V1 عبر قراءة التوقيعات من **ذيل** الحزمة:

```rust
let sig_start = tx_bytes.len() - num_sigs * 64;
```

✅ متوافق مع التخطيط المرصود.

## 8. الاختبارات

اختبارات الحزمة (156 اختباراً في `arb-tx`) تمر، وتشمل:
- تخطيط V1 وحجمه.
- إعادة كتابة فهرس repay.
- رفض تجاوز 64 تعليمة/عنوان.
- استخراج التوقيع لـLegacy/V0/V1.

## 9. ما لم يتم التحقق منه

| البند | الحالة |
|---|---|
| قبول RPC/validator لـV1 في الإنتاج (ليس مجرد canary واحد) | ⚠️ partial — canary واحد ناجح |
| مطابقة parser لجميع الحالات الحدّية (config bit combinations) | ⚠️ not exhaustively verified |
| تعامل Jito block engine مع 4,096 بايت | ⚠️ not verified |

## الخلاصة

| البند | الحالة |
|---|---|
| تخطيط V1 مقابل mainnet | ✅ verified |
| فرض 4,096/64/64 | ✅ verified + tested |
| إعادة كتابة فهرس الاقتراض/السداد | ✅ fixed + tested |
| عدم التحويل الصامت إلى V0 | ✅ verified |
| base64 + maxSupportedTransactionVersion | ✅ verified |
| اعتماد إنتاجي واسع | ⚠️ partial |
