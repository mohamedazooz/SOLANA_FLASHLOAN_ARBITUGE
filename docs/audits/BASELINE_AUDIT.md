# تقرير خط الأساس (Baseline Audit)

- **التاريخ:** 2026-11-19
- **الفرع (branch):** `audit/forensic-repair-v1` (منشأ من `main`)
- **نقطة الأساس (HEAD):** `b7e2970f65ff8001f1daca207223d33396bbd971`
- **الرسالة (commit):** Merge pull request #86 from mohamedazooz/feat/monorepo-unification-and-hft-dashboard
- **النظام (host):** Windows / PowerShell، بناء Linux-only للإنتاج (yellowstone gRPC)
- **سلسلة الأدوات (toolchain):** `rustc 1.99.1 (58cf8f9c3 2026-10-13)` / `cargo 1.99.1`، مثبّتة عبر `rust-toolchain.toml` (channel = 1.99.1, edition 2024)

## حالة الشجرة قبل أي تعديل

| العنصر | القيمة |
|---|---|
| `git status` | نظيف باستثناء `artifacts/` غير المتعقّب |
| عدد ملفات Rust (`.rs`) | 281 |
| عدد الحزم (crates) | 14 |
| `cargo check --workspace --all-targets` | ✅ نجح (≈18.8s عند القياس الأول) |

### الحزم (crates) واتجاه الاعتماديات

```
arb-types / arb-constants   (leaf crates)
      └── arb-math            (quote math + canonical economics, leaf)
            └── arb-dex       (protocol adapters: decode + quote)
                  └── arb-ingest
                        └── arb-engine
                              └── arb-risk
                                    └── arb-tx
                                          └── arb-sim / arb-land / arb-store / arb-metrics
                                                └── arb-bot (binary)
```

الاتجاه للأسفل فقط. `arb-tx` يعتمد على `arb-math`، والعكس ممنوع.

## ما تم فحصه في هذه المرحلة (read-only)

1. **التحقق من وجود الكود المزعوم للتنفيذ:** بناء الكود ينجح بالكامل، لا يوجد TODO/stub في مسارات التنفيذ الأساسية.
2. **التحقق من الحزمة `programs/arb-executor`:** برنامج on-chain موجود ويُنفّذ مسار DLMM ثلاثي الأرجل مع تحقق رصيد تسلسلي.
3. **التحقق من الثوابت الحسّاسة:** عناوين Kamino/Raydium/Orca/Jito، هويات البرامج، وحدود V1.
4. **التحقق من صحة تدفّق V1:** تم فكّ ترميز معاملة حقيقية على mainnet (انظر `MAINNET_TRANSACTION_FORENSICS.md`).

## ملاحظات أساسية (لا تُعدّ عيوباً بحد ذاتها)

- **بيانات الاختبار تُعاد كتابتها:** ملفات `reports/phase15_5/V1_SIZE_LADDER.json` و`reports/phase16/ALT_COMPRESSION_MATRIX.json` تتغيّر عند تشغيل الاختبارات (timestamps/أحجام). هذا سلوك معروف وليس تغييراً جوهرياً؛ يجب `git checkout --` لها قبل الإيداع.
- **أسرار في تقارير قديمة:** `reports/v1_canary/canary_1795101514.json` يحتوي على URL لـ RPC مع `api_key`. لم يتم نسخ المفتاح في أي تقرير جديد، ويجب تدويره/إزالته من التاريخ إن كان حساساً.
- **بناء Linux-only:** `yellowstone-grpc` يمنع البناء على Windows لبعض الحزم؛ التحقق الكامل يجب أن يُنفّذ على Linux (`scripts/verify-linux.sh`).

## نتيجة خط الأساس

✅ **الكود يبني نظيفاً على toolchain المثبّتة.** لا توجد أخطاء ترجمة أو تحذيرات clippy في `cargo check --workspace --all-targets` قبل التعديلات.

## القيود (Limitations)

- لم يتم تشغيل أي معاملة arbitrage حقيقية، ولا أي flash loan، ولا تغيير أي محفظة أو مفتاح.
- لم يتم تفعيل `ARB_LIVE` أو تعطيل `DRY_RUN`.
- جميع استنتاجات التنفيذ على mainnet مبنية على فحص بيانات عامة (RPC reads) أو محاكاة محلية، وليست على ربح محقّق.
