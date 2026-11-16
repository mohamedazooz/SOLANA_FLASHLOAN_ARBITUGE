Comprehensive Audit of arb-dex, arb-math, arb-types, and arb-constants

This audit examines the domain types, protocol constants, DEX adapters, mathematical models, and protocol coverage across the Solana arbitrage bot workspace.

1. DexProtocol and PoolKind in arb-types
1.1 DexProtocol Variants (crates/arb-types/src/lib.rs:218-234)

DexProtocol is an enum representing each supported Solana DEX engine. Every variant corresponds to a unique on-chain program ID, account binary format, fee model, and pricing curve:

DexProtocol::RaydiumV4
Program ID: 675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8 (arb-constants::RAYDIUM_V4_PROGRAM_ID)
Curve / Family: Constant-Product AMM (PoolKind::Cpmm).
Characteristics: Legacy Raydium AMM requiring 18 accounts in its SwapBaseIn instruction (RaydiumV4Accounts), tightly integrated with OpenBook/Serum DEX orderbook accounts (open_orders, serum_market, bids, asks, event queue).
DexProtocol::RaydiumCpmm
Program ID: CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C (arb-constants::RAYDIUM_CPMM_PROGRAM_ID)
Curve / Family: Constant-Product AMM (PoolKind::Cpmm).
Characteristics: Raydium CP-Swap program built with Anchor. Uses a 1e6 (PPM / parts per million) fee denominator on-chain (swap_base_input discriminator: [143, 190, 90, 218, 196, 30, 51, 222]). 13 accounts required for swaps.
DexProtocol::RaydiumClmm
Program ID: CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK (arb-constants::RAYDIUM_CLMM_PROGRAM_ID)
Curve / Family: Concentrated Liquidity AMM (PoolKind::Clmm).
Characteristics: Raydium Concentrated Liquidity program. Uses Q64.64 sqrt-price ticks, tick arrays, observation state PDA, and pool state. 15 accounts required for swaps.
DexProtocol::OrcaWhirlpool
Program ID: whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc (arb-constants::ORCA_WHIRLPOOL_PROGRAM_ID)
Curve / Family: Concentrated Liquidity AMM (PoolKind::Clmm).
Characteristics: Orca Whirlpools CLMM. Uses 88 ticks per TickArray account, native fee rate in hundredths of a bip (1e6 denominator), Q64.64 sqrt-prices. 11 accounts required for swaps.
DexProtocol::MeteoraDlmm
Program ID: LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo (arb-constants::METEORA_DLMM_PROGRAM_ID)
Curve / Family: Dynamic Liquidity Bin Ladder (PoolKind::Dlmm).
Characteristics: Discrete bin pricing where price in bin $i$ is $P_i = (1 + \text{bin_step})^i$. Holds 70 bins per BinArray (METEORA_BINS_PER_BIN_ARRAY = 70). Dynamic fees dynamically scale using a volatility accumulator. 15 accounts required for swap2 instruction (METEORA_SWAP2_DISCRIMINATOR).
DexProtocol::PumpFun
Program ID: 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P (arb-constants::PUMP_FUN_PROGRAM_ID)
Curve / Family: Constant-Product AMM (PoolKind::Cpmm).
Characteristics: Bonding curve with virtual token and virtual SOL reserves, capped by real reserves until graduation at 85 SOL (PUMP_FUN_GRADUATION_LAMPORTS = 85_000_000_000). Interacts directly with native SOL lamports. 12 accounts required for buy/sell instructions.
DexProtocol::PumpSwap
Program ID: pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA (arb-constants::PUMP_SWAP_PROGRAM_ID)
Curve / Family: Constant-Product AMM (PoolKind::Cpmm).
Characteristics: Pump.fun post-graduation AMM (ADyA8hde... GlobalConfig). Features signed virtual_quote_reserves, 3-tier PPM fees (LP, protocol, creator/holder), and un-swept fee buckets. 16 accounts required for swaps.
1.2 DexProtocol::family() Mapping (crates/arb-types/src/lib.rs:241-248)
rust
pub const fn family(self) -> PoolKind {
    match self {
        Self::RaydiumV4 | Self::RaydiumCpmm | Self::PumpFun | Self::PumpSwap => PoolKind::Cpmm,
        Self::RaydiumClmm | Self::OrcaWhirlpool => PoolKind::Clmm,
        Self::MeteoraDlmm => PoolKind::Dlmm,
    }
}
1.3 PoolKind Variants (crates/arb-types/src/lib.rs:318-338)

PoolKind categorizes AMM mathematical curves to prevent cross-family quotation bugs (e.g. quoting a CLMM or DLMM pool using constant-product formulas overstates depth by 5–100x):

PoolKind::Cpmm (#[default]): Constant product $x \cdot y = k$. Used for Raydium AMM v4, Raydium CPMM, Pump.fun, and PumpSwap. Label: "Constant Product".
PoolKind::Clmm: Concentrated liquidity with discrete ticks and Q64.64 $\sqrt{P}$. Used for Orca Whirlpool and Raydium CLMM. Label: "Concentrated Liquidity".
PoolKind::Dlmm: Discrete bin ladder with geometric price progression $P_i = (1 + s)^i$ and zero slippage inside individual bins. Used for Meteora DLMM. Label: "Dynamic Liquidity".
2. Adapters Implemented in crates/arb-dex/src/
2.1 The AmmPool Trait (crates/arb-dex/src/traits.rs:6-80)

Defines the standard dynamic AMM interface:

Identity & Metadata: pool_id(), protocol(), base_mint(), quote_mint(), decimals() (default (0, 0)), kind(), has_risky_token() (default false).
Reserves & Fees: reserves() -> (u128, u128), fee_bps() -> Option<u16>.
Specialized State: clmm_state(), dlmm_state(), pump_fun_state() (all default to None).
Mutation & Quoting: update_from_account(&mut self, data: &[u8]) -> Result<()>, quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64>.
2.2 RaydiumV4Pool (crates/arb-dex/src/raydium_v4.rs)
Account Byte Parsing (raydium_v4.rs:124-174):
Validates minimum account size RAYDIUM_V4_LAYOUT_SIZE = 752 bytes.
Offsets:
176..184: fee_numerator (u64 LE)
184..192: fee_denominator (u64 LE)
336..368: base_vault (Pubkey)
368..400: quote_vault (Pubkey)
400..432: base_mint (Pubkey)
432..464: quote_mint (Pubkey)
496..528: open_orders (Pubkey)
528..560: target_orders (Pubkey)
560..592: serum_market (Pubkey)
Vault Reserves / Tick Arrays:
Incomplete: update_from_account does NOT parse pool reserves from the pool account bytes (reserves reside in the SPL token vaults and Serum OpenOrders accounts).
Reserves default to 0 on construction and must be populated externally via update_reserves(base_reserve, quote_reserve) (raydium_v4.rs:53-56).
Does NOT track OpenBook open_orders balances (total_coin_deposits, total_pc_deposits), causing discrepancy against on-chain Raydium AMM pools that have active OpenBook orders.
Fee Handling:
Static fees: fee_bps() = (fee_numerator * 10_000) / fee_denominator (raydium_v4.rs:113-122).
Token-2022 Transfer Fees:
Not handled or inspected.
Quote Calculation (raydium_v4.rs:176-195):
Fails if reserves are 0 or fee_bps is invalid.
Computes gamma = 10_000.saturating_sub(fee_bps).
Calls arb_math::cpmm_out(amount_in as u128, r_in, r_out, gamma): $$\Delta y = \frac{\gamma \cdot \Delta x \cdot y}{10{,}000 \cdot x + \gamma \cdot \Delta x}$$
2.3 OrcaWhirlpool (crates/arb-dex/src/orca_whirlpool.rs)
Account Byte Parsing (orca_whirlpool.rs:199-250):
Validates WHIRLPOOL_MIN_SIZE = 300 bytes.
Offsets:
41..43: tick_spacing (u16 LE)
45..47: fee_rate (u16 LE, in hundredths of a bip: $10{,}000 = 100\text{ bps} = 1%$)
49..65: liquidity (u128 LE)
65..81: sqrt_price_x64 (u128 LE)
81..85: tick_current_index (i32 LE)
101..133: token_mint_a (Pubkey)
133..165: token_vault_a (Pubkey)
181..213: token_mint_b (Pubkey)
213..245: token_vault_b (Pubkey)
Vault Reserves / Tick Arrays:
reserves(&self) (orca_whirlpool.rs:142-157): Synthesizes instantaneous virtual reserves: $$x_{\text{virt}} = \frac{L \cdot 2^{64}}{\sqrt{P}}, \quad y_{\text{virt}} = \frac{L \cdot \sqrt{P}}{2^{64}}$$
Tick arrays are NOT stored in the Whirlpool account. The adapter holds pub tick_arrays: Vec<TickArrayFacade> which defaults to empty.
External injection required via set_tick_arrays(Vec<TickArrayFacade>) (orca_whirlpool.rs:57-59).
Derives TickArray PDAs via derive_tick_array_pda (orca_whirlpool.rs:62-72) and tick_array_pdas_for_swap (orca_whirlpool.rs:78-101).
Fee Handling:
Static fee_rate in $10^{-6}$ precision (fee_bps() = fee_rate / 100). Passed directly to quote_clmm without BPS loss.
Token-2022 Transfer Fees:
Not handled.
Quote Calculation (orca_whirlpool.rs:252-280):
Fails if liquidity == 0, sqrt_price_x64 == 0, fee_rate == 0, or tick_arrays.is_empty().
Calls arb_math::clmm::quote_clmm: iterates across tick arrays, crossing initialized ticks via compute_swap and adjusting liquidity on tick crossings.
2.4 PumpFunBondingCurve (crates/arb-dex/src/pump_fun.rs)
Account Byte Parsing (pump_fun.rs:134-178):
Validates PUMP_FUN_BONDING_CURVE_SIZE = 83 bytes and discriminator [23, 183, 248, 55, 96, 216, 172, 96].
Offsets:
8..16: virtual_token_reserves (u64 LE)
16..24: virtual_sol_reserves (u64 LE)
24..32: real_token_reserves (u64 LE)
32..40: real_sol_reserves (u64 LE)
40..48: token_total_supply (u64 LE)
48: complete (bool)
49..81: creator (Pubkey)
81: is_mayhem_mode (bool)
82: is_cashback_coin (bool)
Vault Reserves / Capacity Tracking:
Reserves are self-contained in the bonding curve account.
reserves() returns (virtual_sol_reserves as u128, virtual_token_reserves as u128).
Real reserves enforce physical payout ceilings:
Buy ($a \to b$): output tokens cannot exceed real_token_reserves (pump_fun.rs:260-265).
Sell ($b \to a$): gross SOL output cannot exceed real_sol_reserves (pump_fun.rs:277-282).
Fee Handling:
Bonding curve account does not contain fee parameters.
fee_bps defaults to None. Must be explicitly configured using with_fee_bps(fee_bps) from the Pump Fees program. Missing fee schedule causes quote() to fail (pump_fun.rs:197-201).
Fee deduction:
Buy: fee deducted from SOL input: $\Delta x_{\text{net}} = \Delta x - \text{fee}$.
Sell: fee deducted from gross SOL output: $\Delta x_{\text{net}} = \Delta x_{\text{gross}} - \text{fee}$.
Token-2022 Transfer Fees:
Token mint decimals default to None and must be set via with_token_decimals(decimals) (standard SPL token, decimals = 6).
Quote Calculation (pump_fun.rs:205-300):
Fails if complete, is_mayhem_mode, or is_cashback_coin is true.
Quotes via arb_math::cpmm_out_fee(net_in, v_sol, v_token, 10_000, 10_000).
2.5 PumpSwapPool (crates/arb-dex/src/pump_swap.rs)
Account Byte Parsing (PumpSwapPoolHeader::decode, pump_swap.rs:46-116):
Validates minimum length 245 bytes and discriminator [241, 154, 109, 4, 17, 177, 109, 188].
Decodes: pool_bump, index, creator, base_mint, quote_mint, lp_mint, base_vault, quote_vault, lp_supply, coin_creator, is_mayhem_mode, is_cashback_coin.
Appended extension fields:
245..261: virtual_quote_reserves (i128 LE)
261..269: creator_fee_bps (u64 LE)
269: can_edit_creator_fee (bool)
270: is_holder_reward (bool)
271..279: protocol_fees (u64 LE)
279..287: creator_fees (u64 LE)
Vault Reserves:
Vault amounts are not stored in the pool account; they must be fetched from the two SPL token vault accounts.
PumpSwapPool::update_from_account unconditionally returns Err (pump_swap.rs:446-450).
Pricing reserve: effective_quote_reserve = raw_quote_vault + virtual_quote_reserves.
Payout reserve: real_quote_reserve = raw_quote_vault - (protocol_fees + creator_fees).
Fee Handling (PumpSwapFeeConfig, pump_swap.rs:131-210):
3-tier PPM dynamic fee schedule: lp_fee_ppm, protocol_fee_ppm, creator_or_holder_fee_ppm.
Supports market-cap tiered selection via from_live_tiers(market_cap, tiers) from on-chain FeeConfig.
Buy fees are deducted from input per component using ceiling division.
Sell fees are deducted from gross quote output per component using ceiling division.
Respects direction disable flags (bit 3 disables buys, bit 4 disables sells).
Token-2022 Transfer Fees:
Unhandled.
Quote Calculation (pump_swap.rs:319-417):
Uses arb_math::cpmm_out_fee against (base_reserve, quote_reserve) with ceiling fee deductions, clamped against real_quote_reserve.
2.6 Token-2022 Module (crates/arb-dex/src/lib.rs:15-141)

Provides standalone TLV parsing:

decode_transfer_fee_config(mint_data) (lib.rs:32-86): Reads TLV extension type 1 from offset 166.
calculate_transfer_fee(amount, schedule) (lib.rs:89-99): $\lceil \text{amount} \cdot \text{bps} / 10{,}000 \rceil$ capped by maximum_fee.
active_transfer_fee(config, current_epoch) (lib.rs:102-108): Chooses older vs newer schedule.
token2022_has_risky_extension(data) (lib.rs:121-141): Scans for non-zero TLV bytes after offset 166.
3. Math in crates/arb-math/src/ vs Connection to arb-dex
Math Module	Functions / Types Implemented in arb-math	Connected to arb-dex?	Where Connected / Notes
cpmm.rs	cpmm_out, cpmm_out_fee	YES	raydium_v4.rs:4, pump_fun.rs:9, pump_swap.rs:12
	quote_cpmm, quote_cpmm_full, cpmm_price_impact_bps	NO	Disconnected in arb-dex (partially used in arb-engine)
	cpmm_fee_paid, cpmm_fee_paid_with_denom	NO	Disconnected in arb-dex
	isqrt_u384, isqrt_u512, isqrt_u768	NO	Disconnected in arb-dex
	optimal_two_pool (U384 closed-form 2-pool sizing)	NO	Disconnected in arb-dex
	optimal_three_pool (U512 closed-form 3-pool sizing)	NO	Disconnected in arb-dex
	optimal_four_pool (U768 Möbius transformation 4-pool)	NO	Disconnected in arb-dex
	is_past_marginal_degradation_ceiling_two_pool	NO	Disconnected in arb-dex
	marginal_profit_derivative_cpmm, marginal_profit_derivative	NO	Disconnected in arb-dex
	clamp_at_marginal_ceiling_two_pool	NO	Disconnected in arb-dex
clmm.rs	TickArrayFacade, quote_clmm	YES	orca_whirlpool.rs:4
	tick_index_to_sqrt_price_x64, sqrt_price_x64_to_tick_index	NO	Disconnected in arb-dex
	get_amount_delta_a, get_amount_delta_b	NO	Disconnected in arb-dex
	clmm_max_delta_x_in_tick, clmm_max_delta_y_in_tick	NO	Disconnected in arb-dex
	clmm_active_tick_max_input, clmm_max_input_in_tick	NO	Disconnected in arb-dex
	compute_swap_step_checked, compute_swap_step	NO	Disconnected in arb-dex (called internally by quote_clmm)
	parse_tick_array_account (decodes 9988-byte account)	NO	Disconnected in arb-dex (OrcaWhirlpool does not parse tick arrays)
	find_next_tick_boundary, update_liquidity_on_cross	NO	Disconnected in arb-dex (called internally by quote_clmm)
	compute_swap	NO	Disconnected in arb-dex
dlmm.rs	pow_q64, get_price_from_id	NO	100% DISCONNECTED from arb-dex (0 imports)
	dlmm_active_bin_max_input, scale_dlmm_three_hop_input	NO	100% DISCONNECTED from arb-dex
	compute_base_fee, compute_variable_fee, compute_total_fee_rate	NO	100% DISCONNECTED from arb-dex
	compute_fee_to_add, compute_fee_from_amount, compute_protocol_fee	NO	100% DISCONNECTED from arb-dex
	update_references_with_decay, update_volatility_accumulator	NO	100% DISCONNECTED from arb-dex
	consume_bin_liquidity, advance_active_id	NO	100% DISCONNECTED from arb-dex
	quote_dlmm_multi_bin, quote_dlmm_view	NO	100% DISCONNECTED from arb-dex (only used in arb-engine)
pump_fun_raydium.rs	PumpFunParams, RaydiumParams structs	YES	pump_fun.rs:307, raydium_v4.rs:59 (type conversions only)
	calculate_optimal_trade, calculate_optimal_trade_integer	NO	Used only in unit tests (pump_fun.rs:469, raydium_v4.rs:202)
	pump_fun_marginal_price, pump_fun_buy_cost, raydium_sell_output	NO	Disconnected in arb-dex
	verify_marginal_decreasing	NO	Disconnected in arb-dex
economic.rs	CostLedger, CostEntry, CostScope, CostType	NO	100% DISCONNECTED from arb-dex (0 imports)
	EconomicConfig, EconomicQuote, build_economic_quote	NO	100% DISCONNECTED from arb-dex
	lamports_to_base_raw, base_raw_to_lamports	NO	100% DISCONNECTED from arb-dex
	calculate_flash_fee_raw, calculate_net_profit_raw	NO	100% DISCONNECTED from arb-dex
chronos.rs & latency.rs	ExecutionTimeline, RiskAdjustedScore, SlotClock, FirePlan	NO	100% DISCONNECTED from arb-dex (0 imports)
4. Missing Protocols, Incomplete Adapters, and Concrete Bugs
4.1 Missing Protocol Adapters in crates/arb-dex/src/
DexProtocol::RaydiumCpmm Adapter Missing:
DexProtocol::RaydiumCpmm exists in arb-types, and arb-constants defines RAYDIUM_CPMM_PROGRAM_ID and discriminators.
In crates/arb-dex/src/lib.rs:111-147, only a stub pub struct CpmmDecoder; exists with an empty constructor.
There is NO RaydiumCpmmPool implementing AmmPool.
DexProtocol::RaydiumClmm Adapter Missing:
DexProtocol::RaydiumClmm exists in arb-types, but has NO adapter struct or implementation in crates/arb-dex/src/.
Orca Whirlpool is implemented, but Raydium CLMM (which uses distinct pool states, reward configurations, and tick array layouts) is absent.
DexProtocol::MeteoraDlmm Adapter Missing:
DexProtocol::MeteoraDlmm exists in arb-types, and complete DLMM bin math is implemented in crates/arb-math/src/dlmm.rs.
However, crates/arb-dex/src/ contains NO Meteora DLMM adapter struct (MeteoraDlmmPool), and implements no AmmPool for DLMM. (DLMM quoting is only partially wired as a helper in crates/arb-engine/src/adapters.rs:497-564).
Meteora DAMM v1 & v2 (Dynamic AMM) Missing:
Neither arb-types::DexProtocol nor arb-dex contains variants or adapters for Meteora DAMM pools.
4.2 Incomplete Adapters
RaydiumV4Pool (crates/arb-dex/src/raydium_v4.rs):
Reserve Deserialization: update_from_account(&mut self, data: &[u8]) (raydium_v4.rs:124-174) parses pool addresses and fee fractions, but does NOT parse token reserves. base_reserve and quote_reserve remain 0.
OpenOrders Neglect: Raydium AMM v4 pools on-chain store significant liquidity in OpenBook/Serum orders (amm_open_orders account). The adapter never fetches or accounts for open_orders.total_coin_deposits or open_orders.total_pc_deposits.
Token-2022 Neglect: Does not check for Token-2022 mint extensions or apply transfer fees.
OrcaWhirlpool (crates/arb-dex/src/orca_whirlpool.rs):
Tick Array Fetching: update_from_account (orca_whirlpool.rs:199-250) parses only the 300-byte Whirlpool account. It cannot parse tick arrays (separate 9,988-byte accounts).
Default Quote Failure: self.tick_arrays defaults to empty, which causes quote() to fail (orca_whirlpool.rs:258-262) unless set_tick_arrays is manually called beforehand.
Virtual Reserve Exposure: reserves() returns virtual instantaneous reserves $(x_{\text{virt}}, y_{\text{virt}})$, not actual token reserves or active-tick depth.
PumpFunBondingCurve (crates/arb-dex/src/pump_fun.rs):
Fee Disconnect: The on-chain bonding curve account contains no fee information. fee_bps is None by default; calling quote() without calling with_fee_bps() fails.
Decimals Disconnect: token_decimals defaults to None; calling quote() without with_token_decimals() fails (pump_fun.rs:219-223).
PumpSwapPool (crates/arb-dex/src/pump_swap.rs):
Broken Trait Method: Calling AmmPool::update_from_account(&mut self, _data: &[u8]) unconditionally returns Err(ArbError::InvalidInput("PumpSwap requires a coherent Pool + two vault account update")) (pump_swap.rs:446-450).
Multi-Account Dependency: Cannot be instantiated or updated from the pool account alone; requires joining the pool header, base SPL vault balance, quote SPL vault balance, un-swept fee buckets, and external fee tiers.
4.3 Concrete Bugs
Bug 1: Inverted Buy Cost Math in pump_fun_raydium.rs
Location: crates/arb-math/src/pump_fun_raydium.rs:54-61 (pump_fun_buy_cost)
rust
pub fn pump_fun_buy_cost(reserve: u128, supply: u128, delta_t: u128) -> Option<u128> {
    if delta_t == 0 || reserve == 0 || supply == 0 {
        return Some(0);
    }
    let numerator = reserve.checked_mul(delta_t)?;
    let denominator = supply.checked_add(delta_t)?;
    Some(numerator / denominator)
}
Finding: In a bonding curve buy, tokens $\Delta T$ are bought out of the curve, reducing token reserves to $T - \Delta T$. Under constant product $R \cdot T = k$, the SOL required to purchase $\Delta T$ tokens is: $$\Delta R = \frac{R \cdot T}{T - \Delta T} - R = \frac{R \cdot \Delta T}{T - \Delta T}$$ Dividing by supply.checked_add(delta_t) calculates the SOL received when selling $\Delta T$ tokens to the curve ($\frac{R \cdot \Delta T}{T + \Delta T}$), NOT the buy cost. This causes pump_fun_buy_cost to severely underestimate the SOL cost to acquire tokens, resulting in phantom arbitrage profits.
Bug 2: Float Arithmetic in calculate_optimal_trade
Location: crates/arb-math/src/pump_fun_raydium.rs:137-164
rust
let a = ((ray_fee as f64) * (raydium.sol_reserve as f64) * (raydium.token_reserve as f64)).sqrt();
let b = ((loan_factor as f64) * (pump.reserve as f64) * (pump.supply as f64)).sqrt();
Finding: Directly violates workspace rules ("zero lossy float casts", integer-only execution). Casting large $u128$ reserve products ($> 2^{53}$) to f64 loses significant precision and produces discrepancies against the integer square root (isqrt_u384, isqrt_u512) used elsewhere in arb-math.
Bug 3: AmmPool::update_from_account Fails on PumpSwapPool
Location: crates/arb-dex/src/pump_swap.rs:446-450
rust
fn update_from_account(&mut self, _data: &[u8]) -> Result<()> {
    Err(ArbError::InvalidInput(
        "PumpSwap requires a coherent Pool + two vault account update".into(),
    ))
}
Finding: PumpSwapPool implements AmmPool, but its update_from_account trait method unconditionally returns an error. Any generic pipeline calling update_from_account on Box<dyn AmmPool> fails on PumpSwap pools.
Bug 4: Token-2022 Transfer Fees Completely Ignored in arb-dex Quotes
Location:
crates/arb-dex/src/raydium_v4.rs:176-195
crates/arb-dex/src/orca_whirlpool.rs:252-280
crates/arb-dex/src/pump_fun.rs:205-300
crates/arb-dex/src/pump_swap.rs:319-417
Finding: While crates/arb-dex/src/lib.rs:32-108 implements decode_transfer_fee_config and calculate_transfer_fee, NONE of the 4 adapters in arb-dex call these functions or deduct transfer fees in their quote() methods. If a Token-2022 mint with a transfer fee is quoted through any arb-dex adapter, the quote assumes 0% transfer fee, causing on-chain execution to fail due to insufficient output amounts.
Bug 5: Raydium V4 Fee Deduction Rounding Discrepancy
Location: crates/arb-dex/src/raydium_v4.rs:190-194
rust
let gamma = 10_000u128.saturating_sub(fee_bps as u128);
let Some(out) = cpmm_out(u128::from(amount_in), r_in, r_out, gamma) else { ... };
Finding: On-chain Raydium AMM v4 (SwapBaseIn) applies ceiling division to deduct swap fees upfront: $$\text{deducted_fee} = \left\lceil \frac{\text{amount_in} \cdot \text{fee_numerator}}{\text{fee_denominator}} \right\rceil$$ $$\text{actual_in} = \text{amount_in} - \text{deducted_fee}$$ $$\text{out} = \left\lfloor \frac{\text{actual_in} \cdot \text{reserve_out}}{\text{reserve_in} + \text{actual_in}} \right\rfloor$$ Using cpmm_out with $\gamma = 10{,}000 - \text{fee_bps}$ folds fee reduction into the single fraction $\frac{\gamma \cdot \Delta x \cdot y}{10{,}000 \cdot x + \gamma \cdot \Delta x}$, which produces an output 1 atomic unit higher than on-chain execution in several rounding edge cases, leading to slippage tolerance breaches.
Bug 6: OrcaWhirlpool::reserves() Overstates Active-Tick Depth
Location: crates/arb-dex/src/orca_whirlpool.rs:142-157
rust
let x_virt = x_quot.to::<u128>();
let y_virt = y_prod.to::<u128>();
(x_virt, y_virt)
Finding: OrcaWhirlpool::reserves() exposes virtual reserves $(x_{\text{virt}}, y_{\text{virt}})$ as if they were pool token reserves. Any routing or sizing logic treating these as real reserves assumes liquidity across $[0, \infty)$, overstating active-tick depth by up to 100x and generating phantom opportunities.