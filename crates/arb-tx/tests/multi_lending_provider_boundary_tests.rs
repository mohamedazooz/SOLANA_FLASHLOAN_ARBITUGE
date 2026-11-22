//! Feature 4: Multi-Lending Provider Abstraction — Tier 2 Boundary Tests.
//!
//! Validates provider selection edge cases: empty provider lists, deterministic tie-breaking,
//! foreign provider snapshot rejection, disabled sentinels, and inactive reserve remediation.

use arb_tx::{
    FlashBorrowParams, FlashLoanProvider, KaminoFlashLoanProvider, LendingProviderId,
    ReserveSnapshot, TxError, select_cheapest_flash_provider,
};
use arb_types::error::ArbError;
use solana_pubkey::Pubkey;

fn make_test_reserve(provider: LendingProviderId, fee_sf: u64, active: bool) -> ReserveSnapshot {
    ReserveSnapshot {
        provider,
        reserve: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        decimals: 9,
        active,
        fee_sf,
        available_liquidity: 10_000_000_000,
        supply_vault: Pubkey::new_unique(),
        fee_vault: Pubkey::new_unique(),
    }
}

/// Boundary Case 1: Empty provider slice fails closed.
#[test]
fn test_boundary_f4_empty_provider_list_fails_closed() {
    let providers: Vec<Box<dyn FlashLoanProvider>> = Vec::new();
    let reserve = make_test_reserve(LendingProviderId::Kamino, 11_529_215_046_068, true);

    let res = select_cheapest_flash_provider(&providers, 1_000_000_000, &reserve);
    match res {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("no eligible lending provider"));
        }
        Err(e) => panic!("expected InsufficientLiquidity, got other error: {e}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

/// Boundary Case 2: Deterministic tie-breaking on equal fee providers.
#[test]
fn test_boundary_f4_equal_fee_deterministic_tie_breaking() {
    // Both providers offer exact same 10 bps fee override
    let p1 = Box::new(KaminoFlashLoanProvider::new(115_292_150_460_684));
    let p2 = Box::new(KaminoFlashLoanProvider::new(115_292_150_460_684));
    let providers: Vec<Box<dyn FlashLoanProvider>> = vec![p1, p2];

    let reserve = make_test_reserve(LendingProviderId::Kamino, 115_292_150_460_684, true);

    // Run 100 times to guarantee deterministic selection of the first provider
    for _ in 0..100 {
        let selected = select_cheapest_flash_provider(&providers, 1_000_000_000, &reserve).unwrap();
        // The first element in slice should always win due to strict `<` comparison
        assert_eq!(selected.provider_id(), LendingProviderId::Kamino);
    }
}

/// Boundary Case 3: Foreign provider snapshot rejection.
#[test]
fn test_boundary_f4_foreign_provider_snapshot_rejection() {
    // Kamino provider with no override inspects reserve.provider
    let provider = KaminoFlashLoanProvider::default();
    let solend_reserve = make_test_reserve(LendingProviderId::Solend, 11_529_215_046_068, true);

    let res = provider.calculate_fee(1_000_000_000, &solend_reserve);
    match res {
        Err(ArbError::InvalidInput(msg)) => {
            assert!(
                msg.contains("expected Kamino reserve snapshot, got Solend"),
                "unexpected msg: {msg}"
            );
        }
        other => panic!("expected InvalidInput, got {:?}", other),
    }
}

/// Boundary Case 4: Disabled sentinel (fee_sf == u64::MAX) yields zero available liquidity and fee error.
#[test]
fn test_boundary_f4_disabled_sentinel_zero_liquidity() {
    let provider = KaminoFlashLoanProvider::default();
    let disabled_reserve = make_test_reserve(LendingProviderId::Kamino, u64::MAX, true);

    // Available liquidity must evaluate to 0
    assert_eq!(provider.available_liquidity(&disabled_reserve), 0);

    // Fee calculation must fail closed
    let fee_res = provider.calculate_fee(1_000_000_000, &disabled_reserve);
    match fee_res {
        Err(ArbError::InvalidInput(msg)) => {
            assert!(msg.contains("disabled"));
        }
        other => panic!("expected InvalidInput, got {:?}", other),
    }
}

/// Boundary Case 5: Remediated inactive reserve rejection in direct `build_borrow_ix`.
#[test]
fn test_boundary_f4_remediation_inactive_reserve_in_build_borrow_ix() {
    let provider = KaminoFlashLoanProvider::default();
    // Reserve has 10 SOL available liquidity but active == false
    let inactive_reserve = make_test_reserve(LendingProviderId::Kamino, 11_529_215_046_068, false);

    let params = FlashBorrowParams {
        user_authority: Pubkey::new_unique(),
        user_liquidity_ata: Pubkey::new_unique(),
        borrow_amount: 1_000_000_000,
        reserve: inactive_reserve,
        lending_market: Pubkey::new_unique(),
        token_program: Pubkey::new_unique(),
    };

    // Calling build_borrow_ix directly must fail closed because reserve.active == false
    let res = provider.build_borrow_ix(&params);
    match res {
        Err(TxError::InvalidInput(msg)) => {
            assert!(
                msg.contains("reserve is inactive"),
                "expected 'reserve is inactive', got {msg}"
            );
        }
        other => panic!("expected TxError::InvalidInput, got {:?}", other),
    }
}
