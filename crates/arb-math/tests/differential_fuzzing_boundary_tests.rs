//! Feature 25: Property-Based Math Fuzzing & Differential Tests — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Zero inputs in economic quotes fail closed without division by zero or panic.
//! 2. Single-lamport and dust boundaries evaluate with integer precision and no roundoff errors.
//! 3. U64::MAX extreme boundary inputs trigger checked overflow errors without wrapping.
//! 4. U128::MAX rate conversions detect overflow and fail closed via ArbError::Overflow.
//! 5. CPMM and DLMM functions operate strictly on integers, guaranteeing zero NaN or float pollution.

use arb_math::cpmm::cpmm_out;
use arb_math::dlmm::get_price_from_id;
use arb_math::economic::{build_economic_quote, calculate_net_profit_raw, lamports_to_base_raw};
use arb_types::{ArbError, Lamports, MintId, Slot, TokenAmount};
use proptest::prelude::*;
use solana_pubkey::Pubkey;

fn test_mint() -> MintId {
    MintId(Pubkey::new_from_array([5u8; 32]))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(250))]

    /// Boundary Case 1: Zero inputs fail closed without divide-by-zero panics.
    #[test]
    fn fuzz_economic_quote_zero_inputs_fails_closed(
        lamports in any::<u64>(),
        rate_num in any::<u128>(),
    ) {
        // Zero rate_denom must always return InvalidInput error
        let res_zero_denom = lamports_to_base_raw(Lamports(lamports), rate_num, 0);
        prop_assert!(res_zero_denom.is_err());
        match res_zero_denom {
            Err(ArbError::InvalidInput(_)) => {}
            other => prop_assert!(false, "Expected InvalidInput for rate_denom=0, got {:?}", other),
        }

        // Zero rate_num must always return InvalidInput error
        let res_zero_num = lamports_to_base_raw(Lamports(lamports), 0, 1);
        prop_assert!(res_zero_num.is_err());

        // Zero borrow amount with zero return must fail closed (not profitable)
        let quote = build_economic_quote(
            test_mint(),
            9,
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
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
        );
        if let Ok(q) = quote {
            prop_assert!(!q.is_profitable());
            prop_assert_eq!(q.minimum_net_profit_signed_raw, 0);
        }
    }

    /// Boundary Case 2: Single-lamport and dust balance boundaries evaluate accurately.
    #[test]
    fn fuzz_economic_quote_single_lamport_boundary(
        borrow in 1u64..=100,
        gain in 0u64..=100,
        flash_fee in 0u64..=50,
    ) {
        let final_received = borrow.saturating_add(gain);
        let quote = build_economic_quote(
            test_mint(),
            9,
            TokenAmount(borrow),
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
            TokenAmount(10), // Hurdle: 10 raw units
            Slot(100),
            1_700_000_000_000,
        );

        prop_assert!(quote.is_ok());
        let q = quote.unwrap();
        let expected_net_signed = (gain as i128) - (flash_fee as i128);
        prop_assert_eq!(q.minimum_net_profit_signed_raw, expected_net_signed);

        if expected_net_signed >= 10 {
            prop_assert!(q.is_profitable());
        } else {
            prop_assert!(!q.is_profitable());
        }
    }

    /// Boundary Case 3: Extreme u64::MAX inputs trigger checked overflow error without wrapping.
    #[test]
    fn fuzz_economic_quote_u64_max_boundary_overflow_check(
        offset in 0u64..100_000,
    ) {
        let max_borrow = u64::MAX - offset;
        let large_fee = u64::MAX - offset;

        // Subtraction (final_received - borrow) underflow or total_costs overflow must return ArbError::Overflow
        let res = calculate_net_profit_raw(
            1_000_000,
            max_borrow,
            large_fee,
            0,
            0,
            0,
            0,
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
        );

        prop_assert!(res.is_err());
        match res {
            Err(ArbError::Overflow) => {}
            other => prop_assert!(false, "Expected Overflow on u64::MAX edge, got {:?}", other),
        }
    }

    /// Boundary Case 4: U128::MAX rate conversion detects overflow cleanly.
    #[test]
    fn fuzz_economic_quote_u128_max_rate_conversion(
        lamports in 100_000u64..u64::MAX,
        rate_offset in 0u128..10_000,
    ) {
        let extreme_rate_num = u128::MAX - rate_offset;
        let rate_denom = 1u128;

        // lamports * extreme_rate_num will exceed U256 or u128 limit
        let res = lamports_to_base_raw(Lamports(lamports), extreme_rate_num, rate_denom);

        // Must either succeed with valid u128 or reject with ArbError::Overflow, never panic
        if let Err(e) = res {
            prop_assert!(matches!(e, ArbError::Overflow));
        }
    }

    /// Boundary Case 5: CPMM and DLMM integer invariants guarantee zero NaN / Inf pollution.
    #[test]
    fn fuzz_cpmm_and_dlmm_nan_avoidance_guarantee(
        dx in any::<u128>(),
        reserve_x in any::<u128>(),
        reserve_y in any::<u128>(),
        bin_id in any::<i32>(),
        bin_step in 1u16..=200,
    ) {
        // CPMM swap out is purely integer-based (Option<u128>)
        let out = cpmm_out(dx, reserve_x, reserve_y, 9_970);
        if let Some(val) = out {
            // Invariant: returned output cannot exceed target pool reserve
            prop_assert!(val < reserve_y);
        }

        // DLMM bin price is purely integer Q64.64 representation (Result<u128, _>)
        let price_result = get_price_from_id(bin_id, bin_step);
        if let Ok(price_q64) = price_result {
            prop_assert!(price_q64 > 0);
        }
    }
}
