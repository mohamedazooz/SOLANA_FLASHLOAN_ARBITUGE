//! Canonical Jito tip-account validation and basis-point tip sizing.
//!
//! Tip accounts and the WSOL mint are duplicated here as on-chain `Address`
//! constants because the BPF program cannot depend on `arb-constants`.
//! The `tests` module cross-checks every constant against `arb-constants`
//! so drift between the two copies fails the workspace test suite.

use crate::error::ExecutorError;
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_program_error::ProgramResult;

/// System Program ID (owner of every Jito tip account).
pub const SYSTEM_PROGRAM_ID: Address = Address::from_str_const("11111111111111111111111111111111");

/// Canonical WSOL mint. WSOL units are pegged 1:1 to lamports, which is the
/// dimensional premise that allows a WSOL-measured profit edge to back a
/// lamport-denominated tip.
pub const WSOL_MINT: Address =
    Address::from_str_const("So11111111111111111111111111111111111111112");

/// Basis-point denominator for tip share arithmetic.
pub const BPS_DENOMINATOR: u64 = 10_000;

/// The 8 official Jito block-engine tip accounts on Solana Mainnet.
pub const JITO_TIP_ACCOUNTS: [Address; 8] = [
    Address::from_str_const("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49"),
    Address::from_str_const("DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh"),
    Address::from_str_const("DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL"),
    Address::from_str_const("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt"),
    Address::from_str_const("Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY"),
    Address::from_str_const("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"),
    Address::from_str_const("3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT"),
    Address::from_str_const("HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe"),
];

/// Validate that the provided account is a canonical Jito tip account owned
/// by the System Program.
pub fn validate_jito_tip_account(tip_account: &AccountInfo) -> ProgramResult {
    if *tip_account.owner != SYSTEM_PROGRAM_ID || !JITO_TIP_ACCOUNTS.contains(tip_account.key) {
        return Err(ExecutorError::InvalidJitoTipAccount.into());
    }
    Ok(())
}

/// Compute the tip in lamports for a profit edge measured in WSOL units.
///
/// `tip = floor(gross_edge * tip_bps / BPS_DENOMINATOR)`, computed in `u128`
/// to exclude any intermediate overflow. `tip_bps` must not exceed
/// `BPS_DENOMINATOR`, and `tip` never exceeds `gross_edge`.
pub fn tip_amount_for_bps(gross_edge: u64, tip_bps: u64) -> Result<u64, ExecutorError> {
    if tip_bps > BPS_DENOMINATOR {
        return Err(ExecutorError::TipDenominatorInvalid);
    }
    if tip_bps == 0 {
        return Ok(0);
    }
    let product = (gross_edge as u128)
        .checked_mul(tip_bps as u128)
        .ok_or(ExecutorError::CalculationOverflow)?;
    let tip = product / BPS_DENOMINATOR as u128;
    u64::try_from(tip).map_err(|_| ExecutorError::CalculationOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_on_chain_tip_accounts_match_off_chain_constants() {
        for (i, on_chain) in JITO_TIP_ACCOUNTS.iter().enumerate() {
            let off_chain = arb_constants::JITO_TIP_ACCOUNTS[i];
            let parsed = Address::from_str_const(off_chain);
            assert_eq!(on_chain, &parsed, "Jito tip account drift at index {i}");
        }
    }

    #[test]
    fn test_on_chain_wsol_mint_matches_off_chain_constant() {
        assert_eq!(WSOL_MINT, Address::from_str_const(arb_constants::WSOL_MINT));
    }

    #[test]
    fn test_tip_amount_for_bps_boundaries() {
        assert_eq!(tip_amount_for_bps(1_000_000, 0).unwrap(), 0);
        assert_eq!(
            tip_amount_for_bps(1_000_000, BPS_DENOMINATOR).unwrap(),
            1_000_000
        );
        assert_eq!(
            tip_amount_for_bps(1_000_000_000, 7_000).unwrap(),
            700_000_000
        );
        assert_eq!(tip_amount_for_bps(1, 5_000).unwrap(), 0);
        assert_eq!(tip_amount_for_bps(3, 3_333).unwrap(), 0);
        assert_eq!(tip_amount_for_bps(u64::MAX, 5_000).unwrap(), u64::MAX / 2);
    }

    #[test]
    fn test_tip_amount_for_bps_rejects_invalid_denominator() {
        assert_eq!(
            tip_amount_for_bps(1_000, BPS_DENOMINATOR + 1),
            Err(ExecutorError::TipDenominatorInvalid)
        );
        assert_eq!(
            tip_amount_for_bps(1_000, u64::MAX),
            Err(ExecutorError::TipDenominatorInvalid)
        );
        assert_eq!(
            tip_amount_for_bps(u64::MAX, 10_000),
            Ok(u64::MAX),
            "minimal-value test u64::MAX edge never trips the u128 product"
        );
    }
}
