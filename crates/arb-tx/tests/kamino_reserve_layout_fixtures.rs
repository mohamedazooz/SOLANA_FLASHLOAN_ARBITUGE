//! Golden Mainnet fixture for the current Kamino klend Reserve layout.
//!
//! Captured with one getMultipleAccounts request at confirmed commitment.
//! Provenance and account owners are recorded beside the binary fixtures.
//! The official Reserve schema is in:
//! https://github.com/Kamino-Finance/klend/blob/master/libs/klend-interface/src/state/reserve.rs
//!
//! This is a golden on-chain layout regression test, not a claim that the
//! runtime depends on the klend-interface crate or that a live borrow landed.

use std::str::FromStr;

use arb_tx::{
    KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE, KAMINO_WSOL_SUPPLY_VAULT, WSOL_MINT,
    decode_reserve_candidate, decode_reserve_fee_vault, decode_reserve_flash_fee_sf,
    decode_reserve_supply_vault,
};
use solana_pubkey::Pubkey;

const RESERVE_DATA: &[u8] = include_bytes!("fixtures/kamino_wsol_reserve_2026-11-27.bin");
const SUPPLY_VAULT_DATA: &[u8] = include_bytes!("fixtures/kamino_wsol_supply_vault_2026-11-27.bin");

#[test]
fn current_mainnet_wsol_reserve_matches_klend_reserve_layout() {
    let expected_mint = Pubkey::from_str(WSOL_MINT).unwrap();

    // The official klend-interface Reserve is 8,616 bytes; the on-chain
    // account adds the 8-byte Anchor discriminator.
    assert_eq!(RESERVE_DATA.len(), 8_624);
    assert_eq!(
        RESERVE_DATA.get(..8),
        Some(arb_tx::flash::kamino::KAMINO_RESERVE_DISCRIMINATOR.as_slice())
    );

    assert_eq!(
        decode_reserve_flash_fee_sf(RESERVE_DATA, &expected_mint).unwrap(),
        11_529_215_046_068
    );
    assert_eq!(
        decode_reserve_supply_vault(RESERVE_DATA).unwrap(),
        Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap()
    );
    assert_eq!(
        decode_reserve_fee_vault(RESERVE_DATA).unwrap(),
        Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap()
    );

    let candidate = decode_reserve_candidate(
        Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
        RESERVE_DATA,
        SUPPLY_VAULT_DATA,
    )
    .unwrap();

    assert_eq!(candidate.mint, expected_mint);
    assert_eq!(candidate.decimals, 9);
    assert!(candidate.active);
    assert_eq!(candidate.flash_loan_fee_sf, 11_529_215_046_068);
    assert_eq!(
        candidate.supply_vault,
        Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap()
    );
    assert_eq!(
        candidate.fee_vault,
        Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap()
    );
    assert_eq!(candidate.available_liquidity, 321_999_152_973_336);
}

#[test]
fn current_mainnet_wsol_reserve_fixture_rejects_truncation() {
    let expected_mint = Pubkey::from_str(WSOL_MINT).unwrap();

    assert!(decode_reserve_flash_fee_sf(&RESERVE_DATA[..4_903], &expected_mint).is_err());
    assert!(decode_reserve_supply_vault(&RESERVE_DATA[..191]).is_err());
    assert!(decode_reserve_fee_vault(&RESERVE_DATA[..223]).is_err());
}
