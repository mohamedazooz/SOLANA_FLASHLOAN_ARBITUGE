# تدقيق بيانات السوق والتسعير

- **النطاق:** `arb-ingest`, `arb-dex`, `arb-engine` (quote graph + freshness)
- **التاريخ:** 2026-11-19

## 1. مصادر البيانات

| المصدر | الملف | الدور |
|---|---|---|
| Helius WSS / gRPC | `crates/arb-ingest/src/helius_ws.rs` | بثّ تغيّرات الحسابات (account updates) وقياس freshness |
| Yellowstone gRPC | `arb-ingest` (feature) | تدفّق slots/accounts بزمن منخفض |
| RPC polling | `arb-ingest/src/discovery.rs` | اكتشاف البولات وتحديثها عند الحاجة |
| DexScreener / GMGN | `arb-ingest/src/dexscreener.rs`, `gmgn.rs` | بيانات مساعدة/اكتشاف العملات، **ليست** مصدر تسعير تنفيذي |

## 2. قواعد الحداثة (Freshness) — Fail closed

- `arb-engine/src/guarded.rs` يفرض `max_age_slots` قبل استخدام أي quote.
- `arb-engine/src/meteora_fresh_state.rs` يفرض `max_quote_age_slots`.
- `cross_pool_dlmm.rs` يعيد `CrossPoolError::StalePoolState` بدلاً من استخدام حالة قديمة.
- `dual_ingest.rs` يرفض الكتابات الأقدم من حيث slot/write version.

**النتيجة:** لا يوجد مسار معروف يقدّم بيانات سوق قديمة كأنها حديثة. ✅ **verified**.

### ملاحظة تشغيلية

عند فشل التدفّق الأساسي، النظام **يتوقف عن التداول** (fail closed) بدلاً من التحول إلى cache. هذا مطابق للثابت `No stale fallbacks` في `AGENTS.md`.

## 3. فكّ ترميز البولات (Decoding)

تم فحص:

### Orca Whirlpool — `crates/arb-dex/src/orca_whirlpool.rs`

| الحقل | الإزاحة | التحقق |
|---|---|---|
| `tick_spacing` | 41 | ✅ |
| `fee_rate` | 45 | ✅ (hundredths of a bip) |
| `liquidity` | 49 | ✅ |
| `sqrt_price` | 65 | ✅ |
| `tick_current_index` | 81 | ✅ |
| `token_mint_a` | 101 | ✅ |
| `token_mint_b` | 133 | ✅ |
| `token_vault_a` | 181 | ✅ |
| `token_vault_b` | 213 | ✅ |

الحجم المتوقع لحساب Whirlpool هو 653 بايت، والحد الأدنى المتحقَّق يغطي كل الإزاحات أعلاه.

### Raydium AMM v4 — `crates/arb-dex/src/raydium_v4.rs`

- الإزاحات وفكّ `fee_numerator` / `fee_denominator` صحيحة.
- `refresh_effective_reserves` يخصم `need_take_pnl_coin` و`need_take_pnl_pc` من أرصدة الخزائن، وهو ما يفعله برنامج Raydium على السلسلة فعلياً عند حساب الاحتياطي المتاح للـswap.
- **تم التحقق من كود Raydium الرسمي:** برنامج `raydium-io/raydium-amm` يحسب `swap_fee` بـ `checked_ceil_div` (تقريب للأعلى)، ويستخدم `total_coin_without_take_pnl` / `total_pc_without_take_pnl` كاحتياطي. كود المشروع يطابق ذلك: `crates/arb-dex/src/engines/raydium_v4.rs` يستخدم قسمة سقفية صريحة ثم `floor` للمخرجات.
- الحالة: ✅ **verified against upstream source**.

### Meteora DLMM

- فكّ الحسابات و`bin_step` والرسوم المتغيّرة موجود في `arb-dex` وتم اختباره عبر اختبارات `arb-math` العدائية (bin steps حتى 1000 bps، رسوم متغيّرة حتى `u32::MAX`، تقييد 10%).
- الحالة: ✅ verified (math)، ⚠️ التكامل الحيّ يحتاج RPC (لم يُشغَّل هنا).

## 4. محرّك التسعير (Quote Engine)

- `arb-dex/src/engines/*` يوفّر واجهة موحّدة `QuoteEngine`.
- `arb-math` هو المصدر الوحيد للحساب (one source of truth).
- `arb-engine` يبني رسم المسارات ويستخدم `arb-math` مباشرة؛ لا توجد نسخة حساب موازية.

### عيب تم إصلاحه: دقة `fee_rate` في Orca

كان `crates/arb-dex/src/engines/orca_whirlpool.rs` يحوّل `fee_rate` الخام (بمئات من الـbip) عبر:

```rust
let fee_rate = (state.fee_bps().unwrap() as u16).saturating_mul(100);
```

و`fee_bps()` يقسّم على 100 ويُقرّب، ثم يُضرب في 100 → **فقدان دقة** لأي `fee_rate` غير قابل للقسمة على 100 (مثلاً 3001 → 3000). النتيجة: تسعير رسوم أقل من الواقع → ربح تقديري مبالغ فيه.

**الإصلاح:** استخدام `clmm_state.fee_rate` الخام مباشرة عند توفره، والرجوع إلى `fee_bps()` فقط كـfallback موثّق مع رفض القيم غير الصالحة:

```rust
let fee_rate = if clmm_state.fee_rate != 0 {
    u32::from(clmm_state.fee_rate)
} else if let Some(bps) = state.fee_bps() {
    u32::from(bps).saturating_mul(100)
} else { return Err(...) };
if fee_rate == 0 || fee_rate >= 1_000_000 { return Err(...); }
let fee_rate = u16::try_from(fee_rate)?;
```

- **الحالة:** ✅ fixed + compiled + tested (اختبارات `arb-dex` الخضراء).

## 5. ما لم يتم التحقق منه

| البند | الحالة |
|---|---|
| مطابقة quote المشروع لـ`simulateTransaction` حقيقي على mainnet لكل بروتوكول | ⚠️ not verified (يتطلب RPC/صلاحية) |
| صحة اكتشاف البولات تحت ظروف rate-limit | ⚠️ not verified (بيئي) |
| Token-2022 transfer fees في كل المسارات | ⚠️ partial — الكود يعالجها في `token2022.rs` لكن لم تُختبر حياً |

## الخلاصة

- فكّ الترميز الأساسي (Orca/Raydium) **متحقَّق** مقابل المصادر الرسمية.
- سياسة الحداثة **fail closed** وموجودة في المسار.
- عيب دقة رسوم Orca تم إصلاحه.
- التسعير الحيّ يحتاج تحققاً على mainnet قبل أي تفعيل إنتاجي.
