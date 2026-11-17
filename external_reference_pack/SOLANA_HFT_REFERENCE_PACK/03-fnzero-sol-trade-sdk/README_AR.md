# 03 — 0xfnzero Sol Trade SDK

**Upstream:** https://github.com/0xfnzero/sol-trade-sdk  
**Default branch:** `main`  
**License shown by repository search:** MIT  
**Nature:** Rust trading SDK؛ versioned independently. Check current release and migration notes before adapting.

## أهم ما يستحق الدراسة

### A. Hot-path isolation
`docs/LOW_LATENCY_BOTS.md` يحدد hot path صغيراً:
```text
filter -> deduplicate -> reject stale event -> map post-trade state
       -> build params -> sign -> submit
```
ولا يسمح بعمل:
- client initialization.
- synchronous blockhash fetch.
- balance RPC.
- pool discovery.
داخل hot path.

هذا متوافق مع هدف مشروعك: كل bootstrap وdiscovery وrefresh تكون background tasks؛ hot path يستهلك snapshot متماسكاً.

### B. Cached quote vs route search vs preparation
`docs/ARCHITECTURE.md` يفصل المسؤوليات:
- `cached_quote`: حساب amount-specific quote.
- route search: اختيار المسار.
- preparation: إنتاج execution parameters.
- instruction builder: بناء تعليمات مع حماية لكل hop.

لا تنشئ quote engine ثانياً داخل الأمثلة أو الـSupervisor.

### C. Snapshot validity
الوثائق تذكر أن cache يجب أن يحفظ slot/write_version الحقيقيين، ويتحقق من owner/layout/mints/transfer fees/freshness.
كما تفرق بين:
- coherent immutable snapshot.
- pool identity.
- live executable state.

النقطة المهمة: إعادة نشر snapshot أو clear state يجب أن تتعامل مع in-flight preparation بوضوح.

### D. Submit / confirm are different measurements
التوثيق يفرق بين زمن construction وsubmit وconfirmation.
بطء confirmation لا يعني بالضرورة بطء بناء instruction. كما يحذر من retry بعد timeout غامض قبل reconciliation للتوقيع والحالة.

### E. Fee lanes
`docs/GAS_FEE_STRATEGY.md` يشرح CU limit وCU price وtip لكل SWQoS lane، مع normal وhigh/low fee strategies.
لا تستخدم القيم الواردة في الأمثلة كإعداد إنتاجي. يجب أن تكون القيم dynamic ومحدودة بحدود استراتيجية الربح والمخاطر.

### F. Quote protection
SDK docs تشدد على min-output protection، quote-age limits، slippage، transfer-fee awareness، وعدم استخدام `min_out=0` كحل عام للأخطاء.

## الملفات المرجعية
- `docs/ARCHITECTURE.md`
- `docs/LOW_LATENCY_BOTS.md`
- `docs/GAS_FEE_STRATEGY.md`
- `docs/STREAMING_QUOTES.md`
- `docs/PRE_BUY_RISK_GATE.md`
- `examples/pool_quote_tools.rs`
- `examples/simulation_replay.rs`
- `examples/pumpswap_cached_prepare.rs`
- `examples/support/account_snapshot.rs`

## لا تنقل
- SDK كامل إلى Cargo workspace قبل فحص dependency graph والتعارضات.
- API calls إلى hot path.
- fee presets ثابتة.
- retries عمياء بعد submit timeout.
