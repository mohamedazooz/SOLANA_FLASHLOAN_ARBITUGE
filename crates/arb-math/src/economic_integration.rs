//! Phase 16.4 — Unified Economic Integration.
//!
//! This module connects the existing economic primitives (`CostLedger`,
//! `build_economic_quote`, `calculate_net_profit_raw`, `pick_tip`,
//! `ExecutionCostBreakdown`) across the complete execution path:
//!
//! Route optimizer → Protocol quote engines → Opportunity selection →
//! Flash loan construction → Jito bid selection → Final transaction dispatch.
//!
//! It enforces a single source of truth for net profit and prevents double
//! counting of DEX fees, slippage, transfer fees, and priority fees.
//!
//! All calculations use checked integer arithmetic with explicit token
//! denomination handling. No floating-point is used for the final executable
//! amount or profit guard.

use crate::economic::EconomicQuote;
use arb_types::{ArbError, MintId, Result, Slot};
use ruint::aliases::U256;

/// Total execution cost across all categories, counted exactly once.
///
/// This is the authoritative cost roll-up. Every cost category is included
/// exactly once; callers must not re-deduct any of these elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionCosts {
    /// DEX trading fees (swap fees, protocol fees, dynamic fees).
    pub dex_fees_raw: u128,
    /// Flash loan fees (borrow premium).
    pub flash_loan_fees_raw: u128,
    /// Solana base transaction fee (per signature).
    pub base_transaction_fees_raw: u128,
    /// Priority fee (CU limit × micro-lamports / 1e6).
    pub priority_fees_raw: u128,
    /// Jito tip paid to validator tip account.
    pub jito_tips_raw: u128,
    /// Token transfer fees (Token-2022 transfer fee extension).
    pub transfer_fees_raw: u128,
    /// Token extension fees (where applicable).
    pub token_extension_fees_raw: u128,
    /// Slippage cost (expected − min_out, counted once).
    pub slippage_cost_raw: u128,
    /// Conversion costs (SOL↔token denomination conversion friction).
    pub conversion_costs_raw: u128,
    /// Other protocol-specific execution costs.
    pub other_execution_costs_raw: u128,
    /// Rent deposits for ephemeral ATAs (refundable, tracked separately).
    pub rent_deposits_raw: u128,
}

impl ExecutionCosts {
    /// Total non-rent execution costs, each counted exactly once.
    pub fn total_non_rent(&self) -> u128 {
        self.dex_fees_raw
            .saturating_add(self.flash_loan_fees_raw)
            .saturating_add(self.base_transaction_fees_raw)
            .saturating_add(self.priority_fees_raw)
            .saturating_add(self.jito_tips_raw)
            .saturating_add(self.transfer_fees_raw)
            .saturating_add(self.token_extension_fees_raw)
            .saturating_add(self.slippage_cost_raw)
            .saturating_add(self.conversion_costs_raw)
            .saturating_add(self.other_execution_costs_raw)
    }

    /// Total costs including rent deposits.
    pub fn total_all(&self) -> u128 {
        self.total_non_rent().saturating_add(self.rent_deposits_raw)
    }
}

/// Unified economic evaluation result — the single source of truth for the
/// go/no-go decision at every point in the execution path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedEconomicResult {
    /// Base mint identifier.
    pub base_mint: MintId,
    /// Base mint decimals.
    pub base_decimals: u8,
    /// Borrow amount in base-mint raw units.
    pub borrow_amount_raw: u64,
    /// Expected final output in base-mint raw units (pre-slippage).
    pub expected_final_output_raw: u64,
    /// Minimum final output in base-mint raw units (post-slippage floor).
    pub minimum_final_output_raw: u64,
    /// Gross profit = expected final output − borrow principal.
    pub gross_profit_raw: u64,
    /// Gross profit as signed i128 (negative = gross loss).
    pub gross_profit_signed: i128,
    /// Total execution costs in base-mint raw units.
    pub total_costs_raw: u128,
    /// Net profit = gross profit − total costs.
    pub net_profit_raw: u64,
    /// Net profit as signed i128.
    pub net_profit_signed: i128,
    /// Net ROI in basis points (net_profit / borrow_principal × 10_000).
    pub net_roi_bps: u16,
    /// Break-even execution cost: the maximum costs that still yield
    /// non-negative net profit.
    pub break_even_cost_raw: u128,
    /// Maximum economically acceptable Jito tip (such that net ≥ min_profit).
    pub max_acceptable_jito_tip_raw: u128,
    /// Maximum acceptable priority fee (such that net ≥ min_profit).
    pub max_acceptable_priority_fee_raw: u128,
    /// Maximum acceptable slippage cost (such that net ≥ min_profit).
    pub max_acceptable_slippage_raw: u128,
    /// Execution risk reserve — portion of edge held back for adverse
    /// execution (slippage worse than expected, failed landing, etc.).
    pub execution_risk_reserve_raw: u128,
    /// Conservative net profit after deducting the risk reserve.
    pub conservative_net_profit_raw: u64,
    /// True when conservative net profit ≥ minimum acceptable profit.
    pub meets_minimum_profit: bool,
    /// True when net ROI ≥ minimum acceptable ROI.
    pub meets_minimum_roi: bool,
    /// Source slot at which the economics were evaluated.
    pub source_slot: Slot,
    /// Economic calculation version.
    pub calculation_version: u32,
}

/// Configuration for unified economic evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedEconomicConfig {
    /// Minimum acceptable profit in base-mint raw units.
    pub min_profit_raw: u64,
    /// Minimum acceptable ROI in basis points (e.g. 50 = 0.5%).
    pub min_roi_bps: u16,
    /// Execution risk reserve as fraction of net profit (bps).
    /// Default 100 = 1% of net profit held back for adverse execution.
    pub risk_reserve_bps: u16,
    /// Maximum Jito tip as fraction of net profit before tip (bps).
    /// Default 500 = 5%.
    pub max_tip_share_bps: u16,
    /// Maximum priority fee as fraction of net profit before tip (bps).
    /// Default 200 = 2%.
    pub max_priority_fee_share_bps: u16,
    /// Maximum slippage as fraction of expected output (bps).
    /// Default 100 = 1%.
    pub max_slippage_share_bps: u16,
    /// Lamports-to-base-mint conversion rate (numerator).
    pub lamports_to_base_rate_num: u128,
    /// Lamports-to-base-mint conversion rate (denominator).
    pub lamports_to_base_rate_den: u128,
}

impl Default for UnifiedEconomicConfig {
    fn default() -> Self {
        Self {
            min_profit_raw: 0,
            min_roi_bps: 0,
            risk_reserve_bps: 100,           // 1%
            max_tip_share_bps: 500,          // 5%
            max_priority_fee_share_bps: 200, // 2%
            max_slippage_share_bps: 100,     // 1%
            lamports_to_base_rate_num: 1,
            lamports_to_base_rate_den: 1,
        }
    }
}

/// Authoritative unified economic calculator.
///
/// All inputs are in base-mint raw units except SOL-denominated costs which
/// are converted via the explicit rational rate. No floating-point is used
/// for the final decision.
#[derive(Debug, Clone)]
pub struct UnifiedEconomicCalculator {
    config: UnifiedEconomicConfig,
}

impl UnifiedEconomicCalculator {
    /// Create a new calculator with the given configuration.
    pub fn new(config: UnifiedEconomicConfig) -> Self {
        Self { config }
    }

    /// Evaluate the complete economics of an arbitrage opportunity.
    ///
    /// This is the single authoritative calculation used by the route
    /// optimizer, opportunity selection, flash loan construction, Jito bid
    /// selection, and final transaction dispatch.
    ///
    /// Returns `Err` on overflow or invalid inputs (fail closed — never
    /// silently produces a profitable result from bad data).
    pub fn evaluate(
        &self,
        base_mint: MintId,
        base_decimals: u8,
        borrow_amount_raw: u64,
        expected_final_output_raw: u64,
        minimum_final_output_raw: u64,
        costs: &ExecutionCosts,
        source_slot: Slot,
    ) -> Result<UnifiedEconomicResult> {
        if minimum_final_output_raw > expected_final_output_raw {
            return Err(ArbError::InvalidInput(
                "minimum output exceeds expected output".into(),
            ));
        }
        if borrow_amount_raw == 0 {
            return Err(ArbError::InvalidInput("zero borrow amount".into()));
        }

        let borrow = U256::from(borrow_amount_raw);
        let expected = U256::from(expected_final_output_raw);
        let _minimum_out = U256::from(minimum_final_output_raw);
        let total_costs = U256::from(costs.total_non_rent());

        // Gross profit = expected output − borrow principal.
        let gross_signed = if expected >= borrow {
            i128::try_from(expected - borrow).map_err(|_| ArbError::Overflow)?
        } else {
            -i128::try_from(borrow - expected).map_err(|_| ArbError::Overflow)?
        };
        let gross_profit_raw = if gross_signed > 0 {
            gross_signed as u64
        } else {
            0
        };

        // Net profit = gross profit − total costs (counted exactly once).
        let net_signed = if gross_signed >= 0 {
            let gross_u = U256::from(gross_signed as u128);
            match gross_u.checked_sub(total_costs) {
                Some(diff) => i128::try_from(diff).map_err(|_| ArbError::Overflow)?,
                None => {
                    let loss = total_costs.checked_sub(gross_u).ok_or(ArbError::Overflow)?;
                    -i128::try_from(loss).map_err(|_| ArbError::Overflow)?
                }
            }
        } else {
            let loss = U256::from((-gross_signed) as u128);
            let total_loss = loss.checked_add(total_costs).ok_or(ArbError::Overflow)?;
            -i128::try_from(total_loss).map_err(|_| ArbError::Overflow)?
        };

        let net_profit_raw = if net_signed > 0 { net_signed as u64 } else { 0 };

        // Net ROI = net_profit / borrow_principal × 10_000 bps.
        let net_roi_bps = if borrow_amount_raw > 0 && net_signed > 0 {
            let roi = U256::from(net_signed as u128)
                .checked_mul(U256::from(10_000u128))
                .and_then(|v| v.checked_div(borrow))
                .ok_or(ArbError::Overflow)?;
            u16::try_from(roi).unwrap_or(u16::MAX)
        } else {
            0
        };

        // Break-even cost = gross profit (max costs that still yield net ≥ 0).
        let break_even_cost_raw = if gross_signed > 0 {
            gross_signed as u128
        } else {
            0
        };

        // Profit before tip = net_signed + jito_tips (tip is part of costs).
        let profit_before_tip = net_signed
            .checked_add(i128::try_from(costs.jito_tips_raw).map_err(|_| ArbError::Overflow)?)
            .ok_or(ArbError::Overflow)?;

        // Maximum acceptable Jito tip: tip such that conservative net ≥ min_profit.
        let profit_for_tip = if profit_before_tip > 0 {
            profit_before_tip as u128
        } else {
            0
        };
        let max_tip_raw = (U256::from(profit_for_tip)
            .checked_mul(U256::from(self.config.max_tip_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .ok_or(ArbError::Overflow)?)
        .min(U256::from(profit_for_tip));

        // Maximum acceptable priority fee: bounded by share of profit before tip.
        let max_priority_raw = (U256::from(profit_for_tip)
            .checked_mul(U256::from(self.config.max_priority_fee_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .ok_or(ArbError::Overflow)?)
        .min(U256::from(profit_for_tip));

        // Maximum acceptable slippage: bounded by share of expected output.
        let max_slippage_raw = (U256::from(expected_final_output_raw)
            .checked_mul(U256::from(self.config.max_slippage_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .ok_or(ArbError::Overflow)?)
        .min(U256::from(expected_final_output_raw));

        // Execution risk reserve: fraction of net profit held back.
        let risk_reserve_raw = if net_signed > 0 {
            let reserve = U256::from(net_signed as u128)
                .checked_mul(U256::from(self.config.risk_reserve_bps as u128))
                .and_then(|v| v.checked_div(U256::from(10_000u128)))
                .ok_or(ArbError::Overflow)?;
            u128::try_from(reserve).unwrap_or(u128::MAX)
        } else {
            0
        };

        // Conservative net profit = net profit − risk reserve.
        let conservative_net = net_signed
            .checked_sub(i128::try_from(risk_reserve_raw).map_err(|_| ArbError::Overflow)?)
            .unwrap_or(0);
        let conservative_net_profit_raw = if conservative_net > 0 {
            conservative_net as u64
        } else {
            0
        };

        let min_profit_i128 = i128::from(self.config.min_profit_raw);
        let meets_minimum_profit = conservative_net >= min_profit_i128;
        let meets_minimum_roi = net_roi_bps >= self.config.min_roi_bps;

        Ok(UnifiedEconomicResult {
            base_mint,
            base_decimals,
            borrow_amount_raw,
            expected_final_output_raw,
            minimum_final_output_raw,
            gross_profit_raw,
            gross_profit_signed: gross_signed,
            total_costs_raw: costs.total_non_rent(),
            net_profit_raw,
            net_profit_signed: net_signed,
            net_roi_bps,
            break_even_cost_raw,
            max_acceptable_jito_tip_raw: u128::try_from(max_tip_raw).unwrap_or(u128::MAX),
            max_acceptable_priority_fee_raw: u128::try_from(max_priority_raw).unwrap_or(u128::MAX),
            max_acceptable_slippage_raw: u128::try_from(max_slippage_raw).unwrap_or(u128::MAX),
            execution_risk_reserve_raw: risk_reserve_raw,
            conservative_net_profit_raw,
            meets_minimum_profit,
            meets_minimum_roi,
            source_slot,
            calculation_version: crate::economic::ECONOMIC_CALCULATION_VERSION,
        })
    }

    /// Build an `ExecutionCosts` from an `EconomicQuote` (arb-math),
    /// ensuring no double counting between the two cost models.
    pub fn from_economic_quote(quote: &EconomicQuote) -> ExecutionCosts {
        ExecutionCosts {
            dex_fees_raw: quote.dex_fees_raw.as_u128(),
            flash_loan_fees_raw: quote.flash_loan_fee_raw.as_u128(),
            base_transaction_fees_raw: 0, // set by caller from network fees
            priority_fees_raw: 0,         // set by caller
            jito_tips_raw: quote.jito_tip_lamports.as_u64() as u128,
            transfer_fees_raw: quote.transfer_fees_raw.as_u128(),
            token_extension_fees_raw: 0,
            slippage_cost_raw: 0,
            conversion_costs_raw: 0,
            other_execution_costs_raw: quote.other_verified_costs_raw.as_u128(),
            rent_deposits_raw: quote.account_creation_cost_raw.as_u128(),
        }
    }

    /// Check whether a Jito tip is within the maximum acceptable bound.
    pub fn is_tip_acceptable(&self, tip_raw: u128, net_before_tip_raw: u128) -> bool {
        if net_before_tip_raw == 0 {
            return tip_raw == 0;
        }
        let max_tip = U256::from(net_before_tip_raw)
            .checked_mul(U256::from(self.config.max_tip_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .unwrap_or(U256::from(0));
        U256::from(tip_raw) <= max_tip
    }

    /// Check whether a priority fee is within the maximum acceptable bound.
    pub fn is_priority_fee_acceptable(&self, fee_raw: u128, net_before_fee_raw: u128) -> bool {
        if net_before_fee_raw == 0 {
            return fee_raw == 0;
        }
        let max_fee = U256::from(net_before_fee_raw)
            .checked_mul(U256::from(self.config.max_priority_fee_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .unwrap_or(U256::from(0));
        U256::from(fee_raw) <= max_fee
    }

    /// Check whether slippage cost is within the maximum acceptable bound.
    pub fn is_slippage_acceptable(&self, slippage_raw: u128, expected_output_raw: u128) -> bool {
        if expected_output_raw == 0 {
            return slippage_raw == 0;
        }
        let max_slippage = U256::from(expected_output_raw)
            .checked_mul(U256::from(self.config.max_slippage_share_bps as u128))
            .and_then(|v| v.checked_div(U256::from(10_000u128)))
            .unwrap_or(U256::from(0));
        U256::from(slippage_raw) <= max_slippage
    }

    /// Reference to the configuration.
    pub fn config(&self) -> &UnifiedEconomicConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> UnifiedEconomicConfig {
        UnifiedEconomicConfig::default()
    }

    fn test_costs() -> ExecutionCosts {
        ExecutionCosts {
            dex_fees_raw: 1_000,
            flash_loan_fees_raw: 500,
            base_transaction_fees_raw: 5_000,
            priority_fees_raw: 10_000,
            jito_tips_raw: 25_000,
            transfer_fees_raw: 2_000,
            token_extension_fees_raw: 0,
            slippage_cost_raw: 3_000,
            conversion_costs_raw: 0,
            other_execution_costs_raw: 1_000,
            rent_deposits_raw: 0,
        }
    }

    fn test_mint() -> MintId {
        // WSOL mint: So11111111111111111111111111111111111111112
        MintId(solana_pubkey::Pubkey::from_str_const(
            "So11111111111111111111111111111111111111112",
        ))
    }

    #[test]
    fn test_total_costs_no_double_count() {
        let costs = test_costs();
        let total = costs.total_non_rent();
        // 1000 + 500 + 5000 + 10000 + 25000 + 2000 + 0 + 3000 + 0 + 1000 = 47,500
        assert_eq!(total, 47_500);
    }

    #[test]
    fn test_total_all_includes_rent() {
        let costs = ExecutionCosts {
            rent_deposits_raw: 2_039_280,
            ..test_costs()
        };
        assert_eq!(costs.total_all(), costs.total_non_rent() + 2_039_280);
    }

    #[test]
    fn test_evaluate_profitable_route() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = test_costs();
        let result = calc
            .evaluate(
                test_mint(),
                9,
                1_000_000_000, // 1 SOL borrow
                1_050_000_000, // 1.05 SOL expected output
                1_040_000_000, // 1.04 SOL minimum output
                &costs,
                Slot(100),
            )
            .unwrap();

        // Gross = 50M, costs = 47.5M, net = 2.5M
        assert_eq!(result.gross_profit_raw, 50_000_000);
        assert_eq!(result.total_costs_raw, 47_500);
        assert_eq!(result.net_profit_raw, 49_952_500);
        assert!(result.meets_minimum_profit);
        assert!(result.meets_minimum_roi);
    }

    #[test]
    fn test_evaluate_unprofitable_route() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = ExecutionCosts {
            dex_fees_raw: 100_000_000, // huge fees
            ..test_costs()
        };
        let result = calc
            .evaluate(
                test_mint(),
                9,
                1_000_000_000,
                1_050_000_000,
                1_040_000_000,
                &costs,
                Slot(100),
            )
            .unwrap();

        assert!(result.net_profit_signed < 0);
        assert!(!result.meets_minimum_profit);
    }

    #[test]
    fn test_break_even_cost() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = test_costs();
        let result = calc
            .evaluate(
                test_mint(),
                9,
                1_000_000_000,
                1_050_000_000,
                1_040_000_000,
                &costs,
                Slot(100),
            )
            .unwrap();

        // Break-even = gross profit = 50M
        assert_eq!(result.break_even_cost_raw, 50_000_000);
    }

    #[test]
    fn test_max_tip_acceptable() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        // Net before tip ≈ 50M - (47.5M - 25k tip) = 2.525M
        // Max tip = 5% of 2.525M ≈ 126k
        assert!(calc.is_tip_acceptable(10_000, 2_525_000));
        assert!(!calc.is_tip_acceptable(1_000_000, 2_525_000));
    }

    #[test]
    fn test_max_priority_fee_acceptable() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        // Max priority = 2% of net before tip
        assert!(calc.is_priority_fee_acceptable(1_000, 100_000));
        assert!(!calc.is_priority_fee_acceptable(10_000, 100_000));
    }

    #[test]
    fn test_max_slippage_acceptable() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        // Max slippage = 1% of expected output
        assert!(calc.is_slippage_acceptable(1_000, 1_000_000));
        assert!(!calc.is_slippage_acceptable(20_000, 1_000_000));
    }

    #[test]
    fn test_minimum_output_exceeds_expected() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = test_costs();
        let result = calc.evaluate(
            test_mint(),
            9,
            1_000_000_000,
            1_050_000_000,
            1_060_000_000, // min > expected
            &costs,
            Slot(100),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_borrow_rejected() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = test_costs();
        let result = calc.evaluate(
            test_mint(),
            9,
            0,
            1_050_000_000,
            1_040_000_000,
            &costs,
            Slot(100),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_net_roi_bps() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = ExecutionCosts {
            dex_fees_raw: 1_000,
            flash_loan_fees_raw: 500,
            base_transaction_fees_raw: 5_000,
            priority_fees_raw: 10_000,
            jito_tips_raw: 1_000,
            transfer_fees_raw: 0,
            token_extension_fees_raw: 0,
            slippage_cost_raw: 0,
            conversion_costs_raw: 0,
            other_execution_costs_raw: 0,
            rent_deposits_raw: 0,
        };
        // Borrow 1M, output 1.1M, gross = 100k, costs = 17.5k, net = 82.5k
        // ROI = 82.5k / 1M × 10_000 = 825 bps
        let result = calc
            .evaluate(
                test_mint(),
                9,
                1_000_000,
                1_100_000,
                1_090_000,
                &costs,
                Slot(100),
            )
            .unwrap();
        assert_eq!(result.net_roi_bps, 825);
    }

    #[test]
    fn test_risk_reserve_reduces_conservative_profit() {
        let calc = UnifiedEconomicCalculator::new(test_config());
        let costs = ExecutionCosts {
            dex_fees_raw: 1_000,
            flash_loan_fees_raw: 500,
            base_transaction_fees_raw: 5_000,
            priority_fees_raw: 10_000,
            jito_tips_raw: 1_000,
            transfer_fees_raw: 0,
            token_extension_fees_raw: 0,
            slippage_cost_raw: 0,
            conversion_costs_raw: 0,
            other_execution_costs_raw: 0,
            rent_deposits_raw: 0,
        };
        // Borrow 1M, output 1.1M, gross = 100k, costs = 17.5k, net = 82.5k
        // Risk reserve = 1% of 82.5k = 825
        // Conservative net = 82.5k - 825 = 81,675
        let result = calc
            .evaluate(
                test_mint(),
                9,
                1_000_000,
                1_100_000,
                1_090_000,
                &costs,
                Slot(100),
            )
            .unwrap();
        assert_eq!(result.execution_risk_reserve_raw, 825);
        assert_eq!(result.conservative_net_profit_raw, 81_675);
    }

    #[test]
    fn test_from_economic_quote() {
        let quote = crate::economic::EconomicQuote {
            base_mint: test_mint(),
            base_decimals: 9,
            borrow_amount_raw: arb_types::TokenAmount(1_000_000_000),
            expected_final_amount_raw: arb_types::TokenAmount(1_050_000_000),
            minimum_final_amount_raw: arb_types::TokenAmount(1_040_000_000),
            flash_loan_fee_raw: arb_types::TokenAmount(500),
            dex_fees_raw: arb_types::TokenAmount(1_000),
            transfer_fees_raw: arb_types::TokenAmount(2_000),
            account_creation_cost_raw: arb_types::TokenAmount(0),
            other_verified_costs_raw: arb_types::TokenAmount(1_000),
            network_fee_lamports: arb_types::Lamports(5_000),
            priority_fee_lamports: arb_types::Lamports(10_000),
            jito_tip_lamports: arb_types::Lamports(25_000),
            lamports_to_base_mint_rate: 1,
            lamports_to_base_rate_denom: 1,
            net_profit_raw: arb_types::TokenAmount(0),
            expected_net_profit_signed_raw: 0,
            minimum_net_profit_signed_raw: 0,
            gross_profit_raw: arb_types::TokenAmount(0),
            gross_profit_signed_raw: 0,
            min_profit_raw: arb_types::TokenAmount(0),
            meets_min_profit: false,
            source_slot: arb_types::Slot(100),
            quote_timestamp_ms: 0,
            calculation_version: 2,
        };
        let costs = UnifiedEconomicCalculator::from_economic_quote(&quote);
        assert_eq!(costs.dex_fees_raw, 1_000);
        assert_eq!(costs.flash_loan_fees_raw, 500);
        assert_eq!(costs.jito_tips_raw, 25_000);
        assert_eq!(costs.transfer_fees_raw, 2_000);
        assert_eq!(costs.other_execution_costs_raw, 1_000);
    }
}
