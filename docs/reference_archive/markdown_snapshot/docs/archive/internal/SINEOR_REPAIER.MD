
الكلام ده صحيح ولا ايه هي افضل التعديلات من رايك ؟ 

Plan P0 Dex Account Resolution


P0 Implementation Plan: Dynamic Account Resolution & DEX Builders


  Goal Description

  Eliminate all hardcoded accounts, non-existent PDA seed derivations, and dummy SOL-USDC fallbacks across the Solana DEX transaction builder engine (crates/arb-tx/src/lib.rs).


  Currently, 5 out of 6 supported protocols (Raydium V4, Orca Whirlpool, Meteora DLMM, Raydium CPMM, Raydium CLMM) revert 100% of the time on non-canonical pools, while PumpSwap returns UnsupportedProtocol. This change ensures:

\
  Complete, genuine instruction construction for all 6 protocols using verified PoolExecutionAccounts.
\
  Fail-closed protection (TxError::MissingExecutionAccounts) so un-resolved pools are rejected before transaction assembly and simulation, preventing gas and tip burn.
\
  Correct PDA derivation algorithms (fixing Orca Whirlpool's Anchor string seed derivation and Meteora DLMM dynamic bin array vectors).
\
  Full pipeline propagation so execution accounts are preserved across split routing and supervisor dispatch loops.
\


  2. User Review Required


  IMPORTANT


    Strict Fail-Closed Behavior for Non-Canonical Pools:

Previously, if execution_accounts was None, the code silently fell back to mainnet SOL-USDC accounts (for Raydium V4) or derived dummy PDAs that do not exist on Solana.

Under this plan, any pool that does not have its execution_accounts populated in the registry will fail immediately at transaction build time with TxError::MissingExecutionAccounts. This ensures 0 SOL is burned on guaranteed reverts.



  NOTE


    PumpSwap Native Buy/Sell Parity:

We will implement native buy and sell instructions using Anchor discriminators ([102, 6, 61, 18, 1, 218, 235, 234] for buy and [51, 230, 133, 164, 1, 127, 131, 173] for sell) and the full 12+ trailing accounts matrix.



  3. Confirmed Design Decisions (via /grill-me)

\
  Fail-Closed on Missing Accounts:
\

  When execution_accounts is None for a non-canonical pool, build_swap_hop_instruction will immediately return TxError::MissingExecutionAccounts.
  Zero RPC latency on hot path, zero wasted SOL on doomed transactions.
\

\
  PumpSwap Dynamic ATA Derivation:
\
  If protocol_fee_recipient_ata or coin_creator_vault_ata in PoolExecutionAccounts::PumpSwap is Pubkey::default(), build_pump_swap_instruction will derive them on-the-fly using get_associated_token_address(&authority, &quote_mint).
\
\
  CLMM & Whirlpool Tick Array Bounds:
\
  Use the pre-resolved tick_array_pdas directly from PoolExecutionAccounts (3 contiguous arrays covering the current active tick). Avoid hardcoded tick spacing or tick 0 re-derivations.
\
\

  4. Proposed Changes


  Component 1: crates/arb-types (Type System)


  [MODIFY] crates/arb-types/src/lib.rs

\
  Add MissingExecutionAccounts(String) to TxError / error enumeration if not present.
\
  Verify PoolExecutionAccounts struct fields for all protocols.
\

  rust

// In TxError

#[derive(Debug, thiserror::Error)]

pub enum TxError {

    ...

    #[error("Missing execution accounts for pool {0}: {1}")]

    MissingExecutionAccounts(Pubkey, String),

}


  Component 2: crates/arb-ingest (Account Decoding & PDA Derivations)


  [MODIFY] crates/arb-ingest/src/registry.rs

\
  Fix Orca Whirlpool tick array seed derivation in build_execution_accounts:
  rust
// Fix seed: Orca Whirlpool uses string conversion start.to_string().as_bytes()
let start_str = start.to_string();
let seeds: &[&[u8]] = &[b"tick_array", pool_id.as_ref(), start_str.as_bytes()];
Pubkey::find_program_address(seeds, &prog).0
\
\
  Ensure Meteora DLMM bin_array_pdas derivation uses bin_array_index_for_bin with the pool's actual bin_step and active_bin_id.
\
  Ensure Raydium CLMM amm_config is read from byte offset 9..41 (CLMM pool state layout) and NOT hardcoded to CPMM config.
\
  Populate execution_accounts for PumpSwap pools.
\

  Component 3: crates/arb-tx (Transaction Builders)


  [MODIFY] crates/arb-tx/src/lib.rs

\
  Overhaul build_swap_hop_instruction:
\
  Raydium V4:
\
  Extract accounts directly from PoolExecutionAccounts::RaydiumV4.
\
  Reject with TxError::MissingExecutionAccounts if exec_accounts is None and pool != canonical SOL-USDC.
\
\
  Orca Whirlpool:
\
  Consume vault_a, vault_b, oracle, and tick_array_pdas from PoolExecutionAccounts::OrcaWhirlpool.
\
  Build instruction with genuine 11-account matrix using the resolved tick_array_pdas (never hardcode tick spacing 64 or tick 0).
\
\
  Meteora DLMM:
\
  Consume reserve_x, reserve_y, oracle, and bin_array_pdas from PoolExecutionAccounts::MeteoraDlmm.
\
  Pass precomputed bin_array_pdas directly as remaining accounts instead of recalculating with hardcoded bin step 10.
\
\
  Raydium CPMM:
\
  Consume amm_config, authority, vault_0, vault_1, observation_state from PoolExecutionAccounts::RaydiumCpmm.
\
\
  Raydium CLMM:
\
  Consume amm_config, vault_0, vault_1, observation_state, and tick_array_pdas from PoolExecutionAccounts::RaydiumClmm.
\
\
  PumpSwap:
\
  Implement build_pump_swap_instruction:
\
  Determine is_buy = hop.mint_in.0 == WSOL or native SOL.
\
  Construct instruction with PUMP_SWAP_BUY_DISCRIMINATOR or PUMP_SWAP_SELL_DISCRIMINATOR.
\
  Populate accounts: global_config, fee_config, pool_id, base_mint, quote_mint, pool_base_vault, pool_quote_vault, user ATAs, authority, token program, system program, and trailing fee recipient accounts.
\
\
\
\

  Component 4: crates/arb-bot (Pipeline Propagation)


  [MODIFY] crates/arb-bot/src/supervisor/run.rs

\

  In split-routing re-quote/build loop (lines 940-965 and lines 975-990):

  Do NOT pass execution_accounts: None.
  Preserve first_hop.execution_accounts (or resolve from registry by leg.pool_id).
\

\


  5. Verification Plan


  Automated Tests


  Run workspace tests with specific focus on arb-tx, arb-dex, and arb-ingest:


  powershell

cargo test -p arb-tx --lib

cargo test -p arb-tx --test bundle_tests

cargo test -p arb-dex --lib

cargo test -p arb-ingest --lib


  Dedicated new tests:

\
  test_raydium_v4_arbitrary_pool_account_resolution: Verify building instruction for non-SOL-USDC pool with full OpenBook accounts.
\
  test_orca_whirlpool_arbitrary_spacing_and_tick: Verify building instruction with tick_spacing = 8, 16, 128 and negative tick indices.
\
  test_meteora_dlmm_arbitrary_bin_step: Verify building instruction with bin_step = 25, 80 and dynamic bin arrays.
\
  test_raydium_cpmm_and_clmm_distinct_configs: Verify CLMM does not use CPMM amm_config.
\
  test_pumpswap_buy_and_sell_instruction_building: Verify byte-exact discriminators and account metas for PumpSwap.
\

  Manual Verification

\
  Execute mock dry-run compilation against a recorded snapshot of real mainnet pools to confirm account order and keys match on-chain transactions.
…
تقييمي للخطة: P0 Dex Account Resolution
الاتجاه الهندسي صحيح
لا أنصح باعتمادها كما هي
رأيي أن هذه الخطة من أهم الإصلاحات التي يمكن تنفيذها في محرك Solana Arbitrage Engine؛ لأنها تعالج مشكلة في طبقة بناء المعاملات نفسها، وليس مجرد مشكلة في الـquoting أو اكتشاف الفرص.

تقييمي المبدئي: 8/10 من حيث التصميم العام، لكن التنفيذ المقترح يحتاج تعديلات جوهرية قبل السماح له ببناء معاملات Mainnet.

الفكرة الأساسية صحيحة:

لا توجد حسابات تجريبية أو Hardcoded لحسابات Pool غير مؤكدة.

كل Pool له حسابات تنفيذ حقيقية خاصة به.

لا يتم إنشاء Transaction قبل التأكد من اكتمال الحسابات.

كل DEX له Instruction Builder مستقل يطابق بروتوكوله.

الحسابات لا تضيع أثناء انتقال الفرصة بين الـScanner والـRouter والـSupervisor والـExecutor.

لكن توجد خمسة أمور أعتبرها حرجة، وبعضها قد يجعل الإصلاح نفسه ينتج معاملات مرفوضة رغم أن الـPDA صحيح.

1. أهم الأخطاء التي يجب تعديلها
التعديل

تقييمي

Fail-Closed عند غياب الحسابات

صحيح

إصلاح Orca PDA seed

صحيح من حيث المبدأ

اشتقاق Meteora Bin Array باستخدام bin_step

يحتاج تصحيح

اشتقاق PumpSwap ATA باستخدام authority

غير صحيح بهذه الصيغة

نسخ first_hop.execution_accounts لبقية المسارات

خطر

الاعتماد على 3 Tick Arrays دون حساب نطاق السواب

غير كافٍ كقاعدة عامة

أولاً: Meteora DLMM — خطأ في معادلة اشتقاق Bin Array
الخطة تقول:

Use bin_array_index_for_bin with the pool's actual bin_step and active_bin_id.

هنا يجب الفصل بين مفهومين:

bin_step: يستخدم في تحديد المسافة السعرية بين الـBins وحساب السعر.

bin_array_index: يتم تحديده من bin_id نفسه، وليس من bin_step.

الطريقة الموثقة في Meteora:

binArrayIndex
=
⌊
binId
70
⌋
binArrayIndex=⌊ 
70
binId
​
 ⌋
مع ضرورة معالجة الأرقام السالبة باستخدام Floor Division، وليس Integer Division العادية التي قد تقرّب نحو الصفر.

واشتقاق الـPDA يعتمد على:

[
    b"bin_array",
    lb_pair.as_ref(),
    &bin_array_index.to_le_bytes()
]

هذا موضح في وثائق Meteora الرسمية. 

GitHub
+1

والأهم: لا يكفي تحميل الـBin Array الموجود عند active_bin_id فقط.

يجب تحديد الحسابات التي يحتاجها السواب في اتجاه التداول، من خلال الـBitmap والـBitmap Extension وحجم السواب والسيولة الموجودة.

وثائق مكتبة Rust الرسمية لدى Meteora توضح وجود:

get_bin_array_pubkeys_for_swap

وأنها تتعامل مع الـBitmap والـBitmap Extension وتستخرج الحسابات اللازمة حسب اتجاه السواب. كما توضح ضرورة تحميل المزيد من الحسابات عندما تكون السيولة المحملة غير كافية. 

GitHub
+1

التعديل المطلوب
ألغِ ربط اشتقاق Bin Array بـbin_step، واستخدم دوال Meteora الرسمية أو تنفيذًا مطابقًا لها، مع دعم الحسابات الإضافية وToken-2022.

ثانياً: PumpSwap — أخطر نقطة في الخطة
الـDiscriminators المذكورة للـbuy والـsell صحيحة بالنسبة إلى تعليمات PumpSwap القديمة V1:

BUY  = [102, 6, 61, 18, 1, 218, 235, 234]
SELL = [51, 230, 133, 164, 1, 127, 131, 173]

لكن المشكلة ليست في الـDiscriminator؛ المشكلة في الـAccounts.

الخطة تقول إننا لو لم نجد:

protocol_fee_recipient_ata أو coin_creator_vault_ata

نشتقهما باستخدام:

get_associated_token_address(&authority, &quote_mint)

وهذا لا يصح كقاعدة عامة.

الـATA يجب أن يكون تابعًا للـOwner الصحيح، والـOwner هنا ليس بالضرورة PumpSwap Authority ولا المستخدم.

مثلًا:

Protocol fee ATA: مالكه protocol_fee_recipient.

Creator fee ATA: مالكه coin_creator_vault_authority.

يجب استخدام Token Program الصحيح الخاص بالـQuote Mint، خصوصًا مع Token-2022.

ووثائق Pump الرسمية توضح أن coin_creator_vault_authority نفسه PDA يعتمد على coin_creator أو بيانات مشاركة الرسوم في الحالات الخاصة، وبالتالي لا يجوز استخدام Authority عام بدلًا منه. 

GitHub
+1

وهناك تحسين أكبر أوصي به:

Pump.fun أضاف PumpSwap V2، وهو يقدم buy_v2 وsell_v2 بقائمة ثابتة من 17 حسابًا، دون الـRemaining Accounts القديمة، مع الاحتفاظ برسوم البروتوكول والمبدع داخل الـPool لحين تسويتها.

يدعم V2 معظم الـPools، بما فيها Pools الناتجة عن Migration والـPermissionless Pools، لكنه لا يدعم Cashback Pools القديمة، التي تحتاج مسار V1. 

GitHub
+1

لذلك أقترح:

PumpSwap V2 يكون المسار الافتراضي عندما تكون حالة الـPool مؤهلة.

V1 يكون مسارًا بديلًا للـPools التي تحتاجه.

لا يتم اختيار نسخة Instruction بمجرد معرفة اسم DEX.

لا يتم اختيارها إلا بعد التحقق من حالة الـPool والـIDL وقائمة الحسابات المطلوبة.

لا يتم اشتقاق ATA دون التحقق من مالكه وMint وToken Program.

هذا أفضل بكثير من تنفيذ V1 فقط بقائمة حسابات مختصرة.

ثالثاً: Raydium V4 — يجب تحديث الخطة وفق التحديث الجديد
وجدت نقطة مهمة جدًا في توثيق Raydium الرسمي.

اعتبارًا من تحديث يوليو 2026، أزال Raydium AMM V4 اعتماد التنفيذ على OpenBook/Serum في مسار السواب.

أصبح لديه:

SwapBaseInV2

SwapBaseOutV2

وهما مسارا التنفيذ الموصى بهما حاليًا، ويستخدمان 8 حسابات فقط، دون حسابات الـOpenBook.

أما V1 فيحتفظ بقائمة الحسابات القديمة للتوافق، لكن حسابات الـMarket لم تعد مستخدمة فعليًا في التنفيذ. 

Raydium Docs
+1

قراري المقترح لـ Raydium V4

اعتماد V2 افتراضيًا
استخدام SwapBaseInV2 للمسارات Exact Input.

استخدام SwapBaseOutV2 للمسارات Exact Output.

إزالة اشتراط OpenBook accounts من مسار V2.

الإبقاء على V1 فقط عند الحاجة إلى توافق محدد ومثبت.

اختبار عدد الحسابات وترتيبها وصلاحياتها بالضبط.

إذًا اختبار الخطة الحالي:

test_raydium_v4_arbitrary_pool_account_resolution

يجب تغييره ليختبر V2 باستخدام Pool غير تقليدي، وليس اشتراط Full OpenBook Accounts.

هذا تعديل مهم؛ لأنه يقلل عدد الحسابات المطلوبة وCompute ويفصل المحرك عن اعتماد لم يعد مطلوبًا في المسار الحديث.

رابعاً: Orca Whirlpool — التصحيح جيد لكنه يحتاج شروطًا إضافية
اشتقاق Tick Array بالـString Seed صحيح من حيث المبدأ:

let start_str = start.to_string();

let seeds: &[&[u8]] = &[
    b"tick_array",
    pool_id.as_ref(),
    start_str.as_bytes(),
];

والأهم أن ترتيب الحسابات يجب أن يطابق Instruction الأصلي.

تعريف Swap في برنامج Orca الرسمي يتضمن:

Token Program

Token Authority

Whirlpool

User Token Account A

Vault A

User Token Account B

Vault B

Tick Array 0

Tick Array 1

Tick Array 2

Oracle

إذًا الـ11 Accounts المذكورة في الخطة منطقية لهذا المسار. 

GitHub
+1

لكن عندي تعديلان:

1. لا تشتق ثلاثة Tick Arrays بمجرد معرفة Current Tick.

يجب تحديد البداية والترتيب وفقًا إلى:

tick_current

tick_spacing

اتجاه السواب a_to_b

بداية كل Tick Array

التعامل الصحيح مع القيم السالبة

البرنامج يتحقق من أن الـTick Arrays مرتبة بالتسلسل الصحيح لاتجاه الصفقة.

2. دعم Adaptive Fee.

في بعض Pools، يحتاج الـOracle إلى أن يكون Writable، وليس Readonly فقط. برنامج Orca يوضح ذلك صراحةً في تعليمات السواب. 

GitHub
+1

لذلك أضف اختبارًا لـAdaptive Fee، وليس فقط اختبار Tick Spacing.

خامساً: Raydium CLMM وCPMM — الخطة ناقصة في تعريف الحسابات
هنا يجب التفريق بين البروتوكولين وعدم الاكتفاء بأن كليهما Raydium.

Raydium CLMM

التحقق من:

pool_data[9..41]

لاستخراج amm_config صحيح بالنسبة إلى ترتيب PoolState الحالي: 8 بايت Anchor Discriminator، ثم بايت Bump، ثم Pubkey الخاص بالـConfig. 

GitHub
+1

لكن أوصي باستخدام Decoder مولّد من الـIDL أو Struct مطابق للبرنامج، بدل الاعتماد الدائم على Offset يدوي.

هناك نقطة أهم: Raydium CLMM يحتاج إلى التعامل مع Tick Array إضافية وTickArrayBitmapExtension عند الحاجة. لا ينبغي افتراض أن ثلاثة Arrays كافية لكل Swap.

مصدر البرنامج يوضح أن مسار التنفيذ قد يحتاج إلى الانتقال عبر عدة Tick Arrays، وأن غياب الحساب التالي يؤدي إلى أخطاء مثل NotEnoughTickArrayAccount. 

GitHub
+1

كذلك اشتقاق Raydium CLMM مختلف عن Orca:

&start_tick_index.to_be_bytes()

أي لا تستخدم String Seed الخاصة بـOrca في Raydium.

Raydium CPMM

الخطة تذكر:

amm_config

authority

vault_0

vault_1

observation_state

هذه حسابات مهمة، لكنها ليست القائمة الكاملة للـInstruction.

المصدر الرسمي يبين أيضًا الحاجة إلى:

Pool State.

User Input Token Account.

User Output Token Account.

Input/Output Token Mints.

Input/Output Token Programs.

Input/Output Vaults.

Observation State.

والـAuthority والـConfig لهما قيود اشتقاق وربط بالـPool. 

GitHub
+1

يجب التأكد من أن الـBuilder يضيف هذه العناصر من الـExecution Context والـUser Context بصورة صحيحة، حتى لا يظن المطور أن وجود الحسابات الثابتة الخاصة بالـPool يعني اكتمال الـInstruction.

2. أهم تغيير معماري أوصي بإضافته
لا أريد أن يكون عندنا مجرد PoolExecutionAccounts يحتوي مجموعة مفاتيح، بل أريد نظامًا يتحقق من صحة هذه المفاتيح قبل تسليمها إلى الـTransaction Builder.

أقترح فصل المسؤوليات بهذا الشكل:

DEX Account Resolver

اكتشاف وقراءة بيانات الـPool من Registry وRPC Cache

Execution Account Validator

Owner / Mint / PDA / Pool / Token Program / Slot Freshness

Resolved Pool Execution Context

حسابات موثقة ومربوطة بالـPool الصحيح

Protocol Builder

Raydium / Orca / Meteora / PumpSwap

Instruction Validator

Account order / Signer / Writable / Data

Transaction Assembly → Simulation → Executor

الفكرة المهمة هنا أن الـBuilder لا يتصل بـRPC ولا يخترع حسابات ولا يحاول تخمين الـPDA.

هو يستقبل Context موثقًا، ثم يبني Instruction فقط.

الـResolver مسؤول عن قراءة الحسابات، والـValidator مسؤول عن التأكد من سلامتها.

3. خطأ أخطر في Pipeline Propagation
هذه النقطة تحديدًا أريد تغييرها في الخطة:

Preserve first_hop.execution_accounts
(or resolve from registry by leg.pool_id)

أنا أرفض استخدام first_hop.execution_accounts كحل عام.

تخيل هذا المسار:

Hop 1: Orca Whirlpool — Pool A
Hop 2: Meteora DLMM — Pool B
Hop 3: Raydium CLMM — Pool C
كل Pool له حساباته وPDAs وToken Programs مختلفة.

وحتى داخل البروتوكول الواحد، قد تتغير الحسابات المتبقية حسب اتجاه الصفقة وحجمها.

مثلًا:

Orca يحتاج Tick Arrays بترتيب يناسب الاتجاه.

Meteora يحتاج Bin Arrays وفقًا لاتجاه السواب والسيولة المطلوبة.

Raydium CLMM قد يحتاج عدة Tick Arrays وBitmap Extension.

لذلك لا يكفي ربط الحسابات بالـPool فقط.

أوصي بفصل نوعين من الحسابات:

pub struct PoolStaticAccounts {
    pub pool_id: Pubkey,
    pub dex: DexKind,
    pub pool_state: Pubkey,
    pub token_mint_a: Pubkey,
    pub token_mint_b: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_vault_b: Pubkey,
    pub program_id: Pubkey,
}

pub struct HopExecutionAccounts {
    pub pool_id: Pubkey,
    pub mint_in: Pubkey,
    pub mint_out: Pubkey,

    pub user_source_ata: Pubkey,
    pub user_destination_ata: Pubkey,

    pub remaining_accounts: Vec<AccountMeta>,

    pub resolved_slot: u64,
    pub instruction_variant: InstructionVariant,
}

هذا نموذج توضيحي، وليس تعريفًا نهائيًا لكل بروتوكول؛ بعض الحسابات الإضافية يجب أن تبقى داخل Variants متخصصة لكل DEX.

القاعدة الأساسية:

كل Hop يحصل على Execution Context خاص به، ولا يرث حسابات Hop آخر.

في supervisor/run.rs يجب أن يصبح المنطق:

for leg in &route.legs {
    let pool_state = registry
        .get_pool_state(leg.pool_id)
        .ok_or(TxError::MissingExecutionAccounts)?;

    let hop_accounts = resolver
        .resolve_for_hop(
            &pool_state,
            leg.mint_in,
            leg.mint_out,
            leg.amount_in,
        )
        .await?;

    validator.validate(&pool_state, &hop_accounts)?;

    let instruction = build_swap_hop_instruction(
        leg,
        &pool_state,
        &hop_accounts,
    )?;

    instructions.push(instruction);
}

بالطبع، يجب تعديل التواقيع حسب الـTypes الموجودة عندك.

هذا يعالج مشكلة انتقال الحسابات في الـSplit Routing وإعادة التسعير وإعادة البناء، ويمنع استخدام حسابات Pool غير متعلق بالـHop الحالي.

4. التعديلات النهائية التي أعتمدها لكل DEX
البروتوكول

القرار الذي أوصي به

Raydium V4

V2 SwapBaseIn/Out، دون OpenBook accounts

Orca Whirlpool

3 Tick Arrays صحيحة الاتجاه + Oracle

Meteora DLMM

Swap2 + Dynamic Bin Arrays + Bitmap Extension

Raydium CPMM

حسابات Pool والـVaults والـMints والـToken Programs كاملة

Raydium CLMM

Tick Arrays ديناميكية + Bitmap Extension + Config صحيح

PumpSwap

V2 افتراضيًا + V1 للحالات غير المدعومة

ملاحظة مهمة عن Meteora
وثائق Rust الرسمية لدى Meteora تقدم بالفعل commons crate الذي يوفر:

derive_bin_array_pda

get_bin_array_pubkeys_for_swap

quote_exact_in

quote_exact_out

بناء تعليمات Swap2.

كما يدعم التعامل مع Token-2022 وTransfer Hooks وRemainingAccountsInfo. 

GitHub
+2

أنصح باستخدام هذه المكتبة كمرجع تنفيذي، بدل إعادة اختراع كل حسابات DLMM من الصفر.

5. تعديلات الحماية التي أعتبرها إلزامية
A. لا تستخدم Pubkey::default كعلامة على غياب الحساب
بدلًا من:

if account == Pubkey::default() {
    // derive account
}

استخدم:

Option<Pubkey>

ثم حدد هل الحساب:

يمكن اشتقاقه حتميًا من بيانات موثقة.

يجب قراءته من Registry.

يجب أن يكون موجودًا على السلسلة بالفعل.

اختياري بحسب Instruction Version.

لا تخلط بين PDA يمكن اشتقاق عنوانه وبين Account يجب أن يكون موجودًا ومهيأً على Mainnet.

B. أضف Account Validation قبل البناء
لا يكفي التأكد من أن المفتاح ليس Default.

يجب التحقق من:

الفحص

المطلوب

Program Ownership

الحساب مملوك للبرنامج الصحيح

Pool Identity

الحساب مرتبط بنفس Pool

Token Mint

كل Vault يحمل الـMint المتوقع

Token Program

SPL Token أو Token-2022 الصحيح

PDA

إعادة اشتقاقه ومقارنته بالحساب

Tick/Bin Array

ارتباطه بالـPool والفهرس المطلوب

Account Meta

ترتيب الحسابات وWritable وSigner

Freshness

عدم الاعتماد على Snapshot قديم بشكل مؤثر

C. أضف Slot Consistency
هذه مهمة جدًا لمحرك مراجحة سريع.

لا تجمع حسابات Pool من تحديثات متفرقة دون التأكد من اتساقها.

أوصي باستخدام Snapshot من RPC عبر getMultipleAccounts مع تسجيل context.slot، وربط بيانات الحسابات التي تحتاجها نفس مرحلة التسعير والتنفيذ بمرجع Slot واضح.

وخزّن:

pub struct ResolutionMetadata {
    pub resolved_slot: u64,
    pub pool_id: Pubkey,
    pub program_id: Pubkey,
    pub instruction_version: u16,
}

ويجب إعادة التحقق من الحسابات الديناميكية إذا تغيّرت حالة التداول بما يؤثر على اختيار Tick/Bin Arrays.

D. صحح TxError
هناك تناقض في الخطة:

في البداية:

MissingExecutionAccounts(String)

وفي المثال:

MissingExecutionAccounts(Pubkey, String)

الأفضل:

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("Missing execution accounts for pool {pool}: {reason}")]
    MissingExecutionAccounts {
        pool: Pubkey,
        reason: String,
    },

    #[error("Invalid execution accounts for pool {pool}: {reason}")]
    InvalidExecutionAccounts {
        pool: Pubkey,
        reason: String,
    },

    #[error("Stale execution accounts for pool {pool}")]
    StaleExecutionAccounts {
        pool: Pubkey,
        resolved_slot: u64,
    },
}

وبذلك نستطيع تمييز الحسابات المفقودة عن الحسابات غير الصحيحة أو القديمة.

6. Verification Plan — أريد اختبارات أقوى
الاختبارات الموجودة جيدة كبداية، لكن لا أعتبر Instruction Construction Success وحده دليلًا على صحة المعاملة.

نجاح بناء Instruction لا يعني أن البرنامج سيقبلها.

أضيف الاختبارات التالية:

الاختبار

ما الذي يجب إثباته؟

PDA Golden Vectors

تطابق العناوين مع الحسابات الحقيقية

AccountMeta Exact Match

ترتيب الحسابات وصلاحياتها

Pool Ownership Validation

رفض أي حساب يتبع Pool آخر

Wrong Mint Rejection

رفض Vault يحمل Mint غير متوقع

Negative Tick Array Tests

صحة الحسابات عند Tick سالب

Dynamic Bin Arrays

تحميل الحسابات حسب السيولة والاتجاه

Token-2022 Tests

صحة Transfer Fees وTransfer Hooks

Missing Account Tests

توقف البناء دون أي Fallback

Split Routing Isolation

عدم انتقال حسابات Pool بين الـHops

PumpSwap Exact-Input

اختيار buy_exact_quote_in_v2 عند الشراء بالـExact Input

بالنسبة إلى PumpSwap تحديدًا، يجب ألا نخلط بين:

buy_v2: يحدد كمية الـBase المطلوبة.

buy_exact_quote_in_v2: يحدد كمية الـQuote المدفوعة.

sell_v2: يحدد كمية الـBase المباعة.

وهذا فرق مهم جدًا في محرك Arbitrage يعمل بمسارات Exact Input. 

GitHub
+1

اختبار أقوى من الـMock
أوصي بثلاثة مستويات:

المستوى الأول: Offline Fixtures

احفظ Account Snapshots حقيقية من Mainnet، واستخدمها في اختبارات PDA وAccount Decoding وInstruction Construction.

المستوى الثاني: Official SDK Comparison

ابنِ نفس Instruction باستخدام:

محركك Rust.

SDK الرسمية للبروتوكول.

ثم قارن:

Program ID
Instruction Data
Account Count
Account Pubkeys
Account Order
Writable Flags
Signer Flags
أي اختلاف يجب تفسيره قبل قبول الاختبار.

المستوى الثالث: Mainnet Simulation

شغّل simulateTransaction على معاملات مبنية من Pools حقيقية، دون بث المعاملات إلى الشبكة.

سجّل:

Pool ID
DEX
Instruction Variant
Resolved Slot
Account Validation
Simulation Result
Program Logs
Compute Units
Failure Reason
واختبر كل بروتوكول بأكثر من Pool، وبالاتجاهين، وبأحجام مختلفة.

7. ترتيب التنفيذ الذي أوصي به
لا أنصح بتنفيذ التعديلات الستة دفعة واحدة دون فصل مراحل التحقق.

P0.1 — Remove Unsafe Fallbacks

إلغاء Hardcoded Accounts وDummy PDAs وFallback غير الموثق، مع Fail-Closed.

P0.2 — Build Account Resolver

بناء نظام Resolution وValidation وSlot Consistency.

P0.3 — Repair Protocol Builders

تنفيذ الحسابات لكل بروتوكول باستخدام Instruction Variants الصحيحة والـIDL الحالية.

P0.4 — Repair Pipeline Propagation

إصلاح Split Routing وSupervisor وRe-quote وExecution Context لكل Hop.

P0.5 — Differential Testing

المقارنة مع SDK الرسمية وMainnet Snapshots واختبارات PDA.

P0.6 — Mainnet Simulation and Canary

محاكاة جميع البروتوكولات، ثم السماح بتفعيل كل DEX بصورة منفصلة بعد اجتياز اختبارات القبول.

8. ما الذي أريد إضافته إلى تعريف اكتمال P0؟
لا تعتبر المهمة مكتملة لمجرد نجاح:

cargo test -p arb-tx --lib
cargo test -p arb-dex --lib
cargo test -p arb-ingest --lib

أريد أن يكون معيار الإنجاز:

P0 Acceptance Criteria


لا يوجد Hardcoded Pool Account Fallback.

لا يوجد Dummy PDA.

كل Hop يحمل حسابات Pool الصحيح.

جميع PDA Derivations تطابق Golden Vectors.

Account Metas تطابق الـIDL الرسمية.

دعم Token-2022 حيث يدعمه البروتوكول.

دعم Dynamic Tick/Bin Arrays.

Missing Accounts تؤدي إلى رفض محلي قبل إرسال المعاملة.

Split Routing لا يخلط Execution Accounts.

نجاح محاكاة Pools حقيقية للبروتوكولات الستة.

لا يوجد Broadcast تلقائي أثناء مرحلة التحقق.
حكمي النهائي
أنا موافق على تنفيذ المشروع بهذا الاتجاه، لكنني أريد تغيير أربعة قرارات قبل إعطاء الـCoding Agent صلاحية التنفيذ:

Raydium V4: تحويل الـBuilder إلى V2 بدل بناء الاعتماد على OpenBook.

Meteora: تصحيح Bin Array Index وإضافة الحسابات الديناميكية وToken-2022.

PumpSwap: استخدام V2 وExact-Input الصحيح، وتصحيح ملكية الـATA ودعم V1 عند الحاجة.

Pipeline: عدم نسخ first_hop.execution_accounts، وتوليد حسابات خاصة بكل Hop.

وهناك تحفظ أخير: ادعاء أن خمسة بروتوكولات من أصل ستة ترتد بنسبة 100% على Non-Canonical Pools لا أستطيع تأكيده دون رؤية الكود واللوجات. يجب على الوكيل إثباته من خلال تحليل التنفيذ والاختبارات، لا افتراض صحته.

توصيتي: لا تجعل الهدف مجرد إصلاح Transaction Builder. اجعل الهدف بناء طبقة تنفيذ Solana موثوقة، قابلة للتوسع، وتستطيع إضافة DEX جديد إليها دون إدخال حسابات تجريبية أو كسر البروتوكولات الموجودة.

هذا هو التصميم الذي أختاره لمحرك Arbitrage احترافي، خصوصًا عندما تكون تكلفة الخطأ هي خسارة رسوم المعاملة والـPriority Fee والـJito Tip.

اعمل خطه شامله و ابحث اونلاين عن البيست براكتس 


