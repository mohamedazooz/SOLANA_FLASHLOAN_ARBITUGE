# Math & Accounting Audit — Solana Arb Bot

Scope: `crates/arb-math/src/{lib.rs,cpmm.rs,clmm.rs,dlmm.rs,economic.rs,damm_v2.rs,launchlab.rs,pump_fun_raydium.rs,chronos.rs,latency.rs}`,
`crates/arb-types/src/{lib.rs,dimensional.rs,currency.rs}`, `crates/arb-engine/src/lib.rs` (profit paths).
All monetary values are raw atomic units (`TokenAmount(u64)`) or lamports (`Lamports(u64)`) unless noted. Every claim carries file:line. Read-only audit; no code executed.

## 1. Inventory of mathematical functions (module, signature, units)

### arb-math/lib.rs
- `pub const BASIS_POINTS_DIVISOR: u64 = 10_000` — cpmm.rs:18.
- `apply_transfer_fee_ceil(amount: u64, fee_bps: u16, maximum_fee: u64) -> u64` — lib.rs:69-77. Input: raw token units. **Output is the post-fee (net) amount, NOT the fee**; ceil division; max-fee cap applied.
- `apply_transfer_fee(amount: u64, fee_bps: u16) -> u64` — lib.rs:80-82 (bps-only wrapper).
- `pick_tip(net_before_tip: u64, share_bps: u64, floor: u64, cap: u64, min_keep: u64) -> Option<u64>` — lib.rs:85-100. Lamports in/out.
- `calculate_net_profit(gross: Lamports, flash_fee, priority_fee, base_fee, tip, rent_ata: Lamports) -> Option<Lamports>` — lib.rs:103-117. All lamports; `checked_sub`; returns `None` on net-negative (no signed result).
- `check_constant_product_invariant(x_before,y_before,x_after,y_after: u128) -> bool` — lib.rs:124 (test-only).
- `check_slippage_bound(expected_out: u64, min_out: u64, max_slippage_bps: u16) -> bool` — lib.rs:137 (test-only). Integer bps.

### arb-math/cpmm.rs — types: `U384 = Uint<384,6>` :14, `U512` :15, `U768` :16; `BASIS_POINTS_DIVISOR: u64 = 10_000` :18; `NANO_SCALE: u128 = 1e9` :19.
- `cpmm_out_upfront_fee_ceil(dx: u128, x: u128, y: u128, fee_num: u128, fee_den: u128) -> Option<u64>` :41-63. Raw units; fee `ceil(dx*fee_num/fee_den)` deducted upfront; out `floor(net_dx*y/(x+net_dx))`.
- `cpmm_out(dx, x, y: u128, gamma: u128) -> Option<u128>` :67; `cpmm_out_fee(..., fee_denominator: u128) -> Option<u128>` :73. Raw units in/out.
- `quote_cpmm(amount_in: u64, reserve_in: u64, reserve_out: u64, fee_bps: u64) -> Option<u64>` :104. Raw units.
- `quote_cpmm_full(...) -> Option<CpmmQuote>` :117; `CpmmQuote { amount_out: TokenAmount, price_impact_bps: Bps }` :22-26.
- `cpmm_price_impact_bps(dx, x, y: u128, gamma: u128) -> Bps` :139 (fee-free; see §2).
- `cpmm_fee_paid(amount_in: u64, fee_bps: u64) -> u64` :173; `cpmm_fee_paid_with_denom(amount_in, fee_rate, fee_denominator: u64) -> u64` :183. Ceiling.
- `isqrt_u384(val: U384) -> U384` :189; `isqrt_u512` :204; `isqrt_u768` :219. Integer Newton-Raphson floor sqrt.
- `optimal_two_pool(l1: &Leg, l2: &Leg) -> Option<u64>` :244; `optimal_three_pool` :307; `optimal_four_pool` :379. `Leg{r_in,r_out: u128, gamma: u128}` :29-34. Radicand widths: U384/U512/U768.
- `is_past_marginal_degradation_ceiling_two_pool(size: u64, l1, l2: &Leg, flash_fee_bps: u64) -> bool` :455.
- `marginal_profit_derivative_cpmm(size: u64, l1, l2: &Leg, flash_fee_bps: u64) -> i128` :503 (result scaled ×1e9).
- `marginal_profit_derivative(size: u64, out: u64, flash_fee_bps: u64) -> i128` :537 (×1e9).
- `clamp_at_marginal_ceiling_two_pool(candidate_size: u64, l1, l2: &Leg, flash_fee_bps: u64) -> Option<u64>` :557.

### arb-math/clmm.rs — Q64.64 fixed point; `MIN/MAX_TICK_INDEX = ∓443_636` :13-14; `MIN/MAX_SQRT_PRICE_X64` :18-19; `Q64 = 1<<64` :20; `FEE_RATE_DENOMINATOR: u64 = 1_000_000` (hundredths of a bip) :22.
- `tick_index_to_sqrt_price_x64(tick_index: i32) -> Result<u128, ClmmMathError>` :110 (bitwise binary exp; invert for positive ticks via `U256::MAX / ratio` :180). `tick_to_sqrt_price_x64` :190.
- `sqrt_price_x64_to_tick_index(sqrt_price_x64: u128) -> Result<i32, ClmmMathError>` :195 (binary search).
- `div_ceil_u256(num: U256, denom: U256) -> Result<U256, _>` :78; `div_ceil_u64(num: u128, denom: u128) -> Result<u64, _>` :92.
- `get_amount_delta_a(sqrt_price_0, sqrt_price_1: u128, liquidity: u128, round_up: bool) -> Result<u64, _>` :224 — `ΔA = L·(p_upper−p_lower)·2^64 / (p_lower·p_upper)`.
- `get_amount_delta_b(...) -> Result<u64, _>` :260 — `ΔB = L·Δp / 2^64`.
- `get_delta_x` :289, `get_delta_y` :376 (aliases, rounding passthrough).
- `clmm_max_delta_x_in_tick(sqrt_price_current, sqrt_price_lower: u128, liquidity: u128) -> Option<u64>` :304; `clmm_max_delta_y_in_tick` :322; `clmm_max_input_in_tick` :336; `clmm_active_tick_max_input(sqrt_price_current: u128, tick_current: i32, tick_spacing: u16, liquidity: u128, a_to_b: bool) -> Option<u64>` :353 (floor division via `get_amount_delta_*(…, false)`).
- `get_next_sqrt_price_from_a_input(sqrt_price: u128, liquidity: u128, amount_in: u64) -> Result<u128, _>` :389 — ceil.
- `get_next_sqrt_price_from_b_input(...)` :412 — floor.
- `compute_swap_step_checked(sqrt_price_current_x64, sqrt_price_target_x64: u128, liquidity: u128, amount_remaining: u64, fee_rate: u16, a_to_b: bool) -> Result<StepSwapResult, _>` :444; `compute_swap_step` :557; `StepSwapResult{amount_in, amount_out, next_sqrt_price_x64, sqrt_price_next_x64, fee_amount}` :434-441.
- `compute_swap(...)` :869; `compute_swap_with_ticks_per_array` :894; `quote_clmm` :995; `quote_clmm_with_ticks_per_array` :1020 — output `u64` raw units.
- `find_next_tick_boundary[_with_size]` :744/760; `parse_tick_array_account` :607; `parse_raydium_clmm_tick_array_account` :679; `update_liquidity_on_cross` :829; `get_start_tick_index[_with_size]` :592/597.

### arb-math/dlmm.rs — `SCALE_OFFSET: u8 = 64` :19; `ONE_Q64: u128 = 1<<64` :22; `BASIS_POINT_MAX: u128 = 10_000` :25; `FEE_PRECISION: u128 = 1e9` :28; `MAX_FEE_RATE: u128 = 100_000_000` (10%) :31; `MIN/MAX_BIN_ID = ∓443_636` :34/37; `MAX_EXPONENTIAL: u32 = 0x80_000` :40.
- `pow_q64(base: u128, exp: i32) -> Result<u128, DlmmError>` :223 (Q64.64).
- `get_price_from_id(active_id: i32, bin_step: u16) -> Result<u128, DlmmError>` :279 — `P_i = (1 + bin_step/10_000)^i`, Q64.64.
- `dlmm_active_bin_max_input(active_bin_id: i32, bin_step_bps: u16, active_bin_amount_x, active_bin_amount_y: u64, a_to_b: bool) -> Option<u64>` :303 (floor).
- `scale_dlmm_three_hop_input(probe_amount_in: u64, probe_gross_profit: u64, hop1_active_bin_max_in, hop2_active_bin_max_in_equiv, hop3_active_bin_max_in_equiv, max_wallet_cap: u64) -> (u64, u64)` :339 (92% of bottleneck).
- `compute_base_fee(parameters: &DlmmParameters, bin_step: u16) -> Result<u128, _>` :372 — `base_factor × bin_step × 10 × 10^power_factor`, units 1e9.
- `compute_variable_fee(parameters: &DlmmParameters, bin_step: u16, volatility_accumulator: u32) -> Result<u128, _>` :395 — `ceil(vfc × (va×bin_step)^2 / 1e11)`, units 1e9.
- `compute_total_fee_rate(...) -> Result<u128, _>` :425 — sum, clamped at `MAX_FEE_RATE`.
- `compute_fee_to_add(amount: u64, fee_rate: u128) -> Result<u64, _>` :439 — ceil(amount×rate/(1e9−rate)).
- `included_fee_amount(excluded: u64, fee_rate: u128) -> Result<u64, _>` :464 — ceil(excluded×1e9/(1e9−rate)).
- `compute_fee_from_amount(amount_with_fee: u64, fee_rate: u128) -> Result<u64, _>` :488 — ceil(amount_with_fee×rate/1e9).
- `compute_protocol_fee(fee_amount: u64, protocol_share_bps: u16) -> Result<u64, _>` :505 — **floor**(fee×share/10_000).
- `update_references_with_decay` :516; `update_volatility_accumulator` :539; `advance_active_id(active_id: i32, swap_for_y: bool)` :558; `bin_array_index_for_bin(bin_id: i32) -> i64` :574 (`div_euclid(70)`).
- `consume_bin_liquidity(bin: &mut DlmmBin, amount_in: u64, price_q64: u128, swap_for_y: bool, fee_rate: u128, protocol_share_bps: u16, fee_on_input: bool) -> Result<DlmmSwapStepResult, _>` :649; `DlmmSwapStepResult{amount_in_consumed, amount_out, fee_amount, protocol_fee, drained_bin}` :180-191.
- `quote_dlmm_multi_bin(pool, bins: &mut [DlmmBin], amount_in: u64, swap_for_y: bool, current_timestamp: Option<i64>) -> Result<DlmmSwapQuote, _>` :805; `quote_dlmm_view` :904; `quote_dlmm_exact_out` :922; `DlmmSwapQuote{…, price_impact_bps: Bps}` :194-212.
- `calculate_price_impact_bps(amount_in, amount_out: u64, start_price_q64: u128, swap_for_y: bool) -> Bps` :1146 (private).

### arb-math/economic.rs — single source of truth (docs :1-9); `ECONOMIC_CALCULATION_VERSION: u32 = 2` :16; `LAMPORTS_PER_SOL: u128 = 1e9` :19.
- `lamports_to_base_raw(lamports: Lamports, rate_num: u128, rate_denom: u128) -> Result<u128>` :199 — rational conversion, `rate = base_raw per lamport`; **rounds UP (ceil) on remainder** :209-216.
- `base_raw_to_lamports(base_raw: TokenAmount, rate_num, rate_denom: u128) -> Result<Lamports>` :244 — `lamports = base_raw × rate_denom / rate_num`, **floor** (`checked_div`) :258-263.
- `build_economic_quote(…) -> Result<EconomicQuote>` :274; `calculate_net_profit_raw(final_received_raw, borrow_principal_raw, flash_fee_raw, dex_fees_raw, transfer_fees_raw, account_creation_cost_raw, other_verified_costs_raw: u64, network_fee_lamports, priority_fee_lamports, jito_tip_lamports: Lamports, lamports_to_base_mint_rate, lamports_to_base_rate_denom: u128) -> Result<u64>` :399; `calculate_flash_fee_raw(borrow_amount: u64, fee_sf: u64) -> Result<u64>` :460 — `ceil(borrow × sf / 2^64)` :466-475.
- `lamports_to_base_raw_checked` :275-307 & `base_raw_to_lamports_checked` :310-331 live in `arb-types/src/dimensional.rs` (ceil **both directions**).

### arb-math/damm_v2.rs — `DAMM_V2_FEE_DENOMINATOR: u128 = 1e9` :10; `DAMM_V2_MAX_FEE_NUMERATOR: u128 = 5e8` :12.
- `DammV2FeeParams::effective_fee_numerator(current_point: u64, activation_point: u64) -> u128` :34 (dynamic variable fee `v²·vfc/1e11` :55-61, linear/exponential decay :43-54).
- `fee_bps(current_point, activation_point: u64) -> u16` :66 (ceil).
- `quote_damm_v2_exact_in(amount_in: u64, a_to_b: bool, sqrt_price_x64, liquidity, sqrt_min_price_x64, sqrt_max_price_x64: u128, fee_numerator_1e9: u128) -> Option<u64>` :75; `quote_damm_v2_exact_in_with_fee_mode(..., fees_on_input: bool)` :99.
- `damm_v2_virtual_reserves(sqrt_price_x64: u128, liquidity: u128) -> (u128, u128)` :189 — `r_a = L·2^64/p, r_b = L·p/2^64`.

### arb-math/launchlab.rs — `LAUNCHLAB_FEE_DENOM: u128 = 10_000` :10; `_PPM: u128 = 1_000_000` :11.
- `quote_launchlab_buy_exact_in_ppm(quote_amount_in, virtual_base, virtual_quote, real_base, real_quote: u64, total_fee_ppm: u64) -> Option<u64>` :14; `quote_launchlab_sell_exact_in_ppm` :48; `quote_launchlab_buy_exact_in(..., total_fee_bps: u16)` :82; `quote_launchlab_sell_exact_in` :118. Ceil fee, cap by real reserves, `out > real → None`.

### arb-math/pump_fun_raydium.rs — `DEFAULT_FLASH_LOAN_FEE_BPS: u64 = 30` :12; `DEFAULT_RAYDIUM_FEE_BPS: u64 = 25` :13; `MIN_NET_PROFIT_LAMPORTS: u64 = 30_000` :14.
- `pump_fun_marginal_price(reserve, supply: u128) -> Option<u128>` :44; `pump_fun_buy_cost(reserve, supply, delta_t: u128) -> Option<u128>` :54 (ceil); `raydium_sell_output(sol_reserve, token_reserve, delta_t: u128, fee_bps: u64) -> Option<u128>` :68; `verify_marginal_decreasing(...)` :90; `calculate_optimal_trade(pump, raydium, flash_fee_bps: u64, max_delta_t: u128) -> Result<OptimalTrade, String>` :132; `calculate_optimal_trade_integer` :118; `OptimalTrade{delta_t, buy_cost, sell_output: u128, net_profit: i128, …}` :29-36.

### arb-math/chronos.rs — all f64, ranking only.
`BPS_DIVISOR_F64: f64 = 10_000.0` :23; `chronos_edge_weight(p_in, p_out: f64, fee_bps, slippage_bps: u32, latency_ms, lambda_decay, beta_risk: f64) -> Result<f64>` :222; `compute_risk_adjusted_score(net_profit_raw: u64, chronos_weight: f64, uncertainty_bps: u64, state_age_slots: u64, age_penalty_bps_per_slot: u64) -> Result<RiskAdjustedScore>` :407; `evaluate_gate(GateInput) -> Result<GateOutput>` :504; `LatencyWindow`, `VolatilityEstimator::mean_volatility_bps` :334. `MAX_F64_INPUT` :27 is declared and **never used**.

### arb-math/latency.rs — `Z_95: f64` :18; `SLOT_MS: f64 = 400.0` :21; `MAX_MARGIN_MS: u64 = 400` :27; `COLD_MARGIN_MS: u64 = 200` :30; `RttTracker{observe, p50_ms, p95_ms, sigma_ms, lead_ms}` :48-169; `plan_fire` :189; `SlotClock::predict_window_start_ns` :280 (least-squares, f64); `schedule` :314. All f64; timing, not monetary.

### arb-types (see §6)
`Lamports(u64)` lib.rs:17; `TokenAmount(u64)` lib.rs:53; `Bps(u16)` lib.rs:89; `UsdMicros(i64)` lib.rs:448; `dimensional::TokenAmount{raw: u128, mint, decimals}` dimensional.rs:22; `dimensional::ProfitAmount{raw: i128}` dimensional.rs:93; `dimensional::TokenPrice{rate_num, rate_denom}` dimensional.rs:194; `currency::TypedTokenAmount{raw_amount: u64,…}` currency.rs:262; `currency::TimestampedRate` currency.rs:141; `currency::DimensionalProfitabilityReport` currency.rs:601; `SwapHop` lib.rs:1014; `ExecutionOpportunity{gross_profit: Lamports, net_profit: Lamports}` lib.rs:1098-1115; `SizedOpportunity` lib.rs:643; `PoolState` lib.rs:873 (`fee_bps()` :928); `DlmmBin{amount_x, amount_y: u128}` lib.rs:817.

### arb-engine/src/lib.rs profit functions
`quote_pool(kind, dx: u128, x: u128, y: u128, gamma: u128, edge) -> u128` :68; `cycle_profit_at(path, d: u64) -> u64` :190; `golden_section_size(path, lo, hi: u64, iters: u32) -> u64` :245 (**f64**); `make_economic_opportunity(...) -> Option<ExecutionOpportunity>` :345; `find_base_cycles(...) -> Vec<ExecutionOpportunity>` :565; `find_multi_hop_cycles` :969; `dfs_multi_hop` :1003 (**f64 pruning** :1043-1063); `evaluate_arbitrage_cycle` :1070; `score_opportunity_chronos` :1230 (#[cfg(test)], f64).

## 2. Spot price, effective execution price, price impact

**No explicit spot- or execution-price function exists anywhere.** Only price-impact-in-bps is computed:

- CPMM: `cpmm_price_impact_bps(dx, x, y: u128, gamma: u128) -> Bps` (cpmm.rs:139-169). Formula: fee-free out `o = dx·y/(x+dx)` (via `cpmm_out(dx,x,y,10_000)` :147); impact = `(dx·y − o·x)·10_000 / (dx·y)` :159-167, floor, clamped to `Bps(10_000)` on any invalid/degenerate input (fail-closed :141,151,162-163). `gamma` is unused in the math (retained for API stability, :144-146) — comment claims "fees never double-counted". Exec-prod `<` spot-prod always, so impact ∈ [0, 10000]. This is the *curve-only* impact; fee-inclusive slippage is not computed anywhere. Effective execution price is only implicit as `o/dx`.
- DLMM: `calculate_price_impact_bps(amount_in, amount_out: u64, start_price_q64: u128, swap_for_y: bool) -> Bps` (dlmm.rs:1146-1175). Ideal out `⌊in×P/2^64⌋` (X→Y) or `⌊in×2^64/P⌋` (Y→X) :1157-1165; `try_into().unwrap_or(u64::MAX)` silently saturates an over-u64 ideal :1159,1164; impact = `⌊(ideal−out)×10_000 / ideal⌋` :1172; `ideal <= out` → `Bps(0)` :1167.
- CLMM: impact is never reported as bps; only raw `amount_out` is returned by `quote_clmm` (clmm.rs:995). Route-level impact = compounded `∏(1 − bps_i/10_000)` in `quote_route` (arb-engine/src/adapters.rs:284-291), using `price_impact_bps as u128 * 100_000` :287, floor at :291 — note `u16 → u128` cast is lossless, fine; compounding drops sub-1e-5 fractions.
- CPMM output formula (gamma in / D out, all floor): out = `γ·dx·y / (D·x + γ·dx)` (cpmm.rs:94). Upfront-fee variant (fee on input, ceil fee): fee = `ceil(dx·fee_num/fee_den)` (cpmm.rs:54), out = `net_dx·y/(x+net_dx)` floor (:61). Both truncate toward zero → output never exceeds reserves (verified by proptest :817).
- CLMM Δ amounts round by direction: inflows ceil, outflows floor (`round_up` parameter, clmm.rs:224-255, 260-286); `div_ceil_u256/u64` (:78, :92). Next sqrt-price: from A input **ceil** (:404), from B input **floor** (:427) — both "pool-favoring" (input delta never understated).
- Overflow handling: no `checked_*` on the hot U256 paths; operations like `l * delta_p << 64` (clmm.rs:241) and `gamma*dx*y` (cpmm.rs:89) can exceed 256 bits for extreme u128 inputs (wrap in release / panic in debug). U384/U512/U768 sizing claims ("radicand ≤ 293/441/618 bits", cpmm.rs:240,305,377) hold only for reserves ≲ 2^82; u128-max reserves (2^128) need ≈ 566 bits and overflow `U384`. `u64::try_from(out).ok()` (cpmm.rs:62,112) and `u64::try_from(...).map_err(...)` (clmm.rs:254) do fail closed on narrowing. `unwrap_or(u128::MAX)` appears in `dlmm_active_bin_max_input` (dlmm.rs:321,329) and `damm_v2_virtual_reserves` (damm_v2.rs:198-199) — silent saturation instead of error.

## 3. Fee math — ordering & rounding

**CPMM (25 bps type fees):**
- `cpmm_fee_paid_with_denom` (cpmm.rs:183-188): `ceil(amount_in × fee_rate / fee_denominator)` via u128 `div_ceil`, result cast `as u64` (silent truncation if `fee_rate > denominator`, i.e. >100% fee). Ceil ⇒ user never underpays; matches "fee taken up".
- But the *quote* path `cpmm_out_fee` (cpmm.rs:73-100) deducts the fee **implicitly in the numerator** (γ·dx·y), not as a discrete ceiling step; `quote_cpmm(amount_in,…,fee_bps)` (:104-113) therefore yields `floor` output with embedded floor-fee — **the fee the user actually pays and the fee `cpmm_fee_paid` reports disagree by ≤1 unit** (quote assumes fee = dx·(D−γ)/D exactly; `cpmm_fee_paid` assumes ceil). `cpmm_out_upfront_fee_ceil` (cpmm.rs:41-63) is the ceiling-fee variant and is the one wired to Raydium CPMM (`arb-dex/src/raydium_cpmm.rs:5,297`), good; but nothing in `cpmm_out` validates `gamma <= fee_denominator` — **`gamma == 0` rejected but `gamma > D` accepted** (cpmm.rs:80), so a pool whose `fee_numerator > fee_denominator` (or whose engine-derived `fee_bps` wrapped, see §11) prices with gamma > D, overstating output. `is_fee_valid` (arb-types lib.rs:940) only checks `fee_denominator != 0`.
- Engine derives `fee = ((p.fee_numerator as u128 × 10_000) / p.fee_denominator as u128) as u16` (arb-engine lib.rs:431) — `as u16` wraps for malicious numerators (see §11); same pattern in `build_from_pools` → `fee_bps` (lib.rs:928-935 uses `u16::try_from(...).unwrap_or(10_000)` — saturates, better) and `quote_pool`'s CLMM branch `fee_rate_u128 = (10_000 − gamma) × 100` (:87) is safe (γ ≤ 10_000 checked :84).

**CLMM (sqrt_price_x64, fee_den = 1e6):**
- Full tick step: fee = `ceil(amount_in_max × fee_rate / (1_000_000 − fee_rate))` (clmm.rs:488-492), `amount_in = amount_in_max + fee` (:493) — Uniswap-style fee-on-top-to-reach-target; correct numerator/denominator ordering.
- Partial step: `fee_on_remaining = ceil(amount_remaining × fee_rate / 1e6)` (:480-484); net = remaining − fee; next price from net input; `amount_in = amount_remaining` (:546) includes fee. Correct ceiling, pool-favoring.
- `get_amount_delta_*` round inflows up / outflows down (`round_up`), see §2 — correct direction for both input solency and output fabrication prevention.
- Consistency risk: `amount_in ≤ amount_remaining` is only guaranteed by `saturating_sub` at the outer loop (clmm.rs:946); at the exact boundary where `net == amount_in_max`, `amount_in_max + fee_full` **can exceed `amount_remaining`** and `checked_sub` returns `IntegerOverflow` (:947) — fail-closed but spurious-error-prone at the boundary (minor).
- Fee is `u16` hundredths-of-bip; `FEE_RATE_DENOMINATOR` never checked against `fee_rate = 1_000_000` (100%) — `fee_denom.max(1)` (:491) masks a zero subtraction rather than rejecting it.

**DLMM (bin step, fee_precision = 1e9, MAX 10%):**
- Base fee `bf × bs × 10 × 10^pf` (dlmm.rs:372-389) — no division, exact, no rounding direction to audit; overflow checked.
- Variable fee `ceil(vfc × (va × bin_step)² / 1e11)` (:408-422) — explicit div_rem + 1 ⇒ ceil, favors pool.
- Total clamped to `MAX_FEE_RATE` = 1e8 (:433) — on-chain parity.
- Fee-on-top drain: `compute_fee_to_add` = ceil(amount×rate/(1e9−rate)) (:439-456) ✓; `included_fee_amount` mirrors SDK `getIncludedFeeAmount` (:464-483) ✓; `compute_fee_from_amount` = ceil(amount_with_fee×rate/1e9) (:488-502) ✓.
- **Protocol fee floors**: `compute_protocol_fee` = `fee × share / 10_000` (floor, dlmm.rs:509-513) — user keeps up-to-1-unit dust; asymmetric with the ceil trading fees; consistent with "favor the LP" but undocumented as intentional.
- Fee-mode routing `dlmm_fee_on_input(collect_fee_mode, swap_for_y)` (dlmm.rs:103-110) mirrors SDK `getFeeMode`; X→Y + OnlyY ⇒ fee on output, handled by `consume_bin_liquidity` else-branch (:751-794). Exact-in and exact-out dust handling mirrored from SDK with an explicit comment (dlmm.rs:1043-1063); `quote_dlmm_view` is non-mutating (:904).
- `calculate_price_impact_bps` (:1146) includes fees? No — compares ideal-out vs actual out, and out is already net of fee, so reported "price impact" is slightly overstated (fee included in the "impact" number) on the DLMM path — unlike the CPMM path where fees were explicitly excluded (cpmm.rs:143-146). Inconsistent semantics between modules.

**DAMM v2:** fee-on-input branch: `fee = ceil(net_in×num/1e9)` (damm_v2.rs:117-124) deducted upfront; fee-on-output branch: `fee = ceil(gross×num/1e9)`, `net = gross − fee` (:176-182). `fee_bps()` ceils (:66-70). `effective_fee_numerator` variable part is **floor** (`v²×vfc / 1e11`, :58) and dynamic fee uses `saturating_mul` — saturation is silent (overflow → cap, not error). Linear decay: `base − periods × reduction_factor` saturating (:45-46) — fine (fee floor clamped at 0). Exponential: `base × (1 − r/10_000)^passed` via repeated `×keep/10_000` floor (:50-52) — floor per iteration, slightly understates decayed fee (user-favored). `MAX 50%` cap (:62).

**LaunchLab:** fee ceil on both buy (fee from input :102) and sell (fee from output :139-140); `fee >= dx → Some(0)` rather than error (:35,104) — silently zero-output on ~100% fee instead of rejecting (mildly hostile API). Caps: `out > real_base/real_quote → None` (:41,143) — pool-drain guard, correct.

**Cross-module inconsistency:** CPMM upfront-fee path uses ceil for fee; DLMM uses ceil; CLMM ceil; LaunchLab ceil; DAMM v2 ceil — good uniformity. But `cpmm_price_impact_bps` is fee-free, DLMM impact is fee-inclusive, CLMM impact is not reported at all — "slippage" means three different things per protocol.

## 4. Token-2022 transfer fee — correct implementation vs. double-count bug

`apply_transfer_fee_ceil(amount, fee_bps, maximum_fee)` (arb-math/lib.rs:69-77): computes `raw_fee = ceil(amount × bps / 10_000)` :73-74, caps at `maximum_fee` :75, and **returns `amount − capped_fee` — i.e. the NET post-fee amount, not the fee**. Unit/semantics consequence of reading the signature name wrong causes the bug:

**Bug (meteora_quote_engine.rs:118-123 and :134-139) — quote_single_hop.**
```rust
let effective_amount_in = if pool.transfer_fee_bps > 0 {
    let fee = apply_transfer_fee_ceil(amount_in, pool.transfer_fee_bps, u64::MAX);
    amount_in.saturating_sub(fee)
} else { amount_in };
```
Since `apply_transfer_fee_ceil` returns `amount − fee`, `effective_amount_in` collapses to `amount_in − (amount_in − fee) = fee`. Concrete: `amount_in = 10_000`, `bps = 100` (1%): the function returns 9_900 (net); `effective_amount_in = 10_000 − 9_900 = 100` — the DLMM bins are priced against an input equal to **the fee alone (1% of the intended trade)**, not the net input (9_900). Every DLMM cross-pool quote through `quote_single_hop`/`quote_two_hops` understates effective input by 99×; outputs and the resulting `gross_profit = hop2_out − amount_in` (meteora_quote_engine.rs:158) are garbage — no profitable cycle is ever reported (or, with `bins_crossed` inflation, quotes that consume the fee-sized input may even out-price the bin ladder and yield a wildly empty quote). The outbound leg has the identical defect: `effective_amount_out = quote.amount_out − (quote.amount_out − fee) = fee` (:134-139), i.e. the reported `hop2_final_amount_out` is the **fee**, and `spread_bps` (:160) divides that by `amount_in` — meaningless. `hop1_fee`/`hop2_fee` fields carry DLMM pool fees, unaffected, but the two boundary quantities that feed `gross_profit` are both wrong. The fix is to use the function's return directly as the net amount (no second subtraction) — the pattern used correctly in every arb-dex adapter: `let net_in = apply_transfer_fee_ceil(amount_in, tf_in, u64::MAX);` used directly (e.g. `arb-dex/src/meteora_dlmm.rs:426-466`, `orca_whirlpool.rs:324`, `raydium_cpmm.rs:297`, `raydium_clmm.rs:351`, `meteora_damm_v2.rs:226`, `raydium_launchlab.rs:215`). Note even the correct call sites pass `u64::MAX` as `maximum_fee` — they model transfer fees as **bps-only and ignore the per-mint `maximum_fee` cap and the epoch-scheduled old/new rate** (the real `TransferFeeSchedule` machinery exists at `arb-dex/src/lib.rs:99-118`, `calculate_transfer_fee` ceil+cap, but is not wired into any quote path; arb-dex/src/lib.rs:98-109). Also note `apply_transfer_fee_ceil`'s `maximum_fee` parameter then only exists on paper in all quote paths.

**Engine path is silently fee-blind:** `quote_hop` rejects any pool with `transfer_fee_bps != 0` (adapters.rs:205-209) rather than model it — so the main engine never sees Token-2022 fees at all. `make_economic_opportunity` also hardcodes `transfer_fees_raw: 0` (arb-engine/lib.rs:372-374) and `dex_fees_raw: 0`. Net effect: Token-2022 pools are excluded from the engine route search, but the (also live, independently-callable) Meteora cross-pool quote engine misprices them, and `arb-sim` records `transfer_fees_lamports` (profitability_gate.rs:42, meteora_fresh_pipeline.rs:113) in **lamports** although transfer fees are paid in token units — unit mislabeled.

## 5. Profit calculation path end-to-end; raw lamports vs token base units

Authoritative flow (flash-loan cycle, WSOL base):

1. Engine sizes via `optimal_two_pool`/`cycle_profit_at` → `cycle_profit_at` (arb-engine lib.rs:190) returns `q.amount_out − d` in **base-mint raw units** but compares `q.amount_out > d` against `d: u64` — d is raw units, ok — and stores result as `u64`; → `evaluate_arbitrage_cycle` (:1070) recomputes output at clamped size (:1149-1153), then `final_out = curr as u64` (:1154, **truncation**), then `make_economic_opportunity` (:1198).
2. `make_economic_opportunity` (:345-416): flash fee = `calculate_flash_fee_raw(size_in, config.flash_loan_fee_sf)` (:358) — `ceil(size_in × sf / 2^64)`, raw base units (WSOL: raw == lamports). It then builds the authoritative quote via `build_economic_quote` (:365) passing `minimum_final_amount_raw = expected_final_amount_raw` (:370) — **the "conservative minimum-out" leg is fed the expected output, so `minimum_net_profit_signed_raw ≡ expected_net_profit_signed_raw` and `meets_min_profit` is not conservative at all**; `min_out` is never derived from hops (`hop_inputs` is computed but never wired into the economic quote). It converts `gross_profit_raw`/`net_profit_raw` to lamports with `base_raw_to_lamports` (:391,:397; floor division, economic.rs:258-263) and stuffs them into `ExecutionOpportunity.gross_profit/net_profit: Lamports` (arb-types lib.rs:1108-1109) — raw-units-as-Lamports if base ≠ WSOL is avoided only because the rate converts 1:1 for WSOL; for USDC base the conversion is correct units-wise but the *type* lies (see §7).
3. Supervisor (`arb-bot/src/supervisor/run.rs`) re-computes costs independently:
   - `flash_fee_raw = calculate_flash_fee_raw(opp.size_in.as_u64(), verified.flash_loan_fee_sf.value).unwrap_or(u64::MAX)` (:624-632) — raw base units.
   - `priority_fee_lamports = cu_price.saturating_mul(cu_limit as u64) / 1_000_000` (:663).
   - `net_before_tip = gross(lamports, :664 `opp.gross_profit.as_u64()`)  − flash_fee_raw(raw base units) − BASE_FEE_LAMPORTS − priority_fee_lamports` (**run.rs:676-679**). **flash_fee_raw (base-mint raw units) is subtracted from gross (lamports).** Identical units only when base mint is WSOL (1:1). For USDC-anchored flash routes (engine does not forbid them) the subtraction mixes USDC raw with lamports → over-deducts by ~1e8×, or under-deducts for illiquid 9-decimal bases; `t_max = net_before_tip − min_net_floor − 10_000` (:686-689) is therefore unit-contaminated for non-WSOL bases.
   - The same file passes `BASE_FEE_LAMPORTS = 5_000` (arb-bot/src/supervisor.rs:43) as `network_fee_lamports` into the authoritative `calculate_net_profit_raw` (run.rs:805-820) — that call is unit-consistent (lamports → converted via `verified.lamports_per_base_raw/denom` :816-817, ceil via economic.rs:209-216) and a fresh tip value is included; good. But it then wraps the result in `Lamports(real_net_base_raw)` (run.rs:1048 and :1059) while the comment at run.rs:801-804 explicitly warns "It is NOT in lamports. We must never cast it to `Lamports`" — the code does exactly that; the risk gate's `check_min_profit` (arb-risk/src/lib.rs:587-604) compares base-raw units against `limits.min_net_profit_abs` (lamports) and `amount_in.as_u64() × min_net_profit_bps / 10_000` (unit-agnostic but inconsistent with the lamports floor) — apples-to-oranges thresholds.
4. Flash repayment: Kamino repay amount = `borrow + calculate_kamino_flash_fee(borrow, sf)` with `saturating_add` (arb-tx/src/lib.rs:2800-2804, :3186-3204, :3260-3262, :3310-3312; template.rs:283-286, :467-471). `calculate_kamino_flash_fee` (arb-tx/src/kamino.rs:170-179) duplicates `economic::calculate_flash_fee_raw` in u128 (ceil :177) — two implementations of the same formula; identical results for u64 inputs (no u128 overflow possible: 1.8e19 × 1.8e19 = 3.4e38 < u128::MAX = 3.4e38 — just fits; still one mul short of headroom).
5. `jito_tip` is **not** part of `calculate_net_profit_raw`'s base set — it is passed as `jito_tip_lamports` separately and converted via rate (:317 of economic.rs) ✓; `BASE_FEE_LAMPORTS` likewise. `rent_ata` from `calculate_net_profit` (arb-math lib.rs:103-117) is subtracted in the older standalone path but never in the authoritative path (engine passes `account_creation_cost_raw: TokenAmount(0)`, lib.rs:373) — temporary ATA rent is ignored in profitability.
6. `opp.net_profit` (lamports) is what `make_economic_opportunity` reports; supervisor prefers `real_net_base_raw` for the go/no-go (run.rs:821-849) — two different net numbers coexist in the same process (`opp.net_profit` used for ranking sort at lib.rs:963/998 vs `real_net_base_raw` used for gating).

**Every occurrence of raw-lamports/base-unit mixing found:**
| Location | Mix |
|---|---|
| arb-bot/src/supervisor/run.rs:676-679 | `gross` (lamports) − `flash_fee_raw` (base raw) − BASE_FEE − priority |
| arb-bot/src/supervisor/run.rs:686-689 | `t_max` derived from the above, feeds tip ceiling |
| arb-bot/src/supervisor/run.rs:1046-1069 | `Lamports(real_net_base_raw)` (base raw in a lamports wrapper) → risk gate |
| arb-bot/src/supervisor/run.rs:853-858 | `lamports_to_base_raw_checked(min_net_profit_abs(lamports)…)` — correct direction (lamports→raw), but uses the *ceil* dimensional variant while economic.rs uses the same ceil — consistent there; however the value is compared to `real_net_base_raw` floors derived with economic.rs's floor `base_raw_to_lamports` (engine :391-402) — mixed rounding between gate and display |
| arb-bot/src/supervisor/run.rs:663 | `cu_price(micro-lamports/CU) × cu_limit / 1e6` — correct by construction |
| arb-bot/src/supervisor/run.rs:1043-1045 | `tip_share_bps = tip × 10_000 / gross_profit` — lamports/lamports, consistent |
| arb-math/pump_fun_raydium.rs:203-206 | `sell_output`(raydium SOL lamports) − `buy_cost`(pump.fun SOL lamports) ± `MIN_NET_PROFIT_LAMPORTS` — consistent for SOL legs, but hard-coded 30_000 lamport cushion inside the sizing math |
| arb-engine/src/lib.rs:61 & arb-math/pump_fun_raydium.rs:14 | `MIN_NET_PROFIT_LAMPORTS = 30_000` duplicated in two crates |
| arb-sim/profitability_gate.rs:42 & meteora_fresh_pipeline.rs:113 | `transfer_fees_lamports` — token-unit fee denominated as lamports |
| arb-types/src/lib.rs:1108-1109 | `ExecutionOpportunity.gross_profit/net_profit: Lamports` regardless of base mint |
| arb-bot/src/supervisor/run.rs:964-965 (gross > 50_000 checks, e.g. :694, :741) | lamports constants compared against `opp.gross_profit` — fine when WSOL base |
| arb-engine/src/lib.rs:1237-1240 (`#[cfg(test)]`) | `opp.net_profit.0 as f64 / 10f64.powi(base_decimals)` — lamports reinterpreted as 10^-decimals units (double unit shift for USDC: divides lamports by 1e6, mixing SOL/USD scales) |

## 6. Decimals handling & cross-mint arithmetic

- Decimals live as `u8` fields supplied from verified mint accounts, **never hardcoded to 9 in the authoritative path**: `EconomicConfig.base_decimals: u8` (economic.rs:139) — enforced ≤ 18 (:294); `PoolSnapshot.decimals_a/b` (arb-types lib.rs:572-573); `PoolState.decimals_a/b` (:880-883) — documented "`0` means unknown" (:881); `ExecutionOpportunity.base_decimals` (:1112). `calculate_flash_fee_raw` etc. don't consult decimals at all (exact in raw units) ✓.
- **But `cycle_profit_at` builds its internal `PoolState` with `decimals_a: 0, decimals_b: 0`** (arb-engine lib.rs:209-210) — decimals discarded. Any downstream consumer of that `PoolState` that assumes non-zero decimals mis-scales; currently only used for pool identity, but the zero sentinel is exactly what `is_fee_valid`-style guards elsewhere treat as "unknown".
- Cross-mint/unit arithmetic protection exists but is **opt-in and duplicated**: `ProfitAmount::checked_add/checked_sub` reject mint/decimal mismatch (dimensional.rs:144-190); `TypedTokenAmount::checked_add/sub` same (currency.rs:319-376); `DimensionalProfitabilityReport::compute` requires initial/final currency+decimals equal (currency.rs:640-660). **None of these guards sit on the live engine path** — the engine subtracts `out − in` as raw u128 with no mint tag at all (e.g. `cycle_profit_at` lib.rs:238; `cycle_profit_at`/`evaluate_arbitrage_cycle` lib.rs:1114-1180). The safety layer is bypassed where the money actually flows.
- Two different `TokenAmount` types coexist in one crate: `arb_types::TokenAmount(pub u64)` (lib.rs:53, raw u64) vs `arb_types::dimensional::TokenAmount{raw: u128, mint, decimals}` (dimensional.rs:22) vs `currency::TypedTokenAmount` — same name, three semantics, no compile-time unification; `dimensional` and `currency` modules are exported (`pub use currency::*` lib.rs:5) so the collision is user-visible.
- Conversion rounding asymmetry between the two "authoritative" converters: `economic::lamports_to_base_raw` **ceils** liabilities (:209-216); `dimensional::lamports_to_base_raw_checked` also ceils (:299-306) ✓ — but `economic::base_raw_to_lamports` **floors** (:258-263, conservative for profit) while `dimensional::base_raw_to_lamports_checked` **ceils** the same conversion (:323-330) — converting profit identically in two modules yields different answers (up to 1 unit); the two modules disagree on the same economic reality.
- `currency::TimestampedRate::convert_amount` (currency.rs:205-258) floors the SOL→quote conversion of **network fees — a liability — understating the cost** (:245), and truncates the result through `u64::try_from` (:246-247, error on overflow, ok). Inconsistent with economic.rs's ceil-for-liabilities and with dimensional.rs.
- `dimensional::TokenAmount::to_decimal` / `ProfitAmount::to_decimal` (dimensional.rs:61-67,135-141) and `TypedTokenAmount::as_ui_f64` (currency.rs:313-316) — f64 display-only, documented as such; no execution decision consumes them.
- `graduation_progress_bps` uses u128 floor and clamps at 10_000 (arb-types lib.rs:859-863) ✓; `PoolState::fee_bps` floors `fee_numerator×10_000/fee_denominator` and `u16::try_from(...).unwrap_or(10_000)` saturates unknown fees to 100% (lib.rs:929-935) — a garbage fee clamps instead of rejecting.

## 7. Profit bps / percentage calculations; division-by-zero handling

- `cpmm_price_impact_bps` — `spot_prod.is_zero()` → `Bps(0)` (cpmm.rs:161); `out == 0` → `Bps(10_000)` (:150); zero inputs → `Bps(10_000)` (:140). No div-by-zero (denominator `spot_prod`, guarded). bps clamped to ≤ 10_000 via `min` + `u16::try_from(...).unwrap_or(10_000)` (:167).
- DLMM `calculate_price_impact_bps` — guards `amount_in == 0 || amount_out == 0 || start_price_q64 == 0` → `Bps(0)` (dlmm.rs:1152); denominator `ideal_out`, but if `ideal_out == 0` (tiny input), `ideal_out <= amount_out` returns `Bps(0)` (:1167) before the divide ✓. `u16::try_from(bps.to::<u64>().min(10_000)).unwrap_or(10_000)` (:1173).
- `TwoHopDlmmQuote` spread: `if gross_profit > 0 { (gross_profit as u64 * 10_000) / amount_in.max(1) } else { 0 }` (meteora_quote_engine.rs:159-163) — division by `max(1)` ✓, gross_profit `i64` → `as u64` safe only inside the `> 0` guard ✓; `spread_bps` otherwise 0. But `gross_profit = (hop2_out as i64) - (amount_in as i64)` (:158) — u64→i64 cast wraps for values > i64::MAX (theoretical).
- `SizedOpportunity::net_bps(notional_usd: UsdMicros) -> u16` (arb-types lib.rs:663-678): guards `notional_usd.0 <= 0` → 0 (:664); computes **f64** `(net × 10_000) / notional`, clamps negative→0, ≥ `u16::MAX` → `u16::MAX`, then truncates via `as u16` — float rounding decides the boundary; ok for ranking, not for gating.
- `DimensionalProfitabilityReport::net_bps` (currency.rs:717-722): integer i64 division, `initial_input.raw_amount > 0` guard (:718) — correct handling; `net_profit_quote_raw` is i64 and `net_bps` sign reflects losses ✓.
- `check_slippage_bound` (cpmm.rs:137-145): `expected_out == 0` → false ✓. `check_constant_product_invariant` :124.
- `MIN_NET_PROFIT_LAMPORTS` floor enforced twice with different constants (30_000 arb-engine lib.rs:61 / arb-math pump_fun_raydium.rs:14 vs 50_000 hurdle `DEFAULT_STRICT_PROFIT_HURDLE_LAMPORTS` profitability_gate.rs:22 vs `min_net_profit_abs` config) — three inconsistent floors in three crates; engine ranking vs sim gate can disagree on acceptance.
- `marginal_profit_derivative*` return i128×1e9; no div-by-zero (size==0 → 0 :538-540; denom_sq zero → i128::MIN :521-523). `i128::try_from(...).unwrap_or(i128::MAX)` saturates instead of erroring (:529-531,549-551).

## 8. Flash-loan repayment formula; sf=0 fixtures

Exact computation (two copies of the same formula):
- `economic::calculate_flash_fee_raw(borrow_amount: u64, fee_sf: u64) -> Result<u64>` (economic.rs:460-476): `num = U256(borrow) × U256(sf)`, `scale = 1<<64`, `fee = ceil(num / 2^64)` with explicit remainder check (:466-475). Zero borrow → 0 (:461). **Any u64 sf (incl. `u64::MAX ≈ 1.0) ceils to at most the loan itself** (fee < loan, repayable; economic.rs:464-465, test :732). Q64.64 is a u64 scale factor — `flash_loan_fee_sf` is documented as raw Q64.64 (economic.rs:129-134), e.g. main-market WSOL/USDC `sf = 11_529_215_046_068 ≈ 6.25e-7 ≈ 0.006 bps` (:133, test :534-537), value for 1 SOL = ceil(1e9 × 6.25e-7) = **625 lamports**. Correct direction (never understates repayment).
- `kamino::calculate_kamino_flash_fee` (arb-tx/src/kamino.rs:170-179): u128 version, ceil via `num / scale + u128::from(!num.is_multiple_of(scale))` (:177) — identical formula, second implementation to drift.
- Repay instruction amounts: `repay_amount = borrow_amount.saturating_add(fee)` (arb-tx/src/lib.rs:2800-2804, :3186, :3260-3262, :3310-3312; template.rs:283-286, :467-471). `saturating_add` hides overflow instead of trapping (borrow + fee cannot exceed u64 since fee ≤ borrow for valid sf — ok in practice).
- **`sf = 0` in fixtures ⇒ flash fee silently untested.** Every engine/test fixture sets `flash_loan_fee_sf: 0`: `arb-engine/src/lib.rs` test_config :1217 and test at :1363, `arb-engine/tests/pipeline.rs:71`, `arb-engine/src/bellman_ford.rs:789`, `arb-engine/src/bellman_ford.rs:479,567,626` (`.unwrap_or(0)`), `arb-engine/src/lib.rs:1111` (`.ok()?` → opportunity dropped on any error), `arb-engine/src/lib.rs:358` (`.ok()?`). With `sf = 0`, `calculate_flash_fee_raw` returns 0 and every test "passes" the fee path without ever pricing it; only `arb-math/economic.rs::flash_fee_uses_q64_64_sf_exactly` (:520) exercises a non-zero sf. The live supervisor correctly does **not** default sf to 0 — it reads `verified.flash_loan_fee_sf.value` (run.rs:627) with `unwrap_or(u64::MAX)` fail-closed (:629) — but `engine::test_config()` and every `BellmanFordEngine` default config ship `0` (bellman_ford.rs:789), so CI-approved sizes assume a zero flash fee. The `quote` tests that matter (e.g. pipeline.rs:71 with `min_profit_raw: TokenAmount(u64::MAX)` at :?) can pass while repayment is underpriced by the real sf (~625 lamports per SOL).
- `validate.rs:456` and `live_scanner.rs:244` use `calculate_flash_fee_raw(...).unwrap_or(u64::MAX)` — fail-closed in the tools, good.

## 9. Priority fee / CU price; tip cap checks in arb-risk

- `calculate_dynamic_cu_price(profit_lamports: u64, base_priority_fee: u64) -> u64` (arb-tx/src/lib.rs:106-109): `dynamic_elevation = (profit_lamports / 25_000).min(500_000)`; returns `base + elevation`. **It is capped — the claimed "uncapped gross_profit/25_000 inflation" does not hold at this function level**, but it is (a) driven by **gross** profit, not net, so every lamport of edge raises the CU bid before any cost is deducted; (b) still `gross/25_000` — 1 SOL of gross profit buys +40_000 micro-lamports/CU (40× base at 1_000 DEFAULT_PRIORITY_FEE_MICROLAMPORTS, arb-tx/src/pump_fun_raydium.rs:19) with no subtraction of the priority fee itself — the bid is profit-indexed, self-referential, and unbounded below the 500k cap (500_000 micro-lamports/CU × 1.4M CU cap ≈ 0.7 SOL max burn — absurd but technically bounded).
- `calculate_fee_deduplicated_cu_price` (arb-tx/src/lib.rs:114-124): bundle-only mode returns 0.
- `cap_cu_price_by_profit(profit_lamports, cu_limit, desired_cu_price, max_share_bps) -> u64` (arb-tx/src/lib.rs:166-178): `budget = profit × max_share_bps / 10_000`, `max_price = budget × 1e6 / cu_limit`, `desired.min(max_price)` — a real cap. It is called by the supervisor with `PRIORITY_FEE_MAX_SHARE_BPS = 2_000` (arb-bot/src/supervisor.rs:47, usage run.rs:656-661) — total priority fee ≤ 20% of gross. **But** `assemble_arbitrage`/`assemble_flash_arbitrage` *re-add* `calculate_dynamic_cu_price(opportunity.gross_profit.as_u64(), priority_fee_micro_lamports)` on top of whatever CU price the caller passed (arb-tx/src/lib.rs:2893-2898, :3076-3081) — so the caller's 20%-capped price arrives as `base_priority_fee` and an additional gross-indexed elevation (up to +500_000) is applied inside the assembler, **defeating the caller's cap** and double-counting the profit-indexed escalation. The result passed to `build_compute_unit_price` (:2898) is therefore `caller_capped_price + min(gross/25_000, 500_000)`.
- CU price has no wall-clock sanity bound elsewhere; `estimate_route_cu`/static per-hop CU (lib.rs:129-139) fine.
- **Tip cap checks in arb-risk**: `check_tip_share` (:551-559, vs `max_tip_share_bps`), `check_tip_lamports` (:560-568, vs `max_tip_lamports`), `check_tip_spend_ratio` (:642-661, f64 ratio vs `max_tip_spend_ratio`, net==0 → Ok :648) — all exist and are called **only inside `evaluate_flash_loan`** (:517-549), together with `check_max_inflight_bundles` (:544) and `check_min_landing_rate` (:545). **`evaluate_full`** (:450-475, the non-flash-loan path) calls `check_tip_share`/`check_tip_lamports` (:469-470) **but omits `check_tip_spend_ratio`, `check_max_inflight_bundles` and `check_min_landing_rate`** — the "Phase E" checks are absent from the non-flash path. Both are reachable: supervisor selects `evaluate_flash_loan` when `use_flash_loan` (run.rs:1046-1068), else `evaluate_full`. Additionally the whole risk gate is bypassable via `BYPASS_RISK_GATE` env var — only the kill switch still applies (run.rs:1024-1038). And `pick_tip`'s floor/cap ordering bug: `want.max(floor).min(cap)` (arb-math/lib.rs:93) — when `floor > cap` the tip silently lands on `cap < floor`, violating the floor guarantee instead of refusing; `min_keep` is enforced only at the end (:94-99). `pick_tip` is called by arb-land's `pick_tip`/`pick_dynamic_tip` (arb-land/src/lib.rs:143, :131-155) — that path is live for tip sizing.
- arb-types `TipPlan` math (`expected_value: i64`, inclusion f64) — monetary EV in f64 (arb-types lib.rs:1382-1405); `TipFloor::at`/`lerp` are integer (:1224-1244, `u128` interpolation, floor) ✓.

## 10. Floating point on monetary paths

| Site | Value | f64 usage |
|---|---|---|
| arb-math/chronos.rs:23-27, 222-294, 407-451, 504-545 | routing edge weight, risk score, gate EV | f64 formulas on quoted prices; **ranking only, never authorizes execution** (documented :197-220, :385-390, :457-462); input validation is thorough (finite/positive/NaN-reject) :232-258 |
| arb-math/latency.rs (all) | RTT tracking, log-normal p95, slot clock least-squares | f64 throughout; timing, not funds ✓ |
| arb-math/chronos.rs `LatencyWindow::compute` | mean/percentiles | f64; display |
| arb-types/src/lib.rs:95-97 | `Bps::as_fraction` | f64 multiplier helper (usage sites: ranking) |
| arb-types/src/lib.rs:456-471 | `UsdMicros::from_f64` | monetary conversion; saturates to i64 (clamped, not wrapped) :462-468; used by `from_atomic` :493-496 (`atomic as f64 / 10f64.powi(decimals) × price_usd`) — **wallet P&L inputs can differ by 1 micro-USD rounding** |
| arb-types/src/lib.rs:663-678 | `SizedOpportunity::net_bps` | f64 division for profit bps |
| arb-types/src/lib.rs:1267-1270 | `Posterior::PRIOR` etc. | landing-probability posteriors, f64; bet pricing inputs |
| arb-types/src/lib.rs:1122 | `Z_95: f64` | chronology constants (not funds) |
| arb-types/src/currency.rs:313-316, 401-408 | `as_ui_f64`, Display | display only |
| arb-engine/src/lib.rs:245-268 | `golden_section_size` | **f64 golden-section search on profit maximization**; result cast `((a + b)/2.0) as u64` (:267) — sizing (not final amount), but the chosen borrow size is influenced by float roundoff; `c as u64`/`d as u64` truncate inside the loop (:255-256) |
| arb-engine/src/lib.rs:1043-1063 | DFS "marginal rate" pruning | `reserve_out as f64 / reserve_in as f64` across all hops; prunes branches if `current_rate × 2^remaining < 1.0010` — a branch whose marginal rate is borderline can be pruned on float error (prune margin is only 10 bps) |
| arb-engine/src/lib.rs:1078-1087 | `marginal_product` | same f64 product; `<= 1.0010` rejects — same 10 bps float-sensitive threshold |
| arb-engine/src/lib.rs:1237-1247 (#[cfg(test)]) | `score_opportunity_chronos` | f64 |
| arb-risk/src/lib.rs:652-656 | `check_tip_spend_ratio` | f64 comparison vs limit — advisory gate check |
| arb-risk/src/lib.rs:330-448 | `evaluate_chronos_ev` (ChronosEv) | f64 EV model feeding limit checks |
| arb-tx | — | none on monetary paths found ✓ |
| arb-sim profitability_gate | — | integer u64 saturation ✓ |
| arb-land pick_dynamic_tip (arb-land/src/lib.rs:136-141) | tip scaling | f64 `profit_sol = net/1e9`, `scale`, `effective_share` → `as u64` |

Economic-final amounts (`build_economic_quote`, `calculate_net_profit_raw`) are exact integer (U256) ✓ — the float exposure is concentrated in *sizing*, *ranking*, and *ev/tip* heuristics, not the authoritative solvency check.

## 11. u64/u128 casts that truncate

Silent `as` narrowings on numeric paths (inventory of the audited scope):

| Location | Cast | Risk |
|---|---|---|
| arb-engine/src/lib.rs:87-90 | `(10_000u128 − gamma) × 100 → u16::try_from` | `try_from` ok; but gamma > 10_000 wraps the subtraction to huge u128 → caught by `gamma > 10_000` at :84 ✓ (but see cpmm `gamma > D` acceptance) |
| arb-engine/src/lib.rs:122 | `out as u128` | u64→u128 widening, fine |
| arb-engine/src/lib.rs:164-166, 550-551 | `amount_x/y.min(u64::MAX as u128) as u64` | clamp, intentional, loses information silently |
| arb-engine/src/lib.rs:431 | `((p.fee_numerator as u128 × 10_000) / p.fee_denominator as u128) as u16` | **`as u16` wraps** for fee_bps > 65_535 (malicious numerator) |
| arb-engine/src/lib.rs:508, 526 | `quot.min(u64::MAX as u128) as u64` | clamp ok |
| arb-engine/src/lib.rs:630 | `((size_in as u128 × c2 as u128) / out1) as u64` | **u128→u64 `as` truncation**, sizes ≤ u64 since reserves realistic; out1==0 → div-by-zero panic (out1 can be 0 if hop-1 quote reverted to 0 — cycle_profit_at returns 0 → `size_in = x/0` panic in release? No — `/` by zero panics in all builds) |
| arb-engine/src/lib.rs:657, 894-896 | `out1 as u64`, `out2 as u64`, `out3 as u64` | u128→u64 assumed (quotes are u64-based) ✓ but raw |
| arb-engine/src/lib.rs:787 | `(p_out3 − probe_in as u128) as u64` | subtraction underflow guarded by `p_out3 > probe_in` ✓ |
| arb-engine/src/lib.rs:788-795 | `((…).checked_div(p_out1).unwrap_or(0)) as u64` | p_out1==0 → unwrap_or(0) ✓; u128→u64 `as` |
| arb-engine/src/lib.rs:832, 856 | `(… / out2) as u64` | same class; out2==0 → panic (only reached when out1>c2, out1≠0) |
| arb-engine/src/lib.rs:1020 | `route_counter.saturating_add(1)` | ✓ |
| arb-engine/src/lib.rs:1114 | `((flash_fee_raw as u128 × 10_000) / best_size as u128) as u64` | result ≤ 10_000 ✓ |
| arb-engine/src/lib.rs:1154, 1191 | `curr as u64`, `final_out as u128` (1154 then 1203) | **u128→u64 wrap** then re-widened |
| arb-engine/src/lib.rs:1157-1158 | `u128::from(final_out) <= clamped_size as u128` | compares truncated value |
| arb-engine/src/lib.rs:1154 (restated) | `curr` after 3 hops can exceed u64::MAX only if intermediate > ~1.8e19 — possible with u64::MAX-sized borrows | wrap |
| arb-engine/src/lib.rs:267 | `((a + b) / 2.0) as u64` | f64→u64 sizing |
| arb-engine/src/lib.rs:255-256 | `c as u64`, `d as u64` | f64→u64 truncate |
| arb-math/lib.rs:73-77 | `u64::try_from(raw_fee).unwrap_or(amount).min(maximum_fee)` | fee saturates to `amount` on overflow then `min` — masks |
| arb-math/lib.rs:92 | `net_before_tip.checked_mul(share_bps)?` | ✓ checked |
| arb-math/lib.rs:143 | `(u128::from(diff) × 10_000)/u128::from(expected_out)` | u128 ✓ |
| arb-math/cpmm.rs:187 | `(amount_in as u128 × fee_rate as u128).div_ceil(fee_denominator as u128) as u64` | **truncation if fee_rate > denominator** |
| arb-math/cpmm.rs:529-531, 549-551 | `i128::try_from(x).unwrap_or(i128::MAX)` | saturate |
| arb-math/cpmm.rs:73-100 (`cpmm_out_fee`) | `num = gamma_u × dx_u × y_u` (:89) | **unguarded U256 mul** — wraps/panics for dx·y > ~1e73 raw units; returns out ≤ U256::MAX :95 checked only at the end; `u128::try_from` at :98 ok |
| arb-math/cpmm.rs:54 | `(dx_u × num_u + den_u − 1)` | dx·fee_num ≤ 3.4e38×1e6 = 3.4e44 ✓ for fee_num ≤ 1e6 |
| arb-math/cpmm.rs:549 (`marginal_profit_derivative`) | `out_u × NANO_SCALE / size_u` | 1.8e19×1e9 = 1.8e28 ✓ u128 |
| arb-math/clmm.rs:241 | `(l × delta_p) << 64` | product up to 2.7e67 ok; `<<64` drops high bits silently when product ≥ 2^192 (L > ~6e13 with delta_p ~ 8e28) — extreme CLMM liquidity silently loses value |
| arb-math/clmm.rs:254, 285 | `u64::try_from(amount)` | error path ✓ |
| arb-math/clmm.rs:480-483 | `u128::from(amount_remaining) × u128::from(fee_rate)` | ✓ |
| arb-math/dlmm.rs:321, 329 | `max_x/y.try_into().unwrap_or(u64::MAX)` | silent clamp |
| arb-math/dlmm.rs:360 | `((probe_gross_profit as u128 × scale_factor)/(probe_amount_in as u128)) as u64` | gross_profit u64 ✓ fits |
| arb-math/dlmm.rs:530 | `u32::try_from(decayed).unwrap_or(u32::MAX)` | saturate |
| arb-math/dlmm.rs:1159, 1163 | `.try_into().unwrap_or(u64::MAX)` in price-impact ideal | silent clamp → distorts bps |
| arb-math/economic.rs:213-217, 261-265 | `u128::try_from(…).map_err(Overflow)` | error path ✓ good |
| arb-math/economic.rs:222-227 | `i128::try_from(lhs − rhs)` | sign-preserving ✓ |
| arb-math/pump_fun_raydium.rs:64 | `(num + denom_u − 1) / denom_u` | ceil ✓ U256 |
| arb-math/pump_fun_raydium.rs:87 | `u128::try_from(num / denom).ok()` | ✓ |
| arb-math/pump_fun_raydium.rs:180, 204-206 | `u128::try_from(delta_t_wide).unwrap_or(safe_max).min(safe_max)`; `i128::try_from(sell_output).map_err(…)` | saturate / error ✓ |
| arb-math/pump_fun_raydium.rs:203 | `saturating_add(u128::from(MIN_NET_PROFIT_LAMPORTS))` | saturation masks overflow at cost-side |
| arb-types/src/lib.rs:667-675 | `net_profit_usd.0 as f64 × 10_000.0 / notional`; `bps as u16` | f64 truncation after float rounding |
| arb-types/src/lib.rs:460-469 | `scaled as i64` in `UsdMicros::from_f64` | f64→i64 truncation toward zero, saturating bounds ✓ |
| arb-types/src/lib.rs:494 | `atomic as f64 / 10f64.powi(...)` | precision loss for atomic > 2^53 |
| arb-types/src/lib.rs:860-862 | `(real_sol_reserves as u128 × 10_000)/PUMP_FUN_GRADUATION_LAMPORTS as u128; .min(10_000) as u16` | floor + cap ✓ |
| arb-types/src/lib.rs:933-934 | `u16::try_from(…).unwrap_or(10_000)` | saturate-to-100% clamp (garbage fee ≙ 100%) |
| arb-types/src/currency.rs:393 | `(self.raw_amount as i64) − (other.raw_amount as i64)` | **u64→i64 wrap for amounts > i64::MAX** — wrong sign/magnitude, undetected |
| arb-types/src/currency.rs:245-247 | `num / denom` then `u64::try_from` | floor for liability conversion (§6) |
| arb-types/src/dimensional.rs:51-53 | `u64::try_from(self.raw).ok()` | ✓ |
| arb-types/src/dimensional.rs:66, 140 | `self.raw as f64 / scale` | f64 display |
| arb-engine/src/meteora_quote_engine.rs:158 | `(hop2_out as i64) − (amount_in as i64)` | u64→i64 wrap (theoretical) |
| arb-tx/src/kamino.rs:177-178 | `fee.min(u64::MAX as u128) as u64` | clamp ✓ |
| arb-bot/src/supervisor/run.rs:665 | `gross as f64 / 1e9` | gross→SOL display only ✓ (used for logs/telegram) |
| arb-bot/src/supervisor/run.rs:1022 | `opp.size_in.as_u64() as f64 / 10f64.powi(...)` | notional USD for risk gate; f64 price math |
| arb-ingest/…, fixtures | n/a | decimals hardcoded 9/0 in fixtures; fine as test data |

**Net truncation severity ranking:** (1) `arb-engine/src/lib.rs:431` u16 wrap for pool fees; (2) `arb-types/src/currency.rs:393` i64 wrap in `checked_signed_diff`; (3) `arb-engine/src/lib.rs:630/832/856` potential div-by-zero panic when the intermediate quote is 0; (4) `arb-engine/src/lib.rs:1114/1154/1191` u128→u64 `as` narrowings on the profit path; (5) DLMM price-impact's `unwrap_or(u64::MAX)` fabrications; (6) unguarded U256 multiplies (`cpmm_out_fee`, CLMM numerator shift) — theoretical for extreme inputs.

## Summary of the audit's headline findings

1. **Token-2022 transfer-fee double-subtraction bug**, `crates/arb-engine/src/meteora_quote_engine.rs:117-123` and `:133-139`: net amount is subtracted from itself; quoted input collapses to the fee alone (~1% of intended), outputs and `gross_profit` are meaningless; identical outbound. Correct pattern (single call, direct use) exists in all `arb-dex` adapters (e.g. `arb-dex/src/meteora_dlmm.rs:426`).
2. **Raw flash-fee (base-mint units) subtracted from lamport gross**, `arb-bot/src/supervisor/run.rs:676-679` feeding `t_max` (:686-689) — unit-contaminated for any non-WSOL base mint; same units coincide only at 1:1.
3. **Base-raw net wrapped in `Lamports` for the risk gate**, `arb-bot/src/supervisor/run.rs:1048, :1059`, contradicting the adjacent comment at :801-804.
4. **"Conservative min-out" is fed the expected output**, `arb-engine/src/lib.rs:370` — the profit floor guard is not conservative; `hop_inputs` computed at :1186-1192 is never used for it.
5. **Fixture `flash_loan_fee_sf = 0` everywhere in engine tests** (`arb-engine/src/lib.rs:1217,1363`; `arb-engine/tests/pipeline.rs:71`; `bellman_ford.rs:789`) — the flash-fee path is effectively untested outside arb-math's own unit tests.
6. **`evaluate_full` omits Phase-E tip checks** present only in `evaluate_flash_loan` (`arb-risk/src/lib.rs:450-475` vs :542-546); and the supervisor lets `BYPASS_RISK_GATE` skip everything but the kill switch (`supervisor/run.rs:1024-1038`).
7. **Priority-fee escalation double-counts**: caller cap (20% of gross via `cap_cu_price_by_profit`, `supervisor/run.rs:656`, `PRIORITY_FEE_MAX_SHARE_BPS = 2_000`, `supervisor.rs:47`) is undone inside the tx assembler, which re-adds `min(gross/25_000, 500_000)` (`arb-tx/src/lib.rs:107, :2893-2898, :3076-3081`) — the "uncapped gross/25_000" inflation is bounded at 500k micro-lamports/CU but is applied **on top of** the caller's cap and on **gross** profit.
8. **"Authoritative" converters disagree**: floor (`economic.rs:258`) vs ceil (`dimensional.rs:323`) for the same base→lamports direction; liability conversion floored in `currency.rs:245`.
9. **Warnings without enforcement**: two same-named `TokenAmount` types; decimals zeroed in `cycle_profit_at` (`arb-engine/src/lib.rs:209-210`); DLMM "price impact" includes fees; CLMM impact not reported at all; CPMM impact fee-free — three incompatible slippage semantics.
10. Positive controls worth keeping: ceil for liabilities everywhere in the authoritative path, signed i128 PnL (no saturation into TokenAmount for losses), checked/`Option` returns almost everywhere, CLMM/DLMM tick/bin traversal with explicit missing-array errors (`MissingTickArrayCoverage`), and the DLMM CollectFeeMode/rounding matrix mirrored from the SDK with tests (`dlmm.rs:103-110, 1419-1446`).

## 12. Transfer-fee bug — exact mechanics, blast radius, verification detail

`apply_transfer_fee_ceil(amount, fee_bps, maximum_fee)` returns **amount − fee** (post-fee), not the fee:

- arb-math/src/lib.rs:69-77 — `raw_fee = (amount as u128 * bps + 9_999) / 10_000` (ceil, :73-74); `capped_fee = min(raw_fee, maximum_fee)` (:75); `amount.saturating_sub(capped_fee)` (:76).

Caller misreading (meteora_quote_engine.rs:118-123):

```
let fee = apply_transfer_fee_ceil(amount_in, pool.transfer_fee_bps, u64::MAX); // = amount_in − actual_fee
amount_in.saturating_sub(fee)                                                 // = actual_fee
```

Worked example, `amount_in = 1_000_000`, `transfer_fee_bps = 100` (1%): `apply_transfer_fee_ceil` returns `990_000`; `effective_amount_in = 1_000_000 − 990_000 = 10_000`. The quote is priced on **1%** of the intended input. Worse, for `bps > 0` but tiny, `effective ≈ fee ≈` a few units; for `bps = 10_000` (100%): the function returns `0`, so `effective = 1_000_000` — the error flips sign at exactly 50% fee: below that input is understated, above it input is overstated.

Outbound leg identical (:134-139): reported `quote.amount_out` is fed into `apply_transfer_fee_ceil` treated as gross-then-subtracted, so `effective_amount_out` = fee; `hop2_final_amount_out` in `TwoHopDlmmQuote` (:30) is the fee, not the proceeds; `gross_profit = hop2_out − amount_in` (:158) compares fee vs full input → virtually always negative → routes silently dropped (fail-closed, but for the wrong reason), with `bins_crossed`/`end_active_id` telemetry intact.

Additional defects on the same lines of the same file:
- `collect_fee_mode: 0` hardcoded (meteora_quote_engine.rs:72) — `OnlyY` (mode 1) DLMM pools are priced as input-fee pools, violating the SDK getFeeMode semantics modeled correctly in `dlmm_fee_on_input` (dlmm.rs:103-110). For X→Y + OnlyY the fee is on the **output**; this path treats it as input.
- `price_q64: 0` for every bin (meteora_quote_engine.rs:90) — re-derives price per bin via `get_price_from_id`; fine, but the field is dead weight and each call re-powers `pow_q64` (19 iterations).
- `u64::try_from(b.amount_x).ok()?` (:88) — fail-closed truncation of u128 reserves > u64::MAX ✓.

Blast radius (all live call sites of the mis-priced functions):
- arb-engine/src/meteora_size_optimizer.rs:230-233 — size selection for the DLMM cross-pool strategy.
- arb-engine/src/three_leg_meme_pipeline.rs:227-229 — 3-hop WSOL→meme→meme→WSOL pipeline.
- arb-sim/src/meteora_fresh_pipeline.rs:401 — freshness-validated simulation of the strategy.
- arb-sim/src/opportunity_recorder.rs:117-122 — records mis-priced quotes into the opportunity DB.

All require `pool.transfer_fee_bps > 0` to misfire, and every test fixture sets `transfer_fee_bps: 0` (tests/meteora_cross_pool_vertical_slice.rs:102, tests/three_leg_meme_pipeline_tests.rs:129). The bug is therefore latent in CI and live whenever a Token-2022-transfer-fee pool is ingested — the one class of pool this code path exists to price.

Independent confirmation of the intended semantics: the codebase's own doc for `apply_transfer_fee_ceil` at arb-math/src/lib.rs:68 — "Apply Token-2022 transfer fee deduction with ceiling rounding and max-fee cap" — the deduction is applied inside; the arb-dex call sites (arb-dex/src/raydium_cpmm.rs:297-315, raydium_clmm.rs:351-367, orca_whirlpool.rs:324-341, meteora_damm_v2.rs:226-249, meteora_dlmm.rs:426-475, raydium_launchlab.rs:215-240) all consume the net value directly and show the intended contract. Note arb-dex adapters also pass `maximum_fee = u64::MAX` — they model bps-only fees and never consult the on-chain `maximum_fee` cap or the older/newer epoch schedule (`active_transfer_fee`, arb-dex/src/lib.rs:112-118), i.e. even the "correct" sites overstate the fee on token amounts exceeding the cap.

## 13. Is the fee taken up correctly? — on-chain conformance notes (read-only)

Raydium AMM v4 / Raydium CPMM on-chain behavior (documented in cpmm.rs:36-40, :170-188): fee taken from input, ceiling division; out `floor(net_dx*y/(x+net_dx))`. `cpmm_out_upfront_fee_ceil` (cpmm.rs:41-63) implements exactly this; generic `cpmm_out_fee` (:73-100) implements the equivalent γ-in-numerator form; both agree when γ = D·(1−f). Raydium CPMM fee denominator 1_000_000 (PPM) documented in `cpmm_fee_paid_with_denom` (:180-183) ✓.

Orca Whirlpool / Raydium CLMM on-chain: full tick computes `fee = ceil(amount_in_max × rate / (1_000_000 − rate))`; partial steps use `ceil(remaining × rate / 1_000_000)` — matches (clmm.rs:464-493, :480-484, :518-522). Directional rounding of Δ amounts matches Whirlpool (ceil on needed input, floor on produced output). Sqrt-price from A input is ceiling, from B input floor — matches Whirlpool `getNextSqrtPriceFromInput`.

Meteora DLMM on-chain (`lb_clmm`): base+variable fee in 1e9 precision, max 10%, variable fee `ceil(vfc × (va × bin_step)² / 1e11)` — matches the published SDK `getVariableFee`. Fee-on-input drain via `ceil(out × rate / (1e9 − rate))` (`compute_fee_to_add` :439-456) and `included_fee_amount` (:464-483) — matches SDK `getIncludedFeeAmount`. Protocol fee `floor` (:505-513) — consistent with integer truncation in the SDK.

Known conformance risks NOT verified against chain (flagged, not asserted):
- `get_price_from_id` base-fee `pow_q64` inverts via `u128::MAX / squared_base` (dlmm.rs:241-245) which is MAX/base rather than 2^128/base — introduces up-to-2-ulp Q64.64 error for base ≥ 1.
- `isqrt_u384/512/768` (cpmm.rs:189-231) Newton-Raphson floor sqrt; converges for all inputs, but no formal termination proof; `val <= 1` early-returned ✓.
- Tick ↔ sqrt-price roundtrip has ±1 tick tolerance acknowledged in-test (clmm.rs:1230-1240) — a quote can cross one tick boundary differently than on-chain; zero-liquidity edge ticks are traversable with no liquidity change.
- `bin_array_index_for_bin` hardcodes 70 bins/array (dlmm.rs:574-576) — matches current Meteora DLMM layout; not validated against the on-chain IDL in this repo.

## 14. Exact expressions appendix (rounding / width / domain)

| # | Function (file:line) | Exact expression | Rounding | Width | Domain guard |
|---|---|---|---|---|---|
| 1 | `cpmm_out_fee` cpmm.rs:89-98 | out = ⌊γ·dx·y / (D·x + γ·dx)⌋ | floor | U256 in, u128 out | dx,x,y ≠ 0; γ ≠ 0; out ≤ u128::MAX |
| 2 | `cpmm_out_upfront_fee_ceil` cpmm.rs:54-62 | fee = ⌈dx·fn/fd⌉; out = ⌊(dx−fee)·y/(x+dx−fee)⌋ | ceil fee / floor out | U256 | fn < fd; fee < dx else 0 |
| 3 | `cpmm_fee_paid_with_denom` cpmm.rs:187 | fee = ⌈in·rate/den⌉ | ceil | u128 | no rate < den check |
| 4 | `cpmm_price_impact_bps` cpmm.rs:159-167 | impact = ⌊(dx·y − o·x)·10_000/(dx·y)⌋, o fee-free | floor | U256 | inputs ≠ 0 → Bps(10000) |
| 5 | `optimal_two_pool` cpmm.rs:265-293 | d* = (isqrt(N·D0) − D0)/D1; N=γ1γ2y1x2; D0=D²x1y2; D1=D·γ1·y2+γ1γ2y1 | floor twice | U384 | N > D0 else None |
| 6 | `optimal_three_pool` cpmm.rs:335-364 | analogous; D0,3=D³x1y2z3; radicand ≤ ~441 bits | floor | U512 | N3 > D0,3 |
| 7 | `is_past_marginal_degradation_ceiling_two_pool` cpmm.rs:488-493 | past ⇔ N·D0·D ≤ (D0+D1·Δx)²·(D+flash_fee_bps) | n/a | U384 | n ≤ d0 ⇒ true |
| 8 | `marginal_profit_derivative_cpmm` cpmm.rs:526-532 | dPi/dx ≈ (N·D0 − (1+fee_bps/D)·(D0+D1Δx)²)/(D0+D1Δx)², ×1e9 | floor div | U384 | denom_sq == 0 → i128::MIN |
| 9 | `get_amount_delta_a` clmm.rs:239-255 | ΔA = L·Δp·2^64/(p_l·p_u) | ceil in / floor out | U256 | p==0 or L==0 → 0 |
| 10 | `get_amount_delta_b` clmm.rs:275-285 | ΔB = L·Δp/2^64 | ceil in / floor out | U256 | same |
| 11 | `get_next_sqrt_price_from_a_input` clmm.rs:401-404 | p′ = ⌈(L·p·2^64)/((L·2^64)+dx·p)⌉ | ceil | U256 | dx == 0 → p |
| 12 | `get_next_sqrt_price_from_b_input` clmm.rs:427 | p′ = p + ⌊dy·2^64/L⌋ | floor | U256 | L == 0 → DivisionByZero |
| 13 | full-tick fee clmm.rs:489-493 | fee = ⌈in_max·r/(1e6 − r)⌉; in = in_max + fee | ceil | u64/u256 | r < 1e6 (saturating_sub swallows r ≥ 1e6) |
| 14 | partial-tick fee clmm.rs:480-484 | fee = ⌈remaining·r/1e6⌉ | ceil | u128 | — |
| 15 | `compute_base_fee` dlmm.rs:372-389 | bf·bs·10·10^pf | exact | u128 checked | pf ≤ u32 |
| 16 | `compute_variable_fee` dlmm.rs:404-422 | ⌈vfc·(va·bs)²/1e11⌉ | ceil | U256 | vfc==0 or va==0 → 0 |
| 17 | `compute_total_fee_rate` dlmm.rs:425-434 | (base+var) ∧ 1e8 | min-clamp | u128 checked | add overflow → Err |
| 18 | `compute_protocol_fee` dlmm.rs:505-513 | ⌊fee·share/10_000⌋ | floor | U256 | share==0 → 0 |
| 19 | drain fee dlmm.rs:685-689 | gross_in_max = net_in_max + ⌈net_in_max·rate/(1e9−rate)⌉ | ceil | U256 | rate < 1e9 (checked_sub) |
| 20 | `calculate_flash_fee_raw` economic.rs:466-475 | fee = ⌈borrow·sf/2^64⌉ | ceil | U256 | sf == 0 → 0 |
| 21 | `lamports_to_base_raw` economic.rs:199-218 | raw = ⌈lamports·rate_num/rate_den⌉ | ceil | U256 | rate_num/den ≠ 0 |
| 22 | `base_raw_to_lamports` economic.rs:244-265 | lam = ⌊raw·rate_den/rate_num⌋ | floor | U256 | rate_num ≠ 0 |
| 23 | `calculate_net_profit_raw` economic.rs:429-452 | net = (final−borrow) − Σcosts; Err on negative | n/a | U256 | borrow > final → Err(Overflow) |
| 24 | `build_economic_quote` costs economic.rs:333-341 | TC = Σ(fee-ish + network/priority/tip converted ceil) | — | U256 checked | each checked_add |
| 25 | `positive_projection` economic.rs:230-237 | max(0, signed) via u64::try_from | sat at 0 | i128→u64 | loss ⇒ 0 |
| 26 | `graduation_progress_bps` arb-types lib.rs:859-863 | bps = ⌊real_sol·10_000/PUMP_FUN_GRADUATION_LAMPORTS⌋ ∧ 10_000 | floor | u128 | — |
| 27 | `TipFloor::at`/`lerp` arb-types lib.rs:1224-1244 | P65 = p50 + (p75−p50)·3/5; P85 = p75 + (p95−p75)/2; P90 = p75 + (p95−p75)·3/4 | floor | u128 | den ≥ 1 |
| 28 | `net_bps` (Currency report) currency.rs:718-722 | bps = ⌊net × 10_000 / initial_raw⌋ (i64) | trunc to zero | i64 | initial == 0 → 0 |
| 29 | `net_bps` (SizedOpportunity) arb-types lib.rs:663-678 | f64: (n × 10_000)/notional, clamp [0, 65535] | float | f64 | notional ≤ 0 → 0 |
| 30 | `apply_transfer_fee_ceil` arb-math/src/lib.rs:69-77 | fee = ⌈amt·bps/10_000⌉ ∧ max_fee; net = amt − fee | ceil fee | u128 | bps == 0 → amt |
| 31 | repay sum arb-tx/src/lib.rs:3186-3204 | repay = borrow + ⌈borrow·sf/2^64⌉ | ceil | u64 saturating_add | fee ≤ borrow |
| 32 | `cap_cu_price_by_profit` arb-tx/src/lib.rs:166-178 | max_price = ⌊(profit·bps/10_000)·1e6/cu_limit⌋; price' = min(price, max_price) | floor | u64 | cu_limit == 0 → 0 |

## 15. Overflow bounds actually provable in-code

- `cpmm_out_fee` U256 mul overflow requires reserves ≳ 3.4e36 raw each (unrealistic: total supply of any mint is ≤ 2^64 per account; pools hold ≤ u64 sums). Safe in practice; unguarded in principle.
- `optimal_two_pool` U384 overflow requires r ≳ 2^82 (~4.9e24 raw units per side) — unreachable with real supplies.
- `optimal_three_pool` U512 — same reasoning. `optimal_four_pool` U768 ("radicand ≤ 618 bits", cpmm.rs:377): safe ≤ 2^148 reserves.
- CLMM `l * delta_p << 64` (clmm.rs:241): latent wrap for absurd liquidity; unreachable on-chain.
- DLMM `pow_q64` rejects exp_abs ≥ 2^19 and zero results → price never 0 or NaN; `get_price_from_id` domain-checked (bin id range).
- `build_economic_quote` wraps total costs with checked_add and rejects overflow (economic.rs:333-341); signed_difference bounded: |signed| ≤ 3.4e38 but i128::MAX ≈ 1.7e38 ⇒ `i128::try_from(lhs−rhs)` (:222) can fail for differences in (1.7e38, 3.4e38] → Err(Overflow) on absurd losses — fail-closed, acceptable.
- `calculate_net_profit_raw` cannot represent a loss at all — returns Err(Overflow) instead of a negative number (economic.rs:451); supervisor's `.ok().filter(|n| *n > 0)` (run.rs:819-820) treats Err as rejection — intended, but the taxonomy conflates "loss" with "overflow"; monitoring will misclassify every losing route as an arithmetic error.
- `positive_projection` clamps any ≤ 0 result to 0 (economic.rs:231-233) — `net_profit_raw` loses sign; only `minimum_net_profit_signed_raw` carries the truth.

## 16. Final checklist against the task's 11 questions

1. Inventory — §1 (module-by-module, signature + units).
2. Spot/effective price + impact — §2: no explicit spot/effective price functions; three incompatible impact semantics; formulas, rounding and clamping documented with file:line.
3. Fee math — §3: cpmm_fee_paid ceil vs γ-form quote floor; CLMM fee-on-top denominator (1e6 − rate) correct; DLMM ceil fees, protocol fee floor; DAMM v2 ceil; LaunchLab ceil; CPMM impact fee-free while DLMM impact fee-inclusive; `gamma > D` accepted unchecked (cpmm.rs:80); fee > 65535 bps wraps `as u16` (arb-engine/lib.rs:431); DLMM MAX_FEE_RATE clamp ✓; protocol fee floor asymmetry.
4. Token-2022 double-count — §4/§12: verified exactly; correct alternative in arb-dex; engine fee-blind (adapters.rs:205-209); sim denominates transfer fees in lamports (profitability_gate.rs:42).
5. Profit path + unit mixing — §5: WSOL borrowed in → swaps → repay principal + ceil flash fee ✓ (repay wiring exists, arb-tx kamino.rs:170-179, template.rs:283-286); `flash_fee_raw` (base raw) subtracted from lamport `gross` — run.rs:676-679; `Lamports(real_net_base_raw)` — run.rs:1048/1059; raw-fee flash in size_optimizer (bellman_ford.rs:479,567,626 `.unwrap_or(0)`); all occurrences listed in §5.
6. Decimals — §6: u8 from verified mint accounts (EconomicConfig.base_decimals ≤ 18); hardcoded zeros in cycle_profit_at (arb-engine/lib.rs:209-210); cross-mint subtraction guarded only outside live paths; two converters disagree on rounding.
7. Profit bps / div-zero — §7: bps calcs listed; all div-by-zero guarded except `out1==0`/`out2==0` at arb-engine/lib.rs:630, 832, 856 (no zero check; division panics).
8. Flash repayment + sf=0 — §8: exact formula `ceil(borrow × sf / 2^64)`, duplicated (economic.rs:460-476, kamino.rs:170-179); sf = 0 in engine/pipeline/bellman_ford fixtures ⇒ flash fee untested there; mainnet sf 11_529_215_046_068 → 625 lamports/SOL (economic.rs:530-537).
9. Priority fee / tip caps — §9: gross/25_000 capped at 500_000 micro-lamports (arb-tx/src/lib.rs:107) but re-added inside assemblers on top of the caller's 20%-share cap (arb-tx/src/lib.rs:2893-2898, :3076-3081) — double-counted escalation, driven by gross; arb-risk tip-cap checks only in `evaluate_flash_loan` (arb-risk/src/lib.rs:536-546), absent in `evaluate_full` (:450-475); `pick_tip` floor/cap inversion bug (arb-math/lib.rs:93); BYPASS_RISK_GATE (supervisor/run.rs:1024-1038).
10. Floats on monetary paths — §10: chronos/latency ranking/timing-only; `golden_section_size` and DFS pruning are f64 sizing (arb-engine/lib.rs:245-268, 1043-1063); `UsdMicros::from_atomic`/`net_bps` monetized floats (arb-types/src/lib.rs:493-496, 663-678); authoritative solvency check stays integer ✓; arb-land `pick_dynamic_tip` f64 (arb-land/src/lib.rs:136-141).
11. Truncating casts — §11: inventory; worst: u16 pool-fee wrap (arb-engine/lib.rs:431), i64 wrap in checked_signed_diff (currency.rs:393), u128→u64 `as` on profit path (arb-engine/lib.rs:1154, 1191, 657, 894, 630, 832, 856), f64→u64 sizing truncation (arb-engine/lib.rs:255-256, 267), `unwrap_or(u64::MAX)` silent clamps (dlmm.rs:321, 329, 1159, 1164; damm_v2.rs n/a), unguarded U256 multiplies (cpmm.rs:89, clmm.rs:241), div-by-zero panic sites (arb-engine/lib.rs:630, 832, 856).

## 17. Method notes & limitations

- Read-only; no code compiled or run; no fixtures executed. Line numbers verified against working tree at D:\work_new (HEAD unchanged during audit).
- On-chain program bytecode semantics were referenced from in-repo documentation comments (e.g. cpmm.rs:36-40, clmm.rs:1-8, dlmm.rs header), not independently re-verified against deployed programs; conformance claims are therefore "matches the documented on-chain formulas".
- `arb-sim`, `arb-tx`, `arb-risk`, `arb-bot` files were read only to trace the profit path and tip/priority-fee handling; they are not independently audited line-by-line beyond the claims above.
- `arb-types::dimensional::TokenAmount` and `arb_types::TokenAmount` are distinct types both exported; `dimensions` module is not `pub use`d at crate root (only `currency::*` is), so `dimensional::TokenAmount` is reachable as `arb_types::dimensional::TokenAmount`.
- Minor: `MAX_F64_INPUT` (chronos.rs:27) is dead code; `EPSILON_CYCLE_SPREAD_BPS` (arb-engine/src/lib.rs:59) — check usage: defined, referenced nowhere in lib.rs except its own definition; verify before assuming it gates anything.
- `score_opportunity_chronos` is `#[cfg(test)]`-only (arb-engine/src/lib.rs:1230 and the `mod tests` wrapper at :1211) — not reachable in production; f64 usage there is bench/test-only and was included for completeness.
- `EPSILON_CYCLE_SPREAD_BPS` (arb-engine/src/lib.rs:59) is used by `cross_pool_dlmm.rs:32,61` (`min_spread_bps`) — not dead code.
- `arb_math::calculate_net_profit` (arb-math/src/lib.rs:103) is never called anywhere else in the workspace — dead code; and it omits DEX/transfer fees and ATA rent entirely (subtracts only flash, priority, base, tip, ATA rent), so if it is ever wired in, it under-charges.
- `arb_math::calculate_net_profit` (Lamports version) vs `economic::calculate_net_profit_raw` vs `DimensionalProfitabilityReport::compute` — three net-profit implementations coexist; only `economic::*` is treated as authoritative by the engine; the other two are inconsistent subsets (borrowed from lib.rs, currency.rs).

## 18. Duplicate formula definitions — drift risk inventory

| Formula | Copy A | Copy B | Difference |
|---|---|---|---|
| Flash fee ceil(borrow·sf/2^64) | arb-math/src/economic.rs:460-476 (U256, Err on overflow) | arb-tx/src/kamino.rs:170-179 (u128, `min(u64::MAX)` clamp) | A uses U256 and errors on >u64 fee; B saturates. Same math, different failure mode; fee is verified ≤ borrow so both agree for sane inputs |
| Transfer-fee ceil + cap | arb-math/src/lib.rs:69-77 (net returned) | arb-dex/src/lib.rs:99-109 `calculate_transfer_fee` (fee returned) | One returns net, other returns fee — the exact confusion that caused the meteora_quote_engine bug; two opposite-semantics functions with adjacent names |
| Net profit | arb-math/src/lib.rs:103 (Lamports) | arb-math/src/economic.rs:399 (raw + signed) | A silently drops DEX/transfer fees; A returns None on loss; B errors on negative; B is used |
| Net profit | currency.rs `DimensionalProfitabilityReport` (:637-745) | economic.rs | Currency version subtracts only network fees, never flash/DEX/transfer — not used for execution gating |
| Fee paid ceil | cpmm.rs:183-188 | inline `(dx*ppm + 999_999)/1e6` at launchlab.rs:34,70 (PPM ceil) | Duplicated ceil pattern; consistent |
| isqrt | cpmm.rs:189-231 (three copies u384/u512/u768) | — | Mechanical triplication, no drift |
| min-profit floor | engine lib.rs:61 (30_000) | pump_fun_raydium.rs:14 (30_000) | Same constant, two definitions |
| 25_000 priority divisor | arb-tx/src/lib.rs:107 | — | Single definition; cap min(500_000) |
| Repay sum | kamino.rs:170-179 + call sites | template.rs:283-286 patch path | Both recompute `borrow + fee`; template writes LE bytes directly (template.rs:285-286) — same formula duplicated in the hot patch path |

## 19. Severity-ranked consolidated findings

Critical:
- C1 meteora_quote_engine.rs:118-123 & :134-139 — transfer-fee double subtraction; input collapses to the fee (§4, §12). Affects every Token-2022 pool quoted through the cross-pool engine and the three-leg meme pipeline.
- C2 arb-bot/src/supervisor/run.rs:676-679 — lamport gross minus base-raw flash fee; `t_max` tip ceiling corrupt for non-WSOL base (§5).
- C3 arb-engine/src/lib.rs:370 — min-out fed with expected output; conservative profit gate is non-conservative (§5).
- C4 arb-bot/src/supervisor/run.rs:1048/1059 — `Lamports(real_net_base_raw)` base-raw-in-lamports into the risk gate; `check_min_profit` compares incompatible scales (§5, §7).

High:
- H1 arb-engine/src/lib.rs:431 — `as u16` wrap of pool fee_bps for malicious fee_numerator (§3, §11).
- H2 arb-types/src/currency.rs:393 — u64→i64 wrap in `checked_signed_diff` (§11).
- H3 arb-engine/src/lib.rs:630/832/856 — division by `out1`/`out2` with no zero check → panic path; also u128→u64 `as` truncation (§7, §11).
- H4 Evaluation asymmetry in fees: `gamma > fee_denominator` accepted (cpmm.rs:80); `fee_rate ≥ 1e6` slips through `saturating_sub` in CLMM full-tick fee (clmm.rs:488) instead of erroring; DLMM protocol fee floors while trading fees ceil (dlmm.rs:509-513) (§3).
- H5 Priority-fee escalation re-applied inside the tx assembler on top of the caller's 20%-share cap, driven by gross profit (arb-tx/src/lib.rs:107, :2893-2898, :3076-3081; supervisor/run.rs:656) (§9).
- H6 Tip cap checks missing from `evaluate_full` (`check_tip_spend_ratio`, `check_max_inflight_bundles`, `check_min_landing_rate`); BYPASS_RISK_GATE env kill-switch (arb-risk/src/lib.rs:450-475 vs :517-549; supervisor/run.rs:1024-1038) (§9).

Medium:
- M1 sf=0 fixtures in engine tests leave the flash-fee path unpriced in CI (arb-engine/src/lib.rs:1217, :1363; tests/pipeline.rs:71; bellman_ford.rs:789) (§8).
- M2 Converter rounding disagreement: economic floor (:258) vs dimensional ceil (:323); liability conversion floored in currency.rs:245 (§6).
- M3 `pick_tip` floor/cap inversion — floor silently unenforceable when floor > cap (arb-math/src/lib.rs:93) (§9).
- M4 `collect_fee_mode: 0` hardcoded in meteora_quote_engine.rs:72 — OnlyY pools mispriced; also modeled incorrectly versus `dlmm_fee_on_input` semantics (§12).
- M5 `cycle_profit_at` zeroes decimals (arb-engine/src/lib.rs:209-210); two `TokenAmount` types; authoritative converters duplicated (§6).
- M6 DLMM reported "price impact" includes swap fees (dlmm.rs:1146-1175) — overstated impact; CPMM impact is fee-free (cpmm.rs:143-146); CLMM impact not surfaced — inconsistent slippage semantics across protocols (§2, §3).
- M7 `calculate_net_profit_raw` conflates loss with Overflow error (economic.rs:451); `expect_queue`/monitoring misclassifies losing routes as arithmetic faults (§5, §15).
- M8 `quote_clmm` never reports price impact bps; `quote_route` compounds impacts with 1e-5 granularity (adapters.rs:284-291) and truncates `compounded / 100_000` (:291) (§2).
- M9 Transfer fee modeled bps-only everywhere; `maximum_fee` cap and epoch-scheduled rates never applied in quotes (arb-dex call sites pass u64::MAX as cap; `active_transfer_fee` unused in pricing) (§4, §12).

Low:
- L1 `MAX_F64_INPUT` dead constant (chronos.rs:27); `sqrt_price_next_x64` duplicated field (clmm.rs:439); `DEFAULT_FLASH_LOAN_FEE_BPS = 30` hardcoded legacy default (pump_fun_raydium.rs:12) inconsistent with the verified-sf design.
- L2 `graduation_progress_bps` floors (fine) but mixes a wall-clock progress metric into math crate API surface (arb-types lib.rs:859).
- L3 `bin_array_index_for_bin(…).div_euclid(70)` magic constant (dlmm.rs:574).
- L4 Two `MIN_NET_PROFIT_LAMPORTS` constants (engine lib.rs:61, arb-math pump_fun_raydium.rs:14) — drift hazard.
- L5 `is_past_marginal_degradation_ceiling_two_pool` branch inside `clamp_at_marginal_ceiling_two_pool` is unreachable (`clamped > optimal` impossible after `min`) (cpmm.rs:557-572).
- L6 `check_slippage_bound`, `check_constant_product_invariant` test-only; production price sanity relies on quote math alone.
- L7 `is_profitable` requires `minimum_net_profit_signed_raw > 0` strictly — a zero-cost, zero-profit route is rejected even at `min_profit_raw = 0` (economic.rs:191-194).
- L8 `Binance/…` n/a. `apply_transfer_fee_ceil` saturation `unwrap_or(amount)` on overflow (arb-math lib.rs:75) hides cast failure by refunding the whole amount (user-favored but silent).
- L9 `UsdMicros::from_atomic` f64 rounding (arb-types lib.rs:493-496) — ±1 micro-USD display/P&L drift.
- L10 `Score/prune` f64 thresholds at 1.0010 (10 bps) — path pruning sensitive to float error near the boundary (arb-engine/src/lib.rs:1051, 1085).

No changes were made to any file outside `audit_work/02_MATH_ACCOUNTING.md`.
