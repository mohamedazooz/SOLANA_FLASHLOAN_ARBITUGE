# 05 — Meteora DLMM Official SDK

**Upstream:** https://github.com/MeteoraAg/dlmm-sdk  
**Default branch:** `main`  
**Purpose:** مرجع بروتوكولي رسمي للـDLMM math وaccount contract وIDL.

## A. DLMM ليس CPMM
لا تستخدم معادلة `x*y=k` لتسعير DLMM.
الـquote يمر عبر bins ذات أسعار وسيولة منفصلة، وقد يتطلب عبور عدة bins. يجب أن يتطابق اتجاه swap وrounding وfee calculation مع implementation الرسمي.

## B. Exact-in bin traversal
`commons/src/quote.rs` يوضح تقسيم الكمية عبر طبقات السيولة داخل bin:
1. حساب الحد الأقصى من input المطلوب لاستهلاك output liquidity المتاح.
2. إن كان input أقل، حساب output مع rounding المناسب.
3. إن بقي input، متابعة طبقات limit orders عندما تكون مدعومة.
4. جمع input/output مع checked arithmetic.
5. احتساب رسوم التداول في موضعها الصحيح حسب fee-on-input mode.

## C. Fee split
الملف نفسه يحتوي تقسيم رسوم بين MM/user وprotocol وlimit-order participants.
لا تختزل رسوم DLMM إلى bps ثابتة دون التحقق من dynamic fee state، protocol share، limit order support، Token-2022/transfer-fee effects.

## D. IDL contract
`idls/dlmm.json` هو المرجع لحسابات وتعريفات instruction.
بالنسبة إلى `swap2`، يجب أن تراجع:
- discriminator.
- ترتيب Account Metas.
- optional bitmap extension.
- reserve_x / reserve_y.
- user_token_in / user_token_out.
- token_x_mint / token_y_mint.
- oracle.
- optional host_fee_in.
- user signer.
- token_x_program / token_y_program.
- memo_program.
- event_authority / program.
- args: amount_in, min_amount_out, remaining_accounts_info.

هذه القائمة مأخوذة من IDL الذي تمت مراجعته وقت إعداد الحزمة؛ يجب على الوكيل إعادة جلب IDL الحالي ومقارنة كل byte/field قبل التعديل.

## الملفات المرجعية
- `commons/src/quote.rs`
- `commons/src/math/price_math.rs`
- `commons/src/math/u128x128_math.rs`
- `commons/src/extensions/bin.rs`
- `commons/src/extensions/bin_array.rs`
- `commons/src/extensions/bin_array_bitmap.rs`
- `commons/src/extensions/lb_pair.rs`
- `commons/src/pda.rs`
- `commons/src/token_2022.rs`
- `idls/dlmm.json`
- `commons/tests/integration/test_swap.rs`
- `commons/tests/integration/test_swap_token2022.rs`

## ما يجب إثباته في مشروعك
- bin traversal يطابق SDK الرسمي لنفس snapshot والاتجاه والكمية.
- PDA derivation مطابق للـSDK/IDL.
- remaining accounts وbin arrays مكتملة ومرتبة.
- rounding لا ينتج optimistic output.
- protocol fee وdynamic fee وToken-2022 محسوبة.
- الحسابات لا تتجاوز packet/account/CU limits.
- quote يتوقف ويرفض إذا كانت bin arrays المطلوبة ناقصة أو stale.
