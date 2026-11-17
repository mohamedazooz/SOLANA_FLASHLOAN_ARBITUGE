# 01 — Cetipoo On-chain Arbitrage Bot

**Upstream:** https://github.com/Cetipoo/solana-onchain-arbitrage-bot  
**Default branch:** `master`  
**License shown by GitHub/README:** MIT  
**Nature:** Reference implementation، وليس Production-ready.

## ما الذي يستحق الدراسة؟
### A. Route role validation
الملف `src/route.rs` يوضح نهجاً جيداً في:
- التحقق من أن كل Pool يحتوي الـMints المتوقعة للدور المحدد.
- رفض خلط مجموعات Direct وTriangle في نفس instruction إذا كان Executor لا يدعم ذلك.
- التحقق من الـcounterpart mint المشترك.
- عدم بناء route من مجرد أسماء pools في config.

**ما يجب اقتباسه كمبدأ:** route topology يجب أن تتحقق من الحسابات الحقيقية التي تم تحميلها، لا من إعدادات المستخدم وحدها.

### B. فصل resolve عن load
`Route::resolve()` يتحقق من صحة تكوين المسار، بينما `Route::load()` يحمّل الحسابات المطلوبة ويبني Basket من حالة أحدث.
هذه نقطة مفيدة لمشروعك، لكن لا تفترض أن `load()` وحده يضمن freshness أو fork coherence؛ ذلك مسؤولية طبقة الـstate snapshot.

### C. Instruction builder
`src/v10.rs` يبني `Instruction` وAccount Metas ويحتوي على:
- program IDs.
- Token Program / Token-2022 handling.
- mint/ATA derivation.
- tick-array / bin-array PDA candidate derivation.
- account layout حسب نوع البروتوكول.

**تحذير:** لا تنسخ offsets أو PDA seeds قبل مطابقتها مع IDL/SDK الرسمي وإصدار البرنامج الحالي.

### D. CU cost model
`lib/executor-v10-abi/src/cost.rs` يبيّن فكرة وجود نموذج CU مشترك بين client وon-chain executor، مع تكلفة مختلفة حسب venue، crossing، Token-2022، وflash-loan mode.

**ما يجب اقتباسه:** نموذج CU واحد موحد + قياسات replay فعلية + safety margin.
**ما لا يجب اقتباسه:** أرقام CU حرفياً. هذه الأرقام تخص تنفيذهم ونسخة برنامجهم، ولا تصلح كقيم افتراضية لمشروعك.

## ملفات upstream ذات الأولوية
- `src/route.rs`
- `src/v10.rs`
- `src/transaction.rs`
- `lib/executor-v10-abi/src/cost.rs`
- `src/loader.rs`
- `src/config.rs`
- `src/v10_tests.rs`

## مقتطف توضيحي من الفكرة
```rust
fn counterpart(pool_mints: [Pubkey; 2], mint: Pubkey) -> Result<Pubkey> {
    match pool_mints {
        [a, b] if a == mint => Ok(b),
        [a, b] if b == mint => Ok(a),
        _ => bail!("pool does not trade requested mint"),
    }
}
```
هذا مقتطف توضيحي مختصر، وليس نسخاً حرفياً كاملاً من الملف.

## المقارنة المطلوبة مع مشروعك
1. هل Route Discovery ينتج topology صالحة أم مجرد candidate؟
2. هل account metas مرتبة حسب ABI الرسمي لكل DEX؟
3. هل كل mint/vault/token program يتم التحقق منه؟
4. هل يتم حساب account count وserialized transaction size قبل الإرسال؟
5. هل CU model مبني على قياسات من نسخة البرنامج نفسها؟
6. هل تخطي pool فاشل قد يغيّر معنى المسار أو يسمح بمسار ناقص؟ يجب أن يفشل المسار إذا فقد leg إلزامي.

## لا تنقل
- Program IDs أو fee collectors أو vault addresses دون تحقق.
- transaction limits أو CU constants كما هي.
- افتراض أن Flash Loan implementation عندهم مطابق لـKamino أو لعقدك.
