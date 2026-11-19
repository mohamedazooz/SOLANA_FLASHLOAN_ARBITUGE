# سجل الإصلاحات (Repair Changelog)

- **الفرع:** `audit/forensic-repair-v1`
- **الأساس:** `b7e2970`
- **التاريخ:** 2026-11-19
- **سياسة:** لم يُنفَّذ أي transaction حقيقي، ولا تغيير مفاتيح، ولا تفعيل live.

## ملخّص

| # | الخطورة | الملف | الوصف |
|---|---|---|---|
| 1 | **P0** | `crates/arb-math/src/clmm.rs` | إضافة `orca_tick_array_start_indexes` ومطابقة قاعدة البرنامج لاختيار Tick Arrays |
| 2 | **P0** | `crates/arb-tx/src/protocols/orca.rs` | تصحيح `sqrt_price_limit` (كان بُعده خاطئاً) |
| 3 | **P0** | `crates/arb-dex/src/orca_whirlpool.rs` | اشتقاق نافذة PDAs للاتجاهين بدلاً من الاعتماد على ترتيب الوصول |
| 4 | **P0** | `crates/arb-tx/src/protocols/swap_hop.rs` | اختيار نصف نافذة Tick Arrays حسب اتجاه الرحلة |
| 5 | **P1** | `crates/arb-dex/src/engines/orca_whirlpool.rs` | استخدام `fee_rate` الخام بدل `fee_bps()*100` (فقدان دقة) |
| 6 | **P1** | `crates/arb-math/src/clmm.rs` | اجتياز الحافة السفلية لمصفوفة الـticks دون اختلاق tick |
| 7 | **P2** | `crates/arb-tx/src/flash/kamino.rs` | referrer optional accounts يجب أن تكون writable عند توفيرها |
| 8 | **test** | `crates/arb-math/tests/*` | إصلاح fixtures لتوفير مصفوفة الـtick الحالية (مطابقة للبرنامج) |

## التفاصيل

### 1. اختيار Tick Arrays لـOrca (P0)

**المشكلة:** الكود القديم كان يزيح مصفوفة عند `a_to_b && tick_current == base` (البرنامج لا يفعل)، ولا يزيح عند `b_to_a` في الحالة الحدّية (البرنامج يفعل)، ولا يُسقط البدايات غير الصالحة. النتيجة: `InvalidTickArraySequence` أو مبادلة عبر نطاق سعري مختلف عن التسعير.

**الإصلاح:** دالة جديدة `orca_tick_array_start_indexes(tick_current, tick_spacing, a_to_b)` في `arb-math/src/clmm.rs` تطابق `SparseSwapTickSequenceBuilder::get_start_tick_indexes` حرفياً. استُخدمت في:
- `arb-dex/src/orca_whirlpool.rs::tick_array_pdas_for_swap`
- `arb-dex/src/orca_whirlpool.rs::tick_array_pdas_both_directions` (نشر 6 PDAs)
- `arb-tx/src/protocols/orca.rs::for_whirlpool` و`get_tick_arrays_for_swap`
- `arb-tx/src/protocols/swap_hop.rs::select_tick_arrays`

**الاختبار:** `orca_tick_array_window_matches_program_rules` (5 حالات).

### 2. `sqrt_price_limit` (P0)

**المشكلة:** `limit = threshold * 2^64 / amount_in` — بُعد خاطئ. `a_to_b` كان يُبطل الحدّ، و`b_to_a` كان يرفض بـ`SqrtPriceLimitBelowPrice`.

**الإصلاح:** استخدام مظروف البرنامج الرسمي:
- `a_to_b` → `arb_constants::CLMM_MIN_SQRT_PRICE_X64`
- `b_to_a` → `arb_constants::ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64`

مع بقاء `other_amount_threshold` كحارس انزلاق فعلي.

### 3. نوافذ Tick Arrays في الـadapter (P0)

**المشكلة:** `execution_accounts()` كان يستخدم أول 3 PDAs كما وصلت (ترتيب وصول عشوائي) أو يشتق نافذة `a_to_b` فقط.

**الإصلاح:** `tick_array_pdas_both_directions()` تُشتق دائماً من حالة البول، وتنشر `[a_to_b ×3][b_to_a ×3]`. عند غياب 6، يُقبل window قديم من 3 مع توثيق أنه pre-ordered.

### 4. اختيار الاتجاه في swap builder (P0)

**المشكلة:** كان يستخدم `tick_array_pdas[0..3]` دائماً، بصرف النظر عن `a_to_b`.

**الإصلاح:** `OrcaWhirlpoolAccounts::select_tick_arrays(pdas, a_to_b)` تختار النصف المطابق، وترفض zero pubkey.

### 5. دقة رسوم Orca (P1)

**المشكلة:** `fee_bps()` (bps صحيحة) ثم `*100` → تقريب مزدوج يفقد hundredths-of-bip.

**الإصلاح:** `clmm_state.fee_rate` الخام أولاً، مع رفض `0` أو `>= 1_000_000`، وfallback موثّق فقط عند غياب القيمة.

### 6. اجتياز الحدّ السفلي (P1)

**المشكلة:** عند `a_to_b` و`boundary == tick_current`، إمّا `NoProgress` أو إعادة tick غير موجود.

**الإصلاح:** النزول إلى المصفوفة السابقة `current_array_start − T`، والاستمرار؛ عند الخروج من النطاق → `IncompleteQuote`.

**الاختبار:** `a_to_b_lower_edge_descends_to_previous_array`.

### 7. Kamino referrer metas (P2)

**المشكلة:** `referrer_token_state` و`referrer_account` كانا دائماً `readonly` حتى عند توفيرهما.

**الإصلاح:** `match` → `AccountMeta::new(key, false)` عند `Some`، و`program_id` readonly عند `None`.

**مطابقة:** `Kamino-Finance/klend` → `libs/klend-interface/src/instructions/flash.rs` يستخدم `optional_account(..., true)`.

### 8. إصلاح Fixtures (test-only)

اختبارات `adversarial_challenge_tests.rs` و`integration_tests.rs` كانت توفّر مصفوفة `start_tick_index = -5632` فقط بينما الـtick الحالي = 0. البرنامج يحمّل مصفوفة الـtick الحالي (start = 0) دائماً. أُضيفت مصفوفة فارغة عند 0 إلى الـfixtures. هذا **تصحيح** للاختبار وليس إضعافاً: سياسة fail-closed بقيت كما هي.

## ما لم يُغيَّر (مقصود)

- `crates/arb-bot/src/config.rs` و`arb-config`: إعدادات المشغّل لم تُلمس.
- `.env` / أي سر: لم يُقرأ أو يُطبع.
- `ARB_LIVE` / `DRY_RUN`: بقيت كما هي.
- `reports/phase15_5/*`: تغيّرت آلياً عند تشغيل الاختبارات فقط؛ يجب `git checkout --` قبل الإيداع.

## عيوب موثّقة لم تُصلَح

| # | الخطورة | الوصف |
|---|---|---|
| A | P1 | `ArbExecutor` يدعم DLMM فقط، بينما المسار المعلن متعدد البروتوكولات |
| B | P2 | `arb_executor` لا يغلق ATAs ولا يفكّ WSOL بعد repay (rent locked) |
| C | P2 | `AdaptiveBidder` يستخدم `f64` في `keep_share`/`max_tip_share` |
| D | P2 | لا يوجد budget enforcement لزمن ما قبل الإرسال |
| E | P2 | الحارس on-chain لا يخصم priority fee/tip صراحةً من `min_final_wsol_delta` |
| F | P3 | `build_close_token_account` يفترض SPL Token وليس 2022 |
| G | P1 | الإغلاق قد يُدمّر ATA موجودة مسبقاً إذا لم تُملأ `skip_close_intermediate_mints` |

## التحقق

- `cargo check --workspace --all-targets` ✅
- `cargo test -p arb-math` ✅ (كل الاختبارات)
- `cargo test -p arb-dex -p arb-tx` ✅
- تفاصيل الأوامر والنتائج في `FINAL_VERIFICATION_REPORT.md`.
