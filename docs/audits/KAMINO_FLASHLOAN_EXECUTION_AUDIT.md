# تدقيق تنفيذ Flash Loan عبر Kamino

- **النطاق:** `crates/arb-tx/src/flash/kamino.rs`, `kamino_flash.rs`, `flash/arb_executor.rs`, `arb-math/src/economic.rs`, `programs/arb-executor`
- **البرنامج:** `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **التاريخ:** 2026-11-19

## 1. التحقق من واجهة البرنامج الرسمية (upstream)

تمت مقارنة البُنّائين مع المصدر الرسمي:

`Kamino-Finance/klend` → `libs/klend-interface/src/instructions/flash.rs`

### `flash_borrow_reserve_liquidity` — 12 حساباً

| # | الحساب | mut | signer |
|---|---|---|---|
| 0 | `user_transfer_authority` | ❌ | ✅ |
| 1 | `lending_market_authority` | ❌ | ❌ |
| 2 | `lending_market` | ❌ | ❌ |
| 3 | `reserve` | ✅ | ❌ |
| 4 | `reserve_liquidity_mint` | ❌ | ❌ |
| 5 | `reserve_source_liquidity` | ✅ | ❌ |
| 6 | `user_destination_liquidity` | ✅ | ❌ |
| 7 | `reserve_liquidity_fee_receiver` | ✅ | ❌ |
| 8 | `referrer_token_state` (Optional) | (✅ عند التوفير) | ❌ |
| 9 | `referrer_account` (Optional) | (✅ عند التوفير) | ❌ |
| 10 | `sysvar_instructions` | ❌ | ❌ |
| 11 | `token_program` | ❌ | ❌ |

### `flash_repay_reserve_liquidity` — 12 حساباً

نفس الترتيب، مع:
- `6 = user_source_liquidity` (mut)
- `5 = reserve_destination_liquidity` (mut)
- البيانات: `8-byte discriminator || u64 liquidity_amount || u8 borrow_instruction_index` = **17 بايت**.

**النتيجة:** ترتيب الحسابات وأنواع الـmutability مطابقة للواجهة الرسمية. ✅ **verified**.

### عيب تم إصلاحه: referrer optional accounts

كان الكود يضع حسابي الـreferrer دائماً `readonly`. الواجهة الرسمية تستخدم:

```rust
optional_account(&KLEND_PROGRAM_ID, accounts.referrer_token_state, true)
```

أي أن `is_writable = true` عند توفير referrer. عند `None` يصبح `program_id` readonly.

**الإصلاح:** في `kamino.rs` تم استبدال المنطق بـ`match`:
- `Some(key)` → `AccountMeta::new(key, false)` (writable)
- `None` → `AccountMeta::new_readonly(program_id, false)`

**التأثير:** لا تأثير عندما لا يوجد referrer (وهو الوضع الحالي)، لكنه يمنع فشل المعاملة إذا فُعِّل referrer مستقبلاً.
**الحالة:** ✅ fixed + compiled + tested.

## 2. صيغة رسوم Kamino

الرسوم على السلسلة تُحسب في `Reserve::calculate_fees` بنوع `Fraction = U68F60` (60 بت كسرية):

```
origination_fee_rate = Fraction::from_bits(fee_sf)
origination_fee_amount = amount * rate          // Exclusive
fee_f = max(fee_amount, minimum_fee = 1)
if fee_f >= amount { BorrowTooSmall }
fee = round(fee_f)                              // to_ceil
```

المشروع يعيد إنتاجها في `arb-math/src/economic.rs`:

```
fee_bits = amount * sf
fee_bits = max(fee_bits, 1 << 60)
if fee_bits >= amount << 60 { BorrowTooSmall }
fee = (fee_bits + 2^59) >> 60      // ties away from zero
```

- ✅ الصيغة مطابقة (بما فيها `BorrowTooSmall` قبل التقريب، وهو ثابت حرج في `AGENTS.md`).
- ✅ اختبار `flash_fee_matches_bigint_reference` يقارن ضد مرجع 256-بت مستقل.

### تصحيح رقم مهم في التقرير الأصلي

| مصدر | القيمة | المعنى |
|---|---|---|
| `flash_loan_fee_sf` الفعلي (main-market WSOL) | `11_529_215_046_068` | 60-bit fraction |
| التحويل | `11_529_215_046_068 / 2^60` | `1.0e-5` = **0.1 bps** |
| التقرير الأصلي | "1 bps" | ❌ خطأ بمعامل 10× |

على قرض 1 SOL → الرسوم = **10,000 lamports** (وليس 100,000).

## 3. حجم الاقتراض (Borrow Sizing)

- `select_kamino_flash_reserve` يرشّح المرشّحين حسب:
  1. نفس mint الاقتراض،
  2. الحالة `Active`،
  3. `flash_loan_fee_sf != u64::MAX` (القروض مفعّلة)،
  4. `available_liquidity >= min_borrow` من **رصيد خزينة التوريد الفعلي** (`decode_supply_vault_liquidity`).
- إذا لم يوجد مرشّح → `NoEligibleReserve` (fail closed).

**هذا يطابق الثابت:** "الاقتراض على حسب عمق البول" ولا يستخدم سقف الإعدادات وحده. ✅ verified.

## 4. ترتيب التعليمات وذرّية المعاملة

من `arb_executor.rs`:

```
[0] ComputeBudget::SetComputeUnitLimit
[1] ComputeBudget::SetComputeUnitPrice
[2] ATA create-idempotent
[3..] Kamino FlashBorrow
[6] ArbExecutor execute_three_leg
[7] Kamino FlashRepay
[8] Jito tip (اختياري)
```

- `Borrow` و`Repay` في نفس المعاملة الواحدة مع كل الـswaps → **atomicity** ✅.
- `borrow_instruction_index` يُمرَّر إلى `flash_repay` كفهرس تعليمة الاقتراض داخل المعاملة. ✅
- `repay` يستقبل **أصل المبلغ فقط** (`params.borrow_amount`)، والرسوم تُضاف على السلسلة. هذا صحيح: تمرير `principal + fee` كان سيؤدي إلى دفع مزدوج. ✅
- `atomic_integrity::verify_atomic_integrity` يُستدعى للتحقق من الترتيب. ✅

## 5. حارس الربح (Profit Guard)

- `params.repay_amount() = borrow + flash_fee`.
- `assemble_flash_funded_three_pool_cpi_instructions` يتحقق:
  - `repay_amount < borrow + fee` → `InsufficientRepayment`
  - `min_amount_out` لكل رجل يغطي المتطلبات
- برنامج `ArbExecutor` على السلسلة يتحقق من:
  - `wsol_baseline = wsol_initial - leg1_amount_in`
  - `wsol_final >= wsol_baseline + min_final_wsol_delta`
  - `min_final_wsol_delta = repay_amount`
  - عدم استخدام أرصدة وسيطة/سابقة للدعم (`IntermediateInventorySubsidized`)

**النتيجة:** لا يمكن أن تنجح المعاملة إذا لم يغطِّ الناتج القرض + الرسوم. ✅ verified (كود + اختبارات).

### قيود

- الحارس على السلسلة لا يخصم priority fee أو Jito tip بشكل مباشر من `min_final_wsol_delta`؛ هذه تُحسب في `arb-math` قبل البناء (`build_economic_quote`) لكنها ليست مشدّدة on-chain. الفارق يبقى مغطّى فقط إذا كان الربح الصافي المحسوب صحيحاً. **⚠️ P2 — يحتاج قراراً تشغيلياً: هل نمرّر `repay + priority + tip` كـ`min_final_wsol_delta`؟** لم يُغيَّر لتجنّب كسر دلالات العقد الحالية.
- برنامج `ArbExecutor` الموجود يدعم **DLMM فقط** (ثلاث أرجل `swap2`)، بينما اسم الواجهة يوحي بمسارات DEX متعددة. أي مسار غير DLMM يحتاج برنامجاً آخر أو تحديثاً. **⚠️ P1 — documented limitation.**

## الخلاصة

| البند | الحالة |
|---|---|
| تطابق حساب/بيانات البُنّائين مع klend الرسمي | ✅ verified |
| صيغة الرسوم = klend | ✅ verified + tested |
| الإصلاح: referrer writable | ✅ fixed + tested |
| حجم الاقتراض من الخزينة الحقيقية | ✅ verified |
| الذرّية وترتيب التعليمات | ✅ verified |
| تضمين priority/tip في الحارس on-chain | ⚠️ partial / documented risk |
| دعم مسارات غير DLMM في ArbExecutor | ❌ not supported (DLMM-only) |
