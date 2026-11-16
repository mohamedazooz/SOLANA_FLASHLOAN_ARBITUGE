//! Protocol adapters for mixed-AMM routing.
//!
//! Each Solana AMM family (constant product, concentrated liquidity, dynamic
//! liquidity) keeps its own pricing model. A route may mix them freely; every
//! hop runs against a *mutable* view of its pool so that the same pool used
//! twice in one route is updated sequentially instead of being re-quoted against
//! a stale copy.

use arb_dex::PumpFunBondingCurve;
use arb_math::clmm::{self, TickArrayFacade, TickFacade};
use arb_math::dlmm::{self, DlmmBin, DlmmParameters, DlmmPoolState};
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
            "Meteora DAMM v2 remains quote-only until its dedicated fee/curve adapter is accepted",
        ),
        DexProtocol::RaydiumLaunchLab => {
            Err("Raydium LaunchLab remains quote-only until its dedicated fee adapter is accepted")
        }
        DexProtocol::PumpFun => {
            if let Some(ref pf) = pool.pump_fun {
                if pf.is_tradeable() {
                    Ok(())
                } else {
                    Err("Pump.fun bonding curve not tradeable or complete")
                }
            } else {
                Err("Pump.fun pool missing bonding curve state")
            }
        }
        DexProtocol::PumpSwap => Err(
            "PumpSwap remains quote-only until its dedicated fee/creator-fee quote adapter is accepted",
        ),
        DexProtocol::RaydiumClmm => {
            if pool.clmm.is_some() {
                Ok(())
            } else {
                Err("Raydium CLMM pool has no decoded CLMM state")
            }
        }
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
                quote_cpmm_hop(pool, amount_in, a_to_b, $protocol)
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
                quote_clmm_hop(pool, amount_in, a_to_b)
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
        quote_dlmm_hop(pool, amount_in, a_to_b)
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
        quote_pump_fun_hop(pool, amount_in, a_to_b)
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
static PUMP_FUN_ADAPTER: PumpFunQuoteAdapter = PumpFunQuoteAdapter;
static METEORA_DAMM_ADAPTER: QuoteOnlyAdapter = QuoteOnlyAdapter {
    venue: DexProtocol::MeteoraDammV2,
    reason: "Meteora DAMM v2 requires its own layout-version, liquidity-handler, and compounding-fee state adapter",
};
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
        DexProtocol::RaydiumLaunchLab => &RAYDIUM_LAUNCHLAB_ADAPTER,
        DexProtocol::PumpFun => &PUMP_FUN_ADAPTER,
        DexProtocol::PumpSwap => &PUMP_SWAP_ADAPTER,
    }
}

/// Price one hop against a *mutable* pool. The pool's reserves/liquidity/sqrt-price
/// are advanced so that a second hop on the same pool sees the updated state.
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

    // A BPS-only approximation cannot model Token-2022's epoch schedule,
    // maximum fee, or per-mint transfer semantics. Never price it as a swap.
    if pool.has_risky_token || pool.transfer_fee_bps != 0 {
        return Err(ArbError::InvalidInput(
            "Token-2022/risky mint requires a verified per-mint TransferFeeConfig quote".into(),
        ));
    }
    let effective_in = amount_in;
    let transfer_fee = 0u64;

    let protocol = pool.protocol;
    let adapter = protocol_quote_adapter(protocol);
    if adapter.protocol() != protocol {
        return Err(ArbError::InvalidInput(
            "protocol adapter registry mismatch".into(),
        ));
    }
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
    let (amount_out, fee_paid, impact) = adapter.quote_curve(pool, effective_in, a_to_b)?;

    // Token-2022 mints are rejected above until both transfer boundaries are
    // quoted from verified mint schedules.
    let output_transfer_fee = 0u64;
    let received = amount_out;

    // Advance reserves for non-PumpFun constant-product pools (PumpFun hop updates its own reserves).
    if pool.kind == PoolKind::Cpmm && pool.protocol != DexProtocol::PumpFun {
        if a_to_b {
            pool.reserve_in = pool.reserve_in.saturating_add(effective_in as u128);
            pool.reserve_out = pool.reserve_out.saturating_sub(amount_out as u128);
        } else {
            pool.reserve_out = pool.reserve_out.saturating_add(effective_in as u128);
            pool.reserve_in = pool.reserve_in.saturating_sub(amount_out as u128);
        }
    }

    Ok(HopQuote {
        pool_id: pool.pool_id,
        protocol,
        mint_in,
        mint_out,
        family: AmmFamily::from(pool.kind),
        amount_in,
        amount_out: received,
        fee_paid,
        transfer_fee_paid: transfer_fee + output_transfer_fee,
        price_impact_bps: impact,
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

fn quote_pump_fun_hop(
    pool: &mut PoolState,
    amount_in: u64,
    a_to_b: bool,
) -> Result<(u64, u64, u16)> {
    if pool.base_mint.0.to_string() != arb_constants::WSOL_MINT {
        return Err(ArbError::InvalidInput(
            "Pump.fun quote adapter currently supports WSOL-quoted curves only".into(),
        ));
    }
    if pool.fee_denominator != 10_000 {
        return Err(ArbError::InvalidInput(
            "Pump.fun pool fee denominator must be BPS (10,000)".into(),
        ));
    }
    let fee_bps = u16::try_from(pool.fee_numerator)
        .map_err(|_| ArbError::InvalidInput("Pump.fun fee BPS out of range".into()))?;
    let state = pool.pump_fun.as_ref().ok_or_else(|| {
        ArbError::InvalidInput(
            "Pump.fun bonding-curve state is missing; reserve fallback is disabled".into(),
        )
    })?;
    let curve =
        PumpFunBondingCurve::from_state(pool.pool_id, pool.base_mint, pool.quote_mint, state)
            .with_fee_bps(fee_bps)?
            .with_token_decimals(pool.decimals_b);
    let impact_reserve = if a_to_b {
        curve.virtual_sol_reserves
    } else {
        curve.virtual_token_reserves
    };

    let (amount_out, fee_paid) = curve.quote_with_fee(amount_in, a_to_b)?;

    // Mutate the curve by the amount that actually reaches/leaves its reserves.
    // Buy fees are withheld from quote input; sell fees are removed from gross quote output.
    if a_to_b {
        let net_quote_in = amount_in.checked_sub(fee_paid).ok_or(ArbError::Overflow)?;
        let new_v_sol = curve
            .virtual_sol_reserves
            .checked_add(net_quote_in)
            .ok_or(ArbError::Overflow)?;
        let new_v_tok = curve
            .virtual_token_reserves
            .checked_sub(amount_out)
            .ok_or_else(|| {
                ArbError::InvalidInput("Pump.fun buy exceeds virtual token reserve".into())
            })?;
        let new_r_tok = curve
            .real_token_reserves
            .checked_sub(amount_out)
            .ok_or_else(|| {
                ArbError::InvalidInput("Pump.fun buy exceeds real token reserve".into())
            })?;
        let new_r_sol = curve
            .real_sol_reserves
            .checked_add(net_quote_in)
            .ok_or(ArbError::Overflow)?;
        pool.reserve_in = new_v_sol as u128;
        pool.reserve_out = new_v_tok as u128;
        if let Some(st) = pool.pump_fun.as_mut() {
            st.virtual_sol_reserves = new_v_sol;
            st.virtual_token_reserves = new_v_tok;
            st.real_token_reserves = new_r_tok;
            st.real_sol_reserves = new_r_sol;
        }
    } else {
        let gross_quote_out = amount_out.checked_add(fee_paid).ok_or(ArbError::Overflow)?;
        let new_v_tok = curve
            .virtual_token_reserves
            .checked_add(amount_in)
            .ok_or(ArbError::Overflow)?;
        let new_v_sol = curve
            .virtual_sol_reserves
            .checked_sub(gross_quote_out)
            .ok_or_else(|| {
                ArbError::InvalidInput("Pump.fun sell exceeds virtual SOL reserve".into())
            })?;
        let new_r_tok = curve
            .real_token_reserves
            .checked_add(amount_in)
            .ok_or(ArbError::Overflow)?;
        let new_r_sol = curve
            .real_sol_reserves
            .checked_sub(gross_quote_out)
            .ok_or_else(|| {
                ArbError::InvalidInput("Pump.fun sell exceeds real SOL reserve".into())
            })?;
        pool.reserve_in = new_v_sol as u128;
        pool.reserve_out = new_v_tok as u128;
        if let Some(st) = pool.pump_fun.as_mut() {
            st.virtual_token_reserves = new_v_tok;
            st.virtual_sol_reserves = new_v_sol;
            st.real_token_reserves = new_r_tok;
            st.real_sol_reserves = new_r_sol;
        }
    }

    let impact = ((amount_in as u128 * 10_000) / (impact_reserve.max(1) as u128))
        .min(u16::MAX as u128) as u16;

    Ok((amount_out, fee_paid, impact))
}

fn quote_cpmm_hop(
    pool: &mut PoolState,
    amount_in: u64,
    a_to_b: bool,
    _protocol: DexProtocol,
) -> Result<(u64, u64, u16)> {
    let (reserve_in, reserve_out) = if a_to_b {
        (pool.reserve_in, pool.reserve_out)
    } else {
        (pool.reserve_out, pool.reserve_in)
    };
    if reserve_in == 0 || reserve_out == 0 {
        return Err(ArbError::InvalidInput("empty reserves".into()));
    }
    // Use the pool's native integer numerator/denominator. Converting to BPS
    // first truncates fractional fees (and can undercharge Raydium CPMM).
    if pool.fee_denominator == 0 || pool.fee_numerator as u128 > pool.fee_denominator as u128 {
        return Err(ArbError::InvalidInput("invalid CPMM fee ratio".into()));
    }
    let fee = (amount_in as u128)
        .checked_mul(pool.fee_numerator as u128)
        .ok_or_else(|| ArbError::InvalidInput("CPMM fee multiplication overflow".into()))?
        .div_ceil(pool.fee_denominator as u128);
    let fee =
        u64::try_from(fee).map_err(|_| ArbError::InvalidInput("CPMM fee exceeds u64".into()))?;
    let input_after_fee = amount_in.saturating_sub(fee);
    let out = (reserve_out * input_after_fee as u128) / (reserve_in + input_after_fee as u128);
    if out == 0 || out >= reserve_out {
        return Err(ArbError::InvalidInput("insufficient liquidity".into()));
    }
    // Price impact excluding fee: compare fee-free constant-product output to actual.
    let no_fee_out = (reserve_out * amount_in as u128) / (reserve_in + amount_in as u128);
    let impact = if no_fee_out == 0 {
        10_000u16
    } else {
        let diff = no_fee_out.saturating_sub(out);
        ((diff * 10_000) / no_fee_out).min(10_000) as u16
    };
    Ok((out as u64, fee, impact))
}

fn quote_clmm_hop(pool: &mut PoolState, amount_in: u64, a_to_b: bool) -> Result<(u64, u64, u16)> {
    let c0 = pool
        .clmm
        .clone()
        .ok_or(ArbError::InvalidInput("missing CLMM state".into()))?;
    if c0.liquidity == 0 || c0.sqrt_price == 0 {
        return Err(ArbError::InvalidInput("uninitialized CLMM state".into()));
    }
    let fee_rate = c0.fee_rate;
    if fee_rate == 0 {
        return Err(ArbError::InvalidInput(
            "CLMM native fee rate is missing".into(),
        ));
    }

    let arrays: Vec<TickArrayFacade> = c0
        .tick_arrays
        .iter()
        .map(|ta| TickArrayFacade {
            start_tick_index: ta.start_tick_index,
            ticks: ta
                .ticks
                .iter()
                .map(|t| TickFacade {
                    tick_index: t.tick_index,
                    initialized: t.initialized,
                    liquidity_net: t.liquidity_net,
                    liquidity_gross: 0,
                })
                .collect(),
        })
        .collect();

    let q = clmm::compute_swap(
        c0.sqrt_price,
        c0.tick_current,
        c0.liquidity,
        c0.tick_spacing,
        fee_rate,
        amount_in,
        a_to_b,
        &arrays,
    )
    .map_err(|e| ArbError::InvalidInput(format!("CLMM quote failed: {e}")))?;

    if q.amount_in_consumed != amount_in {
        return Err(ArbError::InvalidInput(
            "CLMM insufficient liquidity for full input".into(),
        ));
    }

    let q0 = clmm::compute_swap(
        c0.sqrt_price,
        c0.tick_current,
        c0.liquidity,
        c0.tick_spacing,
        0,
        amount_in,
        a_to_b,
        &arrays,
    )
    .map_err(|e| ArbError::InvalidInput(format!("CLMM baseline failed: {e}")))?;

    if let Some(c) = pool.clmm.as_mut() {
        c.sqrt_price = q.final_sqrt_price_x64;
        c.tick_current = q.final_tick_index;
        c.liquidity = q.final_liquidity;
    }

    let impact = if q0.amount_out_received == 0 {
        10_000u16
    } else {
        let diff = q0.amount_out_received.saturating_sub(q.amount_out_received);
        ((diff as u128 * 10_000) / q0.amount_out_received as u128).min(10_000) as u16
    };

    Ok((q.amount_out_received, q.fee_paid, impact))
}

fn quote_dlmm_hop(pool: &mut PoolState, amount_in: u64, a_to_b: bool) -> Result<(u64, u64, u16)> {
    let d0 = pool
        .dlmm
        .clone()
        .ok_or(ArbError::InvalidInput("missing DLMM state".into()))?;
    let params = d0.params.clone().ok_or(ArbError::InvalidInput(
        "DLMM fee config not ingested".into(),
    ))?;
    if params.function_type != 0 {
        return Err(ArbError::InvalidInput(
            "unsupported Meteora DLMM function type; spot-only quote engine cannot price it".into(),
        ));
    }
    if d0.status != 0 {
        return Err(ArbError::InvalidInput(
            "Meteora DLMM pool status is disabled".into(),
        ));
    }
    if d0.bins.is_empty() {
        return Err(ArbError::InvalidInput("DLMM bins not ingested".into()));
    }
    let pool_state = DlmmPoolState {
        active_id: d0.active_bin_id,
        bin_step: d0.bin_step_bps,
        status: d0.status,
        parameters: DlmmParameters {
            bin_step: d0.bin_step_bps,
            base_factor: params.base_factor,
            filter_period: params.filter_period,
            decay_period: params.decay_period,
            reduction_factor: params.reduction_factor,
            variable_fee_control: params.variable_fee_control,
            max_volatility_accumulator: params.max_volatility_accumulator,
            protocol_share: params.protocol_share,
            base_fee_power_factor: params.base_fee_power_factor,
            collect_fee_mode: params.collect_fee_mode,
        },
        v_parameters: dlmm::DlmmVariableParameters {
            volatility_accumulator: d0.volatility_accumulator,
            volatility_reference: d0.volatility_reference,
            index_reference: d0.index_reference,
            last_update_timestamp: d0.last_update_timestamp,
        },
    };
    let mut bins: Vec<DlmmBin> = d0
        .bins
        .iter()
        .map(|b| {
            Ok(DlmmBin {
                id: b.bin_id,
                amount_x: u64::try_from(b.amount_x).map_err(|_| {
                    ArbError::InvalidInput("DLMM X reserve exceeds u64 domain".into())
                })?,
                amount_y: u64::try_from(b.amount_y).map_err(|_| {
                    ArbError::InvalidInput("DLMM Y reserve exceeds u64 domain".into())
                })?,
                price_q64: 0,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let swap_for_y = a_to_b;
    let q = dlmm::quote_dlmm_multi_bin(&pool_state, &mut bins, amount_in, swap_for_y, None)
        .map_err(|e| ArbError::InvalidInput(format!("DLMM quote failed: {e}")))?;

    if let Some(d) = pool.dlmm.as_mut() {
        d.active_bin_id = q.end_active_id;
        d.volatility_accumulator = q.end_volatility_accumulator;
        d.bins = bins
            .into_iter()
            .map(|b| arb_types::DlmmBin {
                bin_id: b.id,
                amount_x: b.amount_x as u128,
                amount_y: b.amount_y as u128,
            })
            .collect();
    }

    Ok((q.amount_out, q.fee_amount, q.price_impact_bps.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{ClmmTick, ClmmTickArray, MintId, PoolId, Slot};
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
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
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
                        ticks: vec![],
                    },
                    ClmmTickArray {
                        start_tick_index: 0,
                        ticks: vec![],
                    },
                ],
            }),
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
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
            c.tick_arrays = vec![ClmmTickArray {
                start_tick_index: -88 * 4,
                ticks: (0..88)
                    .map(|i| ClmmTick {
                        tick_index: -88 * 4 + i * 4,
                        initialized: true,
                        liquidity_net: 500_000_000_000,
                    })
                    .collect(),
            }];
            // seed the active tick
            c.tick_arrays[0].ticks[0].liquidity_net = 500_000_000_000;
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
                }],
                params: None,
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
                last_update_timestamp: 0,
            }),
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        };
        // Without params, quoting must refuse (honest).
        assert!(quote_hop(&mut p, 10_000, true).is_err());
    }

    /// Regression: `quote_dlmm_hop` must forward the ingested on-chain
    /// `collect_fee_mode` (LbPair byte @36) into the quote engine instead of
    /// hardcoding `0` (InputOnly).
    ///
    /// Discriminator: single bin priced 4x (bin_step=10000 bps, active_id=2),
    /// 1% base fee (base_factor=100, power=0), input 10^9 X-raw:
    /// - InputOnly  -> fee charged on input  ~ 10^7 (X raw)
    /// - OnlyY      -> fee charged on output ~ 4x10^7 (Y raw, = gross_out * 1%)
    ///
    /// `amount_out` is algebraically identical in both modes (fee deduction
    /// commutes with the per-bin price multiplication); only the fee
    /// denomination differs. This is why the hardcode never corrupted profit
    /// gating but did mislabel every OnlyY fee line.
    fn dlmm_mode_pool(collect_fee_mode: u8) -> PoolState {
        PoolState {
            pool_id: PoolId::new([7; 32]),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 6,
            decimals_b: 9,
            reserve_in: 0,
            reserve_out: 0,
            fee_numerator: 0,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(arb_types::DlmmState {
                active_bin_id: 2,
                status: 0,
                bin_step_bps: 10_000,
                bins: vec![arb_types::DlmmBin {
                    bin_id: 2,
                    amount_x: 10u128.pow(12),
                    amount_y: 4 * 10u128.pow(12),
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
                index_reference: 2,
                last_update_timestamp: 0,
            }),
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
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

        // Fee denomination: OnlyY charges 1% on the gross OUTPUT (4x price)
        // => ~4x10^7; InputOnly charges 1% on the input => ~10^7.
        assert!(
            fee_only_y >= 35_000_000,
            "OnlyY fee must be output-denominated (~4x10^7), got {fee_only_y}"
        );
        assert!(
            fee_input_only < 20_000_000,
            "InputOnly fee must be input-denominated (~10^7), got {fee_input_only}"
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
    fn pump_fun_quote_hop_buy_and_sell() {
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
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        };

        // 1. Buy Hop: 1 SOL in -> Tokens out
        let sol_in = 1_000_000_000u64;
        let q_buy = quote_hop(&mut p, sol_in, true).expect("pump_fun buy hop should succeed");
        assert!(q_buy.amount_out > 0);
        assert_eq!(q_buy.protocol, DexProtocol::PumpFun);
        assert_eq!(q_buy.fee_paid, 12_500_000); // 1.25% standard bonding-curve fee on 1 SOL

        // 2. Sell Hop: Tokens in -> SOL out
        let tokens_in = q_buy.amount_out;
        let q_sell = quote_hop(&mut p, tokens_in, false).expect("pump_fun sell hop should succeed");
        assert!(q_sell.amount_out > 0);
        assert!(q_sell.fee_paid > 0); // sell fee is denominated in quote output
        // Roundtrip must lose value due to fees on both legs
        assert!(q_sell.amount_out < sol_in);
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
