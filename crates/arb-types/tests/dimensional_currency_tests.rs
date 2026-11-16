//! Comprehensive Unit, Property, and Negative Integration Tests for Dimensional Currency Engine.
//!
//! Verifies:
//! 1. Deliberate invalid cross-currency operations (USDC + SOL) are rejected at runtime and compile-time.
//! 2. Accurate SOL fee conversion to route quote currency using timestamped conversion rates.
//! 3. Fixes the reported bug:
//!    - Initial: 100,000 raw USDC units (0.10 USDC)
//!    - Final: 99,914 raw USDC units (0.099914 USDC)
//!    - Difference: -86 raw USDC units (-0.000086 USDC)
//!    - Network costs: 9,000 lamports (5,000 base + 4,000 priority)
//!    - Correct converted network cost @ $180/SOL: 1,620 raw USDC units
//!    - Correct net profit: -1,706 raw USDC units (-170 bps), NOT -9,086 lamports and -908 bps!
//! 4. Non-double counting of embedded DEX swap fees.
//! 5. Separation of refundable rent from irrecoverable execution costs.

use arb_types::currency::*;

#[test]
fn test_negative_invalid_cross_currency_addition_rejected() {
    let slot = 450_000_000;
    let ts = 1_700_000_000_000;

    let usdc_amount = TypedTokenAmount::usdc(100_000, slot, ts); // 0.10 USDC
    let sol_amount = TypedTokenAmount::sol_lamports(9_000, slot, ts); // 9,000 lamports

    // Deliberately attempt invalid cross-token addition: USDC + SOL
    let result = usdc_amount.checked_add(&sol_amount);
    assert!(
        result.is_err(),
        "USDC + SOL must fail with CurrencyMismatch"
    );
    match result.unwrap_err() {
        DimensionalArithmeticError::CurrencyMismatch { lhs, rhs } => {
            assert_eq!(lhs, Currency::usdc());
            assert_eq!(rhs, Currency::Sol);
        }
        err => panic!("Expected CurrencyMismatch, got: {err:?}"),
    }

    // Deliberately attempt invalid cross-token subtraction: USDC - SOL
    let sub_result = usdc_amount.checked_sub(&sol_amount);
    assert!(
        sub_result.is_err(),
        "USDC - SOL must fail with CurrencyMismatch"
    );
    match sub_result.unwrap_err() {
        DimensionalArithmeticError::CurrencyMismatch { lhs, rhs } => {
            assert_eq!(lhs, Currency::usdc());
            assert_eq!(rhs, Currency::Sol);
        }
        err => panic!("Expected CurrencyMismatch, got: {err:?}"),
    }

    // Deliberately attempt invalid signed difference: USDC - SOL
    let diff_result = usdc_amount.checked_signed_diff(&sol_amount);
    assert!(
        diff_result.is_err(),
        "USDC diff SOL must fail with CurrencyMismatch"
    );
}

#[test]
fn test_negative_decimals_mismatch_rejected() {
    let slot = 450_000_000;
    let ts = 1_700_000_000_000;

    // Same currency identifier but mismatched decimals (e.g. corrupted metadata)
    let token_a = TypedTokenAmount::new(100, Currency::Sol, 9, ConversionSource::Native, slot, ts);
    let token_b = TypedTokenAmount::new(100, Currency::Sol, 6, ConversionSource::Native, slot, ts);

    let result = token_a.checked_add(&token_b);
    assert!(result.is_err());
    match result.unwrap_err() {
        DimensionalArithmeticError::DecimalMismatch {
            currency,
            lhs_dec,
            rhs_dec,
        } => {
            assert_eq!(currency, Currency::Sol);
            assert_eq!(lhs_dec, 9);
            assert_eq!(rhs_dec, 6);
        }
        err => panic!("Expected DecimalMismatch, got: {err:?}"),
    }
}

#[test]
fn test_compile_time_typed_amount_safety() {
    let slot = 450_000_000;
    let ts = 1_700_000_000_000;

    let sol1 = CompileTypedAmount::<SolTag>::new(5_000, slot, ts);
    let sol2 = CompileTypedAmount::<SolTag>::new(4_000, slot, ts);

    let sol_sum = sol1
        .checked_add(sol2)
        .expect("Same compile-time tag adds cleanly");
    assert_eq!(sol_sum.raw_amount, 9_000);

    let usdc1 = CompileTypedAmount::<UsdcTag>::new(100_000, slot, ts);
    let usdc2 = CompileTypedAmount::<UsdcTag>::new(99_914, slot, ts);

    let usdc_diff = usdc1
        .checked_sub(usdc2)
        .expect("Same compile-time tag subtracts cleanly");
    assert_eq!(usdc_diff.raw_amount, 86);

    // Note: Rust compiler strictly forbids `sol1.checked_add(usdc1)` because `SolTag` != `UsdcTag`.
}

#[test]
fn test_exact_repair_of_reported_p0_profit_accounting_bug() {
    let slot = 467_500_000;
    let ts = 1_700_000_000_000;

    // Reported case:
    // Initial: 100,000 raw USDC units = 0.10 USDC (dec = 6)
    // Final: 99,914 raw USDC units = 0.099914 USDC (dec = 6)
    // Difference: -86 raw USDC units = -0.000086 USDC
    // Hop1 output: 1,160,000 lamports = 0.001160 SOL (dec = 9)
    // Base fee: 5,000 lamports
    // Priority fee: 4,000 lamports
    let initial_quote = TypedTokenAmount::usdc(100_000, slot, ts);
    let hop1_out = TypedTokenAmount::sol_lamports(1_160_000, slot, ts);
    let final_quote = TypedTokenAmount::usdc(99_914, slot, ts);

    let base_fee = FeeItem::network_base_fee(5_000, slot, ts);
    let priority_fee = FeeItem::priority_fee(4_000, slot, ts);
    let hop1_dex_fee = FeeItem::dex_swap_fee("Hop 1 Meteora Fee", 400, Currency::Sol, 9, slot, ts);
    let hop2_dex_fee =
        FeeItem::dex_swap_fee("Hop 2 Meteora Fee", 400, Currency::usdc(), 6, slot, ts);

    let fees = vec![base_fee, priority_fee, hop1_dex_fee, hop2_dex_fee];

    // Conservative timestamped conversion rate: 1 SOL = 180.00 USDC
    // 9,000 lamports * 180 * 10^6 / 10^9 = 1,620 raw USDC units ($0.001620)
    let rate = TimestampedRate::sol_to_usdc(
        180,
        1,
        slot,
        ts,
        ConversionSource::ConfiguredRate {
            label: "Conservative Mainnet SOL/USDC Floor".to_string(),
        },
    )
    .unwrap();

    let report = DimensionalProfitabilityReport::compute(
        "CANDIDATE-REPAIR-001",
        "USDC -> SOL (DLMM) -> USDC (DLMM)",
        initial_quote,
        hop1_out,
        final_quote,
        fees,
        Some(rate),
        slot,
    )
    .expect("Dimensional profitability report computation must succeed");

    // 1. Gross profit must be -86 USDC raw units (NOT lamports!)
    assert_eq!(report.gross_profit_quote_raw, -86);
    assert_eq!(report.quote_mint, Currency::usdc());
    assert_eq!(report.quote_decimals, 6);

    // 2. Network fee in SOL must be 9,000 lamports
    assert_eq!(report.total_network_fees_sol.raw_amount, 9_000);
    assert_eq!(report.total_network_fees_sol.currency, Currency::Sol);

    // 3. Network fee converted to USDC must be 1,620 raw units
    assert_eq!(report.total_network_fees_in_quote.raw_amount, 1_620);
    assert_eq!(
        report.total_network_fees_in_quote.currency,
        Currency::usdc()
    );

    // 4. Total execution cost in quote token is 1,620 raw USDC units
    assert_eq!(report.total_execution_cost_quote_raw, 1_620);

    // 5. Net profit in USDC: -86 - 1,620 = -1,706 raw USDC units (-0.001706 USDC)
    // Note: The previous broken report claimed -9,086 lamports!
    assert_eq!(report.net_profit_quote_raw, -1_706);
    assert_ne!(report.net_profit_quote_raw, -9_086);

    // 6. Net profit in basis points: (-1,706 * 10,000) / 100,000 = -170 bps
    // Note: The previous broken report claimed -908 bps!
    assert_eq!(report.net_profit_bps, -170);
    assert_ne!(report.net_profit_bps, -908);

    // 7. DEX fees are recorded as embedded (800 units) and NOT double-counted
    assert_eq!(report.total_embedded_dex_fees_raw, 800);
    assert!(!report.is_profitable);
}

#[test]
fn test_exact_reconciliation_of_phase15_audit_example() {
    let slot = 467_766_000;
    let ts = 1_700_000_000_000;

    // Phase 15 audit observation:
    // Initial: 100,000 raw USDC = 0.1 USDC (dec = 6)
    // Final: 99,760 raw USDC = 0.099760 USDC (dec = 6)
    // Gross route difference: -240 raw USDC
    // Network costs: 9,000 SOL lamports (5,000 base + 4,000 priority)
    // Converted network cost: 1,620 raw USDC @ 180 USDC/SOL
    // Net loss: 1,860 raw USDC = -186 bps
    let initial_quote = TypedTokenAmount::usdc(100_000, slot, ts);
    let hop1_out = TypedTokenAmount::sol_lamports(1_176_961, slot, ts); // Intermediate WSOL (9 decimals)
    let final_quote = TypedTokenAmount::usdc(99_760, slot, ts);

    let base_fee = FeeItem::network_base_fee(5_000, slot, ts);
    let priority_fee = FeeItem::priority_fee(4_000, slot, ts);
    let hop1_dex_fee = FeeItem::dex_swap_fee(
        "Hop 1 Meteora Fee (USDC)",
        201,
        Currency::usdc(),
        6,
        slot,
        ts,
    );
    let hop2_dex_fee =
        FeeItem::dex_swap_fee("Hop 2 Meteora Fee (WSOL)", 475, Currency::Sol, 9, slot, ts);

    let fees = vec![base_fee, priority_fee, hop1_dex_fee, hop2_dex_fee];

    let rate = TimestampedRate::sol_to_usdc(
        180,
        1,
        slot,
        ts,
        ConversionSource::ConfiguredRate {
            label: "Mainnet SOL/USDC Reference Floor".to_string(),
        },
    )
    .unwrap();

    let report = DimensionalProfitabilityReport::compute(
        "AUDIT-PHASE15-001",
        "USDC -> WSOL (5rCf1DM8...) -> USDC (BVRbyLjj...)",
        initial_quote,
        hop1_out,
        final_quote,
        fees,
        Some(rate),
        slot,
    )
    .expect("Audit report computation must succeed");

    assert_eq!(
        report.gross_profit_quote_raw, -240,
        "Gross route difference must be -240 raw USDC"
    );
    assert_eq!(
        report.total_network_fees_sol.raw_amount, 9_000,
        "Network costs must be 9,000 lamports"
    );
    assert_eq!(
        report.total_network_fees_in_quote.raw_amount, 1_620,
        "Converted network cost must be 1,620 raw USDC"
    );
    assert_eq!(report.total_execution_cost_quote_raw, 1_620);
    assert_eq!(
        report.net_profit_quote_raw, -1_860,
        "Net loss must be exactly -1,860 raw USDC"
    );
    assert_eq!(
        report.net_profit_bps, -186,
        "Net loss bps must be exactly -186 bps"
    );
    assert_eq!(
        report.total_embedded_dex_fees_raw, 676,
        "Embedded DEX fees (201 + 475) must be 676 raw units"
    );
    assert!(!report.is_profitable);

    // Assert that cross-currency arithmetic bug (-240 + (-9000) = -9240) does not occur
    assert_ne!(report.net_profit_quote_raw, -9_240);
    assert_ne!(report.net_profit_bps, -924);
}

#[test]
fn test_profitable_route_with_dimensional_safety() {
    let slot = 467_500_000;
    let ts = 1_700_000_000_000;

    // 100,000 raw USDC in (0.10 USDC) -> 105,000 raw USDC out (0.105 USDC)
    // Gross profit = +5,000 raw USDC units
    let initial_quote = TypedTokenAmount::usdc(100_000, slot, ts);
    let hop1_out = TypedTokenAmount::sol_lamports(1_200_000, slot, ts);
    let final_quote = TypedTokenAmount::usdc(105_000, slot, ts);

    let base_fee = FeeItem::network_base_fee(5_000, slot, ts);
    let priority_fee = FeeItem::priority_fee(4_000, slot, ts);
    let rent = FeeItem::refundable_rent(2_039_280, slot, ts); // Temporary ATA rent

    let fees = vec![base_fee, priority_fee, rent];

    let rate = TimestampedRate::sol_to_usdc(
        180,
        1,
        slot,
        ts,
        ConversionSource::ConfiguredRate {
            label: "Mainnet Floor".into(),
        },
    )
    .unwrap();

    let report = DimensionalProfitabilityReport::compute(
        "PROFITABLE-001",
        "USDC -> SOL -> USDC",
        initial_quote,
        hop1_out,
        final_quote,
        fees,
        Some(rate),
        slot,
    )
    .unwrap();

    assert_eq!(report.gross_profit_quote_raw, 5_000);
    // Refundable rent (2_039_280 lamports) is preserved but NOT deducted from net profit
    assert_eq!(report.total_refundable_rent_lamports, 2_039_280);

    // Network fees = 9,000 lamports = 1,620 raw USDC units
    assert_eq!(report.total_execution_cost_quote_raw, 1_620);

    // Net profit = 5,000 - 1,620 = 3,380 raw USDC units (+0.00338 USDC)
    assert_eq!(report.net_profit_quote_raw, 3_380);
    // Net profit bps = (3,380 * 10,000) / 100,000 = +338 bps
    assert_eq!(report.net_profit_bps, 338);
    assert!(report.is_profitable);
}

#[test]
fn test_property_addition_commutativity_and_identity() {
    let slot = 450_000_000;
    let ts = 1_700_000_000_000;

    let zero = TypedTokenAmount::usdc(0, slot, ts);
    let a = TypedTokenAmount::usdc(45_123, slot, ts);
    let b = TypedTokenAmount::usdc(89_456, slot, ts);

    // Identity: a + 0 == a
    assert_eq!(a.checked_add(&zero).unwrap().raw_amount, a.raw_amount);

    // Commutativity: a + b == b + a
    let sum1 = a.checked_add(&b).unwrap();
    let sum2 = b.checked_add(&a).unwrap();
    assert_eq!(sum1.raw_amount, sum2.raw_amount);
    assert_eq!(sum1.currency, sum2.currency);

    // Subtraction inverse: (a + b) - b == a
    let recovered_a = sum1.checked_sub(&b).unwrap();
    assert_eq!(recovered_a.raw_amount, a.raw_amount);
}
