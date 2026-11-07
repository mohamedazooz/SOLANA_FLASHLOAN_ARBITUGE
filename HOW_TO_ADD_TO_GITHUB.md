# إضافة Rust CI إلى مستودع GitHub

الملف المطلوب نسخه إلى المستودع هو:
`.github/workflows/rust-ci.yml`

## أسهل طريقة من متصفح GitHub

1. افتح المستودع:
   https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE
2. اضغط **Add file** ثم **Create new file**.
3. في خانة اسم الملف اكتب بالضبط:
   `.github/workflows/rust-ci.yml`
   سيُنشئ GitHub المجلدات تلقائيًا.
4. افتح الملف `rust-ci.yml` الموجود داخل هذا الأرشيف، وانسخ محتواه كاملًا إلى محرر GitHub.
5. اضغط **Commit changes**.
6. لو ظهر اختيار مكان الحفظ، اختَر إنشاء فرع جديد وافتح Pull Request إلى `main`.
   اسم مقترح للفرع: `feature/ci-bootstrap`.
7. افتح تبويب **Actions** في المستودع، ثم افتح تشغيل **Rust CI** وتابع كل خطوة.

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
