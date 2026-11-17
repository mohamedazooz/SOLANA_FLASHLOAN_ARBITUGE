# 04 — FnZero Parser SDK + Solana Streamer

**Parser:** https://github.com/0xfnzero/sol-parser-sdk  
**Streamer:** https://github.com/0xfnzero/solana-streamer  
**Default branch:** `main` لكليهما  
**License shown by repository search:** MIT

## لماذا فصلناهما؟
- `sol-parser-sdk`: parsing core، protocol/account decoding، Yellowstone gRPC client، event model.
- `solana-streamer`: streaming orchestration، Yellowstone/ShredStream/RPC stream adapters، subscriptions، event processing، metrics/order buffers.

## A. Raw account updates
`examples/grpc_raw_cache.rs` مثال مهم لأنه يوضح أن:
- stream يرسل incremental updates، وليس startup snapshot كاملاً.
- يمكن استقبال AccountRawSnapshot وBlockMeta/Clock.
- يتم تسجيل slot وwrite_version.
- account closure تظهر كـlamports=0 ويجب التعامل معها كـtombstone.
- عدم وصول update لا يعني أن الحساب لم يعد صالحاً أو أن الحالة الابتدائية معروفة.

لا تعتمد على Yellowstone وحده لملء حسابات لم ترسل update بعد. يلزم bootstrap موثوق أو replay/catch-up ثم continuity check.

## B. Slot/write_version and fork handling
عند دمج أكثر من feed، لا تدمج updates لمجرد وصولها. احتفظ بمصدر الحدث وslot وwrite_version وfork/commitment context، وارفض التكرار أو الترتيب غير المقبول حسب قواعد موثقة.

## C. RPC vs gRPC
الوثائق تفرق بين streaming الحي وhistorical lookup:
- gRPC stream لا يعيد بث أحداث حدثت قبل الاشتراك.
- RPC يستخدم للتحقق التاريخي/backfill، وليس كبديل متزامن داخل hot path.
- replay لبيانات RPC يختبر parser logic لكنه لا يثبت live delivery.

## D. Dynamic subscriptions and lifecycle
`solana-streamer` يحتوي على Yellowstone stream lifecycle، subscription management، event processor، ordering buffers، metrics.
ادرس إدارة:
- subscribe/update/stop.
- duplicate prevention.
- reconnect behavior.
- bounded queues/backpressure.
- event ordering / micro-batching.
لكن قارن كل ذلك مع StateGuard الحالي قبل إضافة أي طبقة جديدة.

## الملفات المرجعية
Parser:
- `docs/ACCOUNT_SUBSCRIPTIONS.md`
- `docs/grpc-vs-rpc.md`
- `examples/grpc_raw_cache.rs`
- `examples/grpc_latency_slot_compare.rs`
- `src/grpc/client.rs`
- `src/grpc/config.rs`
- `src/core/cache.rs`
- `src/accounts/raydium_clmm.rs`
- `src/accounts/orca_whirlpool.rs`
- `idl/meteora_dlmm.json`

Streamer:
- `src/streaming/yellowstone_grpc.rs`
- `src/streaming/grpc/connection.rs`
- `src/streaming/grpc/subscription.rs`
- `src/streaming/common/order_buffer.rs`
- `src/streaming/common/event_processor.rs`
- `docs/SHREDSTREAM_LIMITATIONS.md`

## تحذيرات
- ShredStream لا يثبت execution success ولا يغني عن transaction metadata/logs.
- parser event ليس بالضرورة state snapshot كاملاً.
- لا تعتبر قياسات latency الواردة في README أو docs ضماناً لأداء VPS الخاص بك؛ قس p50/p95/p99 في بيئتك.
- لا تضف WebSocket/RPC fallback تلقائياً إذا كان التصميم الحالي يعتمد Yellowstone؛ أي fallback يجب أن يكون صريحاً ومحدوداً.
