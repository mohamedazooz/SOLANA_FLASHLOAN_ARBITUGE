# تقرير تدقيقي وإصلاحات — SOLANA_FLASHLOAN_ARBITUGE (2026-11-13)

## 1) ملخص تنفيذي
- **العَرَض الأساسي:** `opportunities_total = 0` وتشغيل حي دون فرص/تنفيذ.
- **السبب المرجّح من الكود واللوج:** دورات الاستراتيجية تُهمل fail-closed قبل مرحلة معالجة المرشحين بسبب (أ) تعذر/قدَم المدخلات الاقتصادية المؤكَّدة، (ب) غياب سعر SOL/USDC حيّ صالح داخل نافذة الحداثة، (ج) عدم وجود pools حديثة ضمن `max_graph_age_slots`، مع ضغط RPC (`429`) يعطّل تغذية الحسابات/الأسعار.
- **مشكلات بنيوية:** ثوابت وحدود حداثة مضمنة في الكود (1000/1500 slots وغيرها)، اعتمادات `unwrap_or(0/0.0)` تُخفي “بيانات غير متاحة”، خسارة مقدَّرة ثابتة عند drop/fail، ورسائل تيليجرام ذات ترميز/تفاصيل ناقصة.
- **اتجاه الإصلاح:** نقل التحكم بالقيم إلى المتغيرات البيئية (`.env`) قدر الإمكان، اشتقاق المدخلات الاقتصادية/التسعير/الحدود من بيانات سوق حية ومؤكَّدة (snapshots/pools/tip-floor)، وتحسين منطق الاكتشاف والرفض ليكون قابلًا للتشخيص دون كسر fail-closed.

## 2) حصر الهارد كوديد والقيم الثابتة (أبرز المواضع)

### 2.1 الإعدادات الافتراضية (`crates/arb-bot/src/config.rs`)
| المجال | القيم الافتراضية المضمّنة | الأثر |
|---|---|---|
| Chronos | `decay_lambda=3.0`, `max_age_ms=400`, `min_ev_usd=0.05`, `failed_attempt_cost=10_000`, `beta_risk_factor=0.0005` | حساسية بوابات EV/الزمن |
| Tip/Jito | `keep_share=0.85`, `ladder_growth=1.6`, `tip_floor_poll_ms=1000`, `leader_refresh_secs=60` | تسعير tip وسلوك المزايدة |
| Risk | `max_orders_per_hour=10`, `cooldown_slots=75` | تقييد السرعة/التبريد |
| DLMM cross-pool | `max_pairs=256`, `min_input=100_000`, `max_input=u64::MAX`, `coarse=10`, `refine=8`, `min_spread_bps=10`, `min_liquidity_bin_raw=1_000_000` | نطاق/تكلفة الاكتشاف |
| 3-leg DLMM | `input_size=100_000_000`, `max_input=2_000_000_000`, `slippage_bps=100`, `min_profit_lamports=30_000`, `size_ladder=4` | تحجيم/عتبات المسار الثلاثي |

### 2.2 منطق الاستراتيجية والتنفيذ (`supervisor/strategy.rs`, `execution.rs`)
- **حداثة Kamino/mint:** قبول snapshots ضمن `<= 1000` slots، fallback RPC مقبول إن `snap_slot` ضمن `<= 1500`، ورفض نهائي بـ `is_fresh(1500)` برسالة عامة. (قيم صلبة.)
- **سعر SOL:** يُصفَّر `last_known_live_sol_price = 0.0` في بداية كل tick ثم يُشتق من أول pool SOL/USDC حيّ؛ غيابه ⇒ `max_sizing=0` ⇒ تخطي الاكتشاف.
- **تحجيم:** `max_sizing = floor(max_trade_size_usd / sol_usd * 1e9)` (lamports) — يعتمد كليًا على توفر سعر حيّ.
- **عمر الجراف:** `max_graph_age_slots = min(graph_max_age_slots.unwrap_or(max_state_age_slots), max_state_age_slots)`.

### 2.3 بوابات الفرص والإرسال (`supervisor/opportunity.rs`, `dispatch.rs`, `run.rs`, `recovery.rs`)
- `priority_fee_lamports = (cu_price * cu_limit) / 1_000_000`، `contingency_buffer = 10_000`, `DEFAULT_HOP_SLIPPAGE_BPS`، `ATTEMPT_SEQ % 10_000`.
- `dispatch`: حسابات زمنية بـ `1000.0` ms، وتسجيل خسارة ثابتة `5_000.0/1e9 * sol_price` في مسارات drop/fail (تقريب ثابت لا يعكس التكلفة الفعلية دائمًا).
- `run.rs`: `MAX_SNAPSHOT_AGE_SLOTS=1500`, `ATA_RENT_LAMPORTS=2_039_280`.
- `recovery.rs`: `MAX_PENDING_INTENTS=256`, `POLL_ATTEMPTS=20`, `POLL_INTERVAL=400ms`.

### 2.4 المحرك (`arb-engine/src/lib.rs`)
- `EPSILON_CYCLE_SPREAD_BPS=10`, `MIN_NET_PROFIT_LAMPORTS=30_000`.
- `quote_pool` تُرجع `0` عند تعذر تسعير صادق (fail-closed) — يحمي من أوهام الربح لكنه قد يُسقِط مسارات دون سبب مُفصَّل للمشغل.

## 3) أخطاء/مخاطر منطقية ومسارات مكسورة أو هشة
1. **فرص = 0 بسبب الإنهاء المبكر للدورة:** `inc_opportunities()` لا يُستدعى إلا عند معالجة المرشحين؛ أي skip قبلها يُبقي العداد صفريًا.
2. **اعتماد صلب على حداثة 1000/1500 slots:** غير قابل للضبط من `.env`، وقد لا يلائم ظروف المزود/الشبكة.
3. **سعر SOL متقلّب/مفقود:** تصفير السعر كل tick + اشتقاقه من pool حيّ واحد فقط ⇒ حساسية عالية لفقدان تحديثات SOL/USDC؛ يؤدي لـ `max_sizing=0`.
4. **إخفاء “غير متاح” كصفر:** استخدام `unwrap_or(0.0/0)` في مقاييس/عرض/بذور قد يعطي انطباعًا خاطئًا.
5. **خسارة ثابتة عند الفشل:** `risk_gate.record_loss(5000 lamports * price)` في drop/fail — لا تعكس tip/base/CU الفعلية.
6. **Telegram:** نصوص عربية تالفة ترميزًا (mojibake) وتفاصيل معاملة غير مكتملة في بعض التنبيهات.
7. **اعتمادية RPC:** تكرار `429` (Helius) و`403` (PublicNode) يكسّر حداثة البيانات ويغذي fail-closed.
8. **سلامة التشغيل الحي:** تفعيل live مع مفاتيح حقيقية وتجاوزات (`BYPASS_*`, تعطيل preflight) يزيد المخاطر عند ظهور فرصة.

## 4) الإصلاحات المطبّقة في الكود (هذا التغيير)
- **`supervisor/strategy.rs`:** اشتقاق سعر SOL/USD من أعمق pool SOL/USDC حيّ في اللقطة الحالية (بيانات سوق حية) بدل أخذ أول نتيجة؛ عدم اختراع سعر عند الغياب (يبقى غير متاح)؛ إضافة logs تشخيصية (slot, pools, sol_usd, أسباب skip مع قيَم).
- **`supervisor/execution.rs`:** جعل حدود حداثة المدخلات المؤكَّدة قابلة للضبط من `.env`:
  - `VERIFIED_INPUT_MAX_AGE_SLOTS` (افتراضي: `max_state_age_slots`)
  - `VERIFIED_INPUT_FALLBACK_MAX_AGE_SLOTS` (افتراضي: `2 * VERIFIED_INPUT_MAX_AGE_SLOTS`)
  تُستخدم بدل الثوابت 1000/1500 في فحص snapshots/fallback و`is_fresh(...)` مع رسالة خطأ توضح الحد الفعلي.
- **`supervisor/dispatch.rs`:** إزالة الخسارة الثابتة `5000 lamports` عند drop/fail؛ استبدالها بـ `FAILED_BUNDLE_LOSS_LAMPORTS` (افتراضي `0`, من `.env`) وتسجيل الخسارة فقط عند توفر قيمة > 0 وسعر حيّ (>0) — أي تقدير/قيمة من التحكم التشغيلي أو القياس، دون افتراض ثابت.

## 5) ما الذي يبقى معتمدًا على بيانات حية فعليًا
- **سعر SOL/USD، احتياطيات/رسوم/سيولة pools، رسوم و snapshots Kamino، tip floor، slots/watermarks:** من تغذية ingest/RPC/WS الحية والمؤكَّدة فقط؛ عند غيابها يفشل المسار بأمان.
- **عتبات وحدود تشغيل (freshness caps, loss estimate, intervals, ladders):** تُضبط من `.env` مع افتراضات آمنة — تحكم تشغيلي، وليست أسعار سوق.

## 6) قيود ومخاطر متبقية
- لن تظهر فرص إذا استمرت أخطاء `429/403` أو انقطاع تغذية الحسابات/الـ pools الحية.
- `ARB_LIVE=1` مع مفاتيح حقيقية يعني مخاطرة مالية؛ تجاوزات الأمان يجب أن تكون مقصودة ومراقَبة.
- الربحية محدودة بسيولة/رسوم/منافسة السوق اللحظية.

## 7) تحقق مقترح بعد النشر
1. `tail -f logs/arb-bot-live.log` ومراقبة أسباب skip مع القيم (sol_usd, pools, max_age).
2. `curl -s http://127.0.0.1:19100/metrics | grep opportunities_total`.
3. مراقبة `429`/latency وضبط المزودين وحدود المعدل من `.env`.
