# Integration Map — ما الذي نأخذه إلى SOLANA_FLASHLOAN_ARBITUGE؟

## قاعدة القرار
لا ندمج upstream project كاملاً. نأخذ فكرة/مكوناً واحداً فقط بعد:
1. إثبات وجود gap في مشروعنا.
2. إثبات أن upstream يحل نفس المشكلة.
3. مقارنة protocol/version/dependencies/license.
4. Differential test ضد implementation الحالي.
5. قياس الأداء والذاكرة.
6. PR مستقل مع rollback واضح.

## خريطة المكونات

| المجال | المرجع | ما ندرسه | قرار الدمج |
|---|---|---|---|
| Route topology | Cetipoo `src/route.rs` | mint-role validation، direct/triangle grouping | نقل اختبارات/تحقق فقط إذا ظهر gap |
| Instruction metas | Cetipoo `src/v10.rs` + Meteora IDL | account ordering وPDA وtoken programs | لا نسخ؛ مطابقة حرفية مع IDL الحالي |
| CU model | Cetipoo `cost.rs` + قياساتنا | shared client/on-chain model | إعادة قياس CU في runtime الخاص بنا |
| Integer CPMM | Lorenz `lorenz-amm` | checked math، inverse rounding، properties | differential tests |
| Cycle discovery | Lorenz `lorenz-graph` | graph cycle detection | مقارنة algorithm فقط؛ quote الفعلي من protocol adapters |
| Risk | Lorenz `risk.rs` | hard caps، kill switch، agent can only tighten | إضافة invariant tests إن كانت ناقصة |
| Hot path | FnZero `LOW_LATENCY_BOTS.md` | no RPC/blockhash/discovery in hot path | architecture test + tracing |
| Cache/snapshot | FnZero trade/parser | immutable snapshot, slot/write_version, tombstones | مقارنة StateGuard؛ لا تنشئ cache مكرر |
| Yellowstone | FnZero parser/streamer | raw updates, subscription lifecycle, ordering | reuse patterns, not blind dependency import |
| DLMM math | Meteora official SDK | bin walking, fee split, rounding | differential quote harness |
| DLMM instruction | Meteora IDL | swap2 discriminator/metas/args | source of truth must be pinned IDL |

## لا تضف هذه الأشياء تلقائياً
- لا تضف Lorenz geyser module: `connect()` في المرجع يرجع NotImplemented.
- لا تضف ShadowArb من القائمة القديمة: لم يتم تثبيت مستودع canonical/source tree قابل للتحقق ضمن هذه الحزمة.
- لا تستبدل Yellowstone الحالي بـFnZero SDK دون مقارنة reconnect/fork/slot semantics.
- لا تضف Sol Trade SDK كـrouter إذا كان مشروعك ينفذ atomic multi-DEX CPI؛ SDK التداول العادي ليس بديلاً عن on-chain atomic arbitrage.
- لا تعتبر Cetipoo V10 program أو vault متوافقاً مع Kamino flash loan في مشروعك.

## ترتيب التنفيذ
1. Baseline وcommit pinning.
2. Source-to-source comparison.
3. Math differential tests.
4. Protocol IDL/account-meta verification.
5. State/stream correctness tests.
6. Execution lifecycle and fee accounting.
7. Only then implement confirmed fixes.
