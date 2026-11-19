# تدقيق حسابات الرموز و rent (Token Accounts & Rent)

- **النطاق:** `crates/arb-tx/src/primitives/ata.rs`, `primitives/builder.rs`, `flash/arb_executor.rs`, `token2022.rs`
- **التاريخ:** 2026-11-19

## 1. اشتقاق ATA

```rust
seeds = [wallet, token_program, mint]
(ata, _) = find_program_address(seeds, ASSOCIATED_TOKEN_PROGRAM_ID)
```

- ✅ يطابق مواصفة SPL Associated Token Account.
- ✅ يوجد إصدار `_with_program_id` لدعم Token-2022 (يستخدم برنامج الرمز الصحيح في الـseeds).
- ✅ لا يوجد اشتقاق يدوي مخصّص/خاطئ.

## 2. إنشاء ATA

`build_create_ata_idempotent_for_program`:
- البرنامج: `ASSOCIATED_TOKEN_PROGRAM_ID`
- البيانات: `[1]` = `CreateIdempotent`
- الترتيب: `[payer, ata, wallet_owner, mint, system_program, token_program]`

✅ يطابق واجهة ATA الرسمية. استخدام `CreateIdempotent` يمنع فشل المعاملة إذا كان الحساب موجوداً مسبقاً.

## 3. إغلاق ATA واسترداد rent

`build_close_token_account_for_program`:
- البرنامج: token program (SPL أو 2022)
- البيانات: `[9]` = `CloseAccount`
- الترتيب: `[account, destination, owner(signer)]`

✅ صحيح.

### الاستخدام في المسارين

| المسار | إغلاق ATA | الملاحظة |
|---|---|---|
| `primitives/builder.rs::assemble_flash_arbitrage` | ✅ نعم، بعد repay | يستثني `skip_close_intermediate_mints` |
| `flash/arb_executor.rs` (V1 ArbExecutor) | ❌ **لا** | ATAs تُترك مفتوحة بعد التنفيذ |

**النتيجة:** المسار الأحدث (`arb_executor`) لا يسترد rent للحسابات الوسيطة التي أنشأها. على مسار عالي التكرار، هذا تراكم `~0.00203928 SOL` لكل ATA غير مغلقة.

- **التصنيف:** P2 — تسرّب rent صغير لكنه دائم.
- **لم يُصلَح** لأن إغلاق ATA داخل مسار `ArbExecutor` يتطلب تعديل ترتيب التعليمات والتحقق من أن الحساب أُنشئ في نفس المعاملة (وإلا قد يُغلق حساباً موجوداً مسبقاً بأرصدة)، وهو قرار تشغيلي.

## 4. WSOL wrapping/unwrapping

- عند الحاجة: `system_transfer` → `sync_native` لتغذية ATA.
- عند الإخراج: `close_token_account` لـWSOL لإرجاع native SOL.
- في مسار `ArbExecutor`: لا يوجد unwrap/close لـWSOL بعد repay.

**مخاطرة:** إذا كان الإخراج النهائي WSOL، يبقى ملفوفاً في ATA بدلاً من native SOL. هذا لا يفسد الربح لكنه يجعل السيولة غير قابلة للاستخدام كـtip/رسوم حتى تُفَكّ لاحقاً.

- **التصنيف:** P2.

## 5. Token-2022

`crates/arb-tx/src/token2022.rs` يعالج:
- transfer fee (InTransfer / OutTransfer)،
- withheld amounts،
- اختيار token program حسب مالك الـmint.

- ✅ الكود يمرّر `token_program` الصحيح في `build_create_ata_idempotent_for_program`.
- ⚠️ **لم يتم التحقق حياً** من صحة حساب `transfer_fee` على mainnet.

## 6. Rent Economics

| البند | القيمة |
|---|---|
| rent-exempt minimum لحساب token (165 بايت) | `2,039,280` lamports (≈0.00203928 SOL) |
| مصدر القيمة | ثابت شائع في Solana runtime |
| احتسابها في `account_creation` | ✅ موجود في `arb-math` |

- إذا أُنشئ ATA ثم أُغلق في نفس المعاملة، صافي التكلفة = 0 (باستثناء رسوم CU). ✅
- إذا لم يُغلق (مسار ArbExecutor) → تكلفة دائمة.

## 7. مخاطر أخرى

1. **إغلاق ATA موجود مسبقاً:** `assemble_flash_arbitrage` يعتمد على `skip_close_intermediate_mints` لتفادي إغلاق حسابات قد تحمل أرصدة للمستخدم. إذا نسي المستدعي تعبئتها، قد تُدمَّر أرصدة. **P1 إذا استُخدم المسار مع mints غير مُدارة بعناية.**
2. **`build_close_token_account` (بدون program)** يفترض SPL Token وليس 2022 — الاستخدام في مسار WSOL فقط صحيح، لكنه غير آمن لأي mint آخر. **P3.**

## الخلاصة

| البند | الحالة |
|---|---|
| اشتقاق ATA | ✅ verified |
| CreateIdempotent | ✅ verified |
| CloseAccount | ✅ verified |
| استرداد rent في المسار القديم | ✅ موجود |
| استرداد rent في `arb_executor` | ❌ غير موجود (P2) |
| unwrap WSOL بعد repay في `arb_executor` | ❌ غير موجود (P2) |
| Token-2022 transfer fee حياً | ⚠️ not verified |
