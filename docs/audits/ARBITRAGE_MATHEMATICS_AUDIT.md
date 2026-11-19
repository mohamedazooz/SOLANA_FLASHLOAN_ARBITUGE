# تدقيق رياضيات المراجحة (Arbitrage Mathematics)

- **النطاق:** `crates/arb-math/src/cpmm.rs`, `clmm.rs`, `economic.rs`
- **التاريخ:** 2026-11-19

## 1. رياضيات CPMM — تقييم صيغة الحجم الأمثل

الدالة `optimal_two_pool` تحلّ حالة بولين ثابتَي المنتج مع رسوم نسبية.

### الاشتقاق

لكل بول `i`:
```
out_i = (g_i / D) · Δx_i · y_i / (x_i + (g_i / D) · Δx_i)
```
حيث `g_i = D − fee_bps` و`D = 10_000`.

لرحلة ذهاب وعودة عبر بولين متعاقبين (نفس زوج العملات بترتيب معاكس):

```
f(Δ) = g1·g2·Δ·y1·x2 / [ (D·x2 + g2·Δ)(D·x1 + g1·Δ)/D ] − Δ
```

مشتقة الربح ومساواتها بالصفر تعطي:

```
Δ* = ( √(A·B) − B ) / C
```

حيث:
```
A = g1·g2·y1·x2
B = x1·y2·D²
C = D·g1·y2 + g1·g2·y1
```

وهذا مطابق تماماً لتطبيق الكود:
```rust
let d0 = d*d*x1*y2;              // B
let d1 = d*g1*y2 + g1*g2*y1;     // C
let n  = g1*g2*y1*x2;            // A
let radicand = n * d0;           // A·B
let sqrt_n_d0 = isqrt_u384(radicand);
// Δ* = (sqrt(A·B) − B) / C
```

### شروط الوجود

- `n <= d0` ⇒ لا توجد مراجحة (الفرق السعري لا يغطي الرسوم). ✅ مطبّق.
- `sqrt(A·B) <= B` ⇒ لا يوجد حل موجب. ✅ مطبّق.
- `C = 0` ⇒ لا حل. ✅ مطبّق.
- كل الحسابات في `U384` (راديكاند حتى ~293 بت) و`U512` للمسار الثلاثي (~441 بت) — لا `f64` في أي قرار مالي. ✅

### الاختبارات

- `test_optimal_two_pool_sizing_accuracy`: يتحقق أن `Δ*` هو قمة الربح (مقارنة `Δ*±1000`).
- `test_optimal_two_pool_existence_invariant`: يرفض فروقاً أقل من الرسوم، ونقطة التعادل التامة، والخسارة.
- `proptest optimal_two_pool_never_panics`: مدخلات عشوائية واسعة جداً دون panic.
- `test_u384_overflow_protection_293_bits` / `test_optimal_three_pool_sizing_and_overflow_441_bits`.

**النتيجة:** ✅ **mathematically verified** (الاشتقاق والكود متطابقان، والاختبارات خضراء).

## 2. تقييم CPMM مع الرسوم المخصومة مقدماً

`cpmm_out_upfront_fee_ceil`:

```
fee   = ceil(dx · fee_num / fee_den)
net   = dx − fee
out   = floor(net · y / (x + net))
```

هذا يطابق:
- Raydium AMM v4 (تم التحقق من `processor.rs` الرسمي: `checked_ceil_div` للرسوم، ثم `swap_token_amount_base_in` بأرضية).
- Raydium CPMM (`fee_den = 1_000_000`).
- Meteora DAMM v2 (حساب constant-product).

**النتيجة:** ✅ verified against upstream.

## 3. رياضيات CLMM (Orca Whirlpool)

### 3.1 ثوابت Tick

- `MIN_SQRT_PRICE_X64 = 4_295_048_016`
- `MAX_SQRT_PRICE_X64 = 79_226_673_515_401_279_992_447_579_055`

تم التحقق من `rust-sdk/core/src/constants/swap.rs` الرسمي:
```rust
pub const MIN_SQRT_PRICE: u128 = 4295048016;
pub const MAX_SQRT_PRICE: u128 = 79226673515401279992447579055;
```
✅ مطابق حرفياً.

### 3.2 خطوة المبادلة والرسوم

برنامج Orca الرسمي (`math/swap_math.rs`) يحسب:
```rust
let fee_amount = if amount_specified_is_input && !is_max_swap {
    amount_remaining - amount_in
} else {
    checked_mul_div_round_up(amount_in, fee_rate, FEE_RATE_MUL_VALUE - fee_rate)   // تقريب للأعلى
};
```
وكود المشروع يستخدم `fee_rate` الخام (hundredths of a bip) مع `FEE_RATE_MUL_VALUE = 1_000_000` وتقريب للأعلى. ✅

### 3.3 **عيب حرج مُصلَح**: اختيار Tick Arrays

برنامج Orca يحدّد بداية المصفوفات عبر
`SparseSwapTickSequenceBuilder::get_start_tick_indexes`
(`programs/whirlpool/src/util/sparse_swap.rs`):

```rust
let base = floor_division(tick_current, 88 * tick_spacing) * (88 * tick_spacing);
let offset = if a_to_b {
    [0, -1, -2]
} else {
    let shifted = tick_current + tick_spacing >= base + 88 * tick_spacing;
    if shifted { [1, 2, 3] } else { [0, 1, 2] }
};
```

الكود القديم كان:
- **يزحف** مصفوفة إضافية عند `a_to_b && tick_current == base` (البرنامج **لا** يفعل ذلك).
- **لا يزحف** عند `b_to_a` في الحالة الحدّية (البرنامج **يفعل**).
- لا يُسقط بدايات غير صالحة (خارج `[MIN_TICK, MAX_TICK]`).

النتيجة على mainnet: تمرير مصفوفة خاطئة ⇒ إما `InvalidTickArraySequence` أو مبادلة عبر نطاق سعري غير الذي تم تسعيره (خسارة/رفض).

**الإصلاح:** إضافة `orca_tick_array_start_indexes()` في `arb-math/src/clmm.rs` تطابق البرنامج حرفياً، واستخدامها في:
- `arb-dex/src/orca_whirlpool.rs` (`tick_array_pdas_both_directions`)
- `arb-tx/src/protocols/orca.rs` (`for_whirlpool`, `get_tick_arrays_for_swap`)
- `arb-tx/src/protocols/swap_hop.rs` (`select_tick_arrays` حسب اتجاه الرحلة)

الاختبار `orca_tick_array_window_matches_program_rules` يغطي:
`a_to_b` داخل المصفوفة، `a_to_b` عند البداية (بلا زحف)، `b_to_a` عادي، `b_to_a` عند الحدّ العلوي، وإسقاط البدايات غير الصالحة.

**الحالة:** ✅ fixed + tested.

### 3.4 **عيب حرج مُصلَح**: `sqrt_price_limit` في تعليمة Swap

الكود القديم:
```rust
sqrt_price_limit = other_amount_threshold * 2^64 / amount_in
```
وهذا **بُعد خاطئ**: `sqrt_price_limit` هو جذر سعري Q64.64 بوحدات الرمز الخام، وليس نسبة مخرجات/مدخلات. النتيجة:
- `a_to_b`: قيمة أقل بكثير من السعر الحالي ⇒ الحدّ لا يُفعّل فعلياً.
- `b_to_a`: قيمة أقل من السعر الحالي ⇒ البرنامج يرفض بـ`SqrtPriceLimitBelowPrice`.

**الإصلاح:** استخدام مظروف البرنامج الرسمي:
- `a_to_b` → `MIN_SQRT_PRICE_X64`
- `b_to_a` → `ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64`

مع الإبقاء على `other_amount_threshold` كحارس انزلاق فعلي (البرنامج يفرض `amount_out >= threshold`).

**الحالة:** ✅ fixed + compiled + tested (اختبارات الحزمة خضراء).

### 3.5 **عيب مُصلَح**: اجتياز حدود المصفوفة عند الحافة السفلية

`find_next_tick_boundary` كان يعيد `boundary == tick_current` عند `a_to_b` في بداية المصفوفة، ثم:
- إما `NoProgress` (فشل)، أو
- معالجة خاطئة تفترض وجود سيولة عند الحدّ.

**الإصلاح:** عند `a_to_b` وحافة سفلية مساوية للـtick الحالي، ينزل الكود إلى المصفوفة السابقة (`current_array_start − T`) ويكمل؛ إن خرج عن النطاق يعيد `IncompleteQuote`. الاختبار `a_to_b_lower_edge_descends_to_previous_array` يغطي ذلك.

**الحالة:** ✅ fixed + tested.

### 3.6 سياسة Fail-closed

`find_next_tick_boundary` يتطلب وجود المصفوفة الحالية (التي تحتوي `tick_current`) وإلا يعيد `MissingTickArrayCoverage`. هذا مطابق للبرنامج (الذي يحمّل مصفوفة الـtick الحالي دائماً)، ويمنع تقدير مخرجات أعلى من الواقع عند غياب سيولة غير مرئية. تم تحديث اختبارات التركيب لتوفير مصفوفة البداية الفارغة — وهذا تصحيح للـfixtures وليس إضعافاً للاختبار.

## 4. الثابت الحرج: `BorrowTooSmall`

`kamino_flash_fee_checked` يقارن **البِتات الخام قبل التقريب**:

```rust
let fee_bits = product.max(one_bits);
if fee_bits >= amount_bits { return Err(BorrowTooSmall); }
let rounded = (fee_bits + half) >> 60;
```

لا يجوز "تبسيط" هذا إلى مقارنة الرسوم بعد التقريب. ✅ محفوظ ومُختبَر.

## 5. ما لم يتم التحقق منه

| البند | الحالة |
|---|---|
| مطابقة `quote_clmm` لـ`simulateTransaction` على بول Orca حقيقي | ⚠️ not verified (يحتاج RPC) |
| دقة اجتياز أكثر من 3 مصفوفات متتالية على mainnet | ⚠️ not verified |
| رياضيات DLMM الكاملة مقابل برنامج Meteora الحقيقي | ⚠️ partial (اختبارات محلية فقط) |

## الخلاصة

| البند | الحالة |
|---|---|
| اشتقاق `Δ*` للبولين | ✅ mathematically verified |
| تقييم CPMM والرسوم | ✅ verified against upstream |
| ثوابت Tick لـOrca | ✅ verified against upstream |
| اختيار Tick Arrays | ✅ fixed + tested |
| `sqrt_price_limit` | ✅ fixed + tested |
| اجتياز الحافة السفلية | ✅ fixed + tested |
| تطابق حيّ مع mainnet | ⚠️ not verified |
