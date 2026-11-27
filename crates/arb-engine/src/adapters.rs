//! Protocol adapters for mixed-AMM routing.
//!
//! Each Solana AMM family (constant product, concentrated liquidity, dynamic
//! liquidity) keeps its own pricing model. A route may mix them freely; every
//! hop runs against a *mutable* view of its pool so that the same pool used
//! twice in one route is updated sequentially instead of being re-quoted against
//! a stale copy.

use arb_dex::engines::UniversalQuoteDispatcher;
use arb_dex::quote_engine::{QuoteRequest, QuoteResult};
use arb_dex::token2022::TransferFeeConfig;
#[cfg(test)]
use arb_math::clmm::{self, TickArrayFacade};
use arb_types::{ArbError, DexProtocol, PoolKind, PoolState, Result};
use arb_types::{MintId, PoolId};

/// Human-readable family labels required in logs, reports and configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmmFamily {
    ConstantProduct,
    ConcentratedLiquidity,
    DynamicLiquidity,
}

impl AmmFamily {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ConstantProduct => "Constant Product",
            Self::ConcentratedLiquidity => "Concentrated Liquidity",
            Self::DynamicLiquidity => "Dynamic Liquidity",
        }
    }
}

impl From<PoolKind> for AmmFamily {
    fn from(kind: PoolKind) -> Self {
        match kind {
            PoolKind::Cpmm => Self::ConstantProduct,
            PoolKind::Clmm => Self::ConcentratedLiquidity,
            PoolKind::Dlmm => Self::DynamicLiquidity,
        }
    }
}

/// One hop of a route, identified by its pool and its input mint.
#[derive(Debug, Clone)]
pub struct HopRequest {
    pub pool_id: PoolId,
    pub mint_in: MintId,
}

/// Result of pricing a single hop. The pool state has already been advanced.
#[derive(Debug, Clone)]
pub struct HopQuote {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub mint_in: MintId,
    pub mint_out: MintId,
    pub family: AmmFamily,
    pub amount_in: u64,
    pub amount_out: u64,
    pub fee_paid: u64,
    /// Mint in which fee_paid and QuoteResult.total_fee are denominated.
    pub fee_mint: MintId,
    /// Creator fee is tracked separately because CPMM may charge it in the output mint.
    pub creator_fee_paid: u64,
    pub creator_fee_mint: MintId,
    pub transfer_fee_paid: u64,
    pub price_impact_bps: u16,
    pub cu_estimate: u64,
    pub accounts_required: u8,
}

/// Result of pricing a complete route.
#[derive(Debug, Clone)]
pub struct RouteQuote {
    pub hops: Vec<HopQuote>,
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub amount_in: u64,
    pub amount_out: u64,
    pub price_impact_bps: u16,
    pub cu_estimate: u64,
    pub accounts_required: u64,
}

/// Conservative compute-unit estimate used only for route ranking.
pub const fn cu_estimate(protocol: DexProtocol) -> u64 {
    match protocol {
        DexProtocol::RaydiumV4 => 65_000,
        DexProtocol::RaydiumCpmm => 45_000,
        DexProtocol::RaydiumClmm => 55_000,
        DexProtocol::OrcaWhirlpool => 50_000,
        DexProtocol::MeteoraDlmm => 60_000,
        DexProtocol::MeteoraDammV2 => 70_000,
        DexProtocol::MeteoraDbc => 55_000,
        DexProtocol::RaydiumLaunchLab => 60_000,
        DexProtocol::PumpFun => 40_000,
        DexProtocol::PumpSwap => 48_000,
    }
}

/// Account count for the swap instruction. Used for transaction-size ranking.
pub const fn accounts_required(protocol: DexProtocol) -> u8 {
    match protocol {
        DexProtocol::RaydiumV4 => 18,
        DexProtocol::RaydiumCpmm => 13,
        DexProtocol::RaydiumClmm => 15,
        DexProtocol::OrcaWhirlpool => 11,
        DexProtocol::MeteoraDlmm => 15,
        DexProtocol::MeteoraDammV2 => 13,
        DexProtocol::MeteoraDbc => 14,
        DexProtocol::RaydiumLaunchLab => 14,
        DexProtocol::PumpFun => 16,
        DexProtocol::PumpSwap => 16,
    }
}

/// Whether a pool can be executed live right now.
///
/// Quote-only pools may be priced for evaluation and ranking but must never reach
/// the transaction builder. This enforces "no live execution until the
/// mathematical implementation matches verified protocol behavior".
pub fn execution_capability(pool: &PoolState) -> std::result::Result<(), &'static str> {
    match pool.protocol {
        DexProtocol::RaydiumV4 => Ok(()),
        DexProtocol::OrcaWhirlpool => {
            if pool.clmm.is_some() {
                Ok(())
            } else {
                Err("Orca pool has no decoded CLMM state")
            }
        }
        DexProtocol::RaydiumCpmm => {
            if pool.is_fee_valid() && pool.reserve_in > 0 && pool.reserve_out > 0 {
                Ok(())
            } else {
                Err("Raydium CPMM missing on-chain fee or reserves")
            }
        }
        DexProtocol::MeteoraDammV2 => Err(
            "Meteora DAMM v2 is quote-only until its complete pool, activation, dynamic-fee, collect-fee, and mint snapshot is verified against the official quote model",
        ),
        DexProtocol::MeteoraDbc => Err(
            "Meteora DBC is quote-only until VirtualPool/PoolConfig, complete curve segments, migration state, epoch, and transfer-fee context are verified",
        ),
        DexProtocol::RaydiumLaunchLab => Err(
            "Raydium LaunchLab is quote-only until PoolState, GlobalConfig, PlatformConfig, fee shares, vaults, and token state are verified",
        ),
        DexProtocol::PumpFun => Err(
            "Pump.fun is quote-only until the current FeeConfig/creator-fee snapshot, V3 account matrix, and exact fill semantics are verified against the live instruction",
        ),
        DexProtocol::PumpSwap => Err(
            "PumpSwap remains quote-only until its dedicated fee/creator-fee quote adapter is accepted",
        ),
        DexProtocol::RaydiumClmm => Err(
            "Raydium CLMM is telemetry-only until the live AmmConfig, dynamic-fee state, and tick arrays are joined at a coherent slot",
        ),
        DexProtocol::MeteoraDlmm => {
            if pool.dlmm.is_some()
                && pool.dlmm.as_ref().unwrap().status == 0
                && !pool.dlmm.as_ref().unwrap().bins.is_empty()
                && pool.dlmm.as_ref().unwrap().params.is_some()
            {
                Ok(())
            } else {
                Err("Meteora DLMM bin arrays or params not ingested")
            }
        }
    }
}

/// Protocol-specific quote adapter contract. Each venue gets an explicit adapter;
/// adapters may share low-level math primitives, but protocol state, fee semantics,
/// direction, and execution readiness are never inferred from the AMM family alone.
pub trait ProtocolQuoteAdapter: Sync {
    fn protocol(&self) -> DexProtocol;
    fn quote_curve(
        &self,
        pool: &mut PoolState,
        amount_in: u64,
        a_to_b: bool,
    ) -> Result<(u64, u64, u16)>;
}

macro_rules! cpmm_adapter {
    ($name:ident, $protocol:expr) => {
        pub struct $name;
        impl ProtocolQuoteAdapter for $name {
            fn protocol(&self) -> DexProtocol {
                $protocol
            }
            fn quote_curve(
                &self,
                pool: &mut PoolState,
                amount_in: u64,
                a_to_b: bool,
            ) -> Result<(u64, u64, u16)> {
                if pool.kind != PoolKind::Cpmm {
                    return Err(ArbError::InvalidInput(
                        "protocol/pool family mismatch".into(),
                    ));
                }
                quote_via_dispatcher(pool, amount_in, a_to_b).map(|quote| {
                    (
                        quote.amount_out_raw,
                        quote.fees_breakdown.total_fee,
                        quote.price_impact.0,
                    )
                })
            }
        }
    };
}

macro_rules! clmm_adapter {
    ($name:ident, $protocol:expr) => {
        pub struct $name;
        impl ProtocolQuoteAdapter for $name {
            fn protocol(&self) -> DexProtocol {
                $protocol
            }
            fn quote_curve(
                &self,
                pool: &mut PoolState,
                amount_in: u64,
                a_to_b: bool,
            ) -> Result<(u64, u64, u16)> {
                if pool.kind != PoolKind::Clmm {
                    return Err(ArbError::InvalidInput(
                        "protocol/pool family mismatch".into(),
                    ));
                }
                quote_via_dispatcher(pool, amount_in, a_to_b).map(|quote| {
                    (
                        quote.amount_out_raw,
                        quote.fees_breakdown.total_fee,
                        quote.price_impact.0,
                    )
                })
            }
        }
    };
}

cpmm_adapter!(RaydiumV4QuoteAdapter, DexProtocol::RaydiumV4);
cpmm_adapter!(RaydiumCpmmQuoteAdapter, DexProtocol::RaydiumCpmm);
clmm_adapter!(RaydiumClmmQuoteAdapter, DexProtocol::RaydiumClmm);
clmm_adapter!(OrcaWhirlpoolQuoteAdapter, DexProtocol::OrcaWhirlpool);

pub struct MeteoraDlmmQuoteAdapter;
impl ProtocolQuoteAdapter for MeteoraDlmmQuoteAdapter {
    fn protocol(&self) -> DexProtocol {
        DexProtocol::MeteoraDlmm
    }
    fn quote_curve(
        &self,
        pool: &mut PoolState,
        amount_in: u64,
        a_to_b: bool,
    ) -> Result<(u64, u64, u16)> {
        if pool.kind != PoolKind::Dlmm {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM family mismatch".into(),
            ));
        }
        quote_via_dispatcher(pool, amount_in, a_to_b).map(|quote| {
            (
                quote.amount_out_raw,
                quote.fees_breakdown.total_fee,
                quote.price_impact.0,
            )
        })
    }
}

pub struct PumpFunQuoteAdapter;
impl ProtocolQuoteAdapter for PumpFunQuoteAdapter {
    fn protocol(&self) -> DexProtocol {
        DexProtocol::PumpFun
    }
    fn quote_curve(
        &self,
        pool: &mut PoolState,
        amount_in: u64,
        a_to_b: bool,
    ) -> Result<(u64, u64, u16)> {
        quote_via_dispatcher(pool, amount_in, a_to_b).map(|quote| {
            (
                quote.amount_out_raw,
                quote.fees_breakdown.total_fee,
                quote.price_impact.0,
            )
        })
    }
}

/// Explicit quote-only adapters for venues whose full per-pool state is not yet
/// represented in `PoolState`. They intentionally refuse family-level fallback.
pub struct QuoteOnlyAdapter {
    pub venue: DexProtocol,
    pub reason: &'static str,
}
impl ProtocolQuoteAdapter for QuoteOnlyAdapter {
    fn protocol(&self) -> DexProtocol {
        self.venue
    }
    fn quote_curve(
        &self,
        _pool: &mut PoolState,
        _amount_in: u64,
        _a_to_b: bool,
    ) -> Result<(u64, u64, u16)> {
        Err(ArbError::InvalidInput(self.reason.into()))
    }
}

static RAYDIUM_V4_ADAPTER: RaydiumV4QuoteAdapter = RaydiumV4QuoteAdapter;
static RAYDIUM_CPMM_ADAPTER: RaydiumCpmmQuoteAdapter = RaydiumCpmmQuoteAdapter;
static RAYDIUM_CLMM_ADAPTER: RaydiumClmmQuoteAdapter = RaydiumClmmQuoteAdapter;
static ORCA_ADAPTER: OrcaWhirlpoolQuoteAdapter = OrcaWhirlpoolQuoteAdapter;
static METEORA_DLMM_ADAPTER: MeteoraDlmmQuoteAdapter = MeteoraDlmmQuoteAdapter;
static METEORA_DAMM_ADAPTER: QuoteOnlyAdapter = QuoteOnlyAdapter {
    venue: DexProtocol::MeteoraDammV2,
    reason: "Meteora DAMM v2 requires the complete official pool/fee/activation snapshot; a CPMM reserve fallback is not valid",
};
static METEORA_DBC_ADAPTER: QuoteOnlyAdapter = QuoteOnlyAdapter {
    venue: DexProtocol::MeteoraDbc,
    reason: "Meteora DBC requires the official VirtualPool/PoolConfig and complete piecewise curve snapshot",
};
static PUMP_FUN_ADAPTER: PumpFunQuoteAdapter = PumpFunQuoteAdapter;
static RAYDIUM_LAUNCHLAB_ADAPTER: QuoteOnlyAdapter = QuoteOnlyAdapter {
    venue: DexProtocol::RaydiumLaunchLab,
    reason: "Raydium LaunchLab requires its own platform/global config and bonding-curve state adapter",
};
static PUMP_SWAP_ADAPTER: QuoteOnlyAdapter = QuoteOnlyAdapter {
    venue: DexProtocol::PumpSwap,
    reason: "PumpSwap requires its own creator/protocol/LP fee and v2 reserve-accounting adapter",
};

pub fn protocol_quote_adapter(protocol: DexProtocol) -> &'static dyn ProtocolQuoteAdapter {
    match protocol {
        DexProtocol::RaydiumV4 => &RAYDIUM_V4_ADAPTER,
        DexProtocol::RaydiumCpmm => &RAYDIUM_CPMM_ADAPTER,
        DexProtocol::RaydiumClmm => &RAYDIUM_CLMM_ADAPTER,
        DexProtocol::OrcaWhirlpool => &ORCA_ADAPTER,
        DexProtocol::MeteoraDlmm => &METEORA_DLMM_ADAPTER,
        DexProtocol::MeteoraDammV2 => &METEORA_DAMM_ADAPTER,
        DexProtocol::MeteoraDbc => &METEORA_DBC_ADAPTER,
        DexProtocol::RaydiumLaunchLab => &RAYDIUM_LAUNCHLAB_ADAPTER,
        DexProtocol::PumpFun => &PUMP_FUN_ADAPTER,
        DexProtocol::PumpSwap => &PUMP_SWAP_ADAPTER,
    }
}

/// Resolve the active fee for a mint from the verified PoolState context.
/// SPL mints have no transfer-fee config; Token-2022 without an epoch snapshot
/// never defaults to zero.
fn transfer_fee_config_for_mint(
    pool: &PoolState,
    mint: MintId,
) -> Result<Option<TransferFeeConfig>> {
    let index = if mint == pool.base_mint {
        0
    } else if mint == pool.quote_mint {
        1
    } else {
        return Err(ArbError::InvalidToken(
            "mint is not part of the pool".into(),
        ));
    };
    match pool.token_programs[index] {
        arb_types::TokenProgramKind::SplToken => Ok(None),
        arb_types::TokenProgramKind::Token2022 => {
            if pool.has_risky_token {
                return Err(ArbError::InvalidToken(
                    "Token-2022 mint has unsupported extensions or stale fee context".into(),
                ));
            }
            let context = pool.token2022_fee_context.ok_or_else(|| {
                ArbError::IncompleteState("Token-2022 Clock/fee snapshot missing".into())
            })?;
            if !context.is_valid_for(pool.token_programs) {
                return Err(ArbError::IncompleteState(
                    "Token-2022 fee context does not match mint programs/snapshot".into(),
                ));
            }
            let fee = if index == 0 {
                context.base
            } else {
                context.quote
            }
            .ok_or_else(|| ArbError::IncompleteState("Token-2022 fee schedule missing".into()))?;
            Ok(Some(TransferFeeConfig {
                transfer_fee_basis_points: fee.basis_points,
                maximum_fee: fee.maximum_fee,
            }))
        }
    }
}

/// Build the one authoritative exact-in quote used by route planning and adapters.
/// Fee schedules come only from verified, epoch-aware PoolState context.
pub(crate) fn quote_via_dispatcher(
    pool: &PoolState,
    amount_in: u64,
    a_to_b: bool,
) -> Result<QuoteResult> {
    let (input_mint, output_mint) = if a_to_b {
        (pool.base_mint, pool.quote_mint)
    } else {
        (pool.quote_mint, pool.base_mint)
    };
    let request = QuoteRequest {
        input_mint,
        output_mint,
        amount: amount_in,
        slippage_bps: arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
        input_transfer_fee_config: transfer_fee_config_for_mint(pool, input_mint)?,
        output_transfer_fee_config: transfer_fee_config_for_mint(pool, output_mint)?,
    };
    let quote = UniversalQuoteDispatcher::new().quote_exact_in(pool, &request)?;
    if !quote.quote_validity
        || quote.amount_in_raw != amount_in
        || quote.unfilled_input_raw != 0
        || quote.amount_out_raw == 0
    {
        return Err(ArbError::InvalidInput(
            "authoritative quote dispatcher returned an incomplete/invalid hop".into(),
        ));
    }
    Ok(quote)
}

/// Advance CPMM reserve state from the authoritative quote. Pump.fun, CLMM and
/// DLMM post-swap state is not fully represented by QuoteResult; route planning
/// rejects pool reuse, so those protocols remain immutable snapshot quotes.
fn advance_pool_after_quote(pool: &mut PoolState, a_to_b: bool, quote: &QuoteResult) -> Result<()> {
    let input = u128::from(quote.transfer_fee_breakdown.input_net_into_pool);
    let output = u128::from(quote.transfer_fee_breakdown.output_curve_reserve_debit);
    match pool.protocol {
        DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm => {
            let (reserve_in, reserve_out) = if a_to_b {
                (pool.reserve_in, pool.reserve_out)
            } else {
                (pool.reserve_out, pool.reserve_in)
            };
            // CPMM vaults retain protocol/fund/creator fees physically, but
            // those buckets are excluded from the curve reserves. Only the LP
            // share of the trade fee remains in the invariant.
            let curve_input_credit = if pool.protocol == DexProtocol::RaydiumCpmm {
                let schedule = pool.raydium_cpmm_fee.ok_or_else(|| {
                    ArbError::IncompleteState(
                        "Raydium CPMM fee schedule missing during state advance".into(),
                    )
                })?;
                let creator_on_input = schedule.creator_fee_on_input(a_to_b).ok_or_else(|| {
                    ArbError::InvalidInput("Raydium CPMM creator fee direction is invalid".into())
                })?;
                let non_lp_fees = quote
                    .fees_breakdown
                    .protocol_fee
                    .checked_add(quote.fees_breakdown.fund_fee)
                    .and_then(|v| {
                        v.checked_add(if creator_on_input {
                            quote.fees_breakdown.creator_fee
                        } else {
                            0
                        })
                    })
                    .ok_or(ArbError::Overflow)?;
                input.checked_sub(u128::from(non_lp_fees)).ok_or_else(|| {
                    ArbError::InvalidInput(
                        "Raydium CPMM non-LP fee buckets exceed pool input".into(),
                    )
                })?
            } else {
                input
            };
            let next_in = reserve_in
                .checked_add(curve_input_credit)
                .ok_or(ArbError::Overflow)?;
            let next_out = reserve_out.checked_sub(output).ok_or_else(|| {
                ArbError::InsufficientLiquidity("quote output exceeds reserve snapshot".into())
            })?;
            if a_to_b {
                pool.reserve_in = next_in;
                pool.reserve_out = next_out;
            } else {
                pool.reserve_out = next_in;
                pool.reserve_in = next_out;
            }
        }
        DexProtocol::PumpFun
        | DexProtocol::OrcaWhirlpool
        | DexProtocol::RaydiumClmm
        | DexProtocol::MeteoraDlmm => {}
        DexProtocol::MeteoraDammV2
        | DexProtocol::MeteoraDbc
        | DexProtocol::RaydiumLaunchLab
        | DexProtocol::PumpSwap => {
            return Err(ArbError::UnsupportedOperation(format!(
                "state advancement for {:?} is not supported by this route facade",
                pool.protocol
            )));
        }
    }
    Ok(())
}

/// Price one hop through the canonical DEX dispatcher. Mutable state is retained
/// for compatibility; CPMM reserves are advanced from the returned quote. Routes
/// reject pool reuse so snapshot-only protocols cannot be accidentally re-quoted.
pub fn quote_hop(pool: &mut PoolState, amount_in: u64, a_to_b: bool) -> Result<HopQuote> {
    if amount_in == 0 {
        return Err(ArbError::InvalidInput("zero input".into()));
    }
    if !pool.is_active {
        return Err(ArbError::InvalidInput("pool inactive".into()));
    }
    if !pool.is_fee_valid() {
        return Err(ArbError::InvalidInput(
            "pool has an invalid fee (zero denominator)".into(),
        ));
    }
    if let Err(reason) = execution_capability(pool) {
        return Err(ArbError::InvalidInput(reason.into()));
    }
    if pool.has_risky_token
        || pool.transfer_fee_bps != 0
        || pool.token_programs != [arb_types::TokenProgramKind::SplToken; 2]
    {
        return Err(ArbError::InvalidInput(
            "Token-2022/risky mint requires a quote request with verified per-mint epoch fee schedules".into(),
        ));
    }

    let protocol = pool.protocol;
    let mint_in = if a_to_b {
        pool.base_mint
    } else {
        pool.quote_mint
    };
    let mint_out = if a_to_b {
        pool.quote_mint
    } else {
        pool.base_mint
    };
    let quote = quote_via_dispatcher(pool, amount_in, a_to_b)?;
    advance_pool_after_quote(pool, a_to_b, &quote)?;

    let transfer_fee_paid = quote
        .transfer_fee_breakdown
        .input_transfer_fee
        .checked_add(quote.transfer_fee_breakdown.output_transfer_fee)
        .ok_or(ArbError::Overflow)?;
    Ok(HopQuote {
        pool_id: pool.pool_id,
        protocol,
        mint_in,
        mint_out,
        family: AmmFamily::from(pool.kind),
        amount_in,
        amount_out: quote.amount_out_raw,
        fee_paid: quote.fees_breakdown.total_fee,
        fee_mint: quote.total_fee_mint,
        creator_fee_paid: quote.fees_breakdown.creator_fee,
        creator_fee_mint: quote.creator_fee_mint,
        transfer_fee_paid,
        price_impact_bps: quote.price_impact.0,
        cu_estimate: cu_estimate(protocol),
        accounts_required: accounts_required(protocol),
    })
}

/// Price a complete route. Pools are mutated sequentially on a private route-local
/// view. A pool may appear only once in a route candidate; mint continuity is
/// checked between every adjacent hop.
pub fn quote_route(
    pools: &mut [PoolState],
    hops: &[HopRequest],
    amount_in: u64,
) -> Result<RouteQuote> {
    if hops.is_empty() {
        return Err(ArbError::InvalidInput("empty route".into()));
    }
    let input_mint = hops[0].mint_in;
    let mut current_mint = input_mint;
    let mut current = amount_in;
    let mut hop_quotes = Vec::with_capacity(hops.len());
    let mut total_cu = 0u64;
    let mut total_accounts = 0u64;
    let mut used_pools = std::collections::HashSet::with_capacity(hops.len());

    for hop in hops {
        if hop.mint_in != current_mint {
            return Err(ArbError::InvalidInput(
                "route mint discontinuity: next hop input does not equal previous hop output"
                    .into(),
            ));
        }
        if !used_pools.insert(hop.pool_id) {
            return Err(ArbError::InvalidInput(
                "route reuses a pool; cyclic pool reuse is not accepted by the route planner"
                    .into(),
            ));
        }
        let pool = pools
            .iter_mut()
            .find(|p| p.pool_id == hop.pool_id)
            .ok_or(ArbError::PoolNotFound(hop.pool_id))?;

        let a_to_b = if pool.base_mint == current_mint {
            true
        } else if pool.quote_mint == current_mint {
            false
        } else {
            return Err(ArbError::InvalidInput(
                "hop input mint does not belong to selected pool".into(),
            ));
        };

        let q = quote_hop(pool, current, a_to_b)?;
        current = q.amount_out;
        current_mint = q.mint_out;
        total_cu = total_cu.saturating_add(q.cu_estimate);
        total_accounts = total_accounts.saturating_add(q.accounts_required as u64);
        hop_quotes.push(q);
    }

    // Compounded price impact: 1 - prod(1 - impact_i).
    let mut remaining: u128 = 1_000_000_000;
    for q in &hop_quotes {
        let factor = 1_000_000_000u128.saturating_sub(q.price_impact_bps as u128 * 100_000);
        remaining = remaining * factor / 1_000_000_000;
    }
    let compounded = 1_000_000_000u128.saturating_sub(remaining);
    let compounded_bps = (compounded / 100_000) as u16;

    Ok(RouteQuote {
        hops: hop_quotes,
        input_mint,
        output_mint: current_mint,
        amount_in,
        amount_out: current,
        price_impact_bps: compounded_bps,
        cu_estimate: total_cu,
        accounts_required: total_accounts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{ClmmTick, ClmmTickArray, MintId, PoolExecutionAccounts, PoolId, Slot};
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    fn cpmm_pool(id: u8, r_in: u128, r_out: u128, fee_bps: u16) -> PoolState {
        PoolState {
            pool_id: PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: r_in,
            reserve_out: r_out,
            fee_numerator: fee_bps as u64,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    #[test]
    fn dispatcher_applies_epoch_selected_token2022_input_and_output_fees() {
        let mut pool = cpmm_pool(44, 10_000_000, 20_000_000, 25);
        let input_fee = arb_types::Token2022TransferFee {
            basis_points: 100,
            maximum_fee: 10_000,
        };
        pool.token_programs = [
            arb_types::TokenProgramKind::Token2022,
            arb_types::TokenProgramKind::SplToken,
        ];
        pool.token2022_fee_context = Some(arb_types::Token2022FeeContext {
            snapshot_slot: 123,
            epoch: 456,
            base: Some(input_fee),
            quote: None,
        });

        let input_quote = quote_via_dispatcher(&pool, 100_000, true).unwrap();
        assert_eq!(input_quote.transfer_fee_breakdown.input_transfer_fee, 1_000);
        assert_eq!(input_quote.transfer_fee_breakdown.output_transfer_fee, 0);

        pool.token_programs = [
            arb_types::TokenProgramKind::SplToken,
            arb_types::TokenProgramKind::Token2022,
        ];
        pool.token2022_fee_context = Some(arb_types::Token2022FeeContext {
            snapshot_slot: 123,
            epoch: 456,
            base: None,
            quote: Some(input_fee),
        });
        let output_quote = quote_via_dispatcher(&pool, 100_000, true).unwrap();
        assert_eq!(output_quote.transfer_fee_breakdown.input_transfer_fee, 0);
        assert!(output_quote.transfer_fee_breakdown.output_transfer_fee > 0);
    }

    fn clmm_pool(id: u8, sqrt_price: u128, liquidity: u128, tick: i32, spacing: u16) -> PoolState {
        PoolState {
            pool_id: PoolId::new([id; 32]),
            protocol: DexProtocol::OrcaWhirlpool,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: liquidity,
            reserve_out: sqrt_price,
            fee_numerator: 30,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Clmm,
            clmm: Some(arb_types::ClmmState {
                sqrt_price,
                liquidity,
                tick_current: tick,
                tick_spacing: spacing,
                fee_rate: 3_000,
                // Exact CLMM traversal is fail-closed without the array that
                // covers the active tick: it refuses to synthesize a boundary
                // from tick spacing alone. Supply the array above and below the
                // active tick so a quote can run in either direction.
                tick_arrays: vec![
                    ClmmTickArray {
                        start_tick_index: -88 * i32::from(spacing),
                        account_pubkey: None,
                        account_slot: None,
                        ticks: vec![],
                    },
                    ClmmTickArray {
                        start_tick_index: 0,
                        account_pubkey: None,
                        account_slot: None,
                        ticks: vec![],
                    },
                ],
            }),
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    fn raydium_clmm_pool(
        id: u8,
        sqrt_price: u128,
        liquidity: u128,
        tick: i32,
        spacing: u16,
    ) -> PoolState {
        // Raydium CLMM tick arrays start on 60*spacing multiples (not the
        // Whirlpool 88), and hydration derives the array containing the
        // active tick: around tick 7@ts 1 that array starts at 60.
        PoolState {
            pool_id: PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumClmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: liquidity,
            reserve_out: sqrt_price,
            fee_numerator: 30,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Clmm,
            clmm: Some(arb_types::ClmmState {
                sqrt_price,
                liquidity,
                tick_current: tick,
                tick_spacing: spacing,
                fee_rate: 2_500,
                tick_arrays: vec![
                    ClmmTickArray {
                        start_tick_index: 60 * i32::from(spacing),
                        account_pubkey: None,
                        account_slot: None,
                        ticks: vec![],
                    },
                    ClmmTickArray {
                        start_tick_index: 120 * i32::from(spacing),
                        account_pubkey: None,
                        account_slot: None,
                        ticks: vec![],
                    },
                ],
            }),
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    #[test]
    fn raydium_clmm_route_is_disabled_until_amm_config_snapshot_is_verified() {
        // Regression: both engine consumers defaulted to the Whirlpool
        // 88-tick span and Orca tick rounding. With the active tick inside
        // the second 60-spaced array (start 60), the 88-span lookup aligned
        // down to array start 0, which was never hydrated, so every Raydium
        // CLMM hop through this engine failed closed and Raydium liquidity
        // was invisible to route discovery.
        let legacy = clmm::compute_swap(
            1u128 << 64,
            70,
            1_000_000_000_000,
            1,
            2_500,
            10_000,
            true,
            &[
                TickArrayFacade {
                    start_tick_index: 60,
                    ticks: vec![],
                },
                TickArrayFacade {
                    start_tick_index: 120,
                    ticks: vec![],
                },
            ],
        );
        assert_eq!(
            legacy.unwrap_err(),
            arb_math::clmm::ClmmMathError::MissingTickArrayCoverage(0)
        );

        let mut pools = vec![raydium_clmm_pool(3, 1u128 << 64, 1_000_000_000_000, 70, 1)];
        let hops = vec![HopRequest {
            pool_id: PoolId::new([3; 32]),
            mint_in: MintId(Pubkey::new_from_array([1; 32])),
        }];
        let result = quote_route(&mut pools, &hops, 10_000);
        assert!(
            result.is_err(),
            "CLMM route must fail closed without verified AmmConfig"
        );
    }

    #[test]
    fn raydium_cpmm_state_advance_excludes_non_lp_fee_buckets() {
        for (a_to_b, creator_fee_on) in [(true, 1u8), (false, 1u8)] {
            let mut pool = cpmm_pool(44, 10_000_000_000, 20_000_000_000, 25);
            pool.protocol = DexProtocol::RaydiumCpmm;
            pool.fee_numerator = 2_500;
            pool.fee_denominator = 1_000_000;
            pool.raydium_cpmm_fee = Some(arb_types::RaydiumCpmmFeeState {
                trade_fee_rate_ppm: 2_500,
                creator_fee_rate_ppm: 500,
                protocol_fee_rate: 120_000,
                fund_fee_rate: 40_000,
                enable_creator_fee: true,
                creator_fee_on,
            });
            let amount_in = 1_000_000u64;
            let quote = quote_via_dispatcher(&pool, amount_in, a_to_b).unwrap();
            let reserve_in_before = if a_to_b {
                pool.reserve_in
            } else {
                pool.reserve_out
            };
            let reserve_out_before = if a_to_b {
                pool.reserve_out
            } else {
                pool.reserve_in
            };
            let creator_on_input = pool
                .raydium_cpmm_fee
                .unwrap()
                .creator_fee_on_input(a_to_b)
                .unwrap();
            let non_lp = quote.fees_breakdown.protocol_fee
                + quote.fees_breakdown.fund_fee
                + if creator_on_input {
                    quote.fees_breakdown.creator_fee
                } else {
                    0
                };
            let expected_input_credit = u128::from(amount_in - non_lp);
            let expected_output_debit =
                u128::from(quote.transfer_fee_breakdown.output_curve_reserve_debit);

            advance_pool_after_quote(&mut pool, a_to_b, &quote).unwrap();
            let reserve_in_after = if a_to_b {
                pool.reserve_in
            } else {
                pool.reserve_out
            };
            let reserve_out_after = if a_to_b {
                pool.reserve_out
            } else {
                pool.reserve_in
            };
            assert_eq!(reserve_in_after, reserve_in_before + expected_input_credit);
            assert_eq!(
                reserve_out_after,
                reserve_out_before - expected_output_debit
            );
        }
    }

    #[test]
    fn cpmm_route_price_impact_excludes_upfront_trade_fee() {
        let mut pool = cpmm_pool(7, 1_000_000, 1_000_000, 100);
        let amount_in = 100_000u64;
        let quote = quote_hop(&mut pool, amount_in, true).unwrap();
        let input_after_fee = amount_in.checked_sub(quote.fee_paid).unwrap();
        let expected = arb_math::cpmm::cpmm_price_impact_bps(
            u128::from(input_after_fee),
            1_000_000,
            1_000_000,
            0,
        )
        .0;
        let impact_if_fee_were_double_counted =
            arb_math::cpmm::cpmm_price_impact_bps(u128::from(amount_in), 1_000_000, 1_000_000, 0).0;
        assert!(quote.amount_out > 0);
        assert_eq!(quote.price_impact_bps, expected);
        assert!(quote.price_impact_bps < impact_if_fee_were_double_counted);
    }

    #[test]
    fn cpmm_then_clmm_quotes() {
        let mut pools = vec![
            cpmm_pool(1, 1_000_000, 1_000_000, 25),
            clmm_pool(2, 1u128 << 64, 1_000_000_000_000, 0, 4),
        ];
        let hops = vec![
            HopRequest {
                pool_id: PoolId::new([1; 32]),
                mint_in: MintId(Pubkey::new_from_array([1; 32])),
            },
            HopRequest {
                pool_id: PoolId::new([2; 32]),
                mint_in: MintId(Pubkey::new_from_array([2; 32])),
            },
        ];
        let q = quote_route(&mut pools, &hops, 10_000).unwrap();
        assert_eq!(q.hops.len(), 2);
        assert!(q.amount_out > 0);
        assert_eq!(
            q.cu_estimate,
            cu_estimate(DexProtocol::RaydiumV4) + cu_estimate(DexProtocol::OrcaWhirlpool)
        );
    }

    #[test]
    fn repeated_pool_reuse_is_rejected() {
        // The unified planner is acyclic: reusing one pool for both legs is
        // rejected instead of being quoted as a round-trip that can only lose
        // to fees.
        let mut pools = vec![cpmm_pool(1, 1_000_000, 1_000_000, 25)];
        let hops = vec![
            HopRequest {
                pool_id: PoolId::new([1; 32]),
                mint_in: MintId(Pubkey::new_from_array([1; 32])),
            },
            HopRequest {
                pool_id: PoolId::new([1; 32]),
                mint_in: MintId(Pubkey::new_from_array([2; 32])),
            },
        ];
        let err = quote_route(&mut pools, &hops, 10_000).unwrap_err();
        assert!(
            matches!(err, ArbError::InvalidInput(_)),
            "expected InvalidInput for pool reuse, got {err:?}"
        );
    }

    #[test]
    fn reverse_cpmm_hop_uses_quote_reserve_as_input_reserve() {
        let mut p = cpmm_pool(1, 1_000_000, 2_000_000, 25);
        let quote_mint = p.quote_mint;
        let q = quote_hop(&mut p, 10_000, false).unwrap();
        assert!(
            q.amount_out < 6_000,
            "reverse direction must use 2m input / 1m output reserves"
        );
        assert!(
            p.reserve_out > 2_000_000,
            "reverse input reserve must increase"
        );
        assert!(
            p.reserve_in < 1_000_000,
            "reverse output reserve must decrease"
        );
        assert_eq!(quote_mint, MintId(Pubkey::new_from_array([2; 32])));
    }

    #[test]
    fn cpmm_reserve_advance_is_exact_and_never_saturates() {
        // Forward hop: reserve_in += effective_in, reserve_out -= amount_out,
        // with exact integer accounting (no saturation, no drift).
        let mut p = cpmm_pool(1, 1_000_000, 2_000_000, 25);
        let (r_in, r_out) = (p.reserve_in, p.reserve_out);
        let amount_in = 500_000u64;
        let q = quote_hop(&mut p, amount_in, true).unwrap();
        assert!(
            u128::from(q.amount_out) < r_out,
            "a CPMM swap cannot drain the pool"
        );
        assert_eq!(p.reserve_in, r_in + u128::from(amount_in));
        assert_eq!(p.reserve_out, r_out - u128::from(q.amount_out));

        // Near-drain: a swap that consumes almost the entire output reserve
        // still succeeds with exact bookkeeping.
        let mut p = cpmm_pool(2, 10_000, 1_000_000, 25);
        let q = quote_hop(&mut p, u64::MAX / 2, true).unwrap();
        assert!(q.amount_out > 0);
        assert!(p.reserve_out < 1_000_000);
        assert_eq!(p.reserve_in, 10_000 + u128::from(u64::MAX / 2));

        // Reverse hop advances the opposite pair exactly.
        let mut p = cpmm_pool(3, 1_000_000, 2_000_000, 25);
        let (r_in, r_out) = (p.reserve_in, p.reserve_out);
        let q = quote_hop(&mut p, 10_000, false).unwrap();
        assert_eq!(p.reserve_out, r_out + u128::from(10_000u64));
        assert_eq!(p.reserve_in, r_in - u128::from(q.amount_out));
    }

    #[test]
    fn token2022_transfer_fee_is_rejected_without_verified_mint_schedule() {
        let mut p = cpmm_pool(1, 1_000_000, 1_000_000, 25);
        p.transfer_fee_bps = 100; // Legacy approximation is deliberately unsupported.
        assert!(quote_hop(&mut p, 10_000, true).is_err());
    }

    #[test]
    fn family_labels() {
        assert_eq!(PoolKind::Cpmm.label(), "Constant Product");
        assert_eq!(PoolKind::Clmm.label(), "Concentrated Liquidity");
        assert_eq!(PoolKind::Dlmm.label(), "Dynamic Liquidity");
        assert_eq!(AmmFamily::from(PoolKind::Dlmm).label(), "Dynamic Liquidity");
    }

    #[test]
    fn clmm_tick_crossing() {
        // Pool with a tick array so the swap can cross a tick boundary.
        let mut p = clmm_pool(1, 1u128 << 64, 1_000_000_000_000, 0, 4);
        if let Some(c) = p.clmm.as_mut() {
            // The array containing the current tick (start 0) is always loaded
            // by the program (empty of initialized ticks here); the next array
            // down holds the crossing ticks.
            c.tick_arrays = vec![
                ClmmTickArray {
                    start_tick_index: 0,
                    account_pubkey: None,
                    account_slot: None,
                    ticks: Vec::new(),
                },
                ClmmTickArray {
                    start_tick_index: -88 * 4,
                    account_pubkey: None,
                    account_slot: None,
                    ticks: (0..88)
                        .map(|i| ClmmTick {
                            tick_index: -88 * 4 + i * 4,
                            initialized: true,
                            liquidity_net: 500_000_000_000,
                            limit_order_unfilled: 0,
                        })
                        .collect(),
                },
            ];
            // seed the active tick
            c.tick_arrays[1].ticks[0].liquidity_net = 500_000_000_000;
        }
        let q = quote_hop(&mut p, 1_000_000, true);
        assert!(q.is_ok(), "CLMM quote with tick arrays should succeed");
    }

    #[test]
    fn dlmm_requires_params() {
        let mut p = PoolState {
            pool_id: PoolId::new([3; 32]),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 1_000_000,
            reserve_out: 1_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(arb_types::DlmmState {
                active_bin_id: 0,
                status: 0,
                bin_step_bps: 25,
                bins: vec![arb_types::DlmmBin {
                    bin_id: 0,
                    amount_x: 1_000_000,
                    amount_y: 1_000_000,
                    ..Default::default()
                }],
                params: None,
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
                last_update_timestamp: 0,
                ..Default::default()
            }),
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        };
        // Without params, quoting must refuse (honest).
        assert!(quote_hop(&mut p, 10_000, true).is_err());
    }

    /// Regression: the canonical DLMM dispatcher must forward the ingested on-chain
    /// `collect_fee_mode` (LbPair byte @36) into the quote engine instead of
    /// hardcoding `0` (InputOnly).
    ///
    /// Discriminator: single bin with protocol-valid bin_step=400 and active_id=35.
    /// With base_factor=100 and power=0, the protocol fee rate is 0.04%:
    /// - InputOnly -> fee charged on input (X raw)
    /// - OnlyY     -> fee charged on output (Y raw) for X-to-Y swaps.
    ///
    /// `amount_out` is algebraically identical in both modes (fee deduction
    /// commutes with the per-bin price multiplication); only the fee
    /// denomination differs. This is why the hardcode never corrupted profit
    /// gating but did mislabel every OnlyY fee line.
    fn dlmm_mode_pool(collect_fee_mode: u8) -> PoolState {
        let pool_id = PoolId::new([7; 32]);
        let base_mint = MintId(Pubkey::new_from_array([1; 32]));
        let quote_mint = MintId(Pubkey::new_from_array([2; 32]));
        let bin_array_index =
            arb_dex::engines::MeteoraDlmmQuoteEngine::bin_id_to_bin_array_index(35);
        let (bin_array_pda, _) = arb_dex::engines::MeteoraDlmmQuoteEngine::derive_bin_array_pda(
            &pool_id.0,
            bin_array_index,
        );
        PoolState {
            pool_id,
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint,
            quote_mint,
            decimals_a: 6,
            decimals_b: 9,
            reserve_in: 0,
            reserve_out: 0,
            fee_numerator: 0,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(arb_types::DlmmState {
                active_bin_id: 35,
                status: 0,
                bin_step_bps: 400,
                bins: vec![arb_types::DlmmBin {
                    bin_id: 35,
                    amount_x: 10u128.pow(12),
                    amount_y: 4 * 10u128.pow(12),
                    ..Default::default()
                }],
                params: Some(arb_types::DlmmFeeParams {
                    base_factor: 100,
                    filter_period: 30,
                    decay_period: 600,
                    reduction_factor: 5000,
                    variable_fee_control: 0,
                    max_volatility_accumulator: 0,
                    protocol_share: 0,
                    base_fee_power_factor: 0,
                    function_type: 0,
                    collect_fee_mode,
                }),
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 35,
                last_update_timestamp: 0,
                ..Default::default()
            }),
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                bin_array_pdas: vec![bin_array_pda],
                bitmap_extension: None,
                pool_token_x: base_mint.0,
                pool_token_y: quote_mint.0,
            }),
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    #[test]
    fn dlmm_quote_honors_ingested_collect_fee_mode() {
        let mut input_only = dlmm_mode_pool(0);
        let mut only_y = dlmm_mode_pool(1);

        let (out_input_only, fee_input_only) = {
            let q =
                quote_hop(&mut input_only, 10u64.pow(9), true).expect("InputOnly pool must quote");
            (q.amount_out, q.fee_paid)
        };
        let (out_only_y, fee_only_y) = {
            let q = quote_hop(&mut only_y, 10u64.pow(9), true).expect("OnlyY pool must quote");
            (q.amount_out, q.fee_paid)
        };

        // Fee denomination: OnlyY charges about 0.04% on the gross OUTPUT (~3.95e9 Y raw)
        // => ~1.58e6; InputOnly charges about 0.04% on 1e9 input => ~4e5.
        assert!(
            (1_500_000..=1_700_000).contains(&fee_only_y),
            "OnlyY fee must be output-denominated (~1.58e6), got {fee_only_y}"
        );
        assert!(
            (300_000..=500_000).contains(&fee_input_only),
            "InputOnly fee must be input-denominated (~4e5), got {fee_input_only}"
        );

        // Net output is identical in both modes up to rounding dust.
        let diff = out_input_only.abs_diff(out_only_y);
        assert!(
            diff <= 1_000,
            "amount_out must be mode-invariant up to rounding dust: {out_input_only} vs {out_only_y}"
        );
    }

    #[test]
    fn dlmm_quote_rejects_invalid_collect_fee_mode() {
        let mut p = dlmm_mode_pool(2);
        assert!(
            quote_hop(&mut p, 10u64.pow(9), true).is_err(),
            "invalid on-chain collect_fee_mode must fail closed, not silently quote as InputOnly"
        );
    }

    #[test]
    fn generic_pool_state_refuses_pumpswap_without_dedicated_state() {
        let mut p = cpmm_pool(99, 1_000_000, 2_000_000, 30);
        p.protocol = DexProtocol::PumpSwap;
        assert!(quote_hop(&mut p, 10_000, true).is_err());
        assert!(execution_capability(&p).is_err());
    }

    #[test]
    fn pump_fun_is_not_execution_ready_until_fee_and_v3_instruction_parity() {
        let pool_id = PoolId::new([10; 32]);
        let base_mint = MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap());
        let quote_mint = MintId(Pubkey::new_from_array([2; 32]));

        let mut p = PoolState {
            pool_id,
            protocol: DexProtocol::PumpFun,
            slot: Slot(1),
            write_version: 1,
            base_mint,
            quote_mint,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 30_000_000_000,
            reserve_out: 1_073_000_000_000_000,
            fee_numerator: 125,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: Some(arb_types::PumpFunState {
                virtual_token_reserves: 1_073_000_000_000_000,
                virtual_sol_reserves: 30_000_000_000,
                real_token_reserves: 793_100_000_000_000,
                real_sol_reserves: 5_000_000_000,
                token_total_supply: 1_000_000_000_000_000,
                complete: false,
                creator: solana_pubkey::Pubkey::new_unique(),
                is_mayhem_mode: false,
                is_cashback_coin: false,
                mode_flags_present: true,
            }),
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        };

        // The pool state does not include the complete fee-component schedule,
        // creator-fee override, or the V3 instruction/account model. Do not allow
        // this estimate to pass into live route sizing or transaction compilation.
        assert!(execution_capability(&p).is_err());
        assert!(quote_hop(&mut p, 1_000_000_000, true).is_err());
        assert!(quote_hop(&mut p, 1_000_000_000, false).is_err());
    }

    #[test]
    fn route_rejects_mint_discontinuity_before_second_quote() {
        let first = cpmm_pool(101, 1_000_000, 2_000_000, 25);
        let mut second = cpmm_pool(102, 1_000_000, 2_000_000, 25);
        second.base_mint = MintId(Pubkey::new_from_array([3; 32]));
        second.quote_mint = MintId(Pubkey::new_from_array([4; 32]));
        let wrong_next_mint = second.base_mint;
        let hops = [
            HopRequest {
                pool_id: first.pool_id,
                mint_in: first.base_mint,
            },
            HopRequest {
                pool_id: second.pool_id,
                mint_in: wrong_next_mint,
            },
        ];
        let err = quote_route(&mut [first.clone(), second], &hops, 10_000).unwrap_err();
        assert!(err.to_string().contains("mint discontinuity"));
    }

    #[test]
    fn every_protocol_has_an_explicit_adapter_registration() {
        let protocols = [
            DexProtocol::RaydiumV4,
            DexProtocol::RaydiumCpmm,
            DexProtocol::RaydiumClmm,
            DexProtocol::OrcaWhirlpool,
            DexProtocol::MeteoraDlmm,
            DexProtocol::MeteoraDammV2,
            DexProtocol::RaydiumLaunchLab,
            DexProtocol::PumpFun,
            DexProtocol::PumpSwap,
        ];
        for protocol in protocols {
            assert_eq!(protocol_quote_adapter(protocol).protocol(), protocol);
        }
    }
}
