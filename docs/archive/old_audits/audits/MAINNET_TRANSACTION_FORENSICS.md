> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# تحليل جنائي للمعاملة المعلنة على Mainnet

- **التاريخ:** 2026-11-19
- **التوقيع (signature):** `s9eVzA5J7hgSunKyyjUkJ2nwpQw6FrNL4rQViYHjiJmdznVS9kSaHuk2vSpbca1o9kmU5ijrCgkysHxdkqqa4js`
- **الحالة النهائية:** `finalized`، `meta.err = null`، slot `471383273`، `transactionIndex = 190`
- **version:** `1` (V1 / SIMD-0385 trailing-signature layout)

## الحكم النهائي (Verdict)

> **VERIFIED NON-ARBITRAGE TEST TRANSACTION**
>
> المعاملة **حقيقية ونجحت على mainnet**، لكنها **ليست arbitrage ولا flash loan**.
> تحتوي **تعليمة واحدة فقط** هي SPL-Memo، ولا تحتوي أي تعامل مالي.

### الأدلة القاطعة

| الدليل | القيمة المرصودة |
|---|---|
| عدد التعليمات | 1 |
| نوع التعليمة | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| محتوى Memo | `SOLANA_FLASHLOAN_ARBITUGE V1 TRANSPORT CANARY` (45 بايت) |
| `preTokenBalances` | `[]` |
| `postTokenBalances` | `[]` |
| `innerInstructions` | `[]` |
| برامج Kamino / Raydium / Orca | غير موجودة إطلاقاً |
| lazy loaded addresses | `[]` (readonly) / `[]` (writable) |
| الحسابات | `2Cby3xFT48ifATWhqDVmpnMCe4tiXvY8MtGFWYDfec7n` (signer), `Memo...` (program) |
| CU المستهلك | `17,724` من `200,000` |
| الأجرة (fee) | `6,000` lamports |
| التغيّر في SOL | `-6,000` lamports فقط (رسوم الشبكة) |

## فكّ الترميز الخام (byte-level)

تم حفظ الحزمة الخام في:

- `artifacts/forensics/tx_base64.txt` (base64)
- `artifacts/forensics/tx_base64.bin` (235 بايت بعد فكّ base64)
- `artifacts/forensics/getTransaction_base64.json` (استجابة RPC كاملة)

### بنية الحزمة (235 بايت)

```
offset 0    : 0x81                       (version byte = 1 | 0x80)
[1..4]      : 01 00 01                   (header: 1 required sig, 0 readonly signed, 1 readonly unsigned)
[4..8]      : 0f 00 00 00                (config mask LE u32 = 0x0F)
[8..40]     : blockhash (32 bytes)
[40]        : 0x01                       (numInstructions = 1)
[41]        : 0x02                       (numAddresses = 2)
[42..]      : inline addresses (2 × 32)
...         : config values (CU limit, priority fee, loaded-data limit)
...         : instruction payload (program_id_index, account indices, data)
[171..235]  : signature (64 bytes)       (V1 trailing signature)
```

- **حجم الرسالة (message):** 171 بايت
- **التوقيع:** 64 بايت في النهاية (وليس في البداية كما في Legacy/V0)
- **الإجمالي:** 171 + 64 = **235 بايت** ✅ مطابق للملف الفعلي

### الإعدادات (V1 config) المفكوكة

| الحقل | القيمة |
|---|---|
| `computeUnitLimit` | `200,000` |
| `priorityFee` | `1,000` lamports |
| `loadedAccountsDataSizeLimit` | `67,108,864` |
| `heapSize` | `null` |

> الأجرة الفعلية المرصودة `6,000` lamports = `5,000` base fee + `1,000` priority fee، وهي متسقة مع الإعدادات أعلاه.

## من أين جاءت المعاملة؟

الملف `reports/v1_canary/canary_1795101514.json` يطابق المعاملة **بالتوقيع والـslot والحجم (235 بايت) والـCU (17,724)**، ويصرّح:

- `verdict: "V1_TRANSPORT_VERIFIED"`
- `instruction_summary: "spl-memo (non-financial canary)"`
- `send_latency_ms: 133`
- `wire_size_bytes: 235`
- `version_byte: 129`

ومصدرها هو المثال المخصّص:

```
crates/arb-tx/examples/v1_canary_broadcast.rs
```

وهو مثال **مقصود** للتحقق من النقل (transport canary) فقط، وليس مسار arbitrage.

## الأرقام المعلنة في التقرير الأصلي: ما هو صحيح وما هو غير صحيح

| الادعاء | الحكم |
|---|---|
| التوقيع، slot، الإصدار V1، الحجم 235 بايت | ✅ صحيح ومُتحقَّق |
| CU = 17,724 | ✅ صحيح |
| fee = 6,000 lamports | ✅ صحيح |
| زمن الإرسال 133ms | ✅ مطابق لملف الدليل (لكنه زمن قبول RPC، وليس زمن هبوط/تأكيد) |
| "arbitrage ناجح" أو "flash loan" | ❌ **خاطئ**: لا توجد أي تعليمة مالية |
| "Orca SOL = 190,674.06 / USDC = 4,228,246.89" | ❌ غير متسق داخلياً: النسبة = 22.17 USDC/SOL وليست 75.018 |
| تلك الأرقام موجودة في الريبو | ❌ لا توجد؛ تم البحث ولم يُعثر عليها |
| "Kamino flash fee = 1 bps" | ❌ القيمة الفعلية `11_529_215_046_068` (U68F60) = **0.1 bps** |

### تفصيل عدم اتساق أرقام Orca

```
4,228,246.89 / 190,674.06 ≈ 22.17 USDC per SOL
```
بينما السعر المعلن في نفس التقرير هو `75.018`. الفارق ≈ 3.4×. لذلك هذه الأرقام إمّا مقلوبة البسط/المقام أو مُختلقة، ولا يمكن الاعتماد عليها كدليل اقتصادي.

أرقام Raydium المعلنة:
```
8,049,074.47 / 107,203.45 ≈ 75.082 USDC per SOL
```
متسقة داخلياً، لكنها أيضاً **غير مرتبطة** بالمعاملة المذكورة (لا يوجد أي swap في المعاملة).

## الاستنتاج

1. المعاملة **دليل نجاح على مستوى النقل (transport) فقط**.
2. لا يوجد أي دليل على تنفيذ arbitrage أو flash loan أو ربح.
3. أي تقرير يقدّم هذه المعاملة كدليل ربحية هو **تقرير مضلِّل** ويجب تصحيحه.
4. مسار V1 يعمل تقنياً: الترميز، الحجم، CU، والأجرة كلها متسقة.

## حالة التحقق

| البند | الحالة |
|---|---|
| جلب المعاملة من RPC عام | ✅ verified |
| فكّ base64 ومطابقة الحجم 235 | ✅ verified |
| تحليل التعليمات والحسابات | ✅ verified |
| إثبات أنها ليست arbitrage | ✅ verified |
| ربطها بمصدرها في الريبو | ✅ verified (canary example + evidence file) |

> لم يُنفَّذ أي استدعاء شبكة يعدّل الحالة. كل ما سبق قراءة عامة (read-only).
