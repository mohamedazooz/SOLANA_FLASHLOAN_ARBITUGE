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
        DexProtocol::PumpFun => 12,
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
        DexProtocol::PumpFun => Err(
            "Pump.fun remains quote-only until current on-chain instruction accounts and fee config are verified",
        ),
        DexProtocol::PumpSwap => Err(
            "PumpSwap remains quote-only until pool + vault + global fee config are joined at one slot",
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
    let (amount_out, fee_paid, impact) = match protocol {
        DexProtocol::PumpFun => quote_pump_fun_hop(pool, effective_in, a_to_b)?,
        DexProtocol::PumpSwap => {
            return Err(ArbError::InvalidInput(
                "PumpSwap requires its dedicated quote adapter with live PPM fee components and effective quote reserves".into(),
            ));
        }
        _ => match pool.kind {
            PoolKind::Cpmm => quote_cpmm_hop(pool, effective_in, pool.protocol)?,
            PoolKind::Clmm => quote_clmm_hop(pool, effective_in, a_to_b)?,
            PoolKind::Dlmm => quote_dlmm_hop(pool, effective_in, a_to_b)?,
        },
    };

    // Token-2022 mints are rejected above until both transfer boundaries are
    // quoted from verified mint schedules.
    let output_transfer_fee = 0u64;
    let received = amount_out;

    // Advance reserves for non-PumpFun constant-product pools (PumpFun hop updates its own reserves).
    if pool.kind == PoolKind::Cpmm && pool.protocol != DexProtocol::PumpFun {
        pool.reserve_in = pool.reserve_in.saturating_add(effective_in as u128);
        pool.reserve_out = pool.reserve_out.saturating_sub(amount_out as u128);
    }

    Ok(HopQuote {
        pool_id: pool.pool_id,
        protocol,
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

/// Price a complete route. Pools are mutated sequentially; a pool used twice
/// (or by two hops) is quoted against its already-advanced state.
pub fn quote_route(
    pools: &mut [PoolState],
    hops: &[HopRequest],
    amount_in: u64,
) -> Result<RouteQuote> {
    if hops.is_empty() {
        return Err(ArbError::InvalidInput("empty route".into()));
    }
    let mut current = amount_in;
    let mut hop_quotes = Vec::with_capacity(hops.len());
    let mut total_cu = 0u64;
    let mut total_accounts = 0u64;

    for hop in hops {
        let pool = pools
            .iter_mut()
            .find(|p| p.pool_id == hop.pool_id)
            .ok_or(ArbError::PoolNotFound(hop.pool_id))?;

        let a_to_b = pool.base_mint == hop.mint_in;
        // mint continuity check
        if !a_to_b && pool.quote_mint != hop.mint_in {
            return Err(ArbError::InvalidInput(
                "hop input mint does not match pool".into(),
            ));
        }

        let q = quote_hop(pool, current, a_to_b)?;
        current = q.amount_out;
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
    protocol: DexProtocol,
) -> Result<(u64, u64, u16)> {
    if pool.reserve_in == 0 || pool.reserve_out == 0 {
        return Err(ArbError::InvalidInput("empty reserves".into()));
    }
    // Fee rounding differs by program.
    let fee = match protocol {
        // Raydium CPMM: ceil(amount * trade_fee_rate / 1e6).
        DexProtocol::RaydiumCpmm => {
            let Some(fee_bps) = pool.fee_bps() else {
                return Err(ArbError::InvalidInput("invalid pool fee".into()));
            };
            let rate = fee_bps as u64 * 100;
            (amount_in as u128 * rate as u128).div_ceil(1_000_000)
        }
        // Raydium V4: ceil(amount * fee_bps / 1e4).
        _ => {
            let Some(fee_bps) = pool.fee_bps() else {
                return Err(ArbError::InvalidInput("invalid pool fee".into()));
            };
            let rate = fee_bps as u64;
            (amount_in as u128 * rate as u128).div_ceil(10_000)
        }
    } as u64;
    let input_after_fee = amount_in.saturating_sub(fee);
    let out =
        (pool.reserve_out * input_after_fee as u128) / (pool.reserve_in + input_after_fee as u128);
    if out == 0 || out >= pool.reserve_out {
        return Err(ArbError::InvalidInput("insufficient liquidity".into()));
    }
    // Price impact excluding fee: compare fee-free constant-product output to actual.
    let no_fee_out = (pool.reserve_out * amount_in as u128) / (pool.reserve_in + amount_in as u128);
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
    if d0.bins.is_empty() {
        return Err(ArbError::InvalidInput("DLMM bins not ingested".into()));
    }
    let pool_state = DlmmPoolState {
        active_id: d0.active_bin_id,
        bin_step: d0.bin_step_bps,
        status: 0,
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
        },
        v_parameters: dlmm::DlmmVariableParameters {
            volatility_accumulator: d0.volatility_accumulator,
            volatility_reference: d0.volatility_reference,
            index_reference: d0.active_bin_id,
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
                tick_arrays: vec![],
            }),
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
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
    fn repeated_pool_dependency_updates_state() {
        // Same CPMM pool used twice (base->quote then quote->base) must lose to fees.
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
        let q = quote_route(&mut pools, &hops, 10_000).unwrap();
        assert!(
            q.amount_out < 10_000,
            "round-trip through same pool must lose to fees"
        );
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
                bin_step_bps: 25,
                bins: vec![arb_types::DlmmBin {
                    bin_id: 0,
                    amount_x: 1_000_000,
                    amount_y: 1_000_000,
                }],
                params: None,
                volatility_accumulator: 0,
                volatility_reference: 0,
                last_update_timestamp: 0,
            }),
            pump_fun: None,
            transfer_fee_bps: 0,
            last_update_slot: 1,
            last_update_ms: 0,
        };
        // Without params, quoting must refuse (honest).
        assert!(quote_hop(&mut p, 10_000, true).is_err());
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
}
