//! Pump.fun BOOST Migration Arbitrage Quantitative Modeling.
//!
//! Reconstructs exact on-chain graduation economics derived from empirical
//! reverse-engineering of Pump.fun's BOOST mechanism.
//!
//! Core Mechanics:
//! - Standard curve completes at ~85 SOL real reserves.
//! - Protocol initiates ~17.58 SOL TWAP buyback ("BOOST") on newly formed pool.
//! - Sweet spot is S in [50, 70] SOL where completion cost (85 - S) is small enough
//!   and the acquired token bag is large enough to absorb boost buyback without
//!   extreme slippage destruction.
//!
//! Pure integer math in Lamports (WSOL raw atomic units) and token atomic units (6 decimals).

use ruint::aliases::U256;

/// Standard graduation threshold in Lamports (~85 SOL).
pub const PUMP_GRADUATION_TARGET_LAMPORTS: u64 = 85_000_000_000;

/// Protocol BOOST allocation in Lamports (~17.58 SOL).
pub const PUMP_BOOST_ALLOCATION_LAMPORTS: u64 = 17_580_000_000;

/// Effective opening quote reserve of post-migration pool in Lamports (~85 SOL).
pub const POOL_OPENING_QUOTE_LAMPORTS: u64 = 84_990_000_000;

/// Pooled base tokens seeded into new AMM pool (206.9M tokens with 6 decimals = 206_900_000_000_000).
pub const POOL_OPENING_BASE_TOKENS: u64 = 206_900_000_000_000;

/// Output evaluation of a Pump.fun BOOST migration arbitrage opportunity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoostArbEvaluation {
    /// Cost in lamports required to finish the curve to graduation.
    pub completion_cost_lamports: u64,
    /// Token bag acquired by completing the curve (atomic units).
    pub acquired_tokens: u64,
    /// Estimated gross SOL received from selling bag into boosted AMM pool.
    pub gross_sol_out_lamports: u64,
    /// Portion of protocol boost absorbed.
    pub boost_absorbed_lamports: u64,
    /// Kamino flash loan fee in lamports.
    pub flash_loan_fee_lamports: u64,
    /// Priority fee + Jito tip budget in lamports.
    pub execution_tip_lamports: u64,
    /// Net profit (or loss) in lamports.
    pub net_profit_lamports: i64,
    /// Whether trade meets minimum net profit gate.
    pub is_executable: bool,
}

/// Calculate the SOL required in Lamports to complete a bonding curve.
#[inline]
pub fn completion_cost(real_sol_reserves: u64) -> u64 {
    PUMP_GRADUATION_TARGET_LAMPORTS.saturating_sub(real_sol_reserves)
}

/// Calculate the token amount acquired along the bonding curve by buying `cost_lamports`.
///
/// Invariant: (virtual_sol + cost) * (virtual_tok - delta_tok) = k = virtual_sol * virtual_tok
/// => delta_tok = virtual_tok - (k / (virtual_sol + cost))
pub fn tokens_acquired_on_curve(
    virtual_sol_reserves: u64,
    virtual_token_reserves: u64,
    cost_lamports: u64,
) -> Option<u64> {
    if cost_lamports == 0 || virtual_sol_reserves == 0 || virtual_token_reserves == 0 {
        return Some(0);
    }

    let vsol = U256::from(virtual_sol_reserves);
    let vtok = U256::from(virtual_token_reserves);
    let cost = U256::from(cost_lamports);

    let k = vsol.checked_mul(vtok)?;
    let new_vsol = vsol.checked_add(cost)?;
    let new_vtok = k.checked_div(new_vsol)?;

    let delta_tok = vtok.checked_sub(new_vtok)?;
    let delta_u64 = u64::try_from(delta_tok).ok()?;

    Some(delta_u64)
}

/// Estimate gross SOL return from selling `tokens` into a pool with opening reserves and BOOST buyback.
///
/// Base dump formula:
///   dump = (pool_eff_quote * tokens) / (pool_base + tokens)
/// Boost absorbed:
///   boost = min(PUMP_BOOST_ALLOCATION_LAMPORTS, dump)
///
/// Under 2026 empirical observation with hair-cut for intra-slot TWAP distribution and sniper competition,
/// realizable dump incorporates an efficiency factor (e.g. 70% capture efficiency).
pub fn evaluate_boost_amm_dump(
    acquired_tokens: u64,
    capture_efficiency_bps: u64,
) -> Option<(u64, u64)> {
    if acquired_tokens == 0 {
        return Some((0, 0));
    }

    let tok = U256::from(acquired_tokens);
    let pool_quote = U256::from(POOL_OPENING_QUOTE_LAMPORTS);
    let pool_base = U256::from(POOL_OPENING_BASE_TOKENS);

    // Theoretical CPMM dump output: (quote * tokens) / (base + tokens)
    let numerator = pool_quote.checked_mul(tok)?;
    let denominator = pool_base.checked_add(tok)?;
    let raw_dump = numerator.checked_div(denominator)?;
    let raw_dump_u64 = u64::try_from(raw_dump).ok()?;

    // Portion of boost budget that the dump can claim
    let boost_absorbed = raw_dump_u64.min(PUMP_BOOST_ALLOCATION_LAMPORTS);

    // Apply realization haircut (capture_efficiency_bps out of 10_000)
    let realized_gross = (U256::from(raw_dump_u64)
        .checked_mul(U256::from(capture_efficiency_bps))?
        .checked_div(U256::from(10_000u64))?)
    .try_into()
    .ok()?;

    Some((realized_gross, boost_absorbed))
}

/// Full profitability simulation of a Pump.fun BOOST Migration Arbitrage.
///
/// Returns an evaluation specifying exact completion requirements, estimated profits,
/// and whether the trade safely satisfies minimum net profit criteria.
///
/// `flash_loan_fee_sf` is the RAW Kamino U68F60 scaling factor
/// (`ReserveConfig.fees.flash_loan_fee_sf`, 60 fractional bits) read from the
/// verified reserve account — NEVER a bps integer, which cannot represent the
/// on-chain value (see `economic::calculate_flash_fee_raw`).
pub fn evaluate_pump_boost_arb(
    real_sol_reserves: u64,
    virtual_sol_reserves: u64,
    virtual_token_reserves: u64,
    flash_loan_fee_sf: u64,
    execution_tip_lamports: u64,
    capture_efficiency_bps: u64,
    min_net_profit_lamports: u64,
) -> Option<BoostArbEvaluation> {
    // 1. Completion cost
    let cost = completion_cost(real_sol_reserves);
    if cost == 0 {
        return None; // Already complete
    }

    // 2. Acquired tokens
    let acquired_tokens =
        tokens_acquired_on_curve(virtual_sol_reserves, virtual_token_reserves, cost)?;

    // 3. Gross return from pool dump + boost absorption
    let (gross_sol, boost_absorbed) =
        evaluate_boost_amm_dump(acquired_tokens, capture_efficiency_bps)?;

    // 4. Cost of Flash Loan (principal = cost). Canon: round(cost * sf / 2^60)
    // with the klend floor/disabled semantics — one source of truth with the
    // execution-time fee, no local reimplementation.
    let flash_fee: u64 = match crate::economic::calculate_flash_fee_raw(cost, flash_loan_fee_sf) {
        Ok(f) => f,
        // Flash loans disabled on the reserve — no opportunity at all.
        Err(_) => return None,
    };

    // 5. Total expenses = completion cost + flash fee + Jito tip
    let total_expenses = (cost as u128)
        .checked_add(flash_fee as u128)?
        .checked_add(execution_tip_lamports as u128)?;

    let gross_u128 = gross_sol as u128;
    let net_profit_i128 = (gross_u128 as i128) - (total_expenses as i128);
    let net_profit_lamports = i64::try_from(net_profit_i128).ok()?;

    let is_executable = net_profit_lamports >= (min_net_profit_lamports as i64);

    Some(BoostArbEvaluation {
        completion_cost_lamports: cost,
        acquired_tokens,
        gross_sol_out_lamports: gross_sol,
        boost_absorbed_lamports: boost_absorbed,
        flash_loan_fee_lamports: flash_fee,
        execution_tip_lamports,
        net_profit_lamports,
        is_executable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures mirrored from the arb-ingest discovery test: a curve at
    /// 66 real SOL (graduation cost 19 SOL) with 30 + 66 virtual SOL.
    fn eval(fee_sf: u64) -> BoostArbEvaluation {
        evaluate_pump_boost_arb(
            66_000_000_000,
            96_000_000_000,
            335_000_000_000_000,
            fee_sf,
            100_000_000, // 0.10 SOL tip
            9_000,       // 90% capture
            0,
        )
        .expect("evaluation must produce a target at these reserves")
    }

    /// MATH-03 regression: the fee must come from the canonical
    /// `calculate_flash_fee_raw` chain (round(cost * sf / 2^60), klend
    /// floor semantics), NOT from a bps reinterpretation. The historical
    /// bug passed `sf >> 32` as bps: for the real main-market WBOL reserve
    /// (sf = 11_529_215_046_068 = 1e-5 = 0.1 bps) that read as 2684 "bps"
    /// = 26.84% — a 26,840x fee overstatement that gated the BOOST route
    /// on a phantom cost.
    #[test]
    fn flash_fee_matches_canonical_formula() {
        // 19 SOL graduation cost, 0.1 bps reserve fee -> round(1.9 SOL * 1e-5)...
        // cost * sf / 2^60 = 19e9 * 11_529_215_046_068 / 2^60 = 190_000 lamports
        let ev = eval(11_529_215_046_068);
        assert_eq!(ev.completion_cost_lamports, 19_000_000_000);
        assert_eq!(ev.flash_loan_fee_lamports, 190_000);

        // Exact-equivalence bridge to the canonical implementation the
        // assembler repays with: fee(cost, sf) == calculate_flash_fee_raw.
        let canonical = crate::economic::calculate_flash_fee_raw(
            ev.completion_cost_lamports,
            11_529_215_046_068,
        )
        .expect("canonical fee must compute for a valid reserve sf");
        assert_eq!(flash_loan_fee_lamports_of(ev), canonical);
    }

    fn flash_loan_fee_lamports_of(ev: BoostArbEvaluation) -> u64 {
        ev.flash_loan_fee_lamports
    }

    /// 30 bps must read as `sf = round(30 * 2^60 / 10_000)` — the identity
    /// the arb-ingest test fixture uses — with the fee landing at exactly
    /// `cost * 30 / 10_000` (no bps double-haircut, no >> 32 distortion).
    #[test]
    fn bps_sf_identity_30_bps() {
        let sf_30_bps = ((30u128 * (1u128 << 60)) / 10_000) as u64;
        assert_eq!(sf_30_bps, 3_458_764_513_820_540);
        let ev = eval(sf_30_bps);
        // 19_000_000_000 * 30 / 10_000 = 57_000_000 lamports
        assert_eq!(ev.flash_loan_fee_lamports, 57_000_000);
    }

    /// A disabled reserve (sf == u64::MAX, klend's `FlashLoansDisabled` bit)
    /// yields no opportunity rather than a fabricated fee.
    #[test]
    fn flash_loans_disabled_reserve_yields_none() {
        assert!(
            evaluate_pump_boost_arb(
                66_000_000_000,
                96_000_000_000,
                335_000_000_000_000,
                u64::MAX,
                100_000_000,
                9_000,
                0,
            )
            .is_none()
        );
    }
}
