//! Feature 18: Authoritative Net-Profit Gate Evaluation — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Exact zero breakeven (net profit == 0) fails is_profitable() (requires strictly positive > 0).
//! 2. Minus one lamport loss signed as -1 without underflow panic or saturation.
//! 3. Priority fee exceeding gross profit rejected with negative net PnL.
//! 4. Jito tip exceeding gross profit rejected before execution.
//! 5. RiskGate exact hurdle boundary (min_net_profit_abs: 9999 rejected, 10000 accepted, 10001 accepted).

use arb_math::economic::{
    base_raw_to_lamports_ceil, build_economic_quote, calculate_net_profit_raw,
};
use arb_risk::{
    CircuitBreaker, KillSwitch, RiskGate, RiskLimits, minimum_net_profit_hurdle_lamports,
};
use arb_types::{Lamports, MintId, Slot, TokenAmount};
use solana_pubkey::Pubkey;
use std::sync::Arc;

fn wsol_mint() -> MintId {
    MintId(Pubkey::new_from_array([1u8; 32]))
}

/// Boundary Case 1: Exact zero breakeven (final_received == principal + costs).
/// Net profit is exactly 0 lamports; is_profitable() returns false (requires > 0).
#[test]
fn test_net_profit_gate_exact_zero_breakeven() {
    let borrow_principal = 1_000_000_000u64;
    let flash_fee = 50_000u64;
    let final_received = borrow_principal + flash_fee; // Exactly 1_000_050_000

    let net_raw = calculate_net_profit_raw(
        final_received,
        borrow_principal,
        flash_fee,
        0,
        0,
        0,
        0,
        Lamports(0),
        Lamports(0),
        Lamports(0),
        1,
        1,
    )
    .expect("exact zero net profit calculation must succeed");

    assert_eq!(net_raw, 0, "net profit must be exactly 0 lamports");

    let quote = build_economic_quote(
        wsol_mint(),
        9,
        TokenAmount(borrow_principal),
        TokenAmount(final_received),
        TokenAmount(final_received), // conservative output == expected
        TokenAmount(flash_fee),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        Lamports(0),
        Lamports(0),
        Lamports(0),
        1,
        1,
        TokenAmount(1), // requires at least 1 lamport net profit
        Slot(100),
        1_700_000_000_000,
    )
    .expect("economic quote build must succeed");

    assert_eq!(quote.minimum_net_profit_signed_raw, 0);
    assert_eq!(quote.expected_net_profit_signed_raw, 0);
    assert!(
        !quote.is_profitable(),
        "Exact zero breakeven must NOT be considered profitable when min hurdle > 0"
    );
}

/// Boundary Case 2: Minus one lamport loss (-1 lamport) is signed cleanly without underflow.
#[test]
fn test_net_profit_gate_minus_one_lamport_loss() {
    let borrow_principal = 1_000_000_000u64;
    let flash_fee = 50_000u64;
    let final_received = borrow_principal + flash_fee - 1; // 1_000_049_999

    let quote = build_economic_quote(
        wsol_mint(),
        9,
        TokenAmount(borrow_principal),
        TokenAmount(final_received),
        TokenAmount(final_received),
        TokenAmount(flash_fee),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        Lamports(0),
        Lamports(0),
        Lamports(0),
        1,
        1,
        TokenAmount(10_000),
        Slot(100),
        1_700_000_000_000,
    )
    .expect("quote build must handle negative signed PnL without panic");

    assert_eq!(
        quote.minimum_net_profit_signed_raw, -1,
        "Net profit must be signed as -1 without underflow wrap"
    );
    assert_eq!(quote.expected_net_profit_signed_raw, -1);
    assert!(!quote.is_profitable());
    assert!(!quote.meets_min_profit);
}

/// Boundary Case 3: Priority fee exceeds gross profit by 1 lamport.
#[test]
fn test_net_profit_gate_priority_fee_exceeds_gross_profit() {
    let borrow_principal = 1_000_000_000u64;
    let gross_profit = 5_000u64;
    let final_received = borrow_principal + gross_profit;
    let priority_fee = 5_001u64; // Exceeds gross profit by 1 lamport

    let quote = build_economic_quote(
        wsol_mint(),
        9,
        TokenAmount(borrow_principal),
        TokenAmount(final_received),
        TokenAmount(final_received),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        Lamports(0),
        Lamports(priority_fee),
        Lamports(0),
        1,
        1,
        TokenAmount(1_000),
        Slot(100),
        1_700_000_000_000,
    )
    .expect("quote build must succeed");

    assert_eq!(quote.gross_profit_raw.as_u64(), 5_000);
    assert_eq!(quote.expected_net_profit_signed_raw, -1);
    assert_eq!(quote.minimum_net_profit_signed_raw, -1);
    assert!(!quote.is_profitable());
    assert!(!quote.meets_min_profit);
}

/// Boundary Case 4: Jito tip exceeds gross profit by 5,000 lamports.
#[test]
fn test_net_profit_gate_jito_tip_exceeds_gross_profit() {
    let borrow_principal = 1_000_000_000u64;
    let gross_profit = 10_000u64;
    let final_received = borrow_principal + gross_profit;
    let jito_tip = 15_000u64; // Exceeds gross profit by 5_000 lamports

    let quote = build_economic_quote(
        wsol_mint(),
        9,
        TokenAmount(borrow_principal),
        TokenAmount(final_received),
        TokenAmount(final_received),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        TokenAmount(0),
        Lamports(0),
        Lamports(0),
        Lamports(jito_tip),
        1,
        1,
        TokenAmount(1_000),
        Slot(100),
        1_700_000_000_000,
    )
    .expect("quote build must succeed");

    assert_eq!(quote.gross_profit_raw.as_u64(), 10_000);
    assert_eq!(quote.expected_net_profit_signed_raw, -5_000);
    assert_eq!(quote.minimum_net_profit_signed_raw, -5_000);
    assert!(!quote.is_profitable());
}

/// Boundary Case 5: RiskGate exact hurdle boundary (9,999 vs 10,000 vs 10,001).
#[test]
fn test_risk_gate_min_profit_exact_hurdle_boundary() {
    let limits = RiskLimits {
        min_net_profit_abs: 10_000,
        min_net_profit_bps: 0, // Isolate absolute hurdle boundary
        ..Default::default()
    };
    let breaker = Arc::new(CircuitBreaker::new(5, 50));
    let kill_switch = Arc::new(KillSwitch::new());
    let gate = RiskGate::new(limits, breaker, kill_switch);

    let amount_in = Lamports(1_000_000_000);

    // 1 lamport below hurdle: must be rejected
    let res_below = gate.check_min_profit(Lamports(9_999), amount_in);
    assert!(res_below.is_err());
    assert!(
        res_below
            .unwrap_err()
            .contains("net profit 9999 < min_abs 10000"),
        "Below hurdle must return explicit rejection message"
    );

    // Exactly at hurdle: must pass
    let res_exact = gate.check_min_profit(Lamports(10_000), amount_in);
    assert!(res_exact.is_ok(), "Exact hurdle match must pass");

    // 1 lamport above hurdle: must pass
    let res_above = gate.check_min_profit(Lamports(10_001), amount_in);
    assert!(res_above.is_ok(), "Above hurdle must pass");
}

/// Cross-mint regression: one USDC (6 decimals) converts to 5,000,000
/// lamports at the verified $200/SOL rational rate. The 50 bps hurdle must
/// therefore be 25,000 lamports, not 5,000 (raw USDC units are not lamports).
#[test]
fn bps_hurdle_uses_lamports_for_non_wsol_base_mints() {
    let amount_in =
        base_raw_to_lamports_ceil(TokenAmount(1_000_000), 1, 5).expect("USDC notional conversion");
    assert_eq!(amount_in, Lamports(5_000_000));

    let limits = RiskLimits {
        min_net_profit_abs: 1,
        min_net_profit_bps: 50,
        ..Default::default()
    };
    let gate = RiskGate::new(
        limits,
        Arc::new(CircuitBreaker::new(5, 50)),
        Arc::new(KillSwitch::new()),
    );
    assert!(gate.check_min_profit(Lamports(24_999), amount_in).is_err());
    assert!(gate.check_min_profit(Lamports(25_000), amount_in).is_ok());
}

#[test]
fn bps_hurdle_rounds_up_fractional_lamports() {
    let limits = RiskLimits {
        min_net_profit_abs: 1,
        min_net_profit_bps: 50,
        ..Default::default()
    };
    let gate = RiskGate::new(
        limits,
        Arc::new(CircuitBreaker::new(5, 50)),
        Arc::new(KillSwitch::new()),
    );
    // 5,000,001 * 50 / 10,000 = 25,000.005; required hurdle is 25,001.
    let input_lamports = Lamports(5_000_001);
    assert!(
        gate.check_min_profit(Lamports(25_000), input_lamports)
            .is_err()
    );
    assert!(
        gate.check_min_profit(Lamports(25_001), input_lamports)
            .is_ok()
    );
}

#[test]
fn shared_min_profit_hurdle_uses_max_absolute_or_bps_and_rounds_up() {
    assert_eq!(
        minimum_net_profit_hurdle_lamports(Lamports(10_000_000), Lamports(100_000), 50),
        Lamports(100_000),
    );
    assert_eq!(
        minimum_net_profit_hurdle_lamports(Lamports(5_000_000_000), Lamports(100_000), 50),
        Lamports(25_000_000),
    );
    assert_eq!(
        minimum_net_profit_hurdle_lamports(Lamports(5_000_001), Lamports(1), 50),
        Lamports(25_001),
    );
}
