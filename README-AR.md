# Roo Code — Senior Engineering Modes & Skills

الحزمة دي معمولة لتتحط في جذر مشروعك داخل VS Code/Roo Code. فيها 8 Modes متخصصة و10 Skills تعمل بالتحميل عند الحاجة.

## 1) التثبيت

1. خُد نسخة احتياطية من `.roomodes` لو موجود بالفعل في المشروع؛ لا تستبدله قبل دمج المحتوى.
2. فك ضغط الحزمة في **جذر المشروع** بجوار `package.json` أو `Cargo.toml` أو مجلدات `src`.
3. لو عندك `.roomodes` سابق، ادمج عناصر `customModes` بدل استبدال الملف كله.
4. افتح Roo Code ثم نفّذ `Developer: Reload Window` من VS Code أو أعد فتح المشروع.
5. افتح قائمة Modes وتأكد من ظهور الأدوار الثمانية.
6. Skills تُكتشف تلقائيًا من مجلدات `.roo/skills*`؛ لا تحتاج تسجيلًا يدويًا.

## 2) ربط OpenRouter

من Roo Code: Settings → Providers → أنشئ API Configuration Profile جديدًا → Provider: OpenRouter → أدخل OpenRouter API Key في خانة المفتاح. لا تضع المفتاح داخل ملفات المشروع أو Git.

أنشئ Profileين:

- `OR-HYBRID`: للاستخدام اليومي؛ موديلات مدفوعة رخيصة مع مجانية عند الحاجة.
- `OR-FREE`: للمحاولات المجانية فقط.

اربط كل Profile بالـ Modes من تبويب Prompts حسب إصدار Roo Code. اختر الـ Mode، ثم اختَر الموديل المناسب له من Model Picker. Roo يحفظ آخر موديل لكل Mode (Sticky Models)، والـ API Profiles تدعم إعدادات مختلفة لكل Mode.

## 3) خريطة الموديلات المقترحة

### OR-HYBRID — اليومي الموصى به

| Mode | Model ID | الاستخدام |
|---|---|---|
| Solution Architect | `z-ai/glm-5.3-flash` | تخطيط وتحليل معماري منخفض التكلفة |
| Backend Engineer | `z-ai/glm-5.3-flash` | NestJS / PostgreSQL / Prisma / Redis |
| Frontend Engineer | `z-ai/glm-5.3-flash` | Next.js / React / TypeScript والمهام البصرية |
| Rust & Trading Systems | `deepseek/deepseek-v4.1-flash-1017` | Rust، منطق التنفيذ، سياق طويل، debugging |
| Quant & PnL Auditor | `z-ai/glm-5.3-flash` | مراجعة مستقلة للمعادلات والحسابات بموديل مختلف عن مهندس التنفيذ |
| QA & Security Auditor | `z-ai/glm-5.3-flash` | Test design، مراجعة الأدوات والحواف |
| DevOps & SRE | `cohere/north-mini-code:free` أو GLM عند الحاجة | تشخيص، أوامر، CI/CD؛ لا تستخدم المجاني لتغيير الإنتاج دون مراجعة |
| Documentation Steward | `cohere/north-mini-code:free` | تحديث Markdown والتوثيق |

بديل أقوى للمهام المعقدة جدًا وليس للاستخدام اليومي: `stepfun/step-5-preview`؛ سعره أعلى بكثير، لذا شغّله فقط لمهمة تحتاج reasoning عميقًا فعلًا.

### OR-FREE — بدون تكلفة Token

| Mode | Model ID المقترح |
|---|---|
| Solution Architect | `thinkingmachines/inkling-2-small:free` (جديد؛ اختبره أولًا) |
| Backend Engineer | `nvidia/nemotron-3.5-super-vl:free` |
| Frontend Engineer | `nvidia/nemotron-3.5-super-vl:free` |
| Rust & Trading Systems | `nvidia/nemotron-3-ultra-550b-a55b:free` |
| Quant & PnL Auditor | `nvidia/nemotron-3.5-super-vl:free` |
| QA & Security Auditor | `cohere/north-mini-code:free` أو Nemotron Super VL |
| DevOps & SRE | `cohere/north-mini-code:free` |
| Documentation Steward | `cohere/north-mini-code:free` |

ملاحظات:
- `Inkling 2 Small` أُطلق حديثًا؛ خصائصه المعلنة تشمل 1M context وTool Calling، لكن اعتبره تجريبيًا حتى تختبره على مستودعك.
- اختبار مستقل منشور بتاريخ 6 نوفمبر 2026 وجد أن `Nemotron 3.5 Super VL` و`Nemotron 3 Ultra` و`North Mini Code` كانت من أكثر النهايات المجانية ثباتًا في عينة 282 طلبًا. الاختبار صغير وليس ضمانًا لجودة كل مهمة.
- المتغيرات التي تنتهي بـ`:free` محدودة افتراضيًا إلى 50 طلبًا يوميًا و20 طلبًا في الدقيقة؛ فشل المحاولة قد يُحسب ضمن الحد. شحن 10 دولارات يرفع الحد اليومي المعلن إلى 1000 طلب، لكنه لا يعني أن كل Provider لن يفرض حدًا آخر.
- لا تستخدم أي موديل مجاني وحده لاعتماد حسابات تداول أو مراجعة أمنية أو قرار نشر. استخدم اختبارات deterministic، حسابات مستقلة، ومحاكاة.

## 4) المهارات الموجودة

- `engineering-operating-standard`: معيار عام لكل الأدوار؛ يحمي تغييراتك غير المحفوظة ويمنع ادعاء اختبارات لم تُشغّل.
- `project-documentation-maintenance`: تحديث خريطة المشروع، المعمارية، القرارات، changelog، runbook، والمشكلات.
- `architecture-and-system-design`: للـ Architect فقط.
- `nestjs-postgres-backend`: للـ Backend.
- `nextjs-frontend-quality`: للـ Frontend.
- `rust-solana-evm-execution` و`arbitrage-pnl-accounting`: لمهندس Rust/Trading.
- `independent-math-audit`: للمدقق الكمي بوضع read-only.
- `testing-security-review`: لمراجع QA/Security.
- `safe-devops-operations`: لـ DevOps/SRE.
- `living-documentation`: لمسؤول التوثيق.

## 5) قواعد تشغيل مهمة

- ابدأ دائمًا بـ Architect في التغييرات الكبيرة، ثم نفّذ باستخدام Backend/Frontend/Rust، ثم مرّر النتيجة إلى QA/Quant Auditor، وبعدها Documentation Steward.
- Quant Auditor وQA Auditor معرفان بصلاحيات قراءة وتشغيل فقط. لا تمنحهما صلاحية تعديل التطبيق إذا كان هدفك مراجعة مستقلة.
- لا تفعّل live trading أو mainnet broadcast أو deploy أو migrations مدمرة بمجرد أن الوكيل اقترح ذلك. الموافقة الصريحة مطلوبة لكل عملية مؤثرة.
- في مشروع Sentinel، لا تعتبر وجود فرصة في Scanner أو نجاح Simulation دليلًا على صفقة منفذة أو PnL محقق.

## 6) لا تضع هذه الحزمة في global settings مباشرة

`.roomodes` هنا Project-level. لو عايز نفس الـ Modes في كل مشاريعك، افتح Roo Code → Modes → Edit Global Modes وانقل عناصر `customModes` إلى `custom_modes.yaml`. أما Skills فيمكن جعلها عامة بنسخها إلى `%USERPROFILE%\\.roo\\skills\\` أو إبقاؤها داخل المشروع.
