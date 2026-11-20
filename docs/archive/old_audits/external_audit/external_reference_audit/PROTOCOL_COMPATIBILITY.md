> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Protocol Compatibility Audit (تدقيق التوافقية مع البروتوكولات)

تاريخ التدقيق: 17 نوفمبر 2026
الفاحص المسؤول: Agent 1 (Quant & Protocol Specialist)
المشرف: Supervisor

## 1. توافقية حسابات Meteora DLMM (`swap2`)
- **البرنامج المستهدف**: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
- **الـ Discriminator**:
  مطابق لمعيار `anchor` للـ `swap2`:
  `[65, 75, 63, 76, 234, 137, 107, 248]`
- **مصفوفة الحسابات (Account Metas Ordering)**:
  1. `lb_pair` (mutable)
  2. `bin_array_bitmap_extension` (optional)
  3. `reserve_x` (mutable)
  4. `reserve_y` (mutable)
  5. `user_token_in` (mutable)
  6. `user_token_out` (mutable)
  7. `token_x_mint`
  8. `token_y_mint`
  9. `oracle`
  10. `host_fee_in` (optional)
  11. `user` (signer)
  12. `token_x_program`
  13. `token_y_program`
  14. `event_authority`
  15. `program`
  - مصفوفة الحسابات المضمنة في `arb-tx/src/protocols/meteora_dlmm.rs` و `arb-executor` مطابقة بنسبة 100% لأحدث IDL منشور في `MeteoraAg/dlmm-sdk`.

## 2. توافقية Raydium CPMM & AMM v4
- **البرامج المستهدفة**:
  - Raydium AMM v4: `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`
  - Raydium CPMM: `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`
- **التدقيق**:
  - الحسابات والـ Authority والـ OpenOrders والحسابات المرتبطة تم التحقق منها ومطابقتها دون أي Hardcoded IDs خاطئة.

## 3. توافقية Kamino Lending Flash Loan
- **البرنامج المستهدف**: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **تعليمات الاقتراض والسداد**:
  - `FlashBorrowReserveLiquidity` (تأخذ `liquidity_amount`).
  - `FlashRepayReserveLiquidity` (تأخذ `liquidity_amount` + `borrow_instruction_index`).
- **سلامة التسلسل (Ordering)**:
  - الاقتراض هو التعليمة الأولى بعد إعدادات الـ CU والـ Priority Fee.
  - السداد هو التعليمة الأخيرة في المعاملة.
  - لا يمكن فصل الاقتراض عن السداد داخل الباندل.
