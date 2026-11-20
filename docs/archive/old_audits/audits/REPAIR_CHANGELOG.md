> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

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

## Additional repairs in this session (2026-11-19, extended)

| # | Severity | File | Description |
|---|---|---|---|
| 9 | **P0** | `crates/arb-math/src/clmm.rs` | `tick_index_to_sqrt_price_x64` rewritten to official Orca semantics (positive Q96.32 via `mul_shift_96`, negative Q64.64); `MAX_SQRT_PRICE_X64` corrected to `79226673515401279992447579055`; `get_amount_delta_a` now rejects shift overflow; `compute_swap_step_checked` matches official `compute_swap` (net/fee split, max-swap fee, zero-liquidity traversal); `is_valid_tick_array_start_tick` includes left-edge array. |
| 10 | **P0** | `crates/arb-math/src/clmm.rs` | New Raydium-specific tick math: `RAYDIUM_MAX_SQRT_PRICE_X64 = 79226673521066979257578248091`, `tick_index_to_sqrt_price_x64_raydium`, `sqrt_price_x64_to_tick_index_raydium`, `ClmmTickMath` enum; `compute_swap_with_ticks_per_array_and_math` / `quote_clmm_with_ticks_per_array_and_math` so Orca and Raydium no longer share one rounding regime. |
| 11 | **P0** | `crates/arb-dex/src/raydium_clmm.rs` | Decode `fee_on` (offset 389) and `dynamic_fee_info` (offset 1096); `clmm_state()` returns `None` for dynamic-fee and fee-on-output pools; `quote()` fails closed for dynamic fee and for the fee-on-output direction per official `is_fee_on_input` mapping; fee rate beyond u16 domain rejected (not clamped). `parse_tick_array_account` offsets corrected to `data[8..12]` and `12 + i*113`. |
| 12 | **P0** | `crates/arb-ingest/src/decoders.rs` | `decode_raydium_clmm_state` returns `None` when `fee_on != 0` or `dynamic_fee_info` non-zero. |
| 13 | **P0** | `crates/arb-dex/src/raydium_v4.rs` | `refresh_effective_reserves` now returns `Result<()>` using `checked_sub` (no `saturating_sub` stale-reserve fallback), writes zero reserves instead of keeping stale ones; `is_arbitrage_eligible` accepts status 1 (`Initialized`) and 6 (`SwapOnly`), still rejects 7 (`WaitingTrade`) because `pool_open_time` cannot be proven from a snapshot. |
| 14 | **P0** | `crates/arb-dex/src/raydium_cpmm.rs` | Added `creator_fee_on` decode (offset 389); implemented official fee-on-output branch (`trade` on input, `ceil(out*creator/1e6)` off output) instead of always combining trade+creator on input; fail-closed when `AmmConfig` not loaded; fee rates validated against program maxima (not silently clamped); `amm_config_loaded` gate on `fee_bps()`. |
| 15 | **P1** | `crates/arb-dex/src/engines/raydium_clmm.rs` | Replaced Orca `quote_clmm` + bps-quantised fee with `quote_clmm_with_ticks_per_array_and_math(..., RAYDIUM_CLMM_TICK_ARRAY_SIZE, ClmmTickMath::Raydium)` and raw `clmm_state.fee_rate`; removed silent 2500 ppm default; fails closed when `fee_rate == 0`. |
| 16 | **P2** | `crates/arb-dex/src/engines/{pump_bonding_curve,pump_swap,raydium_cpmm,raydium_v4}.rs`, `token2022.rs` | Replaced manual `(n + d - 1) / d` ceil with `.div_ceil()` to satisfy clippy; fixed `raydium_clmm.rs` `fee_rate` scope error. |
| 17 | **P2** | `crates/arb-engine/src/three_pool_router.rs` | Flash-loan fee uses `.div_ceil()` with `u64::try_from` (no truncation of a cost downward); tip uses `.clamp()`; `KaminoBorrowOptimizer: Default`. |
| 18 | **P2** | `crates/arb-bot/src/supervisor/dispatch.rs`, `supervisor.rs` | Removed needless `return;` in Observe/Analyze arms; introduced `OpenedStore` type alias for `type_complexity`. |
| 19 | **P2** | `crates/arb-store/src/postgres_store.rs` | Removed redundant `as i32` cast. |
| 20 | **test** | `crates/arb-math/tests/{orca_sdk_parity,raydium_clmm_tick_parity_tests,clmm_tick_array_layout_tests}.rs` | Exact-equality expectations vs official SDK bit-tables; new 6-test layout suite. |
| 21 | **test** | `crates/arb-dex/src/raydium_cpmm.rs` (mod tests) | 8 new unit tests covering creator-fee-on-output branches, fail-closed modes, config validation. |

### Verification evidence (this session)

- `cargo fmt --all -- --check`: clean (exit 0).
- `cargo check --workspace --all-targets`: exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: blocked on Windows by `protobuf-src v2.1.1+27.1` custom build (environmental, Linux-only dependency chain); documented, not a code defect.
- `cargo test --workspace`: all suites pass, 0 failures (arb-math 156 unit + integration suites; arb-dex 50 lib tests; arb-bot 78 supervisor tests; etc.).
- Live Kamino reserve offsets re-verified via read-only RPC (`artifacts/forensics/verify_kamino_reserve_offsets.py`): discriminator `[43,242,204,202,26,247,59,127]`, length 8,624, WSOL mint, supply vault `GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U`, fee vault `3JNof8s453bwG5UqiXBLJc77NRQXezYYEBbk3fqnoKph`, status 0 (Active), `flash_loan_fee_sf = 11529215046068` (1e-5), supply 272,151.78 SOL.
- Live Orca tick-array layout verified on mainnet pool `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` (all 88 ticks initialized, trailing pool pubkey matches).
- Reported transaction `s9eVzA5J…4js` decoded: V1, 235 bytes, slot 471383273, CU 17,724, fee 6,000 lamports, **Memo-only canary — not an arbitrage**. Classification unchanged: VERIFIED NON-ARBITRAGE TEST TRANSACTION.

## What remains unverified / blocked

- `--all-features` clippy on Windows (protobuf-src build). Run on Linux per `scripts/verify-linux.sh`.
- No second independent RPC source tested (rate-limit risk).
- Live Kamino `mint_decimals` re-read at offset 272 (8+264) still pending in the probe script (current script reads offset 264); code constant `KAMINO_RESERVE_MINT_DECIMALS_OFFSET = 8 + 264` is correct per layout.
- `ArbExecutor` supports DLMM only (documented, P1).
- ATA close assumes SPL Token not Token-2022 (documented, P3).
- `AdaptiveBidder` uses `f64` in `keep_share`/`max_tip_share` (documented, P2).
- `build_close_token_account` assumes SPL Token (P3).
