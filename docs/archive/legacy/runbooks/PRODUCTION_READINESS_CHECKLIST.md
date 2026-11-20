> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# قائمة الجاهزية للإنتاج (Production Readiness Checklist)

- **آخر تحديث:** 2026-11-19
- **الحالة الحالية:** ❌ **غير جاهز للإنتاج** — انظر `docs/audits/FINAL_VERIFICATION_REPORT.md`.

## 0. الحكم

| المجال | الحالة |
|---|---|
| البناء والاختبارات | ✅ 1,301 نجاح / 0 فشل (Windows) |
| الرياضيات | ✅ متحقَّق منها |
| V1 transport | ⚠️ canary واحد فقط |
| المسار على السلسلة | ⚠️ DLMM فقط |
| الربحية | ❌ غير مُثبتة |
| الأمان التشغيلي | ⚠️ يحتاج ضوابط زمن |

## 1. الشروط التي **يجب** أن تتحقق قبل `ARB_LIVE=1`

### 1.1 صحة مالية

- [ ] محاكاة `simulateTransaction` على mainnet لمسار **ثلاثي الأرجل فعلي** لكل بروتوكول مدعوم (Raydium v4، Orca، Meteora DLMM/DAMM، PumpSwap).
- [ ] التحقق من `postTokenBalances` فعلياً: دلتا > 0 بعد كل التكاليف.
- [ ] التحقق من أن `min_final_wsol_delta` يخصم صراحةً priority fee + Jito tip (P2-6).
- [ ] التحقق من `account_creation` مقابل rent الفعلي.

### 1.2 صحة التنفيذ

- [ ] حل P1-3: `ArbExecutor` يدعم DLMM فقط. إمّا توسيعه أو توجيه المسارات غير DLMM إلى `assemble_flash_arbitrage`.
- [ ] إغلاق ATAs وفكّ WSOL بعد repay في `arb_executor` (P2-2/P2-3).
- [ ] مراجعة `skip_close_intermediate_mints`: لا تُغلق ATA تحمل أرصدة (P1-4).
- [ ] التحقق من `borrow_instruction_index` بعد إزالة ComputeBudget في V1 (اختبار موجود، لكن يحتاج تأكيد mainnet).

### 1.3 الحالة والبيانات

- [ ] تشغيل `latency_bench` في بيئة Linux مع `.env` وقياس زمن هبوط حقيقي.
- [ ] إضافة budget enforcement لزمن ما قبل الإرسال (P2-5).
- [ ] التحقق من `TipFloor` مقابل Jito الحيّ.
- [ ] التحقق من Token-2022 transfer fees من حالة mainnet.

### 1.4 التزامن والاسترداد

- [ ] اختبار `Supervisor::recovery` على timeout حقيقي (لا إعادة توقيع).
- [ ] التحقق من `max_age_slots` تحت rate-limit.

### 1.5 الأمان التشغيلي

- [ ] `WEB_ADMIN_PASSWORD` مضبوط، والـAPI على localhost خلف reverse proxy.
- [ ] `.env` غير متعقَّب في git، ولا يُسجَّل.
- [ ] `ARB_LIVE` و`DRY_RUN` يُضبطان صراحةً، لا افتراضياً.
- [ ] Kill switch مُختبَر.
- [ ] حدود الحجم (`MAX_BORROW`, `MIN_NET_PROFIT_USD`) مضبوطة ومُراجعة.

## 2. الاختبارات المطلوبة قبل التفعيل

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # يجب أن يمرّ (حالياً يفشل بسبب P3)
cargo test --workspace --all-targets
cargo run --bin latency_bench                            # Linux + .env
```

## 3. التفعيل التدريجي (بعد اجتياز ما سبق)

1. **Shadow:** `ARB_LIVE=0`، مراقبة الفرص دون إرسال (24 ساعة على الأقل).
2. **Canary:** مبالغ صغيرة جداً، `MIN_NET_PROFIT_USD` مرتفع، حد أقصى للاقتراض منخفض.
3. **توسيع:** زيادة تدريجية فقط بعد ربح محقَّق موثَّق.

## 4. قائمة منع

- ❌ تفعيل `ARB_LIVE=1` بناءً على تقرير تدقيق فقط.
- ❌ تفعيل بدون محاكاة mainnet لمسار فعلي.
- ❌ تفعيل مع `clippy -D warnings` فاشل دون مراجعة سبب الفشل.
- ❌ تفعيل مع `ArbExecutor` DLMM-only على مسار غير DLMM.
