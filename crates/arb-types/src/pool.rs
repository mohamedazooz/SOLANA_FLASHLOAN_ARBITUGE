//! Pool snapshot/state, hop plans, quotes and execution opportunities.

use super::dex::*;
use super::error::{ArbError, Result};
use super::primitives::*;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fmt;

/// Signed USD value in micro-dollars (1 unit = 1e-6 USD). Negative means a loss.
///
/// This is the ONLY currency used for P&L, cost accounting and risk limits.
/// Token amounts must never be compared against it without a price conversion.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct UsdMicros(pub i64);

impl UsdMicros {
    pub const ZERO: Self = Self(0);

    /// One US dollar.
    pub const ONE: Self = Self(1_000_000);

    pub fn from_f64(usd: f64) -> Self {
        // Saturating conversion: values outside i64 micro-USD are clamped, never wrapped.
        let scaled = usd * 1_000_000.0;
        if scaled.is_nan() {
            return Self::ZERO;
        }
        if scaled >= i64::MAX as f64 {
            return Self(i64::MAX);
        }
        if scaled <= i64::MIN as f64 {
            return Self(i64::MIN);
        }
        #[allow(clippy::cast_possible_truncation)]
        let truncated = scaled as i64;
        Self(truncated)
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    pub fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    /// Convert an atomic token amount into USD using a per-whole-token price.
    ///
    /// `atomic` is in base units, `decimals` is the mint's decimals, `price_usd`
    /// is the value of one whole token.
    pub fn from_atomic(atomic: u64, decimals: u8, price_usd: f64) -> Self {
        let whole = atomic as f64 / 10f64.powi(i32::from(decimals));
        Self::from_f64(whole * price_usd)
    }
}

impl fmt::Display for UsdMicros {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.6}", self.as_f64())
    }
}

/// AMM family. Determines which quote model must be used for the pool.
///
/// Pricing a `Clmm` or `Dlmm` pool with the CPMM constant-product formula is a
/// correctness bug: it overstates depth by 5-100x and produces phantom profit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PoolKind {
    /// Constant product `x * y = k` (Raydium AMM v4, Raydium CPMM).
    #[default]
    Cpmm,
    /// Concentrated liquidity with sqrt-price ticks (Orca Whirlpool, Raydium CLMM).
    Clmm,
    /// Discrete bin ladder (Meteora DLMM).
    Dlmm,
}

impl PoolKind {
    /// Human label required in logs, reports and configuration.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpmm => "Constant Product",
            Self::Clmm => "Concentrated Liquidity",
            Self::Dlmm => "Dynamic Liquidity",
        }
    }
}

/// The pricing curve of a pool, in the units the quote model needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PoolCurve {
    /// Constant product reserves, ordered `(a, b)` to match `PoolSnapshot`.
    Cpmm { reserve_a: u128, reserve_b: u128 },
    /// Concentrated liquidity. `sqrt_price` is Q64.64. The lower/upper bounds are
    /// the active tick range; a swap that exits it must walk the next tick array.
    Clmm {
        liquidity: u128,
        sqrt_price: u128,
        tick_current: i32,
        sqrt_price_lower: u128,
        sqrt_price_upper: u128,
    },
    /// Bin ladder. `bin_liquidity[0]` is the active bin, index `i` is `active_bin + i`.
    Dlmm {
        active_bin: i32,
        bin_step_bps: u16,
        bin_liquidity: Vec<u128>,
    },
    /// Pump.fun virtual constant-product curve with real physical reserve ceilings.
    PumpFun {
        virtual_token_reserves: u64,
        virtual_sol_reserves: u64,
        real_token_reserves: u64,
        real_sol_reserves: u64,
        complete: bool,
    },
}

/// Canonical, self-describing pool state produced by the ingest layer.
///
/// Carries everything needed to price a swap without any further RPC calls, plus
/// the freshness key `(slot, write_version)` used to reject out-of-order updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolSnapshot {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub mint_a: MintId,
    pub mint_b: MintId,
    pub decimals_a: u8,
    pub decimals_b: u8,
    /// Total swap fee in basis points, read from chain (never hardcoded per DEX).
    pub fee_bps: u16,
    pub curve: PoolCurve,
    pub slot: Slot,
    pub write_version: u64,
    /// Wall-clock time the snapshot was produced, milliseconds since UNIX epoch.
    pub observed_at_ms: u64,
}

impl PoolSnapshot {
    /// True when `self` supersedes `other` for the same pool.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        (self.slot, self.write_version) > (other.slot, other.write_version)
    }
}

/// A single leg of a planned route, fully priced.
///
/// `min_out` is derived from `expected_out` here — the transaction builder must
/// never invent it from the input amount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedHop {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub mint_in: MintId,
    pub mint_out: MintId,
    pub decimals_in: u8,
    pub decimals_out: u8,
    pub amount_in: TokenAmount,
    pub expected_out: TokenAmount,
    pub min_out: TokenAmount,
    pub fee_bps: u16,
}

/// Itemised cost of executing one opportunity, in USD micros.
///
/// Every field must be populated before a decision is made. A zero here means
/// "measured and free", never "not modelled".
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub dex_fees_usd: UsdMicros,
    pub flash_fee_usd: UsdMicros,
    pub base_fee_usd: UsdMicros,
    pub priority_fee_usd: UsdMicros,
    pub tip_usd: UsdMicros,
    pub rent_usd: UsdMicros,
    /// Probability-weighted loss from a failed landing (fees burned, no profit).
    pub expected_failure_usd: UsdMicros,
}

impl CostBreakdown {
    pub fn total(&self) -> UsdMicros {
        self.dex_fees_usd
            .saturating_add(self.flash_fee_usd)
            .saturating_add(self.base_fee_usd)
            .saturating_add(self.priority_fee_usd)
            .saturating_add(self.tip_usd)
            .saturating_add(self.rent_usd)
            .saturating_add(self.expected_failure_usd)
    }
}

/// A route that has been sized, priced and costed — the only thing the executor
/// is allowed to sign.
///
/// Profit fields are in `base_mint` atomic units AND in USD. Decisions use USD;
/// atomic units are kept for on-chain `min_out` maths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizedOpportunity {
    pub route_id: RouteId,
    pub slot: Slot,
    pub base_mint: MintId,
    pub base_decimals: u8,
    pub size_in: TokenAmount,
    pub expected_out: TokenAmount,
    pub gross_profit: TokenAmount,
    pub net_profit: TokenAmount,
    pub gross_profit_usd: UsdMicros,
    pub net_profit_usd: UsdMicros,
    pub price_impact_bps: Bps,
    pub hops: Vec<PlannedHop>,
    pub cost: CostBreakdown,
    /// Age of the freshest input snapshot, in slots, at decision time.
    pub state_age_slots: u64,
}

impl SizedOpportunity {
    /// Profit as a fraction of notional, in basis points. Zero when notional is 0.
    pub fn net_bps(&self, notional_usd: UsdMicros) -> u16 {
        if notional_usd.0 <= 0 {
            return 0;
        }
        let bps = (self.net_profit_usd.0 as f64 * 10_000.0) / notional_usd.0 as f64;
        if bps <= 0.0 {
            return 0;
        }
        if bps >= f64::from(u16::MAX) {
            return u16::MAX;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let out = bps as u16;
        out
    }
}

/// Information about a token mint, including Token-2022 hazards.
///
/// A mint with `has_transfer_hook`, a non-zero `transfer_fee_bps`, a live
/// `freeze_authority` or a `permanent_delegate` must be rejected by the universe
/// filter: any of them can silently consume the trade or block the exit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintInfo {
    pub mint: MintId,
    pub decimals: u8,
    pub program_id: Pubkey,
    pub transfer_fee_bps: u16,
    pub is_token_2022: bool,
    pub has_transfer_hook: bool,
    pub has_permanent_delegate: bool,
    #[serde(default)]
    pub has_confidential_transfer: bool,
    pub freeze_authority_active: bool,
}

impl MintInfo {
    /// True when the mint is safe to route through.
    /// Transfer fees are permitted when accounted for in quotes; hooks, permanent
    /// delegates, confidential transfers, and active freeze authorities are rejected.
    pub fn is_tradeable(&self) -> bool {
        !self.has_transfer_hook
            && !self.has_permanent_delegate
            && !self.has_confidential_transfer
            && !self.freeze_authority_active
            && self.transfer_fee_bps < 10_000
    }
}

/// Raw account update stream item from Yellowstone Geyser.
#[derive(Debug, Clone)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub slot: Slot,
    pub write_version: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
}

/// Concentrated-liquidity state (Orca Whirlpool / Raydium CLMM).
/// None for CPMM/DLMM pools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClmmState {
    /// sqrt(price) in Q64.64 fixed point.
    #[serde(default)]
    pub sqrt_price: u128,
    /// Active liquidity.
    #[serde(default)]
    pub liquidity: u128,
    /// Current tick index.
    #[serde(default)]
    pub tick_current: i32,
    /// Tick spacing.
    #[serde(default)]
    pub tick_spacing: u16,
    /// Native protocol fee rate in its original integer precision (Orca uses a 1e-6 denominator).
    /// Quote calculations must not round-trip this through basis points.
    #[serde(default)]
    pub fee_rate: u16,
    /// Decoded tick arrays available for the quote. An empty collection is
    /// insufficient state for an executable CLMM quote.
    #[serde(default)]
    pub tick_arrays: Vec<ClmmTickArray>,
}

/// One initialized tick inside a Whirlpool/CLMM tick array.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClmmTick {
    #[serde(default)]
    pub tick_index: i32,
    /// True when this tick is initialized (provides liquidity).
    #[serde(default)]
    pub initialized: bool,
    #[serde(default)]
    pub liquidity_net: i128,
    /// Raydium CLMM unfilled limit-order amount at this tick
    /// (`orders_amount + part_filled_orders_remaining`). 0 for Orca pools and
    /// for legacy snapshots missing the field (serde default).
    #[serde(default)]
    pub limit_order_unfilled: u64,
}

/// A Whirlpool/CLMM tick array (88 ticks per array).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClmmTickArray {
    #[serde(default)]
    pub start_tick_index: i32,
    #[serde(default)]
    pub ticks: Vec<ClmmTick>,
}

/// Verified fee configuration of a Meteora DLMM `LbPair`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DlmmFeeParams {
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    /// On-chain `StaticParameters.function_type` per the official `FunctionType`
    /// enum (dlmm-sdk commons/src/conversions/function_type.rs):
    /// 0 = Undetermined, 1 = LiquidityMining, 2 = LimitOrder. Values > 2 are
    /// undocumented and fail closed at the quote gate. Do not confuse with the
    /// creation-time `ConcreteFunctionType` argument (0 = LO, 1 = LM), which is
    /// a different encoding.
    #[serde(default)]
    pub function_type: u8,
    /// `CollectFeeMode`: 0 = InputOnly, 1 = OnlyY. Determines the mint in
    /// which the swap fee is denominated.
    #[serde(default)]
    pub collect_fee_mode: u8,
}

/// Discrete-bin state (Meteora DLMM).
/// None for CPMM/CLMM pools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DlmmState {
    /// Active bin ID.
    #[serde(default)]
    pub active_bin_id: i32,
    /// On-chain LbPair status (0 = enabled; non-zero = disabled).
    #[serde(default)]
    pub status: u8,
    /// Bin step in basis points.
    #[serde(default)]
    pub bin_step_bps: u16,
    /// Bin liquidity: index 0 = active bin.
    #[serde(default)]
    pub bins: Vec<DlmmBin>,
    /// Verified fee configuration read from the `LbPair`. When `None` the pool
    /// cannot be priced honestly and must be skipped.
    #[serde(default)]
    pub params: Option<DlmmFeeParams>,
    /// Volatility accumulator tracked across swaps.
    #[serde(default)]
    pub volatility_accumulator: u32,
    #[serde(default)]
    pub volatility_reference: u32,
    /// On-chain `index_reference`: the bin the volatility accumulator is measured
    /// against. It is *not* the active bin and must not be substituted by it.
    #[serde(default)]
    pub index_reference: i32,
    #[serde(default)]
    pub last_update_timestamp: i64,
    /// Whether the pool supports the limit-order liquidity layer
    /// (`LbPair::is_support_limit_order` in the official SDK). When `false`,
    /// quotes use MM reserves only (underquote, fail-safe).
    #[serde(default)]
    pub support_limit_order: bool,
}

/// Single bin in a DLMM ladder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DlmmBin {
    #[serde(default)]
    pub bin_id: i32,
    #[serde(default)]
    pub amount_x: u128,
    #[serde(default)]
    pub amount_y: u128,
    /// Limit-order layer: open (unprocessed) order liquidity, denominated in
    /// the order's committed output token (on-chain `Bin.open_order_amount`).
    #[serde(default)]
    pub open_order_amount: u64,
    /// Limit-order layer: remaining fillable amount of already-processed
    /// orders (on-chain `Bin.processed_order_remaining_amount`).
    #[serde(default)]
    pub processed_order_remaining_amount: u64,
    /// Limit-order side flag (on-chain `Bin.limit_order_ask_side`).
    #[serde(default)]
    pub limit_order_ask_side: bool,
}

/// Pump.fun bonding curve on-chain state for pricing and capacity clamping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PumpFunState {
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    #[serde(default)]
    pub creator: Pubkey,
    #[serde(default)]
    pub is_mayhem_mode: bool,
    #[serde(default)]
    pub is_cashback_coin: bool,
    /// False for legacy/incomplete state that lacks the current account mode flags.
    #[serde(default)]
    pub mode_flags_present: bool,
}

impl PumpFunState {
    /// True when the curve is active and has sufficient liquidity for swaps.
    pub fn is_tradeable(&self) -> bool {
        self.mode_flags_present
            && !self.is_mayhem_mode
            && !self.is_cashback_coin
            && !self.complete
            && self.real_token_reserves > 0
            && self.virtual_sol_reserves > 0
            && self.virtual_token_reserves > 0
    }
}

/// Exact Raydium CPMM fee schedule and per-pool creator-fee routing.
/// Rates use the on-chain 1_000_000 denominator; protocol/fund rates are
/// shares of the trade fee, while creator_fee_rate_ppm is a separate volume rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaydiumCpmmFeeState {
    pub trade_fee_rate_ppm: u64,
    pub creator_fee_rate_ppm: u64,
    pub protocol_fee_rate: u64,
    pub fund_fee_rate: u64,
    pub enable_creator_fee: bool,
    /// 0 = BothToken, 1 = OnlyToken0, 2 = OnlyToken1.
    pub creator_fee_on: u8,
}

impl RaydiumCpmmFeeState {
    pub fn is_valid(&self) -> bool {
        const DENOMINATOR: u64 = 1_000_000;
        self.trade_fee_rate_ppm < DENOMINATOR
            && self.creator_fee_rate_ppm < DENOMINATOR
            && (!self.enable_creator_fee
                || self
                    .trade_fee_rate_ppm
                    .checked_add(self.creator_fee_rate_ppm)
                    .is_some_and(|r| r < DENOMINATOR))
            && self.protocol_fee_rate <= DENOMINATOR
            && self.fund_fee_rate <= DENOMINATOR
            && self
                .protocol_fee_rate
                .checked_add(self.fund_fee_rate)
                .is_some_and(|r| r <= DENOMINATOR)
            && self.creator_fee_on <= 2
    }

    pub fn creator_fee_on_input(&self, a_to_b: bool) -> Option<bool> {
        if !self.is_valid() {
            return None;
        }
        if !self.enable_creator_fee || self.creator_fee_rate_ppm == 0 {
            return Some(true);
        }
        match self.creator_fee_on {
            0 => Some(true),
            1 => Some(a_to_b),
            2 => Some(!a_to_b),
            _ => None,
        }
    }

    pub fn effective_creator_fee_rate(&self) -> u64 {
        if self.enable_creator_fee {
            self.creator_fee_rate_ppm
        } else {
            0
        }
    }
}

/// Fully decoded Meteora DAMM v2 quote state. Values are copied from the
/// Pool account and a Clock sysvar fetched in one coherent RPC context.
/// collect_fee_mode == 2 uses tracked V1 reserves; all other modes use
/// concentrated-liquidity sqrt-price/liquidity math.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DammV2State {
    pub pool_status: u8,
    pub fee_version: u8,
    pub layout_version: u8,
    pub collect_fee_mode: u8,
    pub activation_type: u8,
    pub activation_point: u64,
    pub current_point: u64,
    pub current_timestamp: u64,
    pub clock_slot: u64,
    pub clock_epoch: u64,
    pub sqrt_price_x64: u128,
    pub liquidity: u128,
    pub sqrt_min_price_x64: u128,
    pub sqrt_max_price_x64: u128,
    pub token_a_amount: Option<u64>,
    pub token_b_amount: Option<u64>,
    pub cliff_fee_numerator: u64,
    pub fee_scheduler_mode: u8,
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
    pub sqrt_price_step_bps: u32,
    pub scheduler_expiration_duration: u32,
    pub init_sqrt_price_x64: u128,
    pub protocol_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub compounding_fee_bps: u16,
    pub dynamic_fee_initialized: bool,
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub dynamic_reduction_factor: u16,
    pub last_update_timestamp: u64,
    pub bin_step_u128: u128,
    pub sqrt_price_reference: u128,
    pub volatility_accumulator: u128,
    pub volatility_reference: u128,
    /// True only when Pool, both vaults, both mints and Clock were owner- and
    /// mint-validated from one getMultipleAccounts context.
    pub account_graph_verified: bool,
}

impl DammV2State {
    pub fn is_compounding(&self) -> bool {
        self.collect_fee_mode == 2
    }

    pub fn fee_context_matches(&self, pool_slot: u64) -> bool {
        self.account_graph_verified
            && pool_slot > 0
            && self.clock_slot == pool_slot
            && self.pool_status == 0
            && self.activation_type <= 1
            && self.layout_version <= 1
            && self.fee_version <= 1
            && self.current_point > 0
            && (!self.is_compounding()
                || (self.layout_version == 1
                    && self.token_a_amount.is_some_and(|v| v > 0)
                    && self.token_b_amount.is_some_and(|v| v > 0)))
    }
}

/// Canonical pool state across all DEX protocols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolState {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub slot: Slot,
    pub write_version: u64,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Decimals of the base (quote) mint, read from a verified mint account.
    /// `0` means "unknown — must be resolved before the pool is priced".
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub reserve_in: u128,
    pub reserve_out: u128,
    /// Exact protocol fee-rate numerator. Preserve the source denominator;
    /// converting sub-bps protocols (e.g. Raydium CPMM) to integer bps loses fees.
    pub fee_numerator: u64,
    /// Exact protocol fee-rate denominator (10_000 for bps venues, 1_000_000
    /// for Raydium CPMM/CLMM). Quote engines must use this pair, not fee_bps().
    pub fee_denominator: u64,
    /// Raydium CPMM's separate trade/creator fee schedule and direction mode.
    /// `None` for other venues and for legacy/unverified CPMM snapshots.
    #[serde(default)]
    pub raydium_cpmm_fee: Option<RaydiumCpmmFeeState>,
    pub is_active: bool,
    /// Token-2022 safety: true if either mint has a risky extension
    /// (transfer fee, transfer hook, permanent delegate, non-transferable,
    /// default account state, mint close authority). P1 #14.
    #[serde(default)]
    pub has_risky_token: bool,
    /// Pricing model kind. P1 #12.
    #[serde(default)]
    pub kind: PoolKind,
    /// Concentrated-liquidity state (CLMM pools). None for CPMM/DLMM.
    #[serde(default)]
    pub clmm: Option<ClmmState>,
    /// Discrete-bin state (DLMM pools). None for CPMM/CLMM.
    #[serde(default)]
    pub dlmm: Option<DlmmState>,
    /// Pump.fun bonding curve state. None for other protocols.
    #[serde(default)]
    pub pump_fun: Option<PumpFunState>,
    /// DAMM v2 uses its own scheduler, fee mode and curve state.
    #[serde(default)]
    pub damm_v2: Option<DammV2State>,
    /// Token-2022 transfer fee applied to every swap through this pool, in
    /// basis points. 0 when the mints are legacy SPL Token.
    #[serde(default)]
    pub transfer_fee_bps: u16,
    /// Token programs for `(base_mint, quote_mint)`.
    #[serde(default)]
    pub token_programs: [TokenProgramKind; 2],
    /// Resolved on-chain execution accounts (vaults, configs, ticks/bins, etc.).
    #[serde(default)]
    pub execution_accounts: Option<PoolExecutionAccounts>,
    /// Monitor freshness tracking (P2 #11)
    #[serde(default)]
    pub last_update_slot: u64,
    #[serde(default)]
    pub last_update_ms: u64,
}

/// Convert an exact fee ratio to conservative integer BPS metadata. Ceiling
/// prevents graph-only paths from treating a valid sub-bps fee as zero.
fn conservative_fee_bps(numerator: u64, denominator: u64) -> Option<u16> {
    if denominator == 0 {
        return None;
    }
    let scaled = u128::from(numerator).checked_mul(10_000)?;
    let rounded_up = scaled.checked_add(u128::from(denominator) - 1)? / u128::from(denominator);
    let fee = u16::try_from(rounded_up).ok()?;
    (fee <= PoolState::FEE_BPS_MAX).then_some(fee)
}

#[cfg(test)]
mod conservative_fee_bps_tests {
    use super::conservative_fee_bps;

    #[test]
    fn sub_basis_point_fees_round_up_for_graph_estimates() {
        assert_eq!(conservative_fee_bps(50, 1_000_000), Some(1));
        assert_eq!(conservative_fee_bps(2_500, 1_000_000), Some(25));
        assert_eq!(conservative_fee_bps(2_560, 1_000_000), Some(26));
        assert_eq!(conservative_fee_bps(0, 10_000), Some(0));
        assert_eq!(conservative_fee_bps(50, 0), None);
    }
}

impl PoolState {
    /// Maximum representable fee rate in basis points (100%). Any computed fee
    /// above this is invalid and must be rejected, never silently capped.
    pub const FEE_BPS_MAX: u16 = 10_000;

    /// Conservative, rounded-up basis-point metadata for graph ranking and
    /// display. This is NOT suitable for exact quote math: valid sub-bps rates
    /// round up to 1 bps. Pricing must use the exact ratio fields.
    ///
    /// Returns `None` when the fee is invalid: `fee_denominator == 0`, the
    /// computed value overflows `u16`, or the result exceeds `FEE_BPS_MAX`.
    pub fn fee_bps(&self) -> Option<u16> {
        conservative_fee_bps(self.fee_numerator, self.fee_denominator)
    }

    /// True when the pool carries a usable fee numerator/denominator pair.
    /// A pool with an invalid fee must never be priced or discovered.
    pub fn is_fee_valid(&self) -> bool {
        self.fee_bps().is_some()
    }
}

/// Swap step in an arbitrage route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapHop {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub mint_in: MintId,
    pub mint_out: MintId,
    pub fee_bps: u16,
    /// Active bin ID for DLMM pools (Meteora). Populated by the ingestion
    /// layer from the LbPair account; None for CPMM/CLMM pools.
    #[serde(default)]
    pub active_bin_id: Option<i32>,
    /// Token programs for `(mint_in, mint_out)`.
    #[serde(default)]
    pub token_programs: [TokenProgramKind; 2],
    /// PumpSwap trade intent: exact-in (default) or exact-out base.
    /// Only meaningful for PumpSwap hops; ignored by other protocols.
    #[serde(default)]
    pub trade_intent: PumpSwapTradeIntent,
    /// Verified on-chain execution accounts resolved for this pool.
    #[serde(default)]
    pub execution_accounts: Option<PoolExecutionAccounts>,
}

impl SwapHop {}

/// Quotation result for a swap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    pub amount_in: TokenAmount,
    pub expected_out: TokenAmount,
    pub min_out: TokenAmount,
    pub fee_amount: TokenAmount,
    pub price_impact_bps: Bps,
}

/// Unified quoted amount carrying both raw atomic units and a UI-scaled value.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct QuoteAmount {
    pub mint: MintId,
    pub decimals: u8,
    pub raw: u64,
    pub ui_amount: f64,
}

/// Arbitrage route definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbRoute {
    pub id: RouteId,
    pub hops: Vec<SwapHop>,
    pub base_mint: MintId,
}

/// Detected and sized arbitrage opportunity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOpportunity {
    pub route_id: RouteId,
    pub slot: Slot,
    pub size_in: TokenAmount,
    pub expected_out: TokenAmount,
    /// Per-hop expected input amounts: `hop_inputs[i]` is the input amount for hop `i`,
    /// and `hop_inputs[hops.len()]` is the final expected output. Used to derive a correct
    /// `min_amount_out` (in output units) for each swap leg. Empty => builders fall back.
    #[serde(default)]
    pub hop_inputs: Vec<u64>,
    pub gross_profit: Lamports,
    pub net_profit: Lamports,
    /// Decimals of the base mint used to convert raw base units to SOL-like units.
    /// No default — must be explicitly set from verified on-chain mint data.
    pub base_decimals: u8,
    pub price_impact_bps: Bps,
    pub hops: Vec<SwapHop>,
    /// Conservative slippage-floor final output carried by pipelines that
    /// compute one (e.g. the 3-leg DLMM pipeline). `0` means unknown; repayment
    /// gates then fall back to `expected_out` alone.
    #[serde(default)]
    pub expected_min_final_output: u64,
    /// Per-hop min-out floor slippage (bps) used by tx builders when deriving
    /// each swap leg's `min_amount_out`. Defaults to
    /// `DEFAULT_HOP_SLIPPAGE_BPS`; the supervisor overrides it from strategy
    /// config before dispatch so config is the single source of truth.
    #[serde(default = "default_hop_slippage_bps")]
    pub hop_slippage_bps: u64,
    /// Quote-time CU estimate including exact DLMM/CLMM traversal counts.
    /// Zero identifies an older/external opportunity requiring fallback estimation.
    #[serde(default)]
    pub cu_estimate_units: u32,
    /// Stable route/amount/impact/traversal signature for measured-CU cache reuse.
    #[serde(default)]
    pub cu_shape_key: String,
}

/// Canonical representation of a single swap leg within a multi-hop execution plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HopExecutionLeg {
    /// Exact input amount for this hop.
    pub amount_in: u64,
    /// Conservative minimum output guaranteed by this hop (slippage deducted exactly once).
    pub min_amount_out: u64,
}

/// Canonical multi-hop execution plan enforcing that each hop's input amount is strictly
/// bounded by the previous hop's conservative minimum output (`previous_hop.min_amount_out >= next_hop.amount_in`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HopExecutionPlan {
    pub legs: Vec<HopExecutionLeg>,
}

impl HopExecutionPlan {
    /// Construct a new execution plan from explicit legs and validate the sequential solvency invariant.
    pub fn new(legs: Vec<HopExecutionLeg>) -> Result<Self> {
        let plan = Self { legs };
        if !plan.validate_sequential_solvency() {
            return Err(ArbError::InvalidInput(
                "Sequential solvency violated: previous hop min_amount_out < next hop amount_in"
                    .into(),
            ));
        }
        Ok(plan)
    }

    /// Returns true if and only if every hop's input is less than or equal to the preceding hop's
    /// guaranteed minimum output, preventing pre-existing inventory subsidy or slippage underflow.
    pub fn validate_sequential_solvency(&self) -> bool {
        for window in self.legs.windows(2) {
            if window[0].min_amount_out < window[1].amount_in {
                return false;
            }
        }
        true
    }
}

fn default_hop_slippage_bps() -> u64 {
    arb_constants::DEFAULT_HOP_SLIPPAGE_BPS
}

impl ExecutionOpportunity {
    /// Effective per-hop slippage floor, clamped to `HOP_SLIPPAGE_MAX_BPS`.
    /// Builders MUST use this instead of the raw field so an operator
    /// misconfiguration can never disable the min-out protection entirely.
    pub fn effective_hop_slippage_bps(&self) -> u64 {
        self.hop_slippage_bps
            .min(arb_constants::HOP_SLIPPAGE_MAX_BPS)
    }
}

// ============================================================================
