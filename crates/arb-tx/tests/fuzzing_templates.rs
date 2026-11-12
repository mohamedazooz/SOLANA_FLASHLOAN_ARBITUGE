//! Fuzzing Suite for Transaction Templates, Jito Bundle Validation, and Fees.
//!
//! Fuzzes:
//! 1. PreWarmedTxTemplate offset location against arbitrary byte mutations
//! 2. Jito bundle bounds validation
//! 3. Kamino flash loan fee computation across arbitrary u64 inputs
//! 4. Dynamic CU price and priority fee calculation

use arb_tx::{
    PreWarmedTxTemplate, calculate_dynamic_cu_limit, cap_cu_price_by_profit,
    kamino::calculate_kamino_flash_fee, validate_jito_bundle,
};
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn fuzz_locate_offsets_never_panics(
        data in vec(any::<u8>(), 0..=1000),
        blockhash in any::<[u8; 32]>(),
        tip in any::<u64>(),
    ) {
        // Any random wire byte buffer must fail cleanly without panic
        let _ = PreWarmedTxTemplate::locate_offsets(&data, &blockhash, tip);
    }

    #[test]
    fn fuzz_validate_jito_bundle_never_panics(
        bundle in vec(vec(any::<u8>(), 0..=1500), 0..=10)
    ) {
        let res = validate_jito_bundle(&bundle);
        if bundle.is_empty() || bundle.len() > 5 {
            prop_assert!(res.is_err(), "Empty bundle or > 5 txs must return Err");
        }
        // Version-aware size gate: v1 (0x81 prefix) may reach 4,096 bytes;
        // every other wire format is capped at the 1,232-byte legacy MTU.
        for tx in &bundle {
            let limit = if tx.first() == Some(&0x81) { 4096 } else { 1232 };
            if tx.len() > limit {
                prop_assert!(res.is_err(), "wire-format MTU overflow must return Err");
            }
        }
    }

    #[test]
    fn fuzz_calculate_kamino_flash_fee_invariants(
        borrow in any::<u64>(),
        fee_sf in any::<u64>(),
    ) {
        let fee = calculate_kamino_flash_fee(borrow, fee_sf);
        // Fee computation must never overflow or panic
        if borrow == 0 || fee_sf == 0 {
            prop_assert_eq!(fee, 0);
        }
    }

    #[test]
    fn fuzz_dynamic_cu_limit_and_cap_priority_fee(
        base_cu in 1u32..=1_400_000,
        profit in any::<u64>(),
        desired_price in any::<u64>(),
        max_share_bps in 0u64..=10_000,
    ) {
        let cu_limit = calculate_dynamic_cu_limit(base_cu);
        prop_assert!(cu_limit <= arb_constants::MAX_COMPUTE_UNIT_LIMIT, "CU limit must be <= MAX_COMPUTE_UNIT_LIMIT");
        if (base_cu as u64 * 115 / 100) <= arb_constants::MAX_COMPUTE_UNIT_LIMIT as u64 {
            prop_assert!(cu_limit >= base_cu, "CU limit must include buffer when below cap");
        }

        let capped_price = cap_cu_price_by_profit(profit, cu_limit, desired_price, max_share_bps);
        prop_assert!(capped_price <= desired_price, "Capped price cannot exceed desired price");

        // Verify total priority fee in lamports does not exceed max share of profit
        let total_priority_fee = (capped_price as u128 * cu_limit as u128) / 1_000_000;
        let max_allowed_fee = (profit as u128 * max_share_bps as u128) / 10_000;
        prop_assert!(total_priority_fee <= max_allowed_fee + 1, "Priority fee must respect max_share_bps");
    }
}
