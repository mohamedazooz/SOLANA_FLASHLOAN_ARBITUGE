# 13 — Cross-Protocol DEX Adapter Audit (crates/arb-dex + arb-constants)

Scope: read-only audit of `crates/arb-dex/src/{lib.rs, traits.rs, raydium_v4.rs, raydium_cpmm.rs, raydium_clmm.rs, orca_whirlpool.rs, meteora_dlmm.rs, meteora_damm_v2.rs, raydium_launchlab.rs, pump_fun.rs, pump_swap.rs}` and `crates/arb-constants/src/lib.rs`. Runtime classification was determined by grepping `arb-engine`, `arb-bot`, `arb-ingest`, `arb-tx`, `arb-sim`, `arb-mcp`. Line numbers refer to the audited files.

## 0. Runtime-path summary (all findings below keyed to this)

* `decode_amm_pool` (arb-dex/src/lib.rs:182) is only called from `arb-ingest/src/lib.rs:766,889` (Yellowstone streaming path, "dual-ingestion shadow mode"). It supports **only** V4, CPMM, CLMM, Orca, DLMM, PumpFun (`_ => return None`, lib.rs:233). **PumpSwap, MeteoraDammV2 and RaydiumLaunchLab cannot be constructed through this entry point.**
* `program_id_to_protocol` (lib.rs:167) omits `METEORA_DAMM_V2_PROGRAM_ID` and `RAYDIUM_LAUNCHLAB_PROGRAM_ID`, so the stream path drops those pools before decoding.
* The HTTP-poll ingest path (`arb-ingest/registry.rs:876,905,1930-1933`) hardcodes `fee_bps = None` for PumpSwap, MeteoraDammV2, RaydiumLaunchLab → "on-chain fee undecodable; skipping (fail closed)" (registry.rs:~916). These three never reach `PoolState` via poll.
* `arb-engine` pricing (`adapters.rs::quote_hop`) uses its own math on `PoolState`, not the `AmmPool` adapters, except `PumpFunBondingCurve::from_state` for the Pump.fun hop (adapters.rs:326). Pools with `transfer_fee_bps != 0` or `has_risky_token` are rejected outright (adapters.rs:~192).
* `arb-tx::build_swap_hop_instruction` uses adapter-supplied `PoolExecutionAccounts`; for V4, CPMM, CLMM, Orca, DLMM only. DammV2/LaunchLab builders exist but registry never populates their `execution_accounts`, so they fail closed at tx-build (arb-tx lib.rs:2410,2479).
* Classification therefore splits: **V4, CPMM, CLMM, Orca, DLMM, PumpFun** = quote+decode used on (streaming or poll) runtime path; **PumpSwap** = fully-implemented adapter reachable only via `arb-ingest::pump_state::PumpSwapJoinedState::into_pool` (pump_state.rs:272) — flat path only, not the poll registry, not `decode_amm_pool`; **MeteoraDammV2, RaydiumLaunchLab** = implemented, tested, effectively dead at runtime.

## 1. Raydium AMM v4 — `raydium_v4.rs`

1. Program ID `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` (arb-constants/src/lib.rs:147); no SDK/IDL pin, hand-rolled offsets. Serum/OpenBook market program default `srmqPvym…` (v4.rs:63) is the **legacy Serum v3** program; on-chain value is authoritative (v4.rs:342). Amm authority PDA from constant `RAYDIUM_V4_AUTHORITY` with re-derivation fallback `b"amm authority"` (v4.rs:127-133).
2. Quote: integer `cpmm_out(dx, x, y, gamma)` with `gamma = 10000 - fee_bps` (v4.rs:356-375, arb-math/cpmm.rs:67). Mathematically equals AMM v4's upfront-fee-on-input constant-product. u128/U256 — no floats. Correct for v4 semantics.
3. Fee: numerator/denominator parsed from account @176/184 (v4.rs:298-304), default 25/10_000 (v4.rs:76-77). `fee_bps()` converts (v4.rs:157-166) — fine. Effective reserves subtract `need_take_pnl_coin/pc` from vault balances (v4.rs:88-100) — this is required for v4 and is done. LP/Protocol PnL taken out of `refresh_effective_reserves`. No Token-2022 handling (v4 is SPL-only; `transfer_fees_bps()=(0,0)`, `has_risky_token()=false`, v4.rs:172-178) — acceptable, though it will misprice if fed a Token-2022 mint (defense-in-depth gap, not exploitable on-chain since v4 pools are SPL).
4. Token ordering: `base_mint/quote_mint` read from account @400/432 (v4.rs:322-329); `a_to_b` = base→quote; reverse direction reuses identical account set with swapped reserves — correct for v4 (no X/Y vs A/B ambiguity; coin/pc convention is fixed in layout). Engine maps `a_to_b` by mint equality (arb-engine adapters.rs `quote_route`).
5. Remaining accounts: `execution_accounts()` returns `target_orders = pool_id` when unset (v4.rs:187-191) — **suspect**: target_orders is an independent account on-chain; substituting the pool address will make `build_raydium_v4_swap_instruction_versioned` pass a non-target-orders account as `amm_target_orders`. Inert only while V4 swap runs its V2/optional-account path; flag as latent bug. OpenBook market parse offsets (nonce @45, coin_vault @117, pc_vault @165, event_queue @253, bids @285, asks @317, v4.rs:241-260) match the de-facto Serum-v3/OpenBook layout used across SDKs; vault_signer PDA derived as `[market, nonce_le]` (v4.rs:268-276) — correct.
6. Slippage: exact-in only (`quote(amount_in, a_to_b)`). `min_amount_out` supplied by caller (arb-tx). No exact-out support.
7. Classification: **quote + execution adapter, live** (poll path requires vaults + reserve parse; stream path via `decode_amm_pool`). arb-tx V4 builder defaults `swap_version = V2` (v4.rs:202) — swapBaseIn V2 requires OpenBook (not Serum v3); OK on current mainnet. `is_arbitrage_eligible` requires `status == 6` (v4.rs:103-109) — correct for v4 active.
8. Account layout check: offsets match the canonical LIQUIDITY_STATE_LAYOUT_V4 (status@0, fee_num@176, fee_den@184, need_take_pnl_coin@192, need_take_pnl_pc@200, base_vault@336, quote_vault@368, coin_mint@400, pc_mint@432, open_orders@496, market_id@528, market_program_id@560, target_orders@592, size 752). **Parsing correct.** OpenBook side: `OPENBOOK_MARKET_MIN_SIZE = 388`, offsets 117/165/253/285/317 as above — **consistent**. Serum event queue/bids/asks are not tracked for reserves (correct — they are not part of AMM depth).

## 2. Raydium CP-Swap (CPMM) — `raydium_cpmm.rs`

1. Program `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` (arb-constants:151). No SDK pin; offsets verified against `raydium-cp-swap` `states/pool.rs` (`PoolState`, Anchor zero_copy `repr(C,packed)`, LEN 637) — **every offset matches**: amm_config@8, vault_0@72, vault_1@104, mint_0@168, mint_1@200, program_0@232, program_1@264, observation@296, status@329, mint_0_decimals@331, mint_1_decimals@332, protocol_fees@341/349, fund_fees@357/365, enable_creator_fee@390, creator_fees@397/405 (cpmm.rs:244-274). `RAYDIUM_CPMM_POOL_MIN_SIZE = 373` + creator-fee tail read only if `len >= 413` — correct.
2. Quote: integer `cpmm_out_upfront_fee_ceil` (arb-math/cpmm.rs:41), fee denom `1_000_000` ppm (cpmm.rs:13) — matches on-chain upfront-input-fee semantics. No floats. **Direction handling passes `a_to_b=true` as base→quote consistently.**
3. Fee: `trade_fee_rate` default hardcoded `2_500` ppm (25 bps) "until AmmConfig loaded" (cpmm.rs:76) — **risk**: a pool can be quoted and published as fee-valid at 25 bps before its AmmConfig arrives via stream/poll. `apply_amm_config_data` offsets trade_fee @12..20, creator_fee @108..116 (cpmm.rs:100-124) — **verified correct** against `states/config.rs` (`AmmConfig`: disc(8) bump@8 disable_create_pool@9 index@10 trade_fee@12 protocol_fee@20 fund_fee@28 create_pool_fee@36 protocol_owner@44 fund_owner@76 creator_fee@108). `effective_fee_rate_ppm = trade + creator (only if enable_creator_fee)` (cpmm.rs:126-133). **Deviation:** on-chain `CreatorFeeOn` (BothToken/OnlyToken0/OnlyToken1, see `is_creator_fee_on_input`) decides whether creator fee is taken from input or output; the adapter unconditionally folds creator fee into the input-side fee, mispricing the direction where creator fee is output-side. Protocol/fund fees are correctly treated as locked reserves (`update_reserves`, cpmm.rs:86-95). Token-2022: `transfer_fee_0/1_bps` default 0, never populated from mint decode; `apply_transfer_fee_ceil` applied at both legs with `u64::MAX` max-fee cap (cpmm.rs:297,314) — fee-cap never applied.
4. Direction: `token_0/token_1` from account; mint byte order is canonical (program requires mint_0 < mint_1), so `a_to_b` via caller mint-equality is safe. Programs 0/1 read @232/264 to detect Token-2022 ✓ (cpmm.rs:249-260). Reverse direction swaps `(r_in, r_out)` — correct, same accounts.
5. Remaining accounts (arb-tx `RaydiumCpmmAccounts::for_pool`): input/output vault chosen by `mint_in.to_bytes() < mint_out.to_bytes()` (arb-tx lib.rs ~2331) — consistent with 0/1 ordering but does **not** consult pool state's mints; if a pool violated ordering (impossible per program), this breaks. Discriminator `RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR = [143,190,90,218,196,30,51,222]` (arb-constants:237) — equals `sha256("global:swap_base_input")[..8]`, a widely reproduced constant ✓.
6. Slippage: exact-in only. `min_amount_out` from caller. No exact-out.
7. Classification: **quote + execution, live** (poll path: needs vaults and fee from AmmConfig; stream path via decode). Swap-disabled bit 2 of `status` honored (cpmm.rs:284) — matches PoolStatusBitIndex::Swap=bit2 ✓.
8. Layout check: **fully correct**, see (1). Observation account tracked as required dependency (cpmm.rs:195-202) — needed by `swap_base_input` (it's a required account) ✓.

## 3. Raydium CLMM — `raydium_clmm.rs`

1. Program `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` (arb-constants:153). Hand-rolled; PoolState layout (after disc+bump): amm_config@9, mint_0@73, mint_1@105, vault_0@137, vault_1@169, observation@201, decimals@233/234, tick_spacing@235, liquidity@237(u128), sqrt_price_x64@253, tick_current@269 (clmm.rs:302-323) — matches the public `PoolState` zero_copy layout (bump@8, amm_config@9, owner@41, …, status@~656 area). `status` only read if len > 389 (clmm.rs:324) — the on-chain status field sits near the end (~offset 656 region after `DynamicFeeRate`/`SwapStats` additions in later program upgrades); **@389 is not the canonical status offset on current deployments — status-based swap-disable likely inert.** TickState size 168, 60 ticks/array, array min size 10124 (clmm.rs:16-19) — correct.
2. Quote: integer CLMM across tick arrays via `arb_math::clmm::quote_clmm_with_ticks_per_array`; fee units hundredths-of-a-bip / 1e6 denominator (`fee_rate_ppm` / 100 → bps, clmm.rs:185-187) — consistent with on-chain `trade_fee_rate` (ppm). No floats.
3. Fee: hardcoded default `fee_rate_ppm = 2_500` "Updated by AmmConfig dependency" (clmm.rs:67) with the same pre-config quoting risk as CPMM. `apply_amm_config_data` reads `trade_fee_rate @47..51` as u32 (clmm.rs:145-151): **actual AmmConfig layout is disc(8) bump@8 index@9 protocol_fee_rate(u32)@11..15 trade_fee_rate(u32)@15..19?** — comment itself says `trade_fee_rate(4 @47..51)` after owner(32). Real Raydium CLMM AmmConfig: disc 8, bump 1 @8, index 2 @9, owner 32 @11..43, protocol_fee_rate u32 @43..47, trade_fee_rate u32 @47..51 — ✓ correct as written. Token-2022: same unwired transfer-fee situation; both legs' `apply_transfer_fee_ceil` uses `u64::MAX` cap.
4. Direction: `token_mint_0/1` ordering (program enforces mint_0 < mint_1) — adapter's `a_to_b` comes from caller; arb-tx uses `zero_for_one = mint_in < mint_out` (arb-tx ~2365) — correct under 0/1 ordering. Tick arrays derived with **big-endian** `start_tick_index` (clmm.rs:80-91) — matches Raydium's `tick_array` PDA seed convention (unlike Orca's decimal-string seed — good contrast, both correct for their protocols).
5. Remaining accounts: `PoolExecutionAccounts::RaydiumClmm{ amm_config, vaults, observation_state, tick_array_pdas, bitmap_extension }`; arb-tx builds `RaydiumClmmAccounts::with_tick_arrays` and a `sqrt_price_limit` derived as `(min_amount_out * 2^64)/amount_in` (arb-tx ~2418) — **wrong dimensionality**: CLMM `sqrt_price_limit` must be a Q64.64 *sqrt price* bound; the code instead computes `(min_amount_out * 2^64) / amount_in` — a linear price ratio shifted by 64 bits. For realistic ratios this lands nowhere near the actual price path, so the effective limit is either inert garbage or (if it falls below MIN_SQRT_PRICE_X64) causes every swap to abort with a limit violation. It should clamp to the global MIN/MAX sqrt-price constants (already used in the `else` arm, arb-tx ~2419) or derive a real sqrt-price bound from the same CLMM tick math the engine uses for quoting. Flag: high-severity instruction-layer correctness bug (unreachable or miscomputed limits; masked only because `min_amount_out > 0` forces this bad arm whenever `amount_in > 0`). Discriminator `RAYDIUM_CLMM_SWAP_DISCRIMINATOR = [248,198,158,145,225,117,135,200]` = sha256("global:swap")[..8] ✓; `SWAP_V2_DISCRIMINATOR = [43,4,237,11,26,201,30,98]` = sha256("global:swap_v2") ✓ plausible; CPMM verification test pins the same swap discriminator (arb-constants tests assert both, consistent).
6. Slippage: exact-in only. `amount_specified_is_input=true` hardcoded in arb-tx call (`build_raydium_clmm_swap_instruction(..., true)`) ✓ consistent with exact-in quoting.
7. Classification: **quote + execution, live** (poll: CLMM needs tick arrays fetched via `raydium_clmm_tick_array_pdas`; stream: via decode). Swap-disable bit 4 of status (clmm.rs:336) — Raydium CLMM status bits: bit4 = disable swap? Raydium CPMM uses bit2; CLMM `PoolStatusBitIndex::Swap` is also bit index 2 → value 4, i.e. `(1 << 4)` = 16 is **wrong** for CLMM (should be `status & 4`). Flag as real bug (swap-disable check inert at best, blocks trading on wrong bit at worst).
8. Layout check: tick array header 44 bytes (disc 8 + pool 32 + start_tick_index i32 @40..44) — parse @40..44 (clmm.rs:116) ✓; ticks 168-byte stride, `liquidity_net` @+4 (i128), `liquidity_gross` @+20 (u128) (clmm.rs:118-131) — matches Raydium `TickState` (tick i32 @0, liquidity_net @4, liquidity_gross @20) ✓. Whirlpool-style `initialized = gross > 0` is an approximation (ignores "tick initialized but zeroed" edge cases) — acceptable for quoting.

## 4. Orca Whirlpool — `orca_whirlpool.rs`

1. Program `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` (arb-constants:155). Hand-rolled but **offsets verified against `whirlpools/main` `state/whirlpool.rs`** (fetched read-only for this audit): disc(8) config@8 bump@40 tick_spacing@41..43 `fee_tier_index_seed`@43..45 fee_rate@45..47 protocol_fee_rate@47..49 liquidity@49..65 sqrt_price@65..81 tick_current_index@81..85 owed_a@85 owed_b@93 mint_a@101..133 vault_a@133..165 fee_growth_a@165 mint_b@181..213 vault_b@213..245. Adapter reads exactly 41/45/49/65/81/101/133/181/213 (whirlpool.rs:270-296) — **correct**, and correctly accounts for the `fee_tier_index_seed` field that naive layouts miss (registry.rs independently reads fee @45..47, consistent). RewardInfo size 128, account LEN 653, discriminator `[0x3f,0x95,0xd1,0x0c,…]`. `WHIRLPOOL_MIN_SIZE = 300` (whirlpool.rs:14) — allows truncated accounts; only first 245 bytes used, fine.
2. Quote: integer CLMM `quote_clmm` (arb-math/clmm.rs) with 1e6-denominator fee units; `fee_rate` u16 = hundredths of a bip ✓ matches on-chain. Requires decoded tick arrays from a coherent snapshot; refuses quote otherwise (whirlpool.rs:310-314) — fail-closed ✓. No floats. Reserves() computes virtual reserves from liquidity/sqrt_price via U256 (whirlpool.rs:142-154) ✓.
3. Fee: **no hardcoded default** — `update_from_account` never sets fee; `fee_rate` comes from decoded account (45..47) ✓ good. `fee_bps()` = fee_rate/100 (integer floor, whirlpool.rs:156-162) — 2500→25 ✓. Protocol/adaptive fee tiering: `AdaptiveFeeTier` pools have `fee_tier_index_seed != tick_spacing`; adapter ignores it — quoting uses `fee_rate` regardless (correct: fee_rate is the *current* rate only if the program updates it; for adaptive pools the on-chain fee is dynamic and depends on the oracle — the adapter will **under/overstate fees** for adaptive pools). Token-2022 transfer fee applied at both legs, `u64::MAX` cap; adaptive-fee oracle (required writable in `remaining_accounts` for adaptive pools, see `swap.rs` IDL accounts) is **not** passed in arb-tx's `build_orca_whirlpool_swap_instruction_11` — adaptive pools would fail on-chain or trade without the oracle update. Flag.
4. Direction: `a_to_b` from caller; mints a/b from account @101/181 ✓; reverse swaps roles via `(tf_in, tf_out)` swap and `quote_clmm(…, a_to_b, …)` ✓. **arb-tx direction bug**: `let a_to_b = hop.mint_in.0 == WSOL_MINT` (arb-tx ~2181) — direction derived from "input is WSOL", not from mint_a/mint_b. Any non-SOL whirlpool pair (USDC/USDT) gets `a_to_b=false` permanently → wrong vaults/accounts for zero_for_one swaps → guaranteed on-chain failure or wrong-side trade. High-severity instruction-layer direction bug.
5. Remaining accounts: `derive_tick_array_pda` uses seeds `[b"tick_array", whirlpool, start_tick.to_string().as_bytes()]` (whirlpool.rs:78-88) — matches Orca's decimal-string-seed convention ✓ (verified: FixedTickArray seeds, `swap.rs` account `tick_array_0..2`). Oracle PDA `[b"oracle", whirlpool]` (whirlpool.rs:117-122) ✓ matches `[seeds=[b"oracle", whirlpool]]`. Tick arrays for swap: window around `tick_current_index`, 88 ticks/array, a_to_b path shifts one array back when `tick == start` (whirlpool.rs:91-114) — matches on-chain shifted ranges. `execution_accounts` falls back to `tick_array_pdas_for_swap(true, 3)` for both directions — the `a_to_b` argument is hardcoded `true` (whirlpool.rs:187) so even buys may reference the wrong window. Flag.
6. Slippage: exact-in only. `other_amount_threshold` = min_out ✓ (arb-tx `amount_specified_is_input=true`).
7. Classification: **quote + execution, live** (both poll via `decode_clmm_state` in registry and stream via decode_amm_pool). Tick arrays must be subscribed — done in registry derived pubkeys ✓.
8. Layout check: **correct**, see (1). Tick facade parsing (arb-math/clmm.rs:631-666): FixedTickArray = disc(8) whirlpool(32) start_tick_index i32 @40..44 ✓, ticks 113 bytes stride: `initialized` bool @+0, `liquidity_net` i128 @+1, `liquidity_gross` u128 @+17 (matches `state/tick.rs` `Tick` struct); `parse_tick_array_account` validates `start_tick_index.rem_euclid(88*spacing) == 0` and rejects non-0/1 bool bytes — strong decoder ✓. Whirlpool tick array account size 9988 (44 + 88*113) ✓.

## 5. Meteora DLMM — `meteora_dlmm.rs`

1. Program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (arb-constants:157). Header comment cites "official DLMM v0.12.0 IDL … `repr(C)` bytemuck". Offsets: StaticParameters @8..40 (base_factor@8, filter_period@10, decay_period@12, reduction_factor@14, variable_fee_control@16, max_volatility_accumulator@20, min/max_bin_id, protocol_share@32, base_fee_power_factor@34, function_type@35, collect_fee_mode@36), VariableParameters @40..72, bump_seed@72, bin_step_seed@73..75, pair_type@75, active_id@76, bin_step@80, status@82, token_x_mint@88, token_y_mint@120, reserve_x@152, reserve_y@184, protocol_fee@216..232, reward_infos@264..552, oracle@552, bitmap@584, last_updated_at@712 (dlmm.rs:315-406). **Plausible and internally consistent (LB_PAIR_MIN_SIZE 300); not independently re-verified against a current on-chain account.** Note: `last_update_timestamp` read from VariableParameters @56..64, but the doc comment for the account layout puts the same field at `712..720 last_updated_at` — the adapter uses the VariableParameters one (dlmm.rs:394) which is what `quote_dlmm_multi_bin` wants ✓.
2. Quote: integer bin-walk `quote_dlmm_multi_bin` with Q64.64 prices (`get_price_from_id`), variable fee via volatility accumulator — models on-chain DLMM incl. dynamic fees. No floats. **Limit-order liquidity folding**: open_order_amount + processed_order_remaining folded into `amount_x/amount_y` depending on ask side (dlmm.rs:170-181) — good, but protocol-fee attribution for that liquidity is approximated as curve liquidity (documented in code) — minor.
3. Fee: base fee from **decoded on-chain StaticParameters** — `base_factor * bin_step * 10 * 10^base_fee_power_factor * 10_000 / 1e9` ≈ `base_factor*bin_step*10^p / 10^4` bps (dlmm.rs:229-238) — algebraically equals the on-chain base-fee formula (bps). `fee_bps()` excludes variable/dynamic fee — so `is_fee_valid`/engine fee checks see only the base fee while actual quotes include dynamic fee; conservative, acceptable, but inconsistent. `collect_fee_mode` and `function_type` decoded but variable-fee parameters (`volatility_accumulator`, `index_reference`) are taken from the snapshot without verifying `filter_period` semantics. Token-2022: same both-leg `apply_transfer_fee_ceil` with u64::MAX cap.
4. Direction: `token_x/token_y` from account (x = lower-sorted mint, program enforces x < y); `a_to_b` = X→Y; reverse reuses same bin arrays with direction flipped inside `quote_dlmm_multi_bin` ✓. Bin-array PDA `[b"bin_array", lb_pair, index i64 le]` (dlmm.rs:91-98) — correct seed form (little-endian i64, not BE) ✓. BinArrayBitmapExtension PDA `[b"bitmap", lb_pair]` (dlmm.rs:117-123) ✓ matches `bin_array_bitmap_extension` seed. **arb-tx fallback direction bug**: `swap_for_y = mint_in < mint_out` (arb-tx ~2258) for the mainnet fallback path only; exec-accounts path doesn't compute direction at all — it must match the instruction's intended `swap_for_y`; verify `build_meteora_dlmm_swap_instruction_with_bin_arrays` encodes the correct boolean. Flag for manual check.
5. Remaining accounts: oracle, bitmap_extension (always `Some(derive_…)`, dlmm.rs:282) — on-chain `swap2` accepts optional bitmap extension; passing it unconditionally requires the extension account **to exist** — for pairs that never used the bitmap extension PDA, the account won't exist → `AccountNotInitialized` failure. Should be `Option`-gated on pair size/usage. `required_dependency_pubkeys` includes mints themselves (dlmm.rs:286-296) — unusual but harmless. Discriminator `METEORA_SWAP2_DISCRIMINATOR = [65,75,63,76,235,91,91,136]` = sha256("global:swap2")[..8] — plausible ✓ (used by arb-tx at lib.rs:1187,1744).
6. Slippage: exact-in only (`amount_in`, min_out caller-supplied) ✓ consistent with DLMM's `swap2` args.
7. Classification: **quote + execution, live** — DLMM is the best-supported venue in this repo (registry fully wired, tests exist). `bin_array_pdas_around_active` uses a hardcoded `500 / bin_step` bins window (dlmm.rs:100-115) — heuristic, may miss arrays for very large trades.
8. Layout check: **BinArray**: header 56 (disc 8 + index i64 @8..16 @dlmm.rs:162), 70 bins × 144 B (`BINS_PER_BIN_ARRAY = 70`, METEORA_BINS_PER_BIN_ARRAY, bin @base+0 amount_x u64, +8 amount_y, +112 open_order_amount, +128 processed_order_remaining, +140 ask_side ✓ per in-code doc) ✓ consistent with DLMM v0.12. `start_bin_id = array_index * 70` ✓. `bin_id_to_array_index` floor semantics for negatives handled via `(b - (n-1)) / n` (dlmm.rs:85-89) ✓. LbPair offsets not independently verified — flag as "verified-by-author only".

## 6. Meteora DAMM v2 (cp_amm) — `meteora_damm_v2.rs`

1. Program `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` (arb-constants:266), authority `HLnpSz9h2…` constant. Offsets (commented as "verified binary layout"): cliff_fee_numerator@8, fee_scheduler_mode@16, number_of_period@22, period_frequency@24, reduction_factor@32, token_a_mint@168, token_b_mint@200, vault_a@232, vault_b@264, whitelisted_vault@296?, config read @296 as `config` (**comment says 296..328 is whitelisted_vault but code reads `self.config = read_pk(296)` — offset reused; either the PoolState has config@296 and whitelisted_vault elsewhere, or config is misread** — flag), liquidity@360..376, sqrt_min@424, sqrt_max@440, sqrt_price@456..472, activation_point@472..480, token flags@482/483, collect_fee_mode@484, MIN_SIZE 485 (damm_v2.rs:157-210). Unverified against live account.
2. Quote: integer virtual-reserve CLMM-style math (`damm_v2_virtual_reserves`, `quote_damm_v2_exact_in_with_fee_mode`) — no floats. Bounded by sqrt_min/max — matches cp_amm's concentrated-liquidity-over-full-range model (dammv2 = constant product in price over [sqrt_min, sqrt_max]).
3. Fee: decoded from pool (`cliff_fee_numerator` etc.) ✓ better than LaunchLab; **default `cliff_fee_numerator = 2_500_000` ("25 bps") until decoded, but `update_from_account` overwrites it from data @8..40 — so default only matters for pre-decode quotes; DAMM v2 fee denominator is 1e9 (25bps = 2.5e6 ✓ consistent).** `fee_bps()` uses div_ceil (damm_v2.rs `effective_fee_numerator` in arb-math) — conservative. Dynamic fee: `dynamic_fee_initialized` never set to true by decoder (`DammV2FeeParams::default()` → false) — variable fee leg silently dropped. `collect_fee_mode` mapping is duplicated arms (`1 => !a_to_b`, `2 => !a_to_b`) with comment "OnlyB / Compounding" — and the doc comment in arb-math says collect_fee_mode: 0=BothToken(input?), 1=OnlyB, 2=OnlyA — **contradictory and likely wrong for OnlyA (mode 2 must be `a_to_b`)** — fees taken from wrong leg for OnlyA pools. Flag.
4. Direction: mint ordering from account (a = lower) ✓ convention; arb-tx derives `a_to_b = mint_in < mint_out` (arb-tx ~2429) ✓ consistent with 0/1 ordering. Reverse: same vaults swapped via `(input_vault, output_vault)` selection ✓.
5. Remaining accounts: `config`, pool, `authority` (constant), user ATAs, pool vaults, mints, user authority (readonly!), token programs, `event_authority` (PDA `b"__event_authority"`), program. **Bug: `AccountMeta::new_readonly(user_authority, true)`** (arb-tx ~2493) — the user must be a **signer**, which `new_readonly(.., true)` does encode (is_signer=true) — OK, but the account is marked readonly while it must sign and is never marked writable — fine for a signer that doesn't receive tokens. However DAMM v2's `swap` requires the user to be writable? Usually not. Acceptable. Discriminator written inline `[248,198,158,145,225,117,135,200]` (arb-tx ~2432) instead of the `METEORA_DAMM_V2_SWAP_DISCRIMINATOR` constant — duplicative but correct value.
6. Slippage: exact-in only.
7. Classification: **dead code at runtime** (see §0). No poll-path fee (registry → None), no stream decode (`program_id_to_protocol` omits it), tests only. Execution path untested against devnet/mainnet simulation.
8. Layout check: see (1) — not independently verified; the single most suspicious field is `config`@296.

## 7. Raydium LaunchLab — `raydium_launchlab.rs`

1. Program `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` (arb-constants:279). No SDK pin. Offsets (commented "from official IDL"): status@17, base_decimals@18, quote_decimals@19, virtual_base@37..45, virtual_quote@45..53, real_base@53..61, real_quote@61..69, global_config@141..173, platform_config@173..205, base_mint@205..237, quote_mint@237..269, base_vault@269..301, quote_vault@301..333, token_program_flag@365 (launchlab.rs:157-199). **Unverified**; note the mints @205/237 are past the vaults here (vaults@269/301) — internally consistent with a plausible LaunchLab `PoolState`, but offset 17 for status is unusual (expected near 0 after disc+bump+padding) — flag.
2. Quote: integer bonding-curve via `quote_launchlab_buy_exact_in / sell_exact_in` (arb-math/launchlab.rs) using virtual+real reserves — buy: fee-on-input then constant-product from **virtual** reserves — correct shape for LaunchLab. Sell: fee on gross out. No floats. Note `reserves()` returns `virtual+real` sums (launchlab.rs:93-98) but the actual curve math uses virtual-vs-real separately, so `reserves()` is only an approximation for engine reserve guards — acceptable.
3. Fee: **`total_fee_bps` defaults to 100 (1%) hardcoded** (launchlab.rs:64) and *no code path ever overrides it* (PlatformConfig/GlobalConfig never parsed) — every LaunchLab quote uses 1%, and `fee_bps()` reports `Some(100)` as if verified. This is the worst fee handling in the crateset: it is a hardcoded-fee pool type that cannot be detected from the adapter. Real LaunchLab tiers are config-driven (protocol + platform + creator). Flag high.
4. Direction: a_to_b = base→quote (sell), b_to_a = quote→base (buy); mints from account ✓. `is_buy` in arb-tx LaunchLab builder derives from `mint_in == WSOL` — wrong for USDC-quoted LaunchLab pools (they'd be treated as sells). Same class of bug as Orca. Flag.
5. Remaining accounts: builder (arb-tx ~2479-2530) supplies authority/global_config/platform_config/vaults/event_authority + mints + token programs + program. **Discriminator bug**: `buy exact in = [102,6,61,18,1,218,235,234]` and `sell exact in = [51,230,133,164,1,127,131,173]` are written inline (arb-tx ~2485-2492) — these are the **pump.fun** `PUMP_FUN_BUY/SELL_DISCRIMINATOR` values, not the LaunchLab `buy_exact_in`/`sell_exact_in` discriminators defined in arb-constants:281-285 ([250,234,13,123,213,156,19,236] / [149,39,222,155,211,124,152,26]). LaunchLab swaps built by this code would **not decode** on-chain (WrongDiscriminator) or worse, match an unintended instruction. The LaunchLab constants are defined but never used. High-severity. Also `event_authority` is passed as `new_readonly(..,false)` but LaunchLab expects event authority writable? Not critical. `MIN_SIZE=366` fine.
6. Slippage: exact-in only.
7. Classification: **dead code at runtime** (registry None fee → skipped; stream decode absent). Adapter parses status!=0 as "graduated" (launchlab.rs:205-208) — heuristic, plausible.
8. Layout check: **not verified** — see (1); treat all offsets as unvalidated.

## 8. Pump.fun bonding curve — `pump_fun.rs`

1. Program `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` (arb-constants:159). Account discriminator `[23,183,248,55,96,216,172,96]` = sha256("account:BondingCurve")[..8] — **correct** (well-known). Layout decode: virtual_token_reserves@8, virtual_sol_reserves@16, real_token@24, real_sol@32, total_supply@40, complete bool@48, creator@49..81, is_mayhem_mode@81, is_cashback_coin@82, size 83 (pump_fun.rs:13,134-178) — matches current pump.fun `BondingCurve` (Anchor, with creator + mayhem/cashback flags) ✓. `PUMP_FUN_GRADUATION_LAMPORTS = 85_000_000_000` (arb-constants:381) — stale by design? Current graduation threshold is 85 SOL — OK as heuristic for `progress_bps` only.
2. Quote: integer virtual-curve `cpmm_out_fee` buy: fee-on-input then curve; sell: curve then fee-on-output; buy capped at `real_token_reserves`, sell capped at `real_sol_reserves` (pump_fun.rs:243-299) — correct Pump.fun semantics. Requires `fee_bps` explicitly supplied (`quote_with_fee` errors otherwise — no hardcoded 1.25% fallback) ✓ excellent. No floats. Mayhem/cashback curves rejected (pump_fun.rs:224-228) — conservative ✓.
3. Fee: sourced externally (engine supplies `fee_numerator` from on-chain `GlobalConfig`/Pump Fees via ingest — adapter refuses to guess). Denominator must be 10_000 (arb-engine quote_pump_fun_hop enforces) — BPS convention ✓. Creator fee/mayhem dynamics not modeled beyond refusal. Token-2022 N/A (pump.fun tokens are SPL with pump mint authority — fine).
4. Direction: **naming inversion** — `base_mint` = WSOL (quote asset), `quote_mint` = meme token; `a_to_b` = buy (SOL→token) (pump_fun.rs:20-21, 343-360). Engine and adapter agree (WSOL-only check, adapters.rs `quote_pump_fun_hop`). Reverse = sell ✓ same constants.
5. Remaining accounts: instruction built by `PumpFunAccounts::for_mint` + `build_pump_fun_swap_instruction` in arb-tx; discriminators: BUY `[102,6,61,18,1,218,235,234]` = sha256("global:buy") ✓, SELL `[51,230,133,164,1,127,131,173]` = sha256("global:sell") ✓ (both match well-known pump.fun IDL and arb-constants tests). Note instruction-data uses only 24 bytes (disc + 2×u64) — current pump.fun `buy` requires additional args (e.g. `track_volume: Option<bool>` in newer IDL) — if the deployed program requires the extra arg, instructions will fail to deserialize. Flag as "verify against current on-chain IDL".
6. Slippage: exact-in only.
7. Classification: **quote + execution, live** (engine Pump.fun hop live; stream path supports decode via `decode_amm_pool`; poll path resolves fee; instruction path exists). `progress_bps`, `is_arbitrage_eligible` fine.
8. Layout check: discriminator ✓, bool strictness (0/1 only) ✓, creator@49 ✓ — **correct**. `mode_flags_present` forces the two mode flag bytes to exist — good.

## 9. PumpSwap — `pump_swap.rs`

1. Program `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` (arb-constants:161). Account discriminator `[241,154,109,4,17,177,109,188]` = f19a6d0411b16dbc — matches pump.fun's published PumpSwap Pool discriminator ✓. `PUMP_SWAP_BUY_DISCRIMINATOR` = sha256("global:buy") ✓ shared; `PUMP_SWAP_BUY_V2_DISCRIMINATOR = [184,23,238,97,103,197,211,61]` = sha256("global:buy_v2") plausible; `PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR = [194,171,28,70,104,77,91,47]` used for buys in arb-tx (lib.rs:1862) — plausible ✓; **`PUMP_SWAP_SELL_V2_DISCRIMINATOR = [51,230,133,164,1,127,131,173]` is byte-identical to `PUMP_FUN_SELL_DISCRIMINATOR`** (arb-constants:301) — it is not sha256("global:sell_v2"); sell_v2 in arb-tx for exact-base-in sells will likely fail to decode on-chain. High-severity discriminator copy-paste. (Note: `[51,230,133,164,1,127,131,173]` really is sha256("global:sell") — using it for "sell_v2" is unverified; if on-chain sell_v2 has a different hash, every PumpSwap sell reverts.)
2. Quote: integer CPMM. Buy: ceil fees on input per component, remainder through curve; sell: curve then per-component ceil fees from gross output; PPM (1e6) fees, requires integer-BPS-multiple ppm (pump_swap.rs:319-417) — conservative, matches on-chain per-component ceil rounding (documented). No floats.
3. Fee: **best in class** — no hardcoded fallback; `PumpSwapFeeConfig::from_live_tiers` (tiered by market cap, strictly-increasing check, no averaging/fallback) + canonical-vs-custom pool selection lives in arb-ingest::pump_state (`select_fee`, market_cap = supply × eff_quote / base_reserve, pump_state.rs:~520-560). `virtual_quote_reserves` (signed i128 @245..261) added to raw quote vault for pricing (pump_swap.rs:121-126) ✓ matches PumpSwap's signed virtual reserve — subtle and correct. Un-swept `protocol_fees`/`creator_fees` (@271/279) excluded from spendable quote reserve (real_quote_reserve) ✓ — prevents quoting fees as liquidity. `creator_fee_bps` decoded from header @261..269 but **never used in quoting** (dead field; tiers from FeeConfig win) — flag. Token-2022: none.
4. Direction: base/quote from account; buy = quote→base. arb-tx derives `is_buy` from `mint_in == WSOL || mint_in == USDC` (arb-tx ~2005) — hardcodes canonical quote mints; USDC-quoted pools fine, other quote mints misclassified. Flag. Reverse: same accounts, `base_to_quote` flag flips ✓; disable flags checked per direction (buy bit 3, sell bit 4, pump_swap.rs:322-329) ✓.
5. Remaining accounts (arb-tx V2 path): 17-account matrix — pool, global_config, base_mint, quote_mint, LP mint?, base/quote vaults, user ATAs, protocol_fee_recipient, protocol_fee_recipient_ata, coin_creator_vault_authority, coin_creator_vault_ata, token programs, user authority, event_authority (`b"__event_authority"` — correct seed form), program. `protocol_fee_recipient`/ATAs come from execution_accounts (registry/pump_state join) — **registry poll path can't build execution accounts for PumpSwap (returns None)** so live sells/buys depend on pump_state joined snapshots; otherwise MissingExecutionAccounts. Legacy V1 path exists for non-V2 pools.
6. Slippage: exact-in only.
7. Classification: **fully implemented; reachable only via `arb-ingest::pump_state::PumpSwapJoinedState::{into_pool,select_fee}`** — not via `decode_amm_pool`, not via poll registry (fee=None → skipped), not via stream `program_id_to_protocol` (it IS mapped — PumpSwap → PumpSwap at lib.rs:176 — but decode_amm_pool returns None for it, so stream path emits nothing). Engine integration partial (PoolState → kind Cpmm; adapters' direct `quote()` used only in tests/shadow). Instruction builder live in arb-tx behind execution_accounts.
8. Layout check: discriminator ✓, prefix fields (pool_bump@8, index u16@9, creator@11..43, base_mint@43..75, quote_mint@75..107, lp_mint@107..139, base_vault@139..171, quote_vault@171..203, lp_supply u64@203..211, coin_creator@211..243, is_mayhem@243, is_cashback@244, MIN_LEN 245) — consistent with known PumpSwap Pool prefix ✓. Appended 2026-10 fields (virtual_quote_reserves@245..261 i128, creator_fee_bps@261..269, can_edit_creator_fee@269, is_holder_reward@270, protocol_fees@271..279, creator_fees@279..287) with truncation checks for each range (pump_swap.rs:55-62) — defensive and correctly written; offsets unverified against a live mainnet account.

## 10. arb-constants — program IDs & discriminators (arb-constants/src/lib.rs)

Verified at file:line:
* V4 `675kPX9MH…` :147; V4 authority `5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1` :149 ✓ canonical; CPMM `CPMMoo8L3…` :151 ✓; CLMM `CAMMCzo5…` :153 ✓; Whirlpool `whirLbMiic…` :155 ✓; DLMM `LBUZK…` :157 ✓; Pump.fun `6EF8rrec…` :159 ✓; PumpSwap `pAMMBay6…` :161 ✓; LaunchLab `LanMV9sA…` :279 ✓; DAMM v2 `cpamdpZCG…` :266 ✓. `PUMP_FEES_PROGRAM_ID pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ` :20 — plausible, unverified.
* `RAYDIUM_CLMM_TICK_ARRAY_SEED = b"tick_array"` :253 ✓ (big-endian start tick — see §3.4). `RAYDIUM_CLMM_SWAP_DISCRIMINATOR [248,198,158,145,225,117,135,200]` = sha256("global:swap") ✓; `…SWAP_V2… [43,4,237,11,26,201,30,98]` :257 plausible; `RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR [143,190,90,218,196,30,51,222]` :237 ✓ known-good; `RAYDIUM_CPMM_AUTH_SEED b"vault_and_lp_mint_auth_seed"` :240 ✓; `RAYDIUM_CPMM_OBSERVATION_SEED b"observation"` :244 ✓.
* `METEORA_SWAP2_DISCRIMINATOR [65,75,63,76,235,91,91,136]` :224 ✓-plausible; `METEORA_BINS_PER_BIN_ARRAY = 70` :222 ✓ correct (NOT 64 — good); `METEORA_DAMM_V2_SWAP_DISCRIMINATOR [248,198,158,…]` :270 duplicates CLMM swap disc (same sha256("global:swap")) ✓ de-facto consistent; `METEORA_DAMM_V2_SWAP2_DISCRIMINATOR [65,75,63,…]` :272 duplicates DLMM swap2 disc — same instruction name hash, plausible but couples two protocols to one constant; `MEMO_PROGRAM_ID` :259 ✓.
* `PUMP_FUN_BONDING_CURVE_DISCRIMINATOR [23,183,248,55,96,216,172,96]` :169 ✓ known-good; BUY/SELL discs :171/173 ✓ known-good; PUMP_SWAP pool disc :165 ✓ known-good; `PUMP_SWAP_SELL_V2_DISCRIMINATOR [51,230,…]` :301 **wrong (see §9.1)**; `PUMP_SWAP_GLOBAL_CONFIG ADyA8h…` :163 and `PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR [149,8,156,202,160,252,176,217]` :24 unverified; `PUMP_FUN_GRADUATION_LAMPORTS = 85_000_000_000` :381 (protocol changes; adapter only uses it for progress bar).
* LaunchLab discs `BUY_EXACT_IN [250,234,13,123,213,156,19,236]` :281, `SELL_EXACT_IN [149,39,222,155,211,124,152,26]` :284 — defined, plausible, **never used** (arb-tx inlines pump discs instead — see §7.5).
* `JITO_TIP_ACCOUNTS` :57-66 — 8 accounts; list matches the canonical Jito tip set ✓. `jitodontfront` marker bytemuck form :132-136 ✓ (prefix-preserved base58 "jitodontfront111…" — test at :402-407 checks prefix).

## 11. Cross-cutting findings

* **C-1 (High)** arb-tx LaunchLab builder uses pump.fun discs; LaunchLab — if ever enabled — fails every swap. §7.5.
* **C-2 (High)** orca arb-tx `a_to_b = (mint_in == WSOL)`; wrong for all non-SOL whirlpools; similarly LaunchLab `is_buy = (mint_in == WSOL)`. §4.4, §7.4.
* **C-3 (High)** `PUMP_SWAP_SELL_V2_DISCRIMINATOR` == sha256("global:sell"); sells via `SellExactBaseInV2` unverified/likely broken. §9.1.
* **C-4 (Medium-High)** Raydium CLMM swap-disable check uses bit `1 << 4` = 16 instead of bit index 2 (value 4). §3.7. Orca adaptive-fee pools quoted without oracle/adaptive fee handling. §4.3.
* **C-5 (Medium)** Token-2022 transfer fee **never applied** on the live paths: `decode_transfer_fee_config`/`calculate_transfer_fee` (lib.rs:42-118, offsets verified correct: older@72/80/88, newer@90/98/106, ceil + max_fee cap, epoch selection) exist but are wired only into `token2022_has_risky_extension` for pump_state; all adapters set `transfer_fee_*_bps = 0` and `has_risky = false` by default, and `apply_raw_account`/`decode_yellowstone_account` hardcode `transfer_fee_bps = 0, has_risky_token = pool.has_risky_token()` (defaults false) → Token-2022 fees/extensions are **not priced and not gated on-stream**; `apply_transfer_fee_ceil(.., u64::MAX)` never applies max_fee caps anywhere; BPS-only `transfer_fee_bps` can't express epoch schedules. Engine would reject them anyway (`transfer_fee_bps != 0` → reject) — but they're always 0, so rejection never triggers. Double-counting does NOT occur (ceil applied at each of the two transfer legs, matching Token-2022 withholding).
* **C-6 (Medium)** `program_id_to_protocol` omits DammV2 & LaunchLab; `decode_amm_pool` omits PumpSwap/DammV2/LaunchLab → stream shadow mode silently ignores 3 protocols. §0.
* **C-7 (Medium)** Orca/Raydium `execution_accounts` tick-array fallback hardcodes `a_to_b=true` (whirlpool.rs:187, similarly DLMM/CLMM use current-tick-centered windows ignoring direction) → wrong arrays for reverse swaps on exec path. §4.5.
* **C-8 (Medium)** CLMM `sqrt_price_limit` dimensionally wrong (linear ratio << 64 instead of sqrt price). §3.5.
* **C-9 (Low-Medium)** `RaydiumV4Pool.execution_accounts.target_orders = pool_id` when unset; `is_arbitrage_eligible`/`max_arbitrage_amount` compare raw lamport-denominated reserves across mixed decimals. §1.5. LP `target_orders` default is wrong.
* **C-10 (Low)** Meteora DAMM v2 `collect_fee_mode` duplicate `!a_to_b` arms; `config` offset collision at 296; `dynamic_fee_initialized` never decoded; variable fee silently zero. §6.3/6.5.
* **C-11 (Low)** LaunchLab `total_fee_bps = 100` hardcoded, unoverrideable; LaunchLab/DAMM v2 entirely dead at runtime; DammV2 `BITMAP/event_authority` `__event_authority` correct; `MIN_SIZE` 366 plausible.
* **C-12 (Info)** `fee_bps()` reports base-only fee for DLMM; `reserve()` sums for LaunchLab approximate curve state; `PumpSwapPoolHeader.creator_fee_bps` decoded but unused; Whirlpool adapter handles `fee_tier_index_seed` correctly (impressive) but not adaptive fee tiers; `PumpSwap` sell-side creator fee skipped when `coin_creator == default` ✓ sensible.
* **C-13 (Info)** Slippage is exact-in everywhere; no exact-out; `DEFAULT_HOP_SLIPPAGE_BPS = 10` per-hop (arb-constants:352) — fine for hops; the real floor is final profit. CLMM/DLMM min-out bounds not enforced on-chain beyond min_amount_out.
* **C-14 (Info)** Raydium V4 parse reuses silence-on-error for all pubkeys (`if let Ok(b) = …`) — a truncated-but-≥752 body that fails a sub-slice copy yields half-initialized pool with `update_reserves` never called → reserves 0 → pool skipped (fail closed) ✓ acceptable.
* **C-15 (Info)** Serum vault-signer derivation for V4/OpenBook (`[market, nonce_le]`, v4.rs:268-276) — correct for Serum v3; OpenBook markets on current mainnet use the same derivation ✓.

## 12. Verdicts per adapter

| Adapter | Prog ID ok | Quote math | Fees | Dir. derivation | Exec accounts | Status |
|---|---|---|---|---|---|---|
| raydium_v4 | ✓ | ✓ CPMM upfront | decoded + need_take_pnl | mint order (caller) | V4 V2 builder; target_orders default wrong | Live |
| raydium_cpmm | ✓ | ✓ ppm upfront-ceil | decoded; default 25 bps pre-config risk; creator-fee side wrong | mint order | live; discs ok | Live |
| raydium_clmm | ✓ | ✓ CLMM int | ppm; swap-disable bit bug | mint order; BE tick PDAs | live; sqrt_price_limit dim bug | Live |
| orca_whirlpool | ✓ | ✓ CLMM int | decoded (good); adaptive tiers ignored | caller; arb-tx WSOL bug | live; tick-array fallback dir bug | Live |
| meteora_dlmm | ✓ | ✓ bin-walk + variable fee | base decoded; dynamic excluded from fee_bps() | caller; BE/LE seeds ok | live; bitmap_extension unconditional | Live |
| meteora_damm_v2 | ✓ (id) | ✓ virtual-reserves | decoded; collect_fee_mode arms broken | mint order | dead code | Not live |
| raydium_launchlab | ✓ (id) | ✓ curve | hardcoded 1% forever | caller; WSOL bug in arb-tx | dead; **pump discs in builder** | Not live |
| pump_fun | ✓ | ✓ bounded curve, explicit fee | explicit-only ✓ | WSOL convention (engine-enforced) | live; 24-byte arg may be stale | Live |
| pump_swap | ✓ | ✓ per-component ceil PPM | live tier table ✓ | caller; WSOL/USDC hardcode in arb-tx | live via pump_state only; sell_v2 disc suspect | Partially live |

## A. Byte-offset verification tables (requested: raydium_v4, orca_whirlpool, meteora_dlmm)

### A.1 Raydium AMM v4 (`raydium_v4.rs:284-354`) — VERIFIED against canonical LIQUIDITY_STATE_LAYOUT_V4

| Field | Offset | Type | Adapter line | Verdict |
|---|---|---|---|---|
| status | 0..8 | u64 | v4.rs:294 | ✓ (expect 6 = active; is_arbitrage_eligible requires ==6, v4.rs:104) |
| nonce | 8..16 | u64 | not read | ok |
| orderNum | 16..24 | u64 | not read | ok |
| depth | 24..32 | u64 | not read | ok |
| coinDecimals | 32..40 | u64 | not read | **not read — decimals come from mint snapshot instead (acceptable)** |
| pcDecimals | 40..48 | u64 | not read | same |
| state / resetFlag / minSize / volMaxCutRatio / amountWaveRatio / coinLotSize / pcLotSize / min/maxPriceMultiplier / sysDecimalValue | 48..128 | u64s | not read | ok |
| swap_fee_numerator | 176..184 | u64 | v4.rs:299 | ✓ (25 bps default) |
| swap_fee_denominator | 184..192 | u64 | v4.rs:303 | ✓ |
| need_take_pnl_coin | 192..200 | u64 | v4.rs:307 | ✓ (subtracted from base vault in refresh_effective_reserves) |
| need_take_pnl_pc | 200..208 | u64 | v4.rs:311 | ✓ |
| baseVault | 336..368 | pubkey | v4.rs:315 | ✓ |
| quoteVault (pc) | 368..400 | pubkey | v4.rs:319 | ✓ |
| baseMint (coin mint) | 400..432 | pubkey | v4.rs:323 | ✓ |
| quoteMint (pc mint) | 432..464 | pubkey | v4.rs:327 | ✓ |
| lpMint | 464..496 | pubkey | not read | ok |
| ammOpenOrders | 496..528 | pubkey | v4.rs:332 | ✓ |
| marketId (serum/openbook) | 528..560 | pubkey | v4.rs:337 | ✓ |
| marketProgramId | 560..592 | pubkey | v4.rs:342 | ✓ (on-chain value wins; default srmqPvym only pre-decode) |
| targetOrders | 592..624 | pubkey | v4.rs:347 | ✓ parse; see C-9 for exec fallback bug |
| withdrawQueue / lpVault / owner / lpReserve / padding | 624..752 | — | not read | ok |
| account size | 752 | — | RAYDIUM_V4_LAYOUT_SIZE v4.rs:12 | ✓ |

OpenBook/Serum MarketState parse (`apply_dependency_account`, v4.rs:239-279): vault_signer nonce @45..53 (u64) ✓, coinVault @117..149 ✓, pcVault @165..197 ✓, eventQueue @253..285 ✓, bids @285..317 ✓, asks @317..349 ✓ — the de-facto layout shared by Serum v3 and OpenBook v1 market accounts; vault signer derived as `create_program_address([market, nonce_le], market_program)` — correct. Note: bids/asks/event_queue are parsed but never tracked as WS dependency pubkeys (only vaults+market are) — acceptable: AMM depth lives in the AMM vaults, not the OpenBook book.

Conclusion: V4 layout parsing is **correct**. Remaining issues are semantic (target_orders default, decimals-from-mint, mixed-decimal min() comparison), not offset bugs.

### A.2 Orca Whirlpool (`orca_whirlpool.rs:261-299`) — VERIFIED against upstream `state/whirlpool.rs` (fetched read-only)

| Field | Offset | Type | Adapter line | Upstream | Verdict |
|---|---|---|---|---|---|
| discriminator | 0..8 | [u8;8] | (not checked!) | Whirlpool::DISCRIMINATOR = 3f95d10ce1806309 | **Adapter never checks the 8-byte discriminator.** `update_from_account` enforces only MIN_SIZE. A foreign account owned by the whirlpool program with >=300 bytes would be "decoded". Low practical risk (owner filtered upstream by program_id_to_protocol), but a check is trivial. |
| whirlpools_config | 8..40 | Pubkey | not read | ✓ | ok |
| whirlpool_bump | 40 | u8 | not read | ✓ | ok |
| tick_spacing | 41..43 | u16 | whirlpool.rs:270 | ✓ | ✓ |
| fee_tier_index_seed | 43..45 | [u8;2] | not read | ✓ (adaptive fee tier marker) | **skipped — adaptive fee tiers invisible; see §4.3** |
| fee_rate | 45..47 | u16 | whirlpool.rs:273 | ✓ (hundredths of bip) | ✓ |
| protocol_fee_rate | 47..49 | u16 | not read | ✓ | ok |
| liquidity | 49..65 | u128 | whirlpool.rs:276 | ✓ | ✓ |
| sqrt_price | 65..81 | u128 | whirlpool.rs:279 | ✓ | ✓ |
| tick_current_index | 81..85 | i32 | whirlpool.rs:282 | ✓ | ✓ |
| protocol_fee_owed_a/b | 85..101 | u64x2 | not read | ✓ | ok |
| token_mint_a | 101..133 | Pubkey | whirlpool.rs:285 | ✓ | ✓ |
| token_vault_a | 133..165 | Pubkey | whirlpool.rs:288 | ✓ | ✓ |
| fee_growth_global_a | 165..181 | u128 | not read | ✓ | ok |
| token_mint_b | 181..213 | Pubkey | whirlpool.rs:291 | ✓ | ✓ |
| token_vault_b | 213..245 | Pubkey | whirlpool.rs:294 | ✓ | ✓ |
| account LEN | 653 | — | WHIRLPOOL_MIN_SIZE = 300 | LEN = 8+261+384 | under-sized floor, harmless (only ≤245 used) |

TickArray (FixedTickArray, parsed by `arb_math::clmm::parse_tick_array_account`, clmm.rs:607-666): discriminator(8) whirlpool(32) start_tick_index i32 @40..44 ✓; ticks stride 113 B: initialized bool @+0 (validated to be 0/1), liquidity_net i128 @+1..17, liquidity_gross u128 @+17..33 ✓ (matches upstream Tick zero_copy: bool+i128+u128+u128x2+u128x3 = 113); account total 9988 ✓; rejects unaligned start_tick (`rem_euclid(88*spacing) != 0`) ✓; `initialized == false && (net|gross) != 0 → error` ✓. Oracle PDA `[b"oracle", whirlpool]` ✓. TickArray PDA `[b"tick_array", whirlpool, start_tick.to_string().as_bytes()]` — matches Orca SDK's decimal-string seed ✓ (not re-fetched; from SDK knowledge).

Conclusion: Whirlpool offsets **correct**; two correctness gaps (missing discriminator check; adaptive fee tier seed unread) and one mint-ordering hazard in arb-tx (§4.4).

### A.3 Meteora DLMM (`meteora_dlmm.rs:315-406`) — AUTHOR-VERIFIED ONLY; plausible, internally consistent

| Region | Offset | Content | Adapter line | Verdict |
|---|---|---|---|---|
| disc | 0..8 | LbPair discriminator | not checked! | **No discriminator check.** Owner filter exists upstream; still, mint fields would be garbage on a wrong-owner account of >=300 B. |
| StaticParameters | 8..40 | base_factor u16@8, filter_period@10, decay_period@12, reduction_factor@14, variable_fee_control u32@16, max_volatility_accumulator u32@20, min_bin_id i32@24, max_bin_id i32@28, protocol_share u16@32, base_fee_power_factor u8@34, function_type u8@35, collect_fee_mode u8@36, padding@37 | dlmm.rs:369-378 | Internally consistent; cannot fault or confirm without a live account |
| VariableParameters | 40..72 | volatility_accumulator u32@40, volatility_reference u32@44, index_reference u32@48, padding@52..56, last_update_timestamp i64@56..64 | dlmm.rs:392-394 | Timestamp read from here, NOT the @712 trailing field — consistent with quote needs |
| bump_seed | 72 | u8 | not read | ok |
| bin_step_seed | 73..75 | [u8;2] | not read | ok |
| pair_type | 75 | u8 | not read | ok (permissionless vs permissioned pools) |
| active_id | 76..80 | i32 | dlmm.rs:396 | ✓ structure |
| bin_step | 80..82 | u16 (.max(1)) | dlmm.rs:397 | ✓ clamp prevents div-by-0 in bin math |
| status | 82 | u8 | dlmm.rs:398 | ✓ decode; **but quote() never checks it** — a non-zero (disabled/tombstoned) pair is still quotable. Flag. |
| token_x_mint | 88..120 | Pubkey | dlmm.rs:399 | ✓ structure |
| token_y_mint | 120..152 | Pubkey | dlmm.rs:400 | ✓ |
| reserve_x / reserve_y | 152..216 | Pubkeyx2 | dlmm.rs:401-402 | ✓ |
| protocol_fee | 216..232 | (amount_x, amount_y) u64x2 | not read | ok |
| _padding_1 | 232..264 | — | not read | ok |
| reward_infos | 264..552 | [RewardInfo; 2] x144 B | not read | ok |
| oracle | 552..584 | Pubkey | dlmm.rs:403 | ✓ |
| bin_array_bitmap | 584..712 | [u64; 16] | not read | **bitmap membership never consulted — quote may walk bins not usable; low risk for majors, real risk for exotic pairs** |
| last_updated_at | 712..720 | i64 | not read | ok |
| min size | 300 | — | METEORA_LB_PAIR_MIN_SIZE :18 | under-floor vs real size (~904) — harmless |

BinArray (`parse_bin_array_account`, dlmm.rs:152-191): array_index i64 @8..16 ✓; start_bin_id = index x70 ✓; bin stride 144 B: amount_x u64@0, amount_y u64@8, price u128@16..32, liquidity_supply u64@32..40, reward_per_token_stored u64@40..48, fee_amount_x/y_per_token_stored u128@48..80, fee_infos[2]@80..112, open_order_amount u64@112..120, processed_order_remaining_amount u64@128..136, ask_side bool@140 ✓ per the in-code layout doc; header 56 B ✓. BINS_PER_BIN_ARRAY = 70 ✓ correct for lb_clmm (a 64 assumption would corrupt every bin-array PDA — correctly avoided). Limit-order amounts folded into reserve-side totals with ask⇒X / bid⇒Y mapping ✓ sensible.

Conclusion: DLMM parsing is the most sophisticated in the crate and internally consistent; it lacks discriminator checks and ignores status/pair_type/bitmap — acceptable for routing majors, risky for exotic LB pairs. All offsets should be re-verified against a live LbPair with `solana account <pair>` before mainnet reliance.

## B. Instruction encoding & remaining accounts (arb-tx cross-check)

* **Raydium V4** (arb-tx `build_raydium_v4_swap_instruction_versioned`; `RaydiumV4Accounts`): 18-account matrix ending at serum_vault_signer; `swap_version = V2` selects swapBaseInV2/swapBaseOutV2 over legacy swapBaseIn(9) ✓ OpenBook-coexistence OK. target_orders = pool_id fallback → see C-9. No Token-2022 branch anywhere in V4 — correct for the SPL-only program.
* **Raydium CPMM** (`RaydiumCpmmAccounts::for_pool`): 13-account swap_base_input matrix; authority PDA derived as `[b"vault_and_lp_mint_auth_seed"]` ✓; input/output vault chosen via mint byte-order (see §2.5); observation_state is required by the program (oracle samples) and supplied ✓; discriminator = known-good swap_base_input ✓.
* **Raydium CLMM**: 14–16 accounts + 3 tick arrays; the `true` arg in the arb-tx call is `amount_specified_is_input=true`, so the encoded instruction is V1 `swap` (SWAP_DISCRIMINATOR), not swap_v2 — **V1 swap rejects Token-2022 mints on-chain**, so any Token-2022 CLMM pool wired through this path fails at execution even though the adapter path pretends to support Token-2022. Also the memo program is not appended because V1 needs none ✓ consistent pair (V1 + no memo, but no Token-2022). If V2 were used, MEMO_PROGRAM_ID :259 would be mandatory (swap_v2 requires memo for referral/attribution) — the constants are present for that path but unused.
* **Orca**: 11-account `swap` (token_program, token_authority, whirlpool, owner_a, vault_a, owner_b, vault_b, tick_array_0..2, oracle) — matches the upstream Accounts struct field-for-field (verified against whirlpool.rs in the whirlpools repo) ✓. oracle readonly ✓ (writable only for adaptive). No adaptive-fee remaining-accounts support — adaptive pools cannot be traded correctly.
* **Meteora DLMM**: swap2 matrix: lb_pair, reserve_x, reserve_y, user_x, user_y, token_x_mint, token_y_mint, oracle, event_authority (derived `[b"__event_authority"]` — correct seed form), program + bin_array_0..2 + bitmap_extension. The exact account order/signers must be checked against the DLMM swap2 IDL before live use; the adapter passes bitmap_extension unconditionally (requires the extension account to exist — see §5.5).
* **Meteora DAMM v2**: 14 accounts as listed in §6.5; token programs in/out handled ✓; user_authority as new_readonly(.., true) — signer bit is set, OK; DAMM v2 also expects an optional referral/integrator account in some versions — absent, fine for v1.
* **LaunchLab**: 15 accounts; is_buy derived from WSOL equality (see §7.4); discs wrong (see §7.5).
* **PumpSwap**: V1 = 16 accounts / V2 = 17 accounts (adds __event_authority); exact-quote-in buy + exact-base-in sell; V2 constants plausible except sell_v2 (§9.1). The 2026-10 appended Pool fields (protocol_fees/creator_fees) imply un-swept buckets — the adapter correctly excludes them from the spendable quote reserve when quoting.
* **Pump.fun**: buy/sell builders with fee_recipient + bonding_curve + global + ATAs; 24-byte args may be stale vs current IDL (§8.5).

## C. Slippage & exact-in/exact-out semantics

All adapters expose exact-in quoting only (`AmmPool::quote(amount_in, a_to_b) -> amount_out`). None implements exact-out. Consequences:

1. The engine (arb-engine quote_hop) composes hops as exact-in; the final profit floor is applied after the last hop — acceptable design.
2. min_amount_out passed to instructions is caller-derived; nothing in arb-dex validates that it equals quoted_output × (1 - slippage) — correct separation (policy belongs in the strategy layer).
3. Per-hop DEFAULT_HOP_SLIPPAGE_BPS = 10 (arb-constants:352) — each hop's on-chain min-out allows 0.1% adverse move; worst case compounds mildly; the final profit floor is the real safety net ✓ reasonable.
4. PumpSwap V2 buy uses BuyExactQuoteInV2 — the only exact-out-ish instruction emitted; adapter quote remains exact-in; min_amount_out ≙ min base out for buys ✓ consistent.
5. No adapter asserts min_amount_out > 0; engine profit floor prevents 0 in practice. Noted.
6. RaydiumV4::max_arbitrage_amount compares raw base/quote reserves across differing decimals — for SOL-quoted meme pools (9 vs 6 decimals) this understates the binding side by 1000x; used only for sizing heuristics, not execution ✓ Low.

## D. Token-2022 & transfer-fee handling — detailed verdict

* decode_transfer_fee_config (lib.rs:42-96): TLV walk from offset 166; requires account_type == 1 (Mint) at 165 ✓; extension type 1 with value len 108 ✓; older schedule epoch @72..80, maximum_fee @80..88, basis_points @88..90; newer @90..98 / 98..106 / 106..108 ✓ **matches the on-chain POD layout exactly** (32-byte config_authority + 32-byte withdraw_withheld_authority + 8-byte withheld_amount = 72, then two 18-byte TransferFees) ✓; epoch-aware active_transfer_fee selection (lib.rs:112-118) ✓ correct Token-2022 semantics (newer schedule once current_epoch >= newer.epoch); unit-tested (lib.rs:355-371) ✓.
* calculate_transfer_fee (lib.rs:99-109): ceil(amount x bps / 10_000) capped at maximum_fee ✓ exact Token-2022 withholding formula ✓.
* token2022_has_risky_extension (lib.rs:131-151): any non-zero byte in TLV → risky; fail-closed ✓. Over-conservative (e.g. MetadataPointer with no fee is rejected) but safe.
* **Gap (C-5)**: nothing feeds these into quotes. transfer_fee_*_bps fields exist on every adapter but are always 0 in practice; apply_transfer_fee_ceil(amount, bps, u64::MAX) ignores the decoded maximum_fee; has_risky_token is always false downstream of stream ingest; decode_amm_pool-constructed pools never learn mint extensions at all (mint account data is available only in the poll path via mint snapshots, and that path never sets has_risky either). Net effect: Token-2022 fee mints are **systematically over-quoted** — output too high by the withheld fee — exactly the adverse-selection direction that loses to sandwiching.
* Double counting: **none observed** — input-leg and output-leg caps are applied at the two separate SPL transfers of a swap, mirroring on-chain withholding ✓ (correct modeling, not a bug). The u64::MAX max_fee is the actual defect.
* CPMM additionally removes locked protocol/fund/creator fees from reserves via u128 saturating subtraction ✓ correct vs on-chain vault_amount_without_fee ✓.

## E. Quote-model conformance vs on-chain semantics (integer fidelity)

* CPMM (cpmm_out_fee): out = floor(gamma x dx x y / (D*x + gamma*dx)) with gamma = fee complement — algebraically identical to applying the fee as a multiplier on input; equals Raydium V4 / CP-Swap upfront-fee model ✓; floor rounding prevents overquote ✓; U256 intermediate, no over/underflow panics ✓.
  * Minor: V4's fee_bps() rounds fee_numerator/denominator to bps first (v4.rs:161-165), losing sub-bps precision; harmless at 25/10_000, imprecise for arbitrary owner fees (e.g. 3333 ppm -> 33 bps, under-quote of 1 bp).
* CP-Swap: cpmm_out_upfront_fee_ceil uses ceil on input fee — matches on-chain calculate_swap_fees ✓; floor on output ✓.
* CLMM: crossing logic walks sorted initialized ticks; fees via 1e6 denominator; output floored (arb-math clmm.rs). Fee deduction there uses floor (not ceil) on the LP-fee numerator — 1-unit under-count vs on-chain ceil; immaterial at u64 scale.
* DLMM: ceil on fee amounts, floor on outputs — matches lb_clmm rounding invariants ✓; variable fee scales with volatility_accumulator per bin ✓; quote_dlmm_multi_bin mutates a cloned bins vector so adapter state is not corrupted ✓.
* PumpSwap: per-component ceil on lp/protocol/creator separately, remainder through curve (buy) / fees from gross output (sell) ✓ matches on-chain per-leg ceil ✓.
* Pump.fun: fee on net input (buy) / on gross output (sell) ✓ matches program ✓.
* LaunchLab: buy fee on input then virtual-curve; sell fee on output ✓ math consistent; fee value wrong (hardcoded 100 bps).
* DAMM v2: linear/exponential scheduler reduction from cliff with period_frequency/number_of_period ✓ mirrors BaseFeeStruct; dynamic variable_fee_control/volatility_accumulator not decoded (never set true) → dynamic leg inert ✓ incorrect vs on-chain when dynamic fees are enabled.
* f64/f32 confined to arb-math/chronos.rs (latency/metrics) — no floats on quote paths ✓.

## F. Dead code & test coverage inventory

* RaydiumLaunchLabPool, MeteoraDammV2Pool: constructed only in their own module tests; never in arb-engine / arb-bot / arb-ingest hot paths; absent from program_id_to_protocol, decode_amm_pool, poll registry fee resolution and stream decode. **Fully dead in production; present in the build graph.**
* PumpSwapPool / PumpSwapPoolHeader: used by arb-ingest::pump_state (joined snapshots -> into_pool, pump_state.rs:272) and its own tests; poll registry skips (fee None) → the live PumpSwap path exists but is extremely narrow and barely exercised end-to-end.
* CpmmDecoder (lib.rs:120-157): empty struct with new() — dead code.
* AmmPool defaults: V4/Orca/PumpFun do not override transfer_fees_bps (always (0,0)) ✓ consistent with C-5.
* Engine-side defense in depth that *does* exist: execution_capability() in arb-engine adapters.rs gates CPMM (fee valid + reserves), CLMM/DLMM (state presence), PumpFun (tradeable), PumpSwap (exec accounts + fee + reserves) before a pool may reach tx-build — the main safety net, working as intended.
* fuzzing_decoders.rs covers decoder robustness; not audited in depth.

## G. Prioritized remediation list (for the owning team; no code changed)

1. arb-tx LaunchLab: replace the inline pump discs with RAYDIUM_LAUNCHLAB_BUY/SELL_EXACT_IN_DISCRIMINATOR, or delete the LaunchLab builder until verified. (§7.5, C-1)
2. arb-tx Orca & LaunchLab: derive a_to_b / is_buy from pool mint ordering (mint_a < mint_b), never from WSOL equality. (§4.4, §7.4, C-2)
3. arb-constants: fix or prove PUMP_SWAP_SELL_V2_DISCRIMINATOR; add a static assert that sell_v2 != sha256("global:sell"). (§9.1, C-3)
4. Raydium CLMM: status swap-disable check → status & 4; arb-tx sqrt_price_limit → clamp to [MIN, MAX] sqrt price; V1 swap vs Token-2022 → gate off or switch to swap_v2 + memo. (§3.7, §3.5, C-4)
5. Wire decode_transfer_fee_config + epoch selection into adapter construction at stream/poll decode sites; apply maximum_fee caps; gate has_risky per-mint instead of default-false. (C-5)
6. Registry: return Some(decoded) fee for PumpSwap (via pump_state tiers), DammV2, LaunchLab — or explicitly document them as quote-only and keep them out of exec. (C-6)
7. Orca/CLMM/DLMM execution_accounts: derive tick/bin-array window from swap direction, not a hardcoded true. (§4.5, C-7)
8. V4: fail fast if target_orders == default instead of substituting pool_id; or require verified exec accounts always. (C-9)
9. DLMM: check status != 0 and pair_type in quote; consult bin_array_bitmap for bin usability; verify LbPair/BinArray offsets against a live mainnet pair (or pin the IDL hash in the header).
10. LaunchLab: parse GlobalConfig/PlatformConfig fee rates; delete the 100 bps default or mark the pool unquotable until config arrives. (§7.3)
11. DAMM v2: fix collect_fee_mode match (2 => a_to_b), disambiguate config@296 vs whitelisted_vault, decode dynamic_fee_initialized. (§6.3)
12. Sweep: confirm PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2 / BUY_V2 against the current pump_amm IDL; verify Pump.fun buy/sell argument lists vs the current program (possible extra track_volume arg). Also confirm Whirlpool adaptive-fee pools are excluded or handled with oracle remaining accounts. (§4.3, §8.5, §9.1)

## H. Program ID provenance & verification status (per repo constant, arb-constants/src/lib.rs)

| Constant (line) | Value | Status |
|---|---|---|
| TOKEN_PROGRAM_ID :16 / TOKEN_2022_PROGRAM_ID :18 | TokenkegQfe… / TokenzQdBNb… | ✓ well-known canonical IDs |
| PUMP_FEES_PROGRAM_ID :20 | pfeeUxB6j… | ⚠ unverified (no independent source consulted) |
| ASSOCIATED_TOKEN_PROGRAM_ID :26 | ATokenGPvbd… | ✓ canonical |
| RAYDIUM_V4_PROGRAM_ID :147 / AUTHORITY :149 | 675kPX9MH… / 5Q544fKr… | ✓ canonical (widely published; authority is the v4 amm authority PDA) |
| RAYDIUM_CPMM_PROGRAM_ID :151 / CLMM :153 / CPMM auth seed :240 | CPMMoo8L3… / CAMMCzo5… / vault_and_lp_mint_auth_seed | ✓ canonical published IDs |
| ORCA_WHIRLPOOL_PROGRAM_ID :155 | whirLbMiic… | ✓ canonical |
| METEORA_DLMM_PROGRAM_ID :157 | LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo | ✓ canonical; LBPAIR layout comment cites dlmm v0.12.0 |
| PUMP_FUN_PROGRAM_ID :159 / FEE_RECIPIENT :167 | 6EF8rrec… / CebN5WGQ… | ✓ canonical published |
| PUMP_SWAP_PROGRAM_ID :161 | pAMMBay6… | ✓ canonical published |
| PUMP_SWAP_GLOBAL_CONFIG :163 | ADyA8hdefv… | ⚠ unverified (derivable: PDA of ["global_config"] under pump_swap program — expected; not re-derived here) |
| METEORA_DAMM_V2_PROGRAM_ID :266 / POOL_AUTHORITY :268 | cpamdpZCG… / HLnpSz9h2… | ✓/⚠ program canonical; authority constant unverified (it is a PDA — re-derive before trusting) |
| RAYDIUM_LAUNCHLAB_PROGRAM_ID :279 | LanMV9sAd… | ✓ canonical published |
| JITO_TIP_ACCOUNTS :57-66 | 8 accounts | ✓ match the published Jito mainnet tip set (order differs across docs; set is what matters) |
| jitodontfront marker :132-136 | jitodontfront111… | ✓ correct construction (prefix preserved; see unit test) |
| WSOL :46 / USDC :50 mints, JITO floors :70-81 | standard | ✓ |
| MEMO_PROGRAM_ID :259 / SYSVAR_* :30-32 | published | ✓ |
| HUMIDIFI / SOLFI_V2 / ZERO_FI :308-315 | 9H6tua… / SoLFiHG9… / ZERor4x… | ⚠ unverifiable proprietary AMMs — deliberately Jupiter-routed; not used by adapters |

No `solana program show` / RPC verification was performed (offline audit); "canonical" means consistent with the IDs hardcoded throughout Solana aggregators at the time of writing. Any deployment with a program upgrade must be re-checked against this table — nothing here auto-updates on an IDL bump.

## I. Independently recomputed Anchor discriminators (sha256("<namespace>:<name>")[..8])

Computed fresh for this audit (not copied from the repo). ✓ = matches arb-constants.

| Instruction / account | Computed | Repo constant | Verdict |
|---|---|---|---|
| global:swap | [248,198,158,145,225,117,135,200] | RAYDIUM_CLMM_SWAP_DISCRIMINATOR; METEORA_DAMM_V2_SWAP_DISCRIMINATOR | ✓ both |
| global:swap2 | [65,75,63,76,235,91,91,136] | METEORA_SWAP2_DISCRIMINATOR; METEORA_DAMM_V2_SWAP2_DISCRIMINATOR | ✓ both (same hash — both programs' `swap2` share the name) |
| global:swap_v2 | [43,4,237,11,26,201,30,98] | RAYDIUM_CLMM_SWAP_V2_DISCRIMINATOR | ✓ |
| global:swap_base_input | [143,190,90,218,196,30,51,222] | RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR | ✓ |
| global:buy | [102,6,61,18,1,218,235,234] | PUMP_FUN_BUY / PUMP_SWAP_BUY | ✓ identical by construction |
| global:sell | [51,230,133,164,1,127,131,173] | PUMP_FUN_SELL / PUMP_SWAP_SELL | ✓ |
| global:buy_v2 | [184,23,238,97,103,197,211,61] | PUMP_SWAP_BUY_V2_DISCRIMINATOR | ✓ |
| global:buy_exact_quote_in_v2 | [194,171,28,70,104,77,91,47] | PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR | ✓ |
| global:sell_v2 | **[93,246,130,60,231,233,64,178]** | PUMP_SWAP_SELL_V2_DISCRIMINATOR = [51,230,133,164,1,127,131,173] | **✗ WRONG — repo carries sha256("global:sell"), not sell_v2. Every PumpSwap SellExactBaseInV2 would fail Anchor's discriminator check** |
| global:buy_exact_in | [250,234,13,123,213,156,19,236] | RAYDIUM_LAUNCHLAB_BUY_EXACT_IN_DISCRIMINATOR | ✓ (but never used — arb-tx inlines pump discs, see C-1) |
| global:sell_exact_in | [149,39,222,155,211,124,152,26] | RAYDIUM_LAUNCHLAB_SELL_EXACT_IN_DISCRIMINATOR | ✓ (same note) |
| account:BondingCurve | [23,183,248,55,96,216,172,96] | PUMP_FUN_BONDING_CURVE_DISCRIMINATOR | ✓ |
| account:Pool | [241,154,109,4,17,177,109,188] | PUMP_SWAP_POOL_DISCRIMINATOR | ✓ |
| account:LbPair | [33,11,49,98,181,101,177,13] | (not present) | adapter does not check LbPair discriminator — latent gap noted in §5 / A.3 |

## J. Eight-checkpoint scorecard per adapter

(each cell: ✓ ok / ✗ broken / ~ partial)

| Adapter | 1 Program ID/version | 2 Quote math int | 3 Fees on-chain | 4 Direction | 5 Remaining accts/enc | 6 Exact-in only | 7 Classification | 8 Account offsets |
|---|---|---|---|---|---|---|---|---|
| raydium_v4 | ✓ | ✓ upfront-ceil/floor | ✓ decoded+PnL adj | ✓ mint order | ✓ (target_orders default ✗) | exact-in | live-exec | ✓ verified |
| raydium_cpmm | ✓ | ✓ ppm ceil | ~ default 25bps pre-AmmConfig; creator-fee side wrong | ✓ mint order | ✓ discs/accounts ok | exact-in | live-exec | ✓ verified (PoolState+AmmConfig) |
| raydium_clmm | ✓ | ✓ CLMM | ~ same pre-config risk; status bit ✗ | ✓ mint order (int); BE tick seeds ✓ | ~ sqrt_price_limit ✗ | exact-in | live-exec | ✓ (TickState ✓; status@389 unverified) |
| orca_whirlpool | ✓ | ✓ CLMM | ✓ decoded; adaptive tier ✗ | ✓ mint order (adapter); arb-tx WSOL ✗ | ✓ oracle/tick PDAs; fallback dir ✗ | exact-in | live-exec | ✓ verified vs upstream |
| meteora_dlmm | ✓ | ✓ bin-walk+var fee | ~ base only in fee_bps(); status unchecked | ✓ X/Y mint order | ~ bitmap_extension unconditional | exact-in | live-exec | ~ author-verified; discriminator unchecked |
| meteora_damm_v2 | ✓ (ID) | ✓ virtual-reserves | ~ collect_fee_mode arms ✗ | ✓ mint order | ~ config@296 suspicious | exact-in | dead code | unverified |
| raydium_launchlab | ✓ (ID) | ✓ curve | ✗ hardcoded 100 bps, unoverrideable | ✓ mint order (adapter); arb-tx WSOL ✗ | ✗ builder uses pump discs | exact-in | dead code | unverified |
| pump_fun | ✓ | ✓ bounded curve | ✓ explicit-only (best) | ~ WSOL convention (by design) | ✓ known-good discs; arg list stale? | exact-in | live-exec | ✓ verified |
| pump_swap | ✓ | ✓ per-leg ceil PPM | ✓ live tier table (best); creator_fee_bps unused | ~ arb-tx WSOL/USDC hardcode | ✓ 17-acct V2; sell_v2 disc ✗ | exact-in | exec-via-pump_state only | ~ prefix ✓; 2026-10 tail unverified |

## K. Protocol version pinning & fee-attribution matrix (per adapter)

| Adapter | Program | "Version" tracked | IDL pin | Fee components decoded | Protocol fee vs LP fee |
|---|---|---|---|---|---|
| raydium_v4 | 675kPX9… | swapBaseInV2 (hardcoded V2 in exec accounts) | none | swap_fee_num/den; need_take_pnl (protocol PnL) | V4 has no separate protocol/LP split beyond the 25bps — need_take_pnl is the protocol's cut of LP fees, subtracted from vaults ✓ |
| raydium_cpmm | CPMMoo8… | swap_base_input (current CP-Swap) | none | trade_fee (AmmConfig), creator_fee (AmmConfig, optional), protocol_fees_* & fund_fees_* (pool, locked), creator_fees_* (pool, locked) | LP fee = trade_fee − (protocol+fund shares) inside AmmConfig; pool's stored protocol/fund fees are locked (excluded from reserves) ✓; creator fee side-of-trade decided as in §2.3 (wrong) |
| raydium_clmm | CAMMCzo5… | swap (V1) encoded | none | trade_fee (AmmConfig u32 ppm) | Protocol fee rate exists in AmmConfig but not decoded (protocol_fee_rate @43..47 unread) — fine for pricing (LP receives gross), but protocol_fee_owed is invisible in reserves (gross vaults) |
| orca_whirlpool | whirLbMiic… | Swap (not adaptive; oracle readonly) | none | fee_rate (account), protocol_fee_rate (unread) | Protocol fee portion unread — pricing uses gross fee; output under-stated vs on-chain only by protocol share if creator/adaptive tiers active (negligible for standard tiers) |
| meteora_dlmm | LBUZKhRx… | swap2 (instruction) | none (cited v0.12.0 in comment) | base_factor/power/etc., protocol_share (unread), variable fee params (used in quote) | protocol_share (basis points of the swap fee to protocol) NOT excluded from reserves — quote conservatively gives whole fee to LP; immaterial vs output precision |
| meteora_damm_v2 | cpamdpZCG… | swap (V1) | none | cliff_fee_numerator + scheduler + (dynamic: disabled) | collect_fee_mode (BothToken/OnlyB/OnlyA) fee-side mapping broken (§6.3) |
| raydium_launchlab | LanMV9sA… | buy_exact_in/sell_exact_in | none (cited "official IDL") | NOTHING config-driven — 100bps hardcoded | protocol vs platform vs creator split completely invisible |
| pump_fun | 6EF8rrec… | buy/sell | none | fee from GlobalConfig/Pump Fees externally supplied; creator fee tier not modeled | mayhem/cashback rejected; creator fee of regular coins included in the supplied fee (caller's duty) |
| pump_swap | pAMMBay6… | buy/sell (V1), buy_v2/sell_v2/buy_exact_quote_in_v2 (V2) | none | GlobalConfig + FeeConfig tiers (canonical: pool-derived PDA; non-canonical: custom schedule), protocol fee recipient, creator vault authority | lp_fee + protocol_fee + creator_or_holder_fee — three legs split exactly per tier row; holder-reward pools quoted same as creator-fee pools when verified ✓ |

Notes:
* None of the audited files references an npm/cargo-pinned SDK or an on-chain-fetched IDL hash; all layouts are hand-maintained with comments citing "verified" origins. That is the single largest systemic risk in the subsystem, and it is why §A insists on byte tables.
* PumpSwap is the only adapter whose fee model is *provably complete* (canonical/non-canonical selection, maker/taker split via quote_mint lower/greater comparison happens inside `select_fee`, holder-reward flag honored) — and the only one that correctly refuses to quote without verified fees.
* Raydium LaunchLab is the anti-pattern: hardcoded 1% and no config fetch — every LaunchLab quote is an approximation presented as authoritative.

## L. PumpSwap worked example (sanity-check of §9 fee math)

Pool: base_reserve = 10_000 (e.g. 10k token, 6 dp), raw quote vault = 20_100, header.virtual_quote_reserves = +100, protocol_fees (unswept) = 250, creator_fees = 200.
→ effective_quote_reserve = 20_100 + 100 = 20_200 (pricing) ✓
→ real_quote_reserve = 20_100 − 250 − 200 = 19_650 (spendable) ✓
Fees (live tiers): lp 2000 ppm (0.2%), protocol 500 ppm (0.05%), creator 10000 ppm (1%) with coin_creator set → total 12_500 ppm = 125 bps? No: 2000+500+10000 = 12_500 ppm = 1.25% ✓ (canonical 1% creator + 0.25% combined LP/protocol tiers, plausible).
quote_exact_in(100 base, base_to_quote=true /* sell */):
  gross = cpmm_out_fee(100, 10_000, 20_200, 10_000, 10_000) = floor(100*20200 / (10000+100)) = floor(2_020_000/10_100) = 200 (0 dp truncation) — then lp = ceil(200×2000/10_000... wait: fee_amount(gross, bps) with bps = ppm/100: lp_bps = 20, protocol_bps = 5, creator_bps = 100 (from integer-BPS check 10000/100 = 100). lp = ceil(200×20/10_000)=1 (ceil(0.4)), protocol = ceil(200×5/10_000)=1, creator = ceil(200×100/10_000)=2. out = 200 − 1 − 1 − 2 = 196 ≤ real_quote_reserve ✓ → returns 196.
buy 100 quote: lp = ceil(100×20/10000)=1, protocol = 1, creator = 1, total = 3, net = 97 → out = cpmm_out_fee(97, 20_200, 10_000, ...) = floor(97×10000 / 20_200 + 97) = floor(970_000 / 20_297) = 47 ✓ (> 0, < base_reserve). Demo matches the unit test fixture shape (sell 100 → 195 in their fixture with different fee mix; structure identical).

Off-by-ceil on each of three legs can never undercharge the LP below zero; rounding is monotonic-safe. ✓

The one reachable-path inconsistency worth noting: `fee_bps()` (used by the engine for is_fee_valid/fee_denominator=10_000) returns total ppm/100 with integer floor and returns None on non-integer-BPS totals (pump_swap.rs:436-442) — so a pool with 10_050 ppm total reports fee_bps()=None while quoting fine via quote_exact_in. Engine-callable paths must treat None as "do not price", which is safe but can brick otherwise-valid pools with sub-bps fee tiers.

## M. Where each mandated question is answered (index)

1. Program ID / protocol version / pinned SDK/IDL — §1–9 items (1); §K matrix; §O (no IDL pins anywhere).
2. Quote math vs on-chain + float vs integer — §2–9 items (2); §A.2/A.3/E (integer everywhere on quote path; chronos.rs floats out of scope).

3. Fee handling (tier source, formula, Token-2022 double count, protocol fee) — §1–9 items (3); §D (Token-2022); §E (rounding); §K (protocol vs LP split). Hardcoded fees: V4 25bps default (overridable), CPMM 2500 ppm default (overridable), CLMM 2500 ppm default (overridable), DAMM v2 2.5e6 (overridable), LaunchLab 100 bps NOT overridable (§7.3) — worst offender.

4. Direction derivation & token ordering — §1–9 items (4); §C; arb-tx WSOL/byte-order sites (§B; C-2). X/Y vs A/B: DLMM uses X/Y (sorted), CPMM/CLMM/DAMM v2 use 0/1 (sorted), Whirlpool/V4 use A/B (unsorted! ordering only enforced by program at init for mint_a < mint_b — Whirlpool DOES require a<b; V4 coin/pc is pool-creator-defined). V4 reverse = same accounts, reserves swapped (✓); Whirlpool reverse = same vaults, a_to_b param (✓); reverse reuses tick arrays with direction-flipped PDA window — but the a_to_b=true hardcode in execution_accounts breaks this on the exec path (§4.5).

5. Remaining accounts & discriminators (incl. IDL spot-checks) — §B; §I (recomputed discs); §A (account parsing); §1.5/§5.5 (target_orders, bitmap_extension).

6. Slippage semantics, exact-in vs exact-out — §C; per-instruction amount_specified_is_input flags; per-hop 10 bps; engine profit floor.

7. Fully-implemented / quote-only / instruction-builder-only / dead / unsupported — §0 (path map); §F (dead code); §12 table. Summary: V4/CPMM/CLMM/Orca/DLMM = quote+exec live; Pump.fun = live; PumpSwap = exec-via-pump_state only (poll/stream paths dead); DAMM v2 & LaunchLab = dead code (builder exists but unwired, and LaunchLab builder is broken — C-1).

8. Account-layout offset verification (raydium_v4, orca_whirlpool, meteora_dlmm) — §A.1/A.2/A.3 in full; §E cross-checks.

## N. Severity-ordered risk register

| ID | Sev | One-line |
|---|---|---|
| C-3 | Crit (latent) | PumpSwap sell_v2 discriminator is sha256("global:sell") — all V2 sells revert Anchor check. Verified by recomputation (§I). |
| C-1 | High | LaunchLab builder emits pump.fun buy/sell discs; real LaunchLab discs defined but unused. |
| C-2 | High | Orca arb-tx a_to_b = (mint_in == WSOL); all non-SOL whirlpool hops mis-directioned; LaunchLab same. |
| C-5 | High | Token-2022 transfer fees unwired everywhere; fees systematically ignored on all live paths. |
| C-8 | High (latent) | CLMM sqrt_price_limit dimensionally invalid; swaps revert or limits inert. |
| C-4 | Med | CLMM status swap-disable bit 1<<4 (=16) instead of bit 2 (=4) — check inert. |
| §7.3 | Med | LaunchLab fee hardcoded 1%, unoverrideable — guaranteed mispricing. |
| C-6 | Med | DammV2/LaunchLab absent from program_id_to_protocol & decode_amm_pool; poll registry fee=None — silently unsupported. |
| C-7 | Med | Tick/bin array fallback window hardcodes direction; reverse swaps reference wrong arrays. |
| C-9 | Med | V4 target_orders=pool_id fallback; bogus amm_target_orders if registry never fills it. |
| §6.3 | Med | DAMM v2 collect_fee_mode duplicate arms (2 maps like 1); OnlyA pools charged wrong leg. |
| §4.3 | Med | Orca adaptive-fee pools (fee_tier_index_seed != tick_spacing) quoted at stale stored fee_rate. |
| §2.3 | Low-Med | CPMM creator fee always folded into input fee; OnlyToken1 pools mispriced direction. |
| §5 | Low | DLMM: status/pair_type/bitmap unconsulted; fee_bps() excludes dynamic fee; limit-order fee attribution approximated. |
| C-10 | Low | DAMM v2 config@296 vs whitelisted_vault ambiguity; dynamic fee never initialized. |
| C-11 | Low | LaunchLab/DammV2 unexercised by any runtime path; LaunchLab offsets unverified. |
| C-12 | Info | fee_bps() rounding, PumpSwap creator_fee_bps dead field, LaunchLab reserve() sum approximation. |
| C-14/15 | Info | V4 silent-decode defaults, Serum signer derivation (correct), OpenBook dependency tracking gaps. |

## O. Method & confidence

Method: manual read of all in-scope files with grep cross-references into arb-engine, arb-ingest, arb-tx, arb-bot, arb-sim, arb-mcp; upstream sources fetched read-only for the three checkable layouts (Orca Whirlpool state + tick_array; Raydium CP-Swap PoolState/AmmConfig). NOT done: cargo builds/tests (explicitly out of scope), live account dumps, devnet simulation, Cargo lockfile review for pinned SDK/IDL versions (none of the audited crates pin an on-chain IDL — all offsets are hand-maintained constants), and any changes to code.

Confidence: V4/CPMM/Whirlpool parsing HIGH; DLMM MODERATE (author-verified, internally consistent); DAMM v2/LaunchLab LOW (unverified, flagged); discriminator well-known values HIGH (pump.fun, PumpSwap Pool, CPMM/CLMM swap, sha256("global:*") family); Jito tip list HIGH. Where this report says "live", it means "reachable from the ingest/engine path in this repository", not "production-enabled trading" — execution additionally depends on arb-bot's own gating flags, which were not in scope.

**Bottom line:** integer math throughout (no f64 on quote paths — float use confined to `arb-math/chronos.rs` metrics), fail-closed defaults in most decoders, and correct Token-2022 *decoding* — but the transfer-fee model is unwired, the LaunchLab/DAMM-v2 adapters are dead code with known-bad constants in their builders, and several instruction-layer direction/discriminator bugs (§C-1..C-3) mean live execution of anything outside V4/CPMM/CLMM/Orca/DLMM/Pump.fun should not be trusted without the specific fixes enumerated above.

*End of 13_CROSS_PROTOCOL.md.*
