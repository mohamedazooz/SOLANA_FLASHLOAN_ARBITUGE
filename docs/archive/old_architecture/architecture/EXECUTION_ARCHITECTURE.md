> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# معمارية التنفيذ (Execution Architecture)

- **آخر تحديث:** 2026-11-19
- **الحالة:** موثّق من الكود الفعلي، وليس من التصميم النظري.

## 1. نظرة عامة

```
Opportunity (arb-engine)
   ↓
RiskGate (arb-risk)
   ↓
TxBuilder / AdaptiveTxCompiler (arb-tx)
   ↓
Simulation (arb-sim)  ── optional/fail-closed
   ↓
Landing (arb-land): Jito bundle أو RPC
   ↓
Supervisor::dispatch (arb-bot)
   ↓
Recovery (supervisor/recovery.rs)
```

نقطة الدخول الوحيدة للتشغيل هي `Supervisor` في `arb-bot`. أي خدمة تحكّم (Telegram/Web) تستدعي `ControlService`، وليس التنفيذ مباشرة.

## 2. مسارات بناء المعاملة

يوجد مساران في `arb-tx`:

### 2.1 `primitives/builder.rs::assemble_flash_arbitrage`

المسار «التقليدي» (غير ArbExecutor):

```
[0] ComputeBudget::SetComputeUnitLimit
[1] ComputeBudget::SetComputeUnitPrice
[2..] Create ephemeral ATAs (idempotent)
[..] Kamino FlashBorrow
[..] Swaps (Raydium/Orca/Meteora/Pump)
[..] (اختياري) system transfer + sync_native لتغذية WSOL
[..] Kamino FlashRepay (amount = principal)
[..] Close ephemeral ATAs (rent reclaim)
[..] Jito tip transfer
```

### 2.2 `flash/arb_executor.rs` (V1 / CPI)

المسار المفصّل للـV1 مع برنامج `ArbExecutor`:

```
[0] ComputeBudget::SetComputeUnitLimit     ← يُزال في V1
[1] ComputeBudget::SetComputeUnitPrice     ← يُزال في V1
[2] ATA create-idempotent
[3] Kamino FlashBorrow
[4] ArbExecutor::execute_three_leg (CPI إلى Meteora DLMM)
[5] Kamino FlashRepay (borrow_instruction_index مُعاد كتابته)
[6] Jito tip (اختياري)
```

> **قيد معروف (P1):** `programs/arb-executor` ينفّذ `swap2` على DLMM فقط. المسار 2.2 لا يدعم Raydium/Orca فعلياً رغم تسميته العامة.

## 3. الثوابت التنفيذية

| الثابت | القيمة | الموقع |
|---|---|---|
| Flash borrow + swaps + repay في معاملة واحدة | إلزامي | `atomic_integrity::verify_atomic_integrity` |
| سداد = أصل + رسوم (on-chain) | `repay(principal)` والرسوم تُسحب | `kamino.rs` + klend handler |
| سقف V1 | 4,096 بايت | `SOLANA_PACKET_V1_MTU` |
| حد التعليمات V1 | 64 | `MAX_V1_INSTRUCTIONS` |
| حد العناوين V1 | 64 | `MAX_V1_ADDRESSES` |
| لا ALT في V1 | عناوين مضمّنة | `compile_v1_message` |
| لا تحويل صامت V1→V0 | رفض | `compile_v1_message` |

## 4. ترتيب الحسابات وV1

`compile_v1_message`:
1. يزيل `ComputeBudget`.
2. يعيد كتابة `borrow_instruction_index` في `flash_repay`.
3. يجمع الحسابات في `BTreeMap` (OR للصلاحيات).
4. يرتّب: writable signers → readonly signers → writable non-signers → readonly non-signers.
5. يبني العناوين المضمّنة ثم قيم config ثم رؤوس التعليمات ثم payloads.
6. يضع التوقيعات في الذيل.

## 5. المسار على السلسلة (`programs/arb-executor`)

`execute_three_leg`:
- يتحقق من `leg1_min_out >= leg2_amount_in` و`leg2_min_out >= leg3_amount_in`.
- يتحقق من `leg3_min_out >= min_final_wsol_delta`.
- ينفّذ `swap2` لكل رجل عبر CPI.
- بعد كل رجل يقرأ رصيد المستخدم ويتحقق من الدلتا.
- يمنع دعم الرصيد من أرصدة سابقة (`IntermediateInventorySubsidized`).
- `min_final_wsol_delta = repay_amount (principal + fee)`.

## 6. الأمان والـfail-closed

| الحالة | السلوك |
|---|---|
| بيانات سوق قديمة | رفض (`StalePoolState` / `max_age_slots`) |
| تنفيذ بدون execution accounts موثّقة | رفض (`MissingExecutionAccounts`) |
| تجاوز حجم V1 | رفض (`SanitizationError`) |
| رسوم Kamino غير قابلة للحساب | رفض (`BorrowTooSmall`) |
| فشل محاكاة | لا إرسال |
| timeout في الإرسال | فحص الحالة أولاً، لا إعادة توقيع |

## 7. نقاط الضعف المعروفة

1. **ArbExecutor = DLMM فقط** (P1).
2. **لا ATA close / WSOL unwrap في ArbExecutor** (P2).
3. **الحارس on-chain لا يخصم priority/tip** (P2).
4. **`skip_close_intermediate_mints`** يجب تعبئتها بدقة (P1).
5. **اعتماد V1 الإنتاجي** غير مُثبت على نطاق واسع (P1).

## 8. مراجع الكود

- `crates/arb-tx/src/primitives/builder.rs`
- `crates/arb-tx/src/flash/arb_executor.rs`
- `crates/arb-tx/src/transport/adaptive.rs`
- `crates/arb-tx/src/flash/kamino.rs`
- `crates/arb-tx/src/protocols/orca.rs`
- `programs/arb-executor/src/instructions/execute_three_leg.rs`
