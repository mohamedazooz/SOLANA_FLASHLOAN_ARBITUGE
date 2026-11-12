# إضافة Rust CI إلى مستودع GitHub

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:08577304b0789d0197c94801922ee2e4fe77ea620f49f10f6eb1fd0930b297f5 -->

الـworkflow `.github/workflows/rust-ci.yml` موجود بالفعل في مستودع
[SOLANA_FLASHLOAN_ARBITUGE](https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE).
افتح تبويب **Actions** في المستودع، ثم افتح تشغيل **Rust CI** وتابع كل خطوة.

## إضافة الـworkflow عند الحاجة

إذا كنت تستخدم مستودعًا مختلفًا أو نسخة من المستودع لا تحتوي على الـworkflow:

1. افتح المستودع المستهدف في متصفح GitHub واضغط **Add file** ثم **Create new file**.
2. في خانة اسم الملف اكتب `.github/workflows/rust-ci.yml`؛ سيُنشئ GitHub المجلدات تلقائيًا.
3. انسخ محتوى ملف `rust-ci.yml` من نسخة تحتوي عليه إلى محرر GitHub.
4. اضغط **Commit changes**، وأنشئ فرعًا جديدًا وافتح Pull Request إذا لزم الأمر.
5. افتح تبويب **Actions** في المستودع المستهدف وتابع تشغيل **Rust CI**.

<!-- /coderabbit-ai-generated:v1 -->

## أوامر CI التي سينفذها

- `cargo fmt --all --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`

لا يحتاج هذا الـworkflow إلى API keys أو RPC/gRPC secrets. لا تضف مفاتيح المحافظ أو `.env` إلى GitHub.

## ملاحظات مهمة

- هذا الملف يضيف CI فقط، وليس CD أو نشرًا على VPS.
- لا تعتبر الـCI ناجحًا إلا بعد أن تظهر جميع الخطوات باللون الأخضر في GitHub Actions.
- قد يكشف أول تشغيل أخطاء بناء أو تحذيرات Clippy موجودة مسبقًا في المشروع؛ هذا تشخيص مطلوب، وليس دليلًا على أن الملف نفسه معطوب.
- بعد ظهور نتيجة التشغيل، شارك رابط الـPull Request أو لقطة/نص نتيجة Actions لمراجعة الأخطاء وإصلاحها على فرع منفصل.
