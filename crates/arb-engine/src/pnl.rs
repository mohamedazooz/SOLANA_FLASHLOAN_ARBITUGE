//! Production-grade P&L Accounting for Flash-Loan Arbitrage.
//!
//! Computes **true net profit** in lamports and USD, including every cost:
//! - DEX swap fees (LP + protocol)
//! - Kamino flash loan fee (borrow * fee_sf / 2^64, ceil)
//! - Base network fee (5000 lamports per signature)
//! - Priority fee (compute_unit_price * compute_units)
//! - Jito tip
//! - Rent for ATA creation (if new)
//! - Expected failure cost (probability-weighted)
//!
//! All amounts in **lamports** (WSOL = 9 decimals). USD conversion uses
//! a provided SOL price. No floating point in the hot path.

use arb_math::dlmm::DlmmSwapQuote;
use arb_tx::kamino::calculate_kamino_flash_fee;
use arb_types::UsdMicros;

/// Fixed protocol constants (single source of truth in arb-constants, re-exported here for clarity).
pub const BASE_FEE_PER_SIGNATURE_LAMPORTS: u64 = 5_000;
pub const RENT_PER_ATA_LAMPORTS: u64 = 2_039_280; // 0.00203928 SOL for token account
pub const MAX_COMPUTE_UNITS: u32 = 1_400_000;

/// Complete cost breakdown for one flash-loan arbitrage execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArbCostBreakdown {
    /// Sum of all DEX swap fees paid (LP share + protocol share) across all legs.
    pub dex_fees_lamports: u64,
    /// Kamino flash loan fee: ceil(borrow * fee_sf / 2^64).
    pub flash_fee_lamports: u64,
    /// Base network fee: 5000 lamports per signature (legacy/v0) or included in V1 config.
    pub base_fee_lamports: u64,
    /// Priority fee: compute_unit_price (microlamports/CU) * compute_units / 1_000_000.
    pub priority_fee_lamports: u64,
    /// Jito tip paid to the block builder.
    pub jito_tip_lamports: u64,
    /// Rent for newly created ATAs (0 if ATAs already exist).
    pub rent_lamports: u64,
    /// Expected failure cost: failure_probability * (all_fees + tip).
    /// Not included in `total_lamports` — used for risk-adjusted decisions only.
    pub expected_failure_lamports: u64,
}

impl ArbCostBreakdown {
    /// Total guaranteed cost if the transaction lands successfully.
    pub fn total_lamports(&self) -> u64 {
        self.dex_fees_lamports
            .saturating_add(self.flash_fee_lamports)
            .saturating_add(self.base_fee_lamports)
            .saturating_add(self.priority_fee_lamports)
            .saturating_add(self.jito_tip_lamports)
            .saturating_add(self.rent_lamports)
    }

    /// Total cost including probability-weighted failure cost (for EV decisions).
    pub fn total_with_failure_lamports(&self) -> u64 {
        self.total_lamports()
            .saturating_add(self.expected_failure_lamports)
    }

    /// Convert to USD micros using SOL price.
    pub fn to_usd(&self, sol_price_usd: f64) -> ArbCostBreakdownUsd {
        let lamports_to_usd = |lamports: u64| -> UsdMicros {
            let sol = lamports as f64 / 1_000_000_000.0;
            UsdMicros::from_f64(sol * sol_price_usd)
        };
        ArbCostBreakdownUsd {
            dex_fees_usd: lamports_to_usd(self.dex_fees_lamports),
            flash_fee_usd: lamports_to_usd(self.flash_fee_lamports),
            base_fee_usd: lamports_to_usd(self.base_fee_lamports),
            priority_fee_usd: lamports_to_usd(self.priority_fee_lamports),
            jito_tip_usd: lamports_to_usd(self.jito_tip_lamports),
            rent_usd: lamports_to_usd(self.rent_lamports),
            expected_failure_usd: lamports_to_usd(self.expected_failure_lamports),
        }
    }
}

/// USD-denominated cost breakdown (for reporting and risk limits).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArbCostBreakdownUsd {
    pub dex_fees_usd: UsdMicros,
    pub flash_fee_usd: UsdMicros,
    pub base_fee_usd: UsdMicros,
    pub priority_fee_usd: UsdMicros,
    pub jito_tip_usd: UsdMicros,
    pub rent_usd: UsdMicros,
    pub expected_failure_usd: UsdMicros,
}

impl ArbCostBreakdownUsd {
    pub fn total_usd(&self) -> UsdMicros {
        self.dex_fees_usd
            .saturating_add(self.flash_fee_usd)
            .saturating_add(self.base_fee_usd)
            .saturating_add(self.priority_fee_usd)
            .saturating_add(self.jito_tip_usd)
            .saturating_add(self.rent_usd)
    }

    pub fn total_with_failure_usd(&self) -> UsdMicros {
        self.total_usd().saturating_add(self.expected_failure_usd)
    }
}

/// Input parameters for P&L calculation.
#[derive(Debug, Clone)]
pub struct PnlInput {
    /// Flash loan principal in lamports (WSOL = 9 decimals).
    pub borrow_amount: u64,
    /// Final output amount from the last swap leg (lamports).
    pub final_output: u64,
    /// Per-leg swap quotes (for DEX fee summation).
    pub leg_quotes: Vec<DlmmSwapQuote>,
    /// Kamino reserve's `flash_loan_fee_sf` (Q64.64 fixed point).
    pub kamino_flash_fee_sf: u64,
    /// Compute unit limit for the transaction.
    pub cu_limit: u32,
    /// Priority fee in micro-lamports per CU.
    pub priority_fee_micro_lamports: u64,
    /// Jito tip in lamports.
    pub jito_tip_lamports: u64,
    /// Number of ATAs created by this transaction (0, 1, 2, or 3).
    pub new_atas_count: u8,
    /// Transaction format: affects base fee accounting.
    pub tx_format: TxFormat,
    /// SOL price in USD (for USD conversion).
    pub sol_price_usd: f64,
    /// Estimated failure probability (0.0 to 1.0).
    pub failure_probability: f64,
}

/// Transaction format determines base fee structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxFormat {
    /// Legacy or v0 with ALT: 5000 lamports base fee per signature.
    LegacyOrV0,
    /// V1: base fee included in message config, no separate ComputeBudget instruction.
    V1,
}

/// Result of P&L calculation.
#[derive(Debug, Clone)]
pub struct PnlResult {
    /// Gross profit: final_output - borrow_amount (can be negative).
    pub gross_profit_lamports: i64,
    /// Complete cost breakdown.
    pub costs: ArbCostBreakdown,
    /// Net profit after all guaranteed costs: gross - total_costs.
    pub net_profit_lamports: i64,
    /// Net profit in USD micros.
    pub net_profit_usd: UsdMicros,
    /// ROI in basis points: (net_profit / borrow_amount) * 10_000.
    pub roi_bps: i64,
    /// Whether the opportunity clears the minimum profit hurdle.
    pub passes_hurdle: bool,
    /// Minimum profit hurdle in lamports.
    pub hurdle_lamports: u64,
}

/// Calculate Kamino flash loan fee with exact ceiling arithmetic.
/// `fee = ceil(borrow * fee_sf / 2^64)`.
pub fn compute_kamino_flash_fee(borrow: u64, fee_sf: u64) -> u64 {
    calculate_kamino_flash_fee(borrow, fee_sf)
}

/// Compute DEX fees from a chain of DLMM quotes.
/// The quote's `fee_amount` is the TOTAL trading fee (LP + protocol).
pub fn compute_dex_fees_from_quotes(quotes: &[DlmmSwapQuote]) -> u64 {
    quotes.iter().map(|q| q.fee_amount).sum()
}

/// Compute rent for newly created ATAs.
pub fn compute_rent_for_new_atas(new_atas_count: u8) -> u64 {
    (new_atas_count as u64).saturating_mul(RENT_PER_ATA_LAMPORTS)
}

/// Compute base network fee based on transaction format.
pub fn compute_base_fee(format: TxFormat, signature_count: u8) -> u64 {
    match format {
        TxFormat::LegacyOrV0 => {
            (signature_count as u64).saturating_mul(BASE_FEE_PER_SIGNATURE_LAMPORTS)
        }
        TxFormat::V1 => 0, // V1 embeds compute config in message, no separate ComputeBudget ix
    }
}

/// Compute priority fee: (cu_price_micro_lamports * cu_limit) / 1_000_000, ceiling.
pub fn compute_priority_fee(cu_limit: u32, priority_fee_micro_lamports: u64) -> u64 {
    let num = u128::from(cu_limit).saturating_mul(priority_fee_micro_lamports as u128);
    let div = (num + 1_000_000 - 1) / 1_000_000; // ceiling division
    u64::try_from(div).unwrap_or(u64::MAX)
}

/// Expected failure cost = failure_probability * (guaranteed_costs + jito_tip).
/// The tip is lost on failure; other fees are burned.
pub fn compute_expected_failure_cost(
    guaranteed_costs: u64,
    jito_tip_lamports: u64,
    failure_probability: f64,
) -> u64 {
    if failure_probability <= 0.0 {
        return 0;
    }
    if failure_probability >= 1.0 {
        return guaranteed_costs.saturating_add(jito_tip_lamports);
    }
    let total_at_risk = guaranteed_costs.saturating_add(jito_tip_lamports);
    (total_at_risk as f64 * failure_probability).round() as u64
}

/// Main P&L calculation entry point.
pub fn calculate_pnl(input: PnlInput) -> PnlResult {
    // 1. Gross profit
    let gross_profit = input.final_output as i64 - input.borrow_amount as i64;

    // 2. DEX fees (sum of all legs' total trading fees)
    let dex_fees = compute_dex_fees_from_quotes(&input.leg_quotes);

    // 3. Flash loan fee
    let flash_fee = compute_kamino_flash_fee(input.borrow_amount, input.kamino_flash_fee_sf);

    // 4. Base network fee
    let sig_count = match input.tx_format {
        TxFormat::LegacyOrV0 => 1, // single signer
        TxFormat::V1 => 1,
    };
    let base_fee = compute_base_fee(input.tx_format, sig_count);

    // 5. Priority fee
    let priority_fee = compute_priority_fee(input.cu_limit, input.priority_fee_micro_lamports);

    // 6. Jito tip
    let jito_tip = input.jito_tip_lamports;

    // 7. Rent for new ATAs
    let rent = compute_rent_for_new_atas(input.new_atas_count);

    // 8. Guaranteed costs (if tx lands)
    let guaranteed = dex_fees
        .saturating_add(flash_fee)
        .saturating_add(base_fee)
        .saturating_add(priority_fee)
        .saturating_add(jito_tip)
        .saturating_add(rent);

    // 9. Expected failure cost
    let expected_failure =
        compute_expected_failure_cost(guaranteed, jito_tip, input.failure_probability);

    let costs = ArbCostBreakdown {
        dex_fees_lamports: dex_fees,
        flash_fee_lamports: flash_fee,
        base_fee_lamports: base_fee,
        priority_fee_lamports: priority_fee,
        jito_tip_lamports: jito_tip,
        rent_lamports: rent,
        expected_failure_lamports: expected_failure,
    };

    // 10. Net profit (guaranteed costs only)
    let net_profit_lamports = gross_profit - guaranteed as i64;

    // 11. USD conversion
    let _costs_usd = costs.to_usd(input.sol_price_usd);
    let net_profit_usd =
        UsdMicros::from_f64((net_profit_lamports as f64 / 1_000_000_000.0) * input.sol_price_usd);

    // 12. ROI in basis points
    let roi_bps = if input.borrow_amount > 0 {
        (net_profit_lamports * 10_000) / input.borrow_amount as i64
    } else {
        0
    };

    // 13. Hurdle check (minimum profit floor from config/env)
    let hurdle_lamports = std::env::var("DEFAULT_MIN_PROFIT_LAMPORTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30_000); // 30k lamports default

    let passes_hurdle = net_profit_lamports >= hurdle_lamports as i64;

    PnlResult {
        gross_profit_lamports: gross_profit,
        costs,
        net_profit_lamports,
        net_profit_usd,
        roi_bps,
        passes_hurdle,
        hurdle_lamports,
    }
}

/// Convenience: calculate P&L for a 2-pool flash arb from engine evaluation.
pub fn pnl_from_2pool_eval(
    eval: &crate::dlmm_flash_arbitrage::DlmmFlashArbEvaluation,
    sol_price_usd: f64,
    priority_fee_micro_lamports: u64,
    jito_tip_lamports: u64,
    cu_limit: u32,
    new_atas_count: u8,
    tx_format: TxFormat,
    failure_probability: f64,
) -> PnlResult {
    let leg_quotes = vec![
        DlmmSwapQuote {
            amount_in: eval.borrow_amount,
            amount_out: eval.hop_a_out,
            fee_amount: eval.hop_a_out.saturating_sub(eval.hop_a_min_out), // conservative
            protocol_fee: 0,                                               // included in fee_amount
            end_active_id: 0,
            end_volatility_accumulator: 0,
            bins_crossed: eval.bins_a,
            price_impact_bps: arb_types::Bps(0),
        },
        DlmmSwapQuote {
            amount_in: eval.hop_a_min_out,
            amount_out: eval.hop_b_min_out,
            fee_amount: eval
                .hop_b_min_out
                .saturating_sub(eval.borrow_amount.saturating_add(eval.flash_fee)), // conservative
            protocol_fee: 0,
            end_active_id: 0,
            end_volatility_accumulator: 0,
            bins_crossed: eval.bins_b,
            price_impact_bps: arb_types::Bps(0),
        },
    ];

    calculate_pnl(PnlInput {
        borrow_amount: eval.borrow_amount,
        final_output: eval.hop_b_min_out,
        leg_quotes,
        kamino_flash_fee_sf: ((eval.flash_fee as u128 * (1u128 << 64))
            / eval.borrow_amount.max(1) as u128) as u64,
        cu_limit,
        priority_fee_micro_lamports,
        jito_tip_lamports,
        new_atas_count,
        tx_format,
        sol_price_usd,
        failure_probability,
    })
}

/// Convenience: calculate P&L for a 3-pool flash arb from engine evaluation.
pub fn pnl_from_3pool_eval(
    borrow_amount: u64,
    final_output: u64,
    leg_quotes: [DlmmSwapQuote; 3],
    flash_fee: u64,
    sol_price_usd: f64,
    priority_fee_micro_lamports: u64,
    jito_tip_lamports: u64,
    cu_limit: u32,
    new_atas_count: u8,
    tx_format: TxFormat,
    failure_probability: f64,
) -> PnlResult {
    let kamino_flash_fee_sf =
        ((flash_fee as u128 * (1u128 << 64)) / borrow_amount.max(1) as u128) as u64;
    calculate_pnl(PnlInput {
        borrow_amount,
        final_output,
        leg_quotes: leg_quotes.to_vec(),
        kamino_flash_fee_sf,
        cu_limit,
        priority_fee_micro_lamports,
        jito_tip_lamports,
        new_atas_count,
        tx_format,
        sol_price_usd,
        failure_probability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_math::dlmm::DlmmSwapQuote;

    fn dummy_quote() -> DlmmSwapQuote {
        DlmmSwapQuote {
            amount_in: 1_000_000,
            amount_out: 1_005_000,
            fee_amount: 5_000,
            protocol_fee: 500,
            end_active_id: 0,
            end_volatility_accumulator: 0,
            bins_crossed: 1,
            price_impact_bps: arb_types::Bps(0),
        }
    }

    #[test]
    fn flash_fee_ceiling_matches_bigint_reference() {
        // Mainnet WSOL reserve: flash_loan_fee_sf = 11_529_215_046_068 (1e-5 in Q60.60)
        // borrow = 1_000_000_000 lamports (1 WSOL)
        // fee = 10_000 lamports (0.001%)
        let fee_sf = 11_529_215_046_068;
        let borrow = 1_000_000_000;
        let fee = compute_kamino_flash_fee(borrow, fee_sf);
        assert_eq!(fee, 10_000);
    }

    #[test]
    fn priority_fee_ceiling() {
        assert_eq!(compute_priority_fee(400_000, 10_000), 4_000); // 400k * 10 / 1e6 = 4
        assert_eq!(compute_priority_fee(800_000, 10_000), 8_000);
        assert_eq!(compute_priority_fee(1_400_000, 5_000), 7_000); // 1.4M * 5 / 1e6 = 7
    }

    #[test]
    fn base_fee_format_dependent() {
        assert_eq!(compute_base_fee(TxFormat::LegacyOrV0, 1), 5_000);
        assert_eq!(compute_base_fee(TxFormat::V1, 1), 0);
    }

    #[test]
    fn rent_for_new_atas() {
        assert_eq!(compute_rent_for_new_atas(0), 0);
        assert_eq!(compute_rent_for_new_atas(1), 2_039_280);
        assert_eq!(compute_rent_for_new_atas(3), 6_117_840);
    }

    #[test]
    fn expected_failure_cost() {
        let guaranteed = 100_000;
        let tip = 10_000;
        assert_eq!(compute_expected_failure_cost(guaranteed, tip, 0.0), 0);
        assert_eq!(compute_expected_failure_cost(guaranteed, tip, 1.0), 110_000);
        assert_eq!(compute_expected_failure_cost(guaranteed, tip, 0.5), 55_000);
    }

    #[test]
    fn pnl_2pool_positive() {
        let quotes = vec![dummy_quote(), dummy_quote()];
        let res = calculate_pnl(PnlInput {
            borrow_amount: 1_000_000_000,
            final_output: 1_005_000_000,
            leg_quotes: quotes,
            kamino_flash_fee_sf: 18_446_744_073_709,
            cu_limit: 400_000,
            priority_fee_micro_lamports: 10_000,
            jito_tip_lamports: 10_000,
            new_atas_count: 2,
            tx_format: TxFormat::LegacyOrV0,
            sol_price_usd: 190.0,
            failure_probability: 0.1,
        });
        assert!(res.gross_profit_lamports > 0);
        assert!(res.net_profit_lamports > 0); // 5M gross - fees > 0
        assert!(res.passes_hurdle);
    }

    #[test]
    fn pnl_3pool_positive() {
        let quotes = [dummy_quote(), dummy_quote(), dummy_quote()];
        let res = pnl_from_3pool_eval(
            1_000_000_000,
            1_010_000_000,
            quotes,
            18_446,
            190.0,
            10_000,
            10_000,
            800_000,
            3,
            TxFormat::LegacyOrV0,
            0.1,
        );
        assert!(res.gross_profit_lamports > 0);
        assert!(res.net_profit_lamports > 0);
        assert!(res.passes_hurdle);
    }
}
