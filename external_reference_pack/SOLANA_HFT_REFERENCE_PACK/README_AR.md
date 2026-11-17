# حزمة المراجع الهندسية الانتقائية — Solana HFT / Atomic Arbitrage

تاريخ إعداد الحزمة: 2026-11-16

## الهدف
هذه ليست نسخاً كاملة من المشاريع الخارجية، وليست بوتاً جاهزاً للتشغيل. هي حزمة دراسة هندسية انتقائية:
- تفكيك معماري للمشاريع المرجعية.
- تحديد الملفات والمكونات التي تستحق المقارنة.
- مقتطفات صغيرة توضح الأفكار الحسابية/التصميمية.
- خريطة ربط هذه الأفكار بمشروع `SOLANA_FLASHLOAN_ARBITUGE`.
- Prompt تنفيذي صارم للوكيل البرمجي.

## المشاريع داخل الحزمة
1. `01-cetipoo-onchain-arbitrage` — Route construction، account-role validation، instruction building، CU model.
2. `02-lorenz-protocol` — integer AMM math، graph-cycle detection، invariants، risk/kill-switch، وفصل Data Plane عن Control Plane.
3. `03-fnzero-sol-trade-sdk` — cached trading، hot-path boundaries، quote freshness، fee lanes، retry/confirmation semantics.
4. `04-fnzero-parser-and-streamer` — Yellowstone gRPC، raw account updates، slot/write-version، stream/parser architecture.
5. `05-meteora-dlmm-sdk` — DLMM exact-in bin walking، fee splitting، official IDL/account contract.
6. `06-integration-blueprint` — ما الذي يستحق الاقتباس، وما الذي يجب عدم نسخه، وخطة دمج آمنة.

## تنبيه حول المحتوى
- لا تتضمن الحزمة مستودعات upstream كاملة.
- المقتطفات المحدودة والملخصات مخصصة للمراجعة التقنية؛ راجع LICENSE وNOTICE في كل upstream قبل إعادة استخدام أي كود.
- كل روابط المصدر تشير إلى مستودعات upstream. يجب على الوكيل تثبيت commit SHA لكل مرجع قبل المقارنة، لأن فروع `main/master` متغيرة.
- لا يوجد أي ادعاء بأن أي مشروع خارجي يضمن الربحية أو جاهزية الإنتاج.
- لا تستخدم المفاتيح أو المحافظ أو RPC credentials الحقيقية أثناء الاختبارات.

## نقطة البداية
ابدأ بقراءة:
1. `06-integration-blueprint/INTEGRATION_MAP.md`
2. `06-integration-blueprint/AUDIT_ACCEPTANCE_GATES.md`
3. `PROMPT_FOR_AGENT_AR.md`

ثم نفّذ التدقيق على نسخة منفصلة من مستودع المستخدم. لا تعدّل `main` ولا تدمج PR تلقائياً.
