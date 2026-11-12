//! Core Domain Types (Newtypes) and Canonical Primitives for Solana Arbitrage.

pub mod currency;
pub mod dimensional;
pub use currency::*;

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// SOL unit (lamports). Never mix with TokenAmount.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Lamports(pub u64);

impl Lamports {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_u128(self) -> u128 {
        self.0 as u128
    }
}

impl fmt::Display for Lamports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} lamports", self.0)
    }
}

/// Token amount (in smallest atomic units, e.g. 1e6 for USDC).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct TokenAmount(pub u64);

impl TokenAmount {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_u128(self) -> u128 {
        self.0 as u128
    }
}

impl fmt::Display for TokenAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Basis points (1 bp = 0.01%, 10000 = 100%).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Bps(pub u16);

impl Bps {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(10_000);

    pub fn as_fraction(self) -> f64 {
        self.0 as f64 / 10_000.0
    }
}

/// Slot number on the Solana blockchain.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Slot(pub u64);

impl Slot {
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> u64 {
        self.0.saturating_sub(rhs.0)
    }
}

/// Compute units consumed/requested by a transaction. Distinct from Lamports.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct ComputeUnits(pub u32);

impl ComputeUnits {
    pub const MAX: Self = Self(1_400_000); // current Solana compute budget cap
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Route identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RouteId(pub u32);

/// Pool address identifier wrapping Pubkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct PoolId(pub Pubkey);

impl PoolId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(Pubkey::new_from_array(bytes))
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_bytes()
    }
}

impl From<Pubkey> for PoolId {
    fn from(p: Pubkey) -> Self {
        Self(p)
    }
}

impl From<[u8; 32]> for PoolId {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Display for PoolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PoolId {
    type Err = solana_pubkey::ParsePubkeyError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Pubkey::from_str(s).map(Self)
    }
}

/// Token Mint address identifier wrapping Pubkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct MintId(pub Pubkey);

impl MintId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(Pubkey::new_from_array(bytes))
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_bytes()
    }
}

impl From<Pubkey> for MintId {
    fn from(p: Pubkey) -> Self {
        Self(p)
    }
}

impl From<[u8; 32]> for MintId {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Display for MintId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MintId {
    type Err = solana_pubkey::ParsePubkeyError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Pubkey::from_str(s).map(Self)
    }
}

/// Supported Solana DEX Protocols.
///
/// Every variant is an independent adapter: its own program id, instruction
/// format, fee model and pricing curve. They are deliberately NOT treated as
/// interchangeable Uniswap V2/V3 clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DexProtocol {
    /// Raydium constant-product AMM (legacy program, Serum/OpenBook accounts).
    RaydiumV4,
    /// Raydium CP-Swap (`CPMMoo8…`): constant product with an Anchor API.
    RaydiumCpmm,
    /// Raydium concentrated liquidity (Q64.64 ticks).
    RaydiumClmm,
    /// Orca Whirlpools (concentrated liquidity).
    OrcaWhirlpool,
    /// Meteora DLMM (discrete bin ladder with volatility-scaled fees).
    MeteoraDlmm,
    /// Meteora DAMM v2 (`cp_amm`, `cpamdp…`): configurable constant/concentrated AMM.
    MeteoraDammV2,
    /// Raydium LaunchLab (`LanMV9…`): bonding curve launchpad.
    RaydiumLaunchLab,
    /// Pump.fun bonding curve constant-product AMM.
    PumpFun,
    /// PumpSwap post-graduation constant-product AMM.
    PumpSwap,
}

impl DexProtocol {
    /// Pricing family. Determines which curve the quote model must use.
    ///
    /// Program ids live only in `arb-constants` (single source of truth); this
    /// crate deliberately carries no copy of them.
    pub const fn family(self) -> PoolKind {
        match self {
            Self::RaydiumV4
            | Self::RaydiumCpmm
            | Self::MeteoraDammV2
            | Self::RaydiumLaunchLab
            | Self::PumpFun
            | Self::PumpSwap => PoolKind::Cpmm,
            Self::RaydiumClmm | Self::OrcaWhirlpool => PoolKind::Clmm,
            Self::MeteoraDlmm => PoolKind::Dlmm,
        }
    }
}

/// Token program owning a given mint/token account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TokenProgramKind {
    #[default]
    SplToken,
    Token2022,
}

impl TokenProgramKind {
    pub fn program_id(self) -> solana_pubkey::Pubkey {
        use std::str::FromStr;
        match self {
            Self::SplToken => solana_pubkey::Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)
                .expect("valid SPL token program id"),
            Self::Token2022 => {
                solana_pubkey::Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID)
                    .expect("valid Token-2022 program id")
            }
        }
    }
}

/// Raydium AMM V4 swap instruction schema.
///
/// Raydium added `SwapBaseInV2` (opcode 16, 8 accounts, no OpenBook/Serum
/// dependencies) after the original `SwapBaseIn` (opcode 9, 18 accounts). The
/// V2 schema is the default for every pool that supports it. V1 remains an
/// explicit, opt-in compatibility path for older pools; it is never selected
/// silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaydiumV4SwapVersion {
    /// Canonical `SwapBaseInV2` — opcode 16, 8 accounts.
    #[default]
    V2,
    /// Legacy `SwapBaseIn` — opcode 9, 18 accounts (OpenBook/Serum required).
    V1,
}

impl RaydiumV4SwapVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1 => "v1",
        }
    }
}

/// PumpSwap instruction generation supported by a pool.
///
/// V2 is the canonical schema for current pools (17 accounts, `*_v2`
/// discriminators). V1 is the legacy schema (16 accounts, `buy`/`sell`
/// discriminators, requires the Pump Fees `fee_config` account). A pool is only
/// built with V1 when it is explicitly marked as such; V2 never silently
/// degrades to V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PumpSwapPoolVersion {
    /// Current `buy_v2` / `sell_v2` / `buy_exact_quote_in_v2` schema.
    #[default]
    V2,
    /// Legacy `buy` / `sell` schema with `fee_config`.
    V1Legacy,
}

impl PumpSwapPoolVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1Legacy => "v1_legacy",
        }
    }
}

/// PumpSwap swap direction intent — whether the hop spends an exact input
/// amount or requires an exact output amount.
///
/// `ExactIn` is the canonical default (spend exactly `amount_in`, receive at
/// least `min_amount_out`). `ExactOutBase` requires exactly `min_amount_out`
/// base tokens and spends at most `amount_in` quote tokens.
///
/// Sell + ExactOutBase is not supported by PumpSwap and must fail closed — the
/// program only offers `buy`/`sell` (exact-in) and `buy_exact_base_out_v2`
/// (buy exact-out) discriminators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PumpSwapTradeIntent {
    /// Spend exact input, receive minimum output (canonical path).
    #[default]
    ExactIn,
    /// Receive exact base output, spend at most the given quote input.
    ExactOutBase,
}

/// Verified per-pool on-chain execution accounts resolved during ingestion/decoding.
/// Eliminates hardcoded SOL-USDC account fallbacks and invalid PDA guesses in `arb-tx`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolExecutionAccounts {
    RaydiumV4 {
        amm_authority: Pubkey,
        open_orders: Pubkey,
        target_orders: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        serum_program: Pubkey,
        serum_market: Pubkey,
        serum_bids: Pubkey,
        serum_asks: Pubkey,
        serum_event_queue: Pubkey,
        serum_coin_vault: Pubkey,
        serum_pc_vault: Pubkey,
        serum_vault_signer: Pubkey,
        /// Which instruction schema this pool must use. Defaults to V2.
        #[serde(default)]
        swap_version: RaydiumV4SwapVersion,
    },
    RaydiumCpmm {
        amm_config: Pubkey,
        authority: Pubkey,
        vault_0: Pubkey,
        vault_1: Pubkey,
        observation_state: Pubkey,
    },
    RaydiumClmm {
        amm_config: Pubkey,
        vault_0: Pubkey,
        vault_1: Pubkey,
        observation_state: Pubkey,
        tick_array_pdas: Vec<Pubkey>,
        bitmap_extension: Option<Pubkey>,
    },
    OrcaWhirlpool {
        vault_a: Pubkey,
        vault_b: Pubkey,
        oracle: Pubkey,
        tick_array_pdas: Vec<Pubkey>,
    },
    MeteoraDlmm {
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        oracle: Pubkey,
        bin_array_pdas: Vec<Pubkey>,
        bitmap_extension: Option<Pubkey>,
    },
    MeteoraDammV2 {
        config: Pubkey,
        authority: Pubkey,
        vault_a: Pubkey,
        vault_b: Pubkey,
        event_authority: Pubkey,
    },
    RaydiumLaunchLab {
        authority: Pubkey,
        global_config: Pubkey,
        platform_config: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        event_authority: Pubkey,
    },
    PumpFun {
        creator: Pubkey,
        fee_recipient: Pubkey,
    },
    PumpSwap {
        global_config: Pubkey,
        pool_base_vault: Pubkey,
        pool_quote_vault: Pubkey,
        protocol_fee_recipient: Pubkey,
        protocol_fee_recipient_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        coin_creator_vault_ata: Pubkey,
        fee_config: Pubkey,
        /// Which PumpSwap instruction generation the pool supports.
        #[serde(default)]
        pool_version: PumpSwapPoolVersion,
    },
}

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

    pub fn is_positive(self) -> bool {
        self.0 > 0
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
    /// `ConcreteFunctionType` at StaticParameters.function_type (0 = Spot).
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

    /// Progress towards Raydium migration in basis points (10_000 = 100% complete at 85 SOL).
    pub fn graduation_progress_bps(&self) -> u16 {
        let bps = (self.real_sol_reserves as u128 * 10_000)
            / (arb_constants::PUMP_FUN_GRADUATION_LAMPORTS as u128);
        bps.min(10_000) as u16
    }

    /// Remaining SOL in lamports required to complete graduation.
    pub fn remaining_sol_to_graduate(&self) -> u64 {
        arb_constants::PUMP_FUN_GRADUATION_LAMPORTS.saturating_sub(self.real_sol_reserves)
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
    pub fee_numerator: u64,
    pub fee_denominator: u64,
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

impl PoolState {
    /// Total swap fee in basis points, derived from the stored numerator/denom.
    ///
    /// Returns `None` when `fee_denominator == 0` (invalid fee). Callers MUST handle
    /// the `None` case and reject the pool rather than price it with a guessed fee.
    pub fn fee_bps(&self) -> Option<u16> {
        if self.fee_denominator == 0 {
            return None;
        }
        Some(
            u16::try_from((self.fee_numerator as u128 * 10_000) / self.fee_denominator as u128)
                .unwrap_or(10_000),
        )
    }

    /// True when the pool carries a usable fee numerator/denominator pair.
    /// A pool with an invalid fee must never be priced or discovered.
    pub fn is_fee_valid(&self) -> bool {
        self.fee_denominator != 0
    }
}

/// Complete 18-Account Matrix for Raydium v4 SwapBaseIn instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaydiumV4Accounts {
    pub token_program: solana_pubkey::Pubkey,
    pub amm_pool: solana_pubkey::Pubkey,
    pub amm_authority: solana_pubkey::Pubkey,
    pub amm_open_orders: solana_pubkey::Pubkey,
    pub amm_target_orders: solana_pubkey::Pubkey,
    pub pool_coin_vault: solana_pubkey::Pubkey,
    pub pool_pc_vault: solana_pubkey::Pubkey,
    pub serum_program: solana_pubkey::Pubkey,
    pub serum_market: solana_pubkey::Pubkey,
    pub serum_bids: solana_pubkey::Pubkey,
    pub serum_asks: solana_pubkey::Pubkey,
    pub serum_event_queue: solana_pubkey::Pubkey,
    pub serum_coin_vault: solana_pubkey::Pubkey,
    pub serum_pc_vault: solana_pubkey::Pubkey,
    pub serum_vault_signer: solana_pubkey::Pubkey,
}

impl RaydiumV4Accounts {
    pub fn mainnet_sol_usdc() -> Self {
        use std::str::FromStr;
        Self {
            token_program: solana_pubkey::Pubkey::from_str(
                "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            )
            .unwrap(),
            amm_pool: solana_pubkey::Pubkey::from_str(
                "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
            )
            .unwrap(),
            amm_authority: solana_pubkey::Pubkey::from_str(
                "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1",
            )
            .unwrap(),
            amm_open_orders: solana_pubkey::Pubkey::from_str(
                "HmiHHzq4Fym9e1D4qzLS6LDDM3tNsCTBPDWHTLZ763jY",
            )
            .unwrap(),
            amm_target_orders: solana_pubkey::Pubkey::from_str(
                "CZza3Ej4Mc58MnxWA385itCC9jCo3L1D7zc3LKy1bZMR",
            )
            .unwrap(),
            pool_coin_vault: solana_pubkey::Pubkey::from_str(
                "DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz",
            )
            .unwrap(),
            pool_pc_vault: solana_pubkey::Pubkey::from_str(
                "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz",
            )
            .unwrap(),
            serum_program: solana_pubkey::Pubkey::from_str(
                "srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX",
            )
            .unwrap(),
            serum_market: solana_pubkey::Pubkey::from_str(
                "8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6",
            )
            .unwrap(),
            serum_bids: solana_pubkey::Pubkey::from_str(
                "5jWUncPNBMZJ3sTHKmMLszypVkoRK6bfEQMQUHweeQnh",
            )
            .unwrap(),
            serum_asks: solana_pubkey::Pubkey::from_str(
                "EaXdHx7x3mdGA38j5RSmKYSXMzAFzzUXCLNBEDXDn1d5",
            )
            .unwrap(),
            serum_event_queue: solana_pubkey::Pubkey::from_str(
                "8CvwxZ9Db6XbLD46NZwwmVDZZRDy7eydFcAGkXKh9axa",
            )
            .unwrap(),
            serum_coin_vault: solana_pubkey::Pubkey::from_str(
                "CKxTHwM9fPMRRvZmFnFoqKNd9pQR21c5Aq9bh5h9oghX",
            )
            .unwrap(),
            serum_pc_vault: solana_pubkey::Pubkey::from_str(
                "6A5NHCj1yF6urc9wZNe6Bcjj4LVszQNj5DwAWG97yzMu",
            )
            .unwrap(),
            serum_vault_signer: solana_pubkey::Pubkey::from_str(
                "CTz5UMLQm2SRWHzQnU62Pi4yJqbNGjgRBHqqp6oDHfF7",
            )
            .unwrap(),
        }
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

impl SwapHop {
    /// True when this hop executes against the Pump.fun bonding curve.
    pub fn is_pump_fun(&self) -> bool {
        self.protocol == DexProtocol::PumpFun
    }

    /// True when this leg interacts directly with Native SOL rather than SPL WSOL (e.g. Pump.fun).
    pub fn is_native_sol(&self) -> bool {
        self.protocol == DexProtocol::PumpFun
    }
}

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
}

// ============================================================================
// Jito tip market — quantiles, posteriors, bid decisions
// ============================================================================

/// Z-score for a one-sided 95% confidence bound (`Phi^-1(0.95)`).
pub const Z_95: f64 = 1.644_853_626_951_472_2;

/// Rung of the landed-tip distribution published by the Jito tip floor.
///
/// The rung is the *treatment* the bidder learns from: bidding at `P90` implies
/// roughly 90% inclusion, and the observed landing rate at that rung corrects
/// that assumption. It is deliberately not a per-block-engine concept — Jito
/// resolves all regions as a single auction, so a landing cannot be attributed
/// to one region and per-region credit assignment would be fabricated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Percentile {
    P50,
    P65,
    P75,
    P85,
    P90,
    P95,
    P99,
}

impl Percentile {
    /// Number of rungs in the grid.
    pub const COUNT: usize = 7;

    /// The rung grid, cheapest first.
    pub const GRID: [Self; Self::COUNT] = [
        Self::P50,
        Self::P65,
        Self::P75,
        Self::P85,
        Self::P90,
        Self::P95,
        Self::P99,
    ];

    /// Stable storage index, `0..COUNT`.
    pub const fn index(self) -> usize {
        match self {
            Self::P50 => 0,
            Self::P65 => 1,
            Self::P75 => 2,
            Self::P85 => 3,
            Self::P90 => 4,
            Self::P95 => 5,
            Self::P99 => 6,
        }
    }

    /// Stable, human-readable label (`"q50"` .. `"q99"`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::P50 => "q50",
            Self::P65 => "q65",
            Self::P75 => "q75",
            Self::P85 => "q85",
            Self::P90 => "q90",
            Self::P95 => "q95",
            Self::P99 => "q99",
        }
    }

    /// Market-implied inclusion probability of a bid priced at this rung.
    ///
    /// Used as the prior when the ledger holds no observation for the rung: a
    /// bid at the 95th percentile of *landed* tips is assumed to be included
    /// about 95% of the time.
    pub fn probability(self) -> f64 {
        match self {
            Self::P50 => 0.50,
            Self::P65 => 0.65,
            Self::P75 => 0.75,
            Self::P85 => 0.85,
            Self::P90 => 0.90,
            Self::P95 => 0.95,
            Self::P99 => 0.99,
        }
    }
}

impl fmt::Display for Percentile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Immutable snapshot of the Jito landed-tip distribution, in lamports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipFloor {
    /// Wall-clock time the snapshot was produced, milliseconds since UNIX epoch.
    pub observed_at_ms: u64,
    pub p25: Lamports,
    pub p50: Lamports,
    pub p75: Lamports,
    pub p95: Lamports,
    pub p99: Lamports,
    /// Exponential moving average of the 50th percentile.
    pub ema50: Lamports,
}

impl TipFloor {
    /// Price a rung. Rungs the endpoint does not publish (`P65`, `P85`, `P90`)
    /// are linearly interpolated between their bracketing published rungs.
    pub fn at(&self, rung: Percentile) -> Lamports {
        match rung {
            Percentile::P50 => self.p50,
            Percentile::P65 => Self::lerp(self.p50, self.p75, 3, 5),
            Percentile::P75 => self.p75,
            Percentile::P85 => Self::lerp(self.p75, self.p95, 1, 2),
            Percentile::P90 => Self::lerp(self.p75, self.p95, 3, 4),
            Percentile::P95 => self.p95,
            Percentile::P99 => self.p99,
        }
    }

    /// Integer linear interpolation, `a + (b - a) * num / den`.
    fn lerp(a: Lamports, b: Lamports, num: u64, den: u64) -> Lamports {
        let den = u128::from(den.max(1));
        let a_wide = u128::from(a.0);
        let b_wide = u128::from(b.0);
        let delta = b_wide.saturating_sub(a_wide);
        let out = a_wide.saturating_add(delta.saturating_mul(u128::from(num)) / den);
        Lamports(u64::try_from(out).unwrap_or(u64::MAX))
    }

    /// Age of the snapshot in milliseconds, saturating at zero for clock skew.
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.observed_at_ms)
    }

    /// True when the snapshot is young enough to price a bid from.
    pub fn is_fresh(&self, now_ms: u64, max_age_ms: u64) -> bool {
        self.age_ms(now_ms) <= max_age_ms
    }
}

/// Beta posterior over a landing probability.
///
/// `alpha` counts landings, `beta` counts misses, with a `Beta(1, 1)` uniform
/// prior so an unobserved rung reports the prior mean (0.5) rather than 0.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Posterior {
    pub alpha: f64,
    pub beta: f64,
}

impl Posterior {
    /// Uniform prior, `Beta(1, 1)`.
    pub const PRIOR: Self = Self {
        alpha: 1.0,
        beta: 1.0,
    };

    /// Fold one observation into the posterior.
    pub fn record(&mut self, landed: bool) {
        if landed {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// Number of observations folded in, excluding the prior.
    pub fn observations(&self) -> f64 {
        (self.alpha + self.beta - 2.0).max(0.0)
    }

    /// Posterior mean.
    pub fn mean(&self) -> f64 {
        let total = self.alpha + self.beta;
        if total <= 0.0 {
            return 0.5;
        }
        (self.alpha / total).clamp(0.0, 1.0)
    }

    /// Wilson score lower bound on the success probability.
    ///
    /// A raw mean over a handful of samples is optimistic; the lower bound
    /// refuses to price an unproven rung as if it were certain.
    pub fn wilson_lower(&self, z: f64) -> f64 {
        let n = self.alpha + self.beta;
        if n <= 0.0 || !z.is_finite() || z <= 0.0 {
            return 0.0;
        }
        let p = (self.alpha / n).clamp(0.0, 1.0);
        let z_sq = z * z;
        let denom = 1.0 + z_sq / n;
        let centre = p + z_sq / (2.0 * n);
        let variance = p * (1.0 - p) / n + z_sq / (4.0 * n * n);
        let spread = z * variance.max(0.0).sqrt();
        ((centre - spread) / denom).clamp(0.0, 1.0)
    }
}

impl Default for Posterior {
    fn default() -> Self {
        Self::PRIOR
    }
}

/// Why a bid was declined. A refused opportunity is not repriced — it is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefuseReason {
    /// The edge does not clear the profit hurdle, so no tip can satisfy it.
    EdgeBelowHurdle,
    /// The ceiling cannot fund the protocol minimum tip of 1000 lamports.
    EdgeBelowMinimumTip,
    /// No fresh tip-floor snapshot, so there is no published price to bid against.
    MarketUnavailable,
    /// Every rung priced at or above the hurdle produced a non-positive EV.
    NoPositiveExpectedValue,
    /// The target slot's leader does not run Jito-Solana, so no auction exists.
    LeaderNotJito,
}

impl fmt::Display for RefuseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EdgeBelowHurdle => f.write_str("edge below profit hurdle"),
            Self::EdgeBelowMinimumTip => f.write_str("edge cannot fund the minimum Jito tip"),
            Self::MarketUnavailable => f.write_str("no fresh tip-floor snapshot"),
            Self::NoPositiveExpectedValue => {
                f.write_str("no rung produced positive expected value")
            }
            Self::LeaderNotJito => f.write_str("leader does not run Jito-Solana"),
        }
    }
}

/// The bid itself, or the reason there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TipDecision {
    Bid {
        tip: Lamports,
        rung: Percentile,
        /// True when the escalation cap reduced an otherwise larger bid.
        escalated: bool,
    },
    Refuse {
        reason: RefuseReason,
    },
}

impl TipDecision {
    /// The tip to actually pay, if any.
    pub fn tip(self) -> Option<Lamports> {
        match self {
            Self::Bid { tip, .. } => Some(tip),
            Self::Refuse { .. } => None,
        }
    }

    /// True when a bid may be built.
    pub fn is_bid(self) -> bool {
        matches!(self, Self::Bid { .. })
    }
}

/// Full trace of one pricing decision, kept for audit and telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TipPlan {
    pub decision: TipDecision,
    /// Net profit before the tip is deducted.
    pub edge: Lamports,
    /// Portion of the edge that must survive all costs.
    pub hurdle: Lamports,
    /// Hard price ceiling `t_max`.
    pub ceiling: Lamports,
    /// Land probability used for the winning rung.
    pub inclusion_p: f64,
    /// `inclusion_p * (edge - tip)`, in lamports. Negative when the tip outstrips the edge.
    pub expected_value: i64,
    /// Market snapshot the plan was priced against, if one was fresh.
    pub market: Option<TipFloor>,
    /// Tip paid on the previous attempt in the same window, if any.
    pub previous_tip: Option<Lamports>,
}

impl TipPlan {
    /// True when the plan produced a payable bid.
    pub fn is_bid(&self) -> bool {
        self.decision.is_bid()
    }
}

/// Status of a submitted bundle reported by `getInflightBundleStatuses`.
///
/// The endpoint only looks back five minutes, so `Invalid` means "no longer in
/// the block engine's records" — never "failed to land".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InflightStatus {
    /// Bundle id unknown to the block engine (outside the 5 minute window).
    Invalid,
    /// Not failed, not landed, not invalid.
    Pending,
    /// Every region that received the bundle marked it failed.
    Failed,
    /// Landed on chain.
    Landed { slot: u64 },
}

impl InflightStatus {
    /// True when polling may stop.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Landed { .. } | Self::Failed | Self::Invalid)
    }

    /// Evidence to fold into the landing ledger.
    ///
    /// `Some(true)` landed, `Some(false)` failed, `None` when the status carries
    /// no evidence about the bid — which covers both `Pending` and `Invalid`.
    pub fn evidence(self) -> Option<bool> {
        match self {
            Self::Landed { .. } => Some(true),
            Self::Failed => Some(false),
            Self::Pending | Self::Invalid => None,
        }
    }
}

/// Result type for bot operations.
pub type Result<T> = std::result::Result<T, ArbError>;

/// Unified error types across workspace crates.
#[derive(Error, Debug)]
pub enum ArbError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("overflow in calculation")]
    Overflow,

    #[error("pool not found: {0}")]
    PoolNotFound(PoolId),

    #[error("route not found: {0:?}")]
    RouteNotFound(RouteId),

    #[error("insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: Lamports, need: Lamports },

    #[error("stale state: slot {slot:?} too old, max {max:?}")]
    StaleState { slot: Slot, max: Slot },

    #[error("risk gate denied: {reason}")]
    RiskDenied { reason: String },

    #[error("simulation failed: {0}")]
    SimulationFailed(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("deserialization error: {0}")]
    DeserializationError(String),

    #[error("dimensional arithmetic error: {0}")]
    DimensionalArithmetic(#[from] currency::DimensionalArithmeticError),

    #[error("pump.fun bonding curve complete/migrated")]
    PumpFunComplete,

    #[error("pump.fun reserve exceeded: requested {requested}, available {available}")]
    PumpFunReserveExceeded { requested: u64, available: u64 },
}
