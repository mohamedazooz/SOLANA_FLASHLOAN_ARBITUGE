# التدقيق الهندسي المستقل للمرحلة 17: مسار التنفيذ الذري عبر المنصات (Phase 17 Independent Audit)
## Meteora DLMM → Raydium CPMM → Kamino Flash Loan V1 Execution

**تاريخ التدقيق**: 2026-11-20  
**الدور**: كبير مهندسي النظم وهندسة الأنظمة الكمية على سولانا (Principal Solana & Quant Systems Engineer)  
**الحكم النهائي الصارم (Final Verdict)**: **CONDITIONALLY VERIFIED — BLOCKERS REMAIN**  
(تم التحقق المشروط — مسار المشرف والتجميع الداخلي سليم تماماً، ولكن توجد عوائق تشغيلية حاسمة تمنع النشر الحي المباشر قبل تصحيحها).

---

## 1. الملخص التنفيذي وحكم التدقيق (Executive Summary & Verdict)

بصفتي مدققاً هندسياً مستقلاً، قمت بإجراء فحص شامل وصارم لجميع أسطر الشيفرة البرمجية، واختبارات التكامل، وتدفق البيانات، والمصفوفة الحسابية، وحجم الحزم المجمعة، دون الاعتماد على التقارير السابقة أو مجرد اجتياز الاختبارات.

### الحكم النهائي:
```text
================================================================================
VERDICT: CONDITIONALLY VERIFIED — BLOCKERS REMAIN
================================================================================
```

### الأسباب الجوهرية للحكم المشروط:
1. **عائق حجم الحزمة السلكية (BLOCKER)**: المعاملة المجمعة بدون جداول العناوين (ALT) يبلغ حجمها **1,248 بايت**. هذا الحجم يتجاوز حد الحزمة القياسي لشبكة سولانا (`PACKET_DATA_SIZE = 1,232 بايت`) بفارق 16 بايت. على الرغم من أن المعاملة مطابقة لسقف مقترح V1 النظري (4,096 بايت)، فإن إرسالها عبر قنوات TPU / UDP أو حزم Jito التقليدية التي تطبق معيار 1232 بايت سيؤدي إلى إسقاط الحزمة على مستوى المقبس (Socket Layer). يتطلب هذا العائق تفعيل ضغط جداول العناوين (ALT Compression) لتقليص الحجم إلى ~380 بايت قبل البث الحي.
2. **حدود الحماية الذرية للأرباح (HIGH)**: حماية رأس المال مضمونة على السلسلة من خلال ارتداد القرض السريع (Flash Loan Rollback) إذا فشلت المبادلة في توفير حد السيولة الأدنى (`min_amount_out`). ومع ذلك، فإن رسوم الشبكة (Base Fee + Priority Fee) تُخصم مسبقاً من رصيد الدافع (`payer`) ولا يتم استردادها عند تراجع المعاملة. لا يوجد برنامج مخصص لفحص رصيد المحفظة النهائي على السلسلة (On-chain Balance Assertion Program)، والاعتماد بالكامل على قيد الانزلاق في ساق المبادلة الأخيرة.
3. **تصنيف الاختبارات بدقة (MEDIUM)**: تم تصنيف اختبارات حقن الفشل في `arb-tx` كاختبارات وحدة هيكلية (Static Unit Tests) تتحقق من ترتيب التعليمات ودوال الفحص الذاتي، وليست محاكاة تشغيلية داخل جهاز افتراضي (LiteSVM / Local Validator).

---

## 2. التدقيق الهيكلي لمسار المشرف (Real Supervisor Call-Graph Audit)

### المسار الإنتاجي المعتمد:
تم تتبع مسار الاستدعاء الحقيقي للتأكد من أن المجمّع المتخصص يتم استدعاؤه تلقائياً في دورة الإنتاج وليس في بيئة الاختبارات فقط:

1. **دخول الفرصة**: `Supervisor::process_opportunities` في [opportunity.rs#L30-L54](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-bot/src/supervisor/opportunity.rs#L30-L54).
2. **فحص التبعيات والحداثة**: استدعاء `validate_route_account_dependencies` في [opportunity.rs#L94](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-bot/src/supervisor/opportunity.rs#L94).
3. **تجميع التعليمات**: يتم استدعاء `assemble_native_ixs` في [opportunity.rs#L995](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-bot/src/supervisor/opportunity.rs#L995).
4. **التفويض للمجمّع المتقاطع المتخصص**:
   في [execution.rs#L27-L40](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-bot/src/supervisor/execution.rs#L27-L40):
   ```rust
   if arb_tx::cross_dex_flash_arb::is_cross_dex_opportunity(opp) {
       return arb_tx::cross_dex_flash_arb::assemble_cross_dex_opportunity(
           payer,
           tip_account,
           tip,
           cu_limit,
           cu_price,
           opp,
           kamino_params,
           flash_loan_fee_sf,
           min_profit_raw,
           &skip_close,
       );
   }
   ```
5. **التحقق من عدم إمكانية التجاوز (No Accidental Override)**:
   - دالة `is_cross_dex_opportunity` تفحص ما إذا كان المسار يحتوي على منصتين مختلفتين على الأقل (`platforms.len() >= 2`).
   - في حال تطابق الشرط، يتم الخروج فوراً (`return`) عبر المجمّع المتخصص، مما يمنع سقوط المسار في المجمّع التقليدي الفردي.
   - في حال فشل التجميع أو عدم تطابق الحسابات، تُرجع الدالة `Err`، ويتوقف المشرف عن معالجة الفرصة مع إغلاق مغلق (`fail-closed`).

---

## 3. التدقيق الهندسي لتعليمات كامينو (Kamino Instruction Correctness)

تمت مطابقة مصفوفة الحسابات والبيانات مع واجهة برنامج `klend` الرسمية على شبكة سولانا:
- **معرّف البرنامج**: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` ([kamino.rs#L2](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-tx/src/flash/kamino.rs#L2)).
- **حسابات تعليمات الاقتراض السريع (Flash Borrow)**:
  - عدد الحسابات: **12 حساباً بالضبط** مطابقة لـ Anchor IDL.
  - حجم حمولة البيانات: **16 بايت** (8 بايت معرّف التمييز `FLASH_BORROW_DISCRIMINATOR` + 8 بايت لمبلغ القرض `amount` بصيغة Little-Endian).
  - الترتيب: `user_authority`, `lending_market_authority`, `lending_market`, `reserve`, `reserve_liquidity_mint`, `reserve_supply_vault`, `user_liquidity_ata`, `reserve_fee_vault`, `referrer_token_state`, `referrer_account`, `sysvar_instructions`, `token_program`.
- **حسابات تعليمات سداد القرض السريع (Flash Repay)**:
  - عدد الحسابات: **12 حساباً**.
  - حجم حمولة البيانات: **17 بايت** (8 بايت معرّف التمييز `FLASH_REPAY_DISCRIMINATOR` + 8 بايت لمبلغ السداد + 1 بايت لمؤشر تعليمة الاقتراض `borrow_instruction_index`).
  - التحقق من المطابقة: يمرر المشرف `borrow_idx as u8` بدقة لتمكين تعليمة السداد من فحص تعليمة الاقتراض عبر الـ Sysvar Instructions.
  - التحقق من المبلغ: يطابق مبلغ السداد أصل القرض المستلف تماماً، ويتم تحصيل الرسوم تلقائياً فوق الأصل من نفس حساب الـ ATA.
  - معالجة رسوم كامينو: مطبقة بصيغة النقطة الثابتة `U68F60` مع 60 بت كسري لمنع أخطاء التقريب.

---

## 4. تدقيق الحماية الذرية للأرباح (Atomic Profit Protection Audit)

### آلية التطبيق على السلسلة:
تم فحص الكيفية التي تضمن بها المعاملة عدم تراجع الأرباح الصافية دون الحد الأدنى المطلوب:
- في [builder.rs#L318-L355](file:///c:/Users/azoozz/Downloads/my%20arbitrge/crates/arb-tx/src/primitives/builder.rs#L318-L355):
  $$\text{floor} = \text{borrow} + \text{flash\_fee\_raw} + \text{tip\_raw} + \text{min\_profit\_raw}$$
- في ساق المبادلة الأخيرة (Raydium CPMM):
  يتم ضبط `min_amount_out` إلى:
  $$\text{min\_amount\_out} = \max(\text{min\_out\_slippage}, \text{floor})$$
- **الضمان على السلسلة**:
  إذا أنتجت المبادلة الثانية مبلغاً أقل من `floor`، يفشل برنامج Raydium CPMM على السلسلة بسبب خرق الانزلاق (`ExceededSlippage`).
  يؤدي هذا الفشل إلى ارتداد المعاملة بأكملها، وتراجع تعليمة اقتراض كامينو، مما يحمي أصل القرض بالكامل بنسبة 100%.

### الفجوات الهندسية والمخاطر المحددة (Identified Gaps):
1. **خسارة رسوم المعاملة عند الارتداد**:
   - لا تحمي المعاملة الذرية من خسارة رسوم الشبكة الأساسية (5,000 لامبورت) ورسوم الأولوية (`cu_limit * cu_price`).
   - تخصم هذه الرسوم مباشرة من حساب الدافع (`payer`) بواسطة بيئة تشغيل سولانا قبل بدء تنفيذ المعاملة، ولا تُسترد عند فشل أي تعليمة.
   - بقشيش Jito محفوظ لأن تعليمة التحويل تقع في نهاية المعاملة وتتراجع تلقائياً عند أي فشل سابق.
2. **غياب برنامج فحص الرصيد الصافي المخصص**:
   - لا يوجد برنامج On-chain Balance Assertion مستقل يقوم بمقارنة رصيد المحفظة قبل وبعد المعاملة، والاعتماد قائم على فحص المخرجات الدنيا في مسبح المبادلة الأخير.

---

## 5. تدقيق سلامة الحسابات ونافذة الحداثة (Account & Slot Integrity)

- **مصفوفات خانات Meteora DLMM**:
  - يتم التحقق من وجود مصفوفة الخانات النشطة (`bin_array_pdas`) في متجر اللقطات.
  - يتم استخراج وتمرير الحسابات الإلزامية: `reserve_x`, `reserve_y`, `oracle`, `pool_token_x`, `pool_token_y`.
- **حسابات Raydium CPMM**:
  - يتم استخراج وتمرير `amm_config`, `vault_0`, `vault_1`, وحساب المراقبة `observation_state`.
  - يتم تحديد اتجاه المبادلة برمجياً عبر المقارنة البايتية الصارمة للمفاتيح:
    `let a_to_b = hop.mint_in.0.to_bytes() < hop.mint_out.0.to_bytes();`
    وهو الترتيب الإلزامي لمسابح Raydium CPMM.
- **نافذة حداثة اللقطات (`max_state_age_slots = 5`)**:
  - يتم تطبيق القيد مرتين:
    1. في مرحلة فحص التبعيات: `validate_route_account_dependencies` ترفض أي حساب عمره > 5 سلوت.
    2. في مرحلة ما قبل الإرسال الفوري (`GUARD-01` داخل `dispatch.rs`): يتم فحص الفارق الزمني وإلغاء الإرسال إذا تقادم المسبح.

---

## 6. تدقيق حزم V1 وحجم المعاملة (V1 Transaction Validation)

### العائق الحرج: فجوة الـ 16 بايت في حجم الحزمة
- تم قياس المعاملة المجمعة V1 الناتجة عن المسار:
  $$\text{Wire Size} = 1,248 \text{ بايت}$$
- سقف حزمة سولانا القياسي للشبكة عبر UDP/TPU:
  $$\text{PACKET\_DATA\_SIZE} = 1,232 \text{ بايت}$$
- **النتيجة الهندسية**:
  المعاملة تتجاوز حد الـ TPU بـ 16 بايت ($1,248 > 1,232$).
  على الرغم من أن المعاملة تتوافق مع سقف مقترح V1 الأقصى (4,096 بايت المحدد في `SOLANA_PACKET_V1_MTU`)، فإن عقد المدققين (Validators) التي لا تزال تستقبل الحزم عبر مآخذ UDP العادية ستسقط الحزمة فوراً بسبب تجزئة حزم IPv6 MTU.

### الحل المطلوب للإطلاق الحي:
تطبيق ضغط جداول العناوين (Address Lookup Tables - ALT):
- تتضمن المعاملة 28 حساباً مدمجاً تأخذ $28 \times 32 = 896$ بايت.
- عند استبدالها بفهارس ALT ذات البايت الواحد ($28 \times 1 = 28$ بايت)، يتم توفير **868 بايت**.
- يصبح حجم المعاملة الفعلي **~380 بايت** فقط، وهو ما يقع بأمان تام تحت سقف الـ 1,232 بايت.

---

## 7. تدقيق الاستعادة وإعادة التشغيل (Recovery & Reconciliation Audit)

- **حفظ نوايا الإرسال (Dispatch Intent)**:
  - يسجل المشرف محاولة الإرسال في جدول `dispatch_intents` متضمناً: `attempt_id`, `signature`, `last_valid_block_height`, و `dispatch_slot`.
- **المصالحة عند إعادة التشغيل**:
  - دالة `reconcile_pending_dispatches` تفحص حالة المعاملة عبر `getSignatureStatuses`.
  - إذا لم تكن المعاملة مؤكدة، تقارن السلوت الحالي بـ `last_valid_block_height`.
  - إذا انتهت صلاحية الكتلة، تصنف المعاملة كـ `expired_unconfirmed`.
  - **قاعدة الأمان الصارمة**: لا يتم أبداً إعادة توقيع أو إعادة إرسال معاملة منتهية الصلاحية دون إثبات عدم هبوطها على السلسلة.

---

## 8. تدقيق وتصنيف مصفوفة الاختبارات الـ 14 (Test Matrix Classification)

بموجب تعليمات التدقيق الصارمة، تم فحص كل اختبار وتصنيفه وفق طبيعته الهندسية الحقيقية:

| # | اسم الاختبار (Test Name) | التصنيف الحقيقي (Type) | النتيجة | الملاحظات الهندسية |
|:---:|:---|:---:|:---:|:---|
| **1** | `test_matrix_01_profitable_dlmm_to_cpmm_v1_compilation` | **Unit** | PASS | يتحقق من تجميع بايتات V1 وترتيب التعليمات |
| **2** | `test_matrix_02_unprofitable_candidate_rejected` | **Unit** | PASS | يتحقق من رفض حساب المخرجات الدنيا |
| **3** | `test_matrix_03_missing_dlmm_bin_array_fails_closed` | **Unit** | PASS | يتحقق من رفض غياب الحسابات التنفيذية |
| **4** | `test_matrix_07_08_09_failure_simulation_and_atomicity_rollback` | **Unit** | PASS | **فحص هيكلي ثابت**؛ لا ينفذ كود eBPF داخل VM |
| **5** | `test_matrix_10_minimum_profit_guard_enforced_in_swap_legs` | **Unit** | PASS | فحص حسابي لمعادلة الربح الأدنى والانزلاق |
| **6** | `test_matrix_11_v1_packet_size_boundary` | **Unit** | PASS | فحص حجم 1,248 بايت مقابل سقف 4,096 |
| **7** | `test_matrix_12_compute_budget_boundary` | **Unit** | PASS | فحص سقف وحدات الحساب الديناميكية |
| **8** | `test_cross_dex_flash_arb_params_direct_assembly` | **Unit** | PASS | فحص المجمّع المباشر لمعاملات الفلاش عبر المنصات |
| **9** | `test_matrix_01_supervisor_full_cross_dex_shadow_reconciliation` | **Integration** | PASS | تدفق المشرف الكامل، وتخزين SQLite في وضع المحاكاة |
| **10** | `test_matrix_02_supervisor_unprofitable_candidate_rejected` | **Integration** | PASS | رفض المشرف الكامل للمرشح الخاسر |
| **11** | `test_matrix_03_supervisor_missing_account_dependency_fails_closed` | **Integration** | PASS | رفض المشرف عند فقدان حساب المسبح |
| **12** | `test_matrix_04_supervisor_stale_account_snapshot_fails_closed` | **Integration** | PASS | رفض المشرف للقطات الحسابات المتقادمة (> 5 سلوت) |
| **13** | `test_matrix_05_supervisor_token2022_transfer_fee_fails_closed` | **Integration** | PASS | رفض المشرف لرسوم تحويل الرموز |
| **14** | `test_matrix_06_insufficient_kamino_liquidity_rejected` | **Integration** | PASS | رفض المشرف لاقتراض يتجاوز مخزن كامينو |
| **15** | `test_matrix_13_supervisor_duplicate_dispatch_deduplication` | **Integration** | PASS | تصفية المرشحات المكررة لنفس مسبح المبادلة |
| **16** | `test_matrix_14_restart_unresolved_dispatch_intent_recovery` | **Integration** | PASS | استعادة النوايا ومصالحتها دون إعادة إرسال |

---

## 9. سجل الملاحظات والعيوب المكتشفة (Audit Findings Register)

### FINDING-01: حجم الحزمة السلكية يتجاوز حد TPU القياسي (1,232 بايت)
- **مستوى الخطورة**: **BLOCKER (لبث TPU الحي)** / **MEDIUM (لوضع المحاكاة والظل)**
- **الملف والسطر**: `crates/arb-tx/src/flash/cross_dex_flash_arb.rs:607`
- **التفاصيل**: الحزمة المجمعة بحجم **1,248 بايت** تتجاوز سقف `PACKET_DATA_SIZE = 1232` بـ 16 بايت.
- **التصحيح الإلزامي**: دمج جداول العناوين (ALT) لتقليص الحجم إلى ~380 بايت قبل تفعيل البث على الشبكة الحية.
- **الحالة**: **مفتوح (OPEN) بانتظار دمج ALT**.

### FINDING-02: خسارة رسوم الشبكة الأساسية ورسوم الأولوية عند تراجع المعاملة
- **مستوى الخطورة**: **HIGH (مخاطر تشغيلية ورأس مال)**
- **الملف والسطر**: `crates/arb-tx/src/primitives/builder.rs:316-355`
- **التفاصيل**: الارتداد الذري يمنع خسارة أصل قرض كامينو، ولكنه لا يسترد رسوم المعاملة (5,000 لامبورت) ورسوم الأولوية المدفوعة للمدقق.
- **التصحيح الإلزامي**: تحديث نماذج إدارة المخاطر وتنبيهات التليجرام لتعكس أن الفشل يحرق رسوم التوقيع والأولوية.
- **الحالة**: **محدد وموثق (DOCUMENTED)**.

### FINDING-03: عدم تنفيذ محاكاة تشغيلية داخل بيئة eBPF في اختبارات الوحدة
- **مستوى الخطورة**: **MEDIUM (دقة التوثيق)**
- **الملف والسطر**: `crates/arb-tx/tests/cross_dex_dlmm_cpmm_v1_atomic_tests.rs:262`
- **التفاصيل**: الاختبار تم وصفه سابقاً كمحاكاة ارتداد، بينما هو اختبار وحدة هيكلي لخوارزمية `verify_atomic_integrity`.
- **التصحيح الإلزامي**: تم تصحيح الوصف الهيكلي في تقرير التدقيق المستقل.
- **الحالة**: **تم التصحيح في هذا التقرير (RESOLVED_IN_AUDIT)**.

---

## 10. أوامر إعادة الإنتاج المستقلة (Reproduction Commands)

تم التحقق من تشغيل الأوامر بالكامل ونجاحها دون أي أخطاء أو تحذيرات:

```powershell
# 1. تدقيق تنسيق الشيفرة البرمجية
cargo fmt --all -- --check

# 2. تدقيق التحذيرات عبر Clippy (سياسة 0 تحذيرات)
cargo clippy -p arb-tx -p arb-bot --all-targets -- -D warnings

# 3. تشغيل اختبارات التجميع الذري V1 في arb-tx
cargo test -p arb-tx --test cross_dex_dlmm_cpmm_v1_atomic_tests

# 4. تشغيل اختبارات تكامل المشرف وإعادة التشغيل في arb-bot
cargo test -p arb-bot --test cross_dex_dlmm_cpmm_supervisor_execution
```

---

## 11. الخاتمة والتوصيات (Final Recommendations)

1. **جاهزية وضع الظل والمحاكاة**: مسار التنفيذ عبر المشرف، واكتشاف السيولة، وحساب الرسوم، وتجميع المعاملات جاهز تماماً للعمل في بيئة المحاكاة وظل الإنتاج (`Simulate / Shadow Mode`).
2. **شرط الانتقال للبث الحي الحقيقي (Mainnet Pilot Gate)**:
   يُحظر تفعيل `ARB_LIVE=1` لهذا المسار المتقاطع تحديداً حتى يتم حل **FINDING-01** من خلال ربط جداول العناوين (ALT) في المجمّع المتخصص لضمان عدم تجاوز حجم الحزمة السلكية حاجز الـ 1,232 بايت.
