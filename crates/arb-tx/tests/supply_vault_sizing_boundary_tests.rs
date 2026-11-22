//! Feature 2: Dynamic Supply-Vault Liquidity Sizing — Tier 2 Boundary Tests.
//!
//! Enforces borrow sizing bounded strictly by live decoded supply-vault token balance,
//! preventing inactive reserve bypass, zero liquidity borrow, and buffer overruns.

use arb_tx::{
    FlashBorrowParams, KaminoFlashLoanProvider, KaminoReadError, LendingError, LendingProviderId,
    ReserveSnapshot, TxError, decode_supply_vault_liquidity,
};
use solana_pubkey::Pubkey;

fn make_test_reserve(available_liquidity: u64, active: bool) -> ReserveSnapshot {
    ReserveSnapshot {
        provider: LendingProviderId::Kamino,
        reserve: Pubkey::new_unique(),
        mint: Pubkey::new_unique(),
        decimals: 9,
        active,
        fee_sf: 11_529_215_046_068,
        available_liquidity,
        supply_vault: Pubkey::new_unique(),
        fee_vault: Pubkey::new_unique(),
    }
}

fn make_borrow_params(reserve: &ReserveSnapshot, borrow_amount: u64) -> FlashBorrowParams {
    FlashBorrowParams {
        user_authority: Pubkey::new_unique(),
        user_liquidity_ata: Pubkey::new_unique(),
        borrow_amount,
        reserve: reserve.clone(),
        lending_market: Pubkey::new_unique(),
        token_program: Pubkey::new_unique(),
    }
}

/// Boundary Case 1: Zero available liquidity in active reserve.
#[test]
fn test_boundary_f2_zero_available_liquidity() {
    let provider = KaminoFlashLoanProvider::default();
    let reserve = make_test_reserve(0, true);
    let params = make_borrow_params(&reserve, 1);

    let res = provider.validate_and_build_borrow(&params, &reserve);
    assert_eq!(
        res.err(),
        Some(LendingError::InsufficientLiquidity {
            requested: 1,
            available: 0,
        })
    );
}

/// Boundary Case 2: Borrow amount exactly equals available liquidity (valid boundary).
#[test]
fn test_boundary_f2_borrow_equals_available_boundary() {
    let provider = KaminoFlashLoanProvider::default();
    let reserve = make_test_reserve(10_000_000_000, true);
    let params = make_borrow_params(&reserve, 10_000_000_000);

    let res = provider.validate_and_build_borrow(&params, &reserve);
    assert!(
        res.is_ok(),
        "borrow amount == available liquidity must succeed"
    );
}

/// Boundary Case 3: Borrow amount exceeds available liquidity by exactly 1 lamport.
#[test]
fn test_boundary_f2_borrow_exceeds_available_by_one_lamport() {
    let provider = KaminoFlashLoanProvider::default();
    let reserve = make_test_reserve(10_000_000_000, true);
    let params = make_borrow_params(&reserve, 10_000_000_001);

    let res = provider.validate_and_build_borrow(&params, &reserve);
    assert_eq!(
        res.err(),
        Some(LendingError::InsufficientLiquidity {
            requested: 10_000_000_001,
            available: 10_000_000_000,
        })
    );
}

/// Boundary Case 4: Inactive reserve fails closed in both validate_and_build_borrow and direct build_borrow_ix.
#[test]
fn test_boundary_f2_inactive_reserve_fails_closed() {
    use arb_tx::FlashLoanProvider;

    let provider = KaminoFlashLoanProvider::default();
    let reserve = make_test_reserve(50_000_000_000, false);
    assert_eq!(provider.available_liquidity(&reserve), 0);

    let params = make_borrow_params(&reserve, 1_000_000_000);

    // validate_and_build_borrow checks available_liquidity and returns InsufficientLiquidity
    let res = provider.validate_and_build_borrow(&params, &reserve);
    assert_eq!(
        res.err(),
        Some(LendingError::InsufficientLiquidity {
            requested: 1_000_000_000,
            available: 0,
        })
    );

    // Direct build_borrow_ix checks reserve.active and fails closed with InvalidInput("reserve is inactive")
    let raw_res = provider.build_borrow_ix(&params);
    match raw_res {
        Err(TxError::InvalidInput(msg)) => {
            assert!(
                msg.contains("reserve is inactive"),
                "expected 'reserve is inactive', got {msg}"
            );
        }
        other => panic!("expected TxError::InvalidInput, got {:?}", other),
    }
}

/// Boundary Case 5: Supply-vault account decoding length and mint boundaries.
#[test]
fn test_boundary_f2_supply_vault_decoding_length_and_mint_boundaries() {
    let mint = Pubkey::new_unique();

    // Sub-case A: 71 bytes (1 byte short of 72-byte minimum)
    let short_account = vec![0u8; 71];
    assert_eq!(
        decode_supply_vault_liquidity(&short_account, &mint),
        Err(KaminoReadError::TokenAccountTooShort(71))
    );

    // Sub-case B: 72 bytes with balance = 0
    let mut zero_bal_account = vec![0u8; 72];
    zero_bal_account[0..32].copy_from_slice(mint.as_ref());
    zero_bal_account[64..72].copy_from_slice(&0u64.to_le_bytes());
    assert_eq!(
        decode_supply_vault_liquidity(&zero_bal_account, &mint),
        Ok(0)
    );

    // Sub-case C: 72 bytes with balance = u64::MAX
    let mut max_bal_account = vec![0u8; 72];
    max_bal_account[0..32].copy_from_slice(mint.as_ref());
    max_bal_account[64..72].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        decode_supply_vault_liquidity(&max_bal_account, &mint),
        Ok(u64::MAX)
    );

    // Sub-case D: 165 bytes (standard SPL token account) with mismatched mint
    let foreign_mint = Pubkey::new_unique();
    let mut standard_account = vec![0u8; 165];
    standard_account[0..32].copy_from_slice(foreign_mint.as_ref());
    standard_account[64..72].copy_from_slice(&5_000_000u64.to_le_bytes());
    match decode_supply_vault_liquidity(&standard_account, &mint) {
        Err(KaminoReadError::MintMismatch { expected, found }) => {
            assert_eq!(expected, mint.to_string());
            assert_eq!(found, foreign_mint.to_string());
        }
        other => panic!("expected MintMismatch, got {:?}", other),
    }
}
