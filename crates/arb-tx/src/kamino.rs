//! Kamino Lend (`klend`) Flash Loan Program Integration.
//! Program ID: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
//!
//! Provides genuine on-chain instruction builders for `flash_borrow_reserve_liquidity`
//! and `flash_repay_reserve_liquidity`, Anchor discriminators, 12-account matrix,
//! PDA derivations, and zero-fee / dynamic fee computation.

use crate::{AccountMeta, CompiledInstruction};
use solana_pubkey::Pubkey;
use std::str::FromStr;

// All Kamino program/market addresses live in `arb-constants`; re-exported here
// so `arb_tx::kamino::X` paths stay stable.
pub use arb_constants::{
    FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID,
    KAMINO_MAIN_MARKET, KAMINO_MAIN_MARKET_AUTHORITY, KAMINO_USDC_RESERVE, KAMINO_WSOL_FEE_VAULT,
    KAMINO_WSOL_RESERVE, KAMINO_WSOL_SUPPLY_VAULT, LENDING_MARKET_AUTH_SEED,
    SYSVAR_INSTRUCTIONS_ID, TOKEN_PROGRAM_ID, USDC_MINT, WSOL_MINT,
};

/// Derive Lending Market Authority PDA: `seeds = [b"lma", lending_market.as_ref()]`.
#[inline]
pub fn derive_lending_market_authority(
    lending_market: &Pubkey,
    program_id: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[LENDING_MARKET_AUTH_SEED, lending_market.as_ref()],
        program_id,
    )
}

/// Kamino Flash Loan Account Set representing the canonical 12 on-chain accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaminoFlashAccounts {
    pub user_authority: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub reserve: Pubkey,
    pub reserve_liquidity_mint: Pubkey,
    pub reserve_supply_vault: Pubkey,
    pub user_liquidity_ata: Pubkey,
    pub reserve_fee_vault: Pubkey,
    pub referrer_token_state: Option<Pubkey>,
    pub referrer_account: Option<Pubkey>,
    pub sysvar_instructions: Pubkey,
    pub token_program: Pubkey,
    pub klend_program_id: Pubkey,
}

impl KaminoFlashAccounts {
    /// Create standard WSOL flash loan account matrix for Main Market.
    pub fn new_wsol_main_market(user_authority: Pubkey, user_wsol_ata: Pubkey) -> Self {
        let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).expect("valid klend pubkey");
        let market = Pubkey::from_str(KAMINO_MAIN_MARKET).expect("valid market pubkey");
        let (authority, _) = derive_lending_market_authority(&market, &klend);
        let wsol_reserve = Pubkey::from_str(KAMINO_WSOL_RESERVE).expect("valid wsol reserve");
        let wsol_supply = Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).expect("valid wsol supply");
        let wsol_fee = Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).expect("valid wsol fee");
        let wsol_mint = Pubkey::from_str(WSOL_MINT).expect("valid wsol mint");
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).expect("valid token program");
        let sysvar = Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).expect("valid sysvar");

        Self {
            user_authority,
            lending_market: market,
            lending_market_authority: authority,
            reserve: wsol_reserve,
            reserve_liquidity_mint: wsol_mint,
            reserve_supply_vault: wsol_supply,
            user_liquidity_ata: user_wsol_ata,
            reserve_fee_vault: wsol_fee,
            referrer_token_state: None,
            referrer_account: None,
            sysvar_instructions: sysvar,
            token_program,
            klend_program_id: klend,
        }
    }
}

/// Build Kamino `flash_borrow_reserve_liquidity` instruction.
/// Output data length: 16 bytes (8-byte discriminator + 8-byte LE u64 amount).
/// AccountMetas count: Exactly 12 accounts matching Anchor IDL.
pub fn build_kamino_flash_borrow(
    accounts: &KaminoFlashAccounts,
    amount: u64,
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&FLASH_BORROW_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());

    // When no referrer is provided, Anchor optional accounts default to program_id (readonly)
    let ref_token_state = accounts
        .referrer_token_state
        .unwrap_or(accounts.klend_program_id);
    let ref_acc = accounts
        .referrer_account
        .unwrap_or(accounts.klend_program_id);

    CompiledInstruction {
        program_id: accounts.klend_program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.user_authority, true), // 0. User authority (signer)
            AccountMeta::new_readonly(accounts.lending_market_authority, false), // 1. Market authority
            AccountMeta::new_readonly(accounts.lending_market, false), // 2. Lending market
            AccountMeta::new(accounts.reserve, false),                 // 3. Reserve (mut)
            AccountMeta::new_readonly(accounts.reserve_liquidity_mint, false), // 4. Reserve mint
            AccountMeta::new(accounts.reserve_supply_vault, false),    // 5. Source vault (mut)
            AccountMeta::new(accounts.user_liquidity_ata, false), // 6. User destination ATA (mut)
            AccountMeta::new(accounts.reserve_fee_vault, false),  // 7. Fee receiver vault (mut)
            AccountMeta::new_readonly(ref_token_state, false), // 8. Referrer token state (or program_id)
            AccountMeta::new_readonly(ref_acc, false), // 9. Referrer account (or program_id)
            AccountMeta::new_readonly(accounts.sysvar_instructions, false), // 10. Sysvar instructions
            AccountMeta::new_readonly(accounts.token_program, false),       // 11. Token program
        ],
        data,
    }
}

/// Build Kamino `flash_repay_reserve_liquidity` instruction.
/// Output data length: 17 bytes (8-byte discriminator + 8-byte LE u64 amount + 1-byte borrow_instruction_index).
/// AccountMetas count: Exactly 12 accounts matching Anchor IDL.
pub fn build_kamino_flash_repay(
    accounts: &KaminoFlashAccounts,
    amount: u64,
    borrow_instruction_index: u8,
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(17);
    data.extend_from_slice(&FLASH_REPAY_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(borrow_instruction_index);

    let ref_token_state = accounts
        .referrer_token_state
        .unwrap_or(accounts.klend_program_id);
    let ref_acc = accounts
        .referrer_account
        .unwrap_or(accounts.klend_program_id);

    CompiledInstruction {
        program_id: accounts.klend_program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.user_authority, true), // 0. User authority (signer)
            AccountMeta::new_readonly(accounts.lending_market_authority, false), // 1. Market authority
            AccountMeta::new_readonly(accounts.lending_market, false), // 2. Lending market
            AccountMeta::new(accounts.reserve, false),                 // 3. Reserve (mut)
            AccountMeta::new_readonly(accounts.reserve_liquidity_mint, false), // 4. Reserve mint
            AccountMeta::new(accounts.reserve_supply_vault, false),    // 5. Destination vault (mut)
            AccountMeta::new(accounts.user_liquidity_ata, false),      // 6. User source ATA (mut)
            AccountMeta::new(accounts.reserve_fee_vault, false), // 7. Fee receiver vault (mut)
            AccountMeta::new_readonly(ref_token_state, false), // 8. Referrer token state (or program_id)
            AccountMeta::new_readonly(ref_acc, false), // 9. Referrer account (or program_id)
            AccountMeta::new_readonly(accounts.sysvar_instructions, false), // 10. Sysvar instructions
            AccountMeta::new_readonly(accounts.token_program, false),       // 11. Token program
        ],
        data,
    }
}

/// Number of fractional bits in klend's `Fraction` type.
///
/// klend's `utils::fraction` defines `pub use fixed::types::U68F60 as Fraction`
/// and every `*_sf` field is a raw `Fraction` bit pattern, so `flash_loan_fee_sf`
/// is a **U68F60** value with 60 fractional bits — *not* Q64.64.
pub const KAMINO_FRACTION_FRAC_BITS: u32 = 60;

/// `Fraction::ONE.to_bits()` for klend's U68F60 `Fraction` (`1 << 60`).
pub const KAMINO_FRACTION_ONE_BITS: u128 = 1u128 << KAMINO_FRACTION_FRAC_BITS;

/// Failure modes when reproducing klend's on-chain flash-loan fee exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KaminoFeeError {
    /// `flash_loan_fee_sf == u64::MAX` — klend's sentinel for a reserve whose
    /// flash loans are disabled (`ReserveFees` serde maps `"disabled"` to it).
    #[error("flash loans are disabled on this reserve (flash_loan_fee_sf == u64::MAX)")]
    FlashLoanDisabled,
    /// klend reverts with `LendingError::BorrowTooSmall` when the origination
    /// fee (after the 1-unit floor) is `>=` the principal.
    #[error("borrow amount too small: origination fee >= principal (klend BorrowTooSmall)")]
    BorrowTooSmall,
    /// The wide-integer `borrow_amount * fee_sf` product overflowed `u128`.
    #[error("flash-loan fee computation overflowed u128")]
    Overflow,
}

/// Raw, un-rounded `Fraction` bits of `max(amount * sf / 2^60, 1.0)`.
///
/// klend computes `amount.mul(rate)` on `Fraction` (U68F60). For a `u64` amount
/// this is `(amount << 60) * sf >> 60 == amount * sf` **exactly** (the shift
/// cancels), then `.max(minimum_fee)` with `minimum_fee == 1` (`1 << 60` bits).
/// Returns `None` when `fee_sf` is klend's disabled sentinel (`u64::MAX`) or
/// when the product overflows `u128`. Failing closed here keeps the infallible
/// helper from inventing a plausible-looking fee for a disabled reserve.
#[inline]
fn kamino_fee_bits(borrow_amount: u64, fee_sf: u64) -> Option<u128> {
    if fee_sf == u64::MAX {
        // klend's `ReserveFees` serde maps `"disabled"` to `u64::MAX`; no fee
        // is defined and the borrow must not be attempted.
        return None;
    }
    if fee_sf == 0 || borrow_amount == 0 {
        return Some(0);
    }
    let raw = (borrow_amount as u128).checked_mul(fee_sf as u128)?;
    Some(raw.max(KAMINO_FRACTION_ONE_BITS))
}

/// Round `Fraction` bits to the nearest integer, ties away from zero.
///
/// This matches klend's `FractionExtra::to_round` (= `self.round().to_num()`),
/// where `round` returns the nearest integer and, on an exact half, the value
/// with the larger magnitude. For `bits = q·2^60 + r` that is `q + (r >= 2^59)`.
#[inline]
fn round_fraction_bits_to_u64(bits: u128) -> u64 {
    let q = bits >> KAMINO_FRACTION_FRAC_BITS;
    let r = bits & (KAMINO_FRACTION_ONE_BITS - 1);
    let rounded = q + u128::from(r >= (KAMINO_FRACTION_ONE_BITS >> 1));
    rounded.min(u64::MAX as u128) as u64
}

/// Exact klend flash-loan origination fee in atomic token units, with all
/// on-chain failure modes surfaced.
///
/// Reproduces `ReserveFees::calculate_fees(amount, flash_loan_fee_sf,
/// FeeCalculation::Exclusive, .., has_referrer = false)`:
///
/// ```text
/// rate_bits = flash_loan_fee_sf
/// fee_bits  = (amount << 60) * rate_bits >> 60   == amount * rate_bits   (exact)
/// fee_bits  = max(fee_bits, 1 << 60)             // minimum_fee = 1
/// if fee_bits >= amount << 60 { BorrowTooSmall }
/// fee       = round(fee_bits / 2^60)             // ties away from zero
/// ```
///
/// The fee is collected **at repay**, on top of the principal: the handler
/// transfers `liquidity_amount` to the reserve vault and `protocol_fee` to the
/// fee vault, so the caller's total outlay is `principal + fee`. It is therefore
/// wrong to pass `principal + fee` as the repay instruction's `amount` argument.
pub fn calculate_kamino_flash_fee_checked(
    borrow_amount: u64,
    fee_sf: u64,
) -> Result<u64, KaminoFeeError> {
    if fee_sf == u64::MAX {
        return Err(KaminoFeeError::FlashLoanDisabled);
    }
    if fee_sf == 0 || borrow_amount == 0 {
        return Ok(0);
    }
    let amount_bits = (borrow_amount as u128) << KAMINO_FRACTION_FRAC_BITS;
    let fee_bits = kamino_fee_bits(borrow_amount, fee_sf).ok_or(KaminoFeeError::Overflow)?;
    if fee_bits >= amount_bits {
        return Err(KaminoFeeError::BorrowTooSmall);
    }
    Ok(round_fraction_bits_to_u64(fee_bits))
}

/// Calculate the Kamino flash-loan fee in atomic token units from the raw
/// U68F60 on-chain value (`ReserveConfig.fees.flash_loan_fee_sf`).
///
/// Fee = `round(borrow * sf / 2^60)`, with a floor of 1 atomic unit and
/// round-half-away-from-zero — the exact klend `FeeCalculation::Exclusive`
/// formula. A bps integer cannot represent the on-chain value (main-market
/// WSOL carries `sf = 11_529_215_046_068` = 1e-5 = 0.1 bps → 10,000 lamports
/// per 1 SOL), so the fee is never expressed as a bps rate.
///
/// This infallible helper **fails closed**: it returns `u64::MAX` for a disabled
/// reserve (`sf == u64::MAX`) and saturates on overflow, so a caller that
/// compares the fee against available funds will reject rather than understate
/// the cost. It returns `0` for a zero rate or zero amount, and does **not**
/// apply the `BorrowTooSmall` guard. Prefer
/// [`calculate_kamino_flash_fee_checked`] on any borrow path.
#[inline(always)]
pub fn calculate_kamino_flash_fee(borrow_amount: u64, fee_sf: u64) -> u64 {
    match kamino_fee_bits(borrow_amount, fee_sf) {
        Some(bits) => round_fraction_bits_to_u64(bits),
        None => u64::MAX,
    }
}

// ============================================================================
// On-chain reserve fee reader (klend `ReserveConfig.fees.flash_loan_fee_sf`)
// ============================================================================

/// Anchor discriminator of the klend `Reserve` account:
/// `sha256("global:Reserve")[..8]`, read live from the mainnet WSOL reserve
/// (`d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q`) on 2026-11-04.
pub const KAMINO_RESERVE_DISCRIMINATOR: [u8; 8] = [0x2b, 0xf2, 0xcc, 0xca, 0x1a, 0xf7, 0x3b, 0x7f];

/// Byte offsets into the **account** (after the 8-byte discriminator) of the
/// deployed klend program (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`).
/// Verified empirically against live mainnet accounts for both the WSOL and the
/// USDC reserve on 2026-11-04: `liquidity.mint_pubkey` @120, `mint_decimals`
/// @264, `config.status` @4848, `config.fees.flash_loan_fee_sf` @4896.
/// (The deployed layout predates the master-branch `LastUpdate` 24-byte form;
/// re-verify these offsets if the program is upgraded.)
pub const KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET: usize = 8 + 120;
pub const KAMINO_RESERVE_MINT_DECIMALS_OFFSET: usize = 8 + 264;
pub const KAMINO_RESERVE_STATUS_OFFSET: usize = 8 + 4848;
pub const KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET: usize = 8 + 4896;
/// `config.status == 0` means the reserve is Active.
pub const KAMINO_RESERVE_STATUS_ACTIVE: u8 = 0;

// ============================================================================
// SPL Token account decoding (reserve supply vault liquidity)
// ============================================================================

/// Unpacked SPL Token / Token-2022 account layout: `mint` (32 bytes) at 0.
pub const SPL_TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
/// Unpacked SPL Token / Token-2022 account layout: `owner` (32 bytes) at 32.
pub const SPL_TOKEN_ACCOUNT_OWNER_OFFSET: usize = 32;
/// Unpacked SPL Token / Token-2022 account layout: `amount` (u64 LE) at 64.
pub const SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;

/// Errors reading on-chain Kamino flash-loan inputs. Fail closed: any
/// malformed or unverifiable account is an error, never a default value.
#[derive(Debug, thiserror::Error)]
pub enum KaminoReadError {
    #[error("reserve account too short: {0} bytes")]
    AccountTooShort(usize),
    #[error("reserve account discriminator mismatch")]
    BadDiscriminator,
    #[error("reserve liquidity mint mismatch: expected {expected}, found {found}")]
    MintMismatch { expected: String, found: String },
    #[error("reserve is not active (status {0})")]
    ReserveNotActive(u8),
    #[error("token account too short: {0} bytes (need at least 72)")]
    TokenAccountTooShort(usize),
}

/// Pure decoder: extract `flash_loan_fee_sf` (U68F60 u64, 60 fractional bits)
/// from a klend `Reserve` account payload. Verifies discriminator, length, the
/// reserve's liquidity mint, and active status before trusting the fee — fail
/// closed.
pub fn decode_reserve_flash_fee_sf(
    account: &[u8],
    expected_mint: &Pubkey,
) -> Result<u64, KaminoReadError> {
    let fee_end = KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET + 8;
    if account.len() < fee_end {
        return Err(KaminoReadError::AccountTooShort(account.len()));
    }
    if account[0..8] != KAMINO_RESERVE_DISCRIMINATOR {
        return Err(KaminoReadError::BadDiscriminator);
    }
    let found_mint =
        &account[KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET..KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET + 32];
    if found_mint != expected_mint.as_ref() {
        return Err(KaminoReadError::MintMismatch {
            expected: expected_mint.to_string(),
            found: Pubkey::new_from_array(<[u8; 32]>::try_from(found_mint).unwrap_or([0u8; 32]))
                .to_string(),
        });
    }
    let status = account[KAMINO_RESERVE_STATUS_OFFSET];
    if status != KAMINO_RESERVE_STATUS_ACTIVE {
        return Err(KaminoReadError::ReserveNotActive(status));
    }
    let sf = u64::from_le_bytes(
        <[u8; 8]>::try_from(&account[KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET..fee_end])
            .map_err(|_| KaminoReadError::AccountTooShort(account.len()))?,
    );
    Ok(sf)
}

/// Pure decoder: extract the token balance of the klend reserve **supply
/// vault** (an SPL Token or Token-2022 account). This is the actual on-chain
/// liquidity a flash borrow can draw from; a configured risk ceiling is never
/// a substitute. Verifies the account is long enough and holds the expected
/// mint before trusting the amount — fail closed.
pub fn decode_supply_vault_liquidity(
    account: &[u8],
    expected_mint: &Pubkey,
) -> Result<u64, KaminoReadError> {
    let amount_end = SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET + 8;
    if account.len() < amount_end {
        return Err(KaminoReadError::TokenAccountTooShort(account.len()));
    }
    let found_mint = &account[SPL_TOKEN_ACCOUNT_MINT_OFFSET..SPL_TOKEN_ACCOUNT_MINT_OFFSET + 32];
    if found_mint != expected_mint.as_ref() {
        return Err(KaminoReadError::MintMismatch {
            expected: expected_mint.to_string(),
            found: Pubkey::new_from_array(<[u8; 32]>::try_from(found_mint).unwrap_or([0u8; 32]))
                .to_string(),
        });
    }
    Ok(u64::from_le_bytes(
        <[u8; 8]>::try_from(&account[SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET..amount_end])
            .map_err(|_| KaminoReadError::TokenAccountTooShort(account.len()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal synthetic klend `Reserve` payload matching the verified
    /// deployed layout: discriminator + expected mint + active status + fee.
    fn fixture_reserve(mint: Pubkey, fee_sf: u64) -> Vec<u8> {
        let mut v = vec![0u8; 8 + 4896 + 8];
        v[0..8].copy_from_slice(&KAMINO_RESERVE_DISCRIMINATOR);
        v[8 + 120..8 + 152].copy_from_slice(mint.as_ref());
        v[8 + 264..8 + 272].copy_from_slice(&9u64.to_le_bytes());
        v[8 + 4848] = KAMINO_RESERVE_STATUS_ACTIVE;
        v[8 + 4896..8 + 4904].copy_from_slice(&fee_sf.to_le_bytes());
        v
    }

    fn wsol() -> Pubkey {
        Pubkey::from_str(WSOL_MINT).unwrap()
    }

    #[test]
    fn test_market_authority_pda_derivation() {
        let market = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
        let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
        let (pda, bump) = derive_lending_market_authority(&market, &klend);
        let expected = Pubkey::from_str(KAMINO_MAIN_MARKET_AUTHORITY).unwrap();
        assert_eq!(
            pda, expected,
            "Lending Market Authority PDA must match verified on-chain address"
        );
        assert_eq!(bump, 248, "Bump seed must match verified on-chain bump");
    }

    #[test]
    fn test_flash_borrow_layout_and_discriminator() {
        let user = Pubkey::new_from_array([1; 32]);
        let ata = Pubkey::new_from_array([2; 32]);
        let accounts = KaminoFlashAccounts::new_wsol_main_market(user, ata);
        let amount = 10_000_000_000u64; // 10 SOL

        let ix = build_kamino_flash_borrow(&accounts, amount);

        // Verify data length and discriminator
        assert_eq!(ix.data.len(), 16);
        assert_eq!(&ix.data[0..8], &FLASH_BORROW_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount.to_le_bytes());

        // Verify 12 accounts
        assert_eq!(ix.accounts.len(), 12);
        assert!(ix.accounts[0].is_signer); // User authority is signer
        assert_eq!(ix.accounts[0].pubkey, user);
        assert_eq!(
            ix.accounts[1].pubkey,
            Pubkey::from_str(KAMINO_MAIN_MARKET_AUTHORITY).unwrap()
        );
        assert_eq!(
            ix.accounts[2].pubkey,
            Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap()
        );
        assert_eq!(
            ix.accounts[3].pubkey,
            Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap()
        );
        assert_eq!(
            ix.accounts[5].pubkey,
            Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap()
        );
        assert_eq!(ix.accounts[6].pubkey, ata);
        assert_eq!(
            ix.accounts[7].pubkey,
            Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap()
        );
        assert_eq!(
            ix.accounts[10].pubkey,
            Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
        );
        assert_eq!(
            ix.accounts[11].pubkey,
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
        );
    }

    #[test]
    fn test_flash_repay_layout_and_discriminator() {
        let user = Pubkey::new_from_array([1; 32]);
        let ata = Pubkey::new_from_array([2; 32]);
        let accounts = KaminoFlashAccounts::new_wsol_main_market(user, ata);
        let amount = 10_000_000_000u64;
        let borrow_ix_index = 2u8;

        let ix = build_kamino_flash_repay(&accounts, amount, borrow_ix_index);

        // Verify data length is strictly 17 bytes (discriminator + amount + borrow_ix_index)
        assert_eq!(ix.data.len(), 17);
        assert_eq!(&ix.data[0..8], &FLASH_REPAY_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount.to_le_bytes());
        assert_eq!(ix.data[16], 2);

        // Verify 12 accounts
        assert_eq!(ix.accounts.len(), 12);
        assert_eq!(
            ix.accounts[10].pubkey,
            Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
        );
    }

    #[test]
    fn decode_returns_known_fee_fixture() {
        // Captured fee-account fixture; this is not a live-rate assertion.
        let mainnet_sf: u64 = 11_529_215_046_068;
        let acc = fixture_reserve(wsol(), mainnet_sf);
        assert_eq!(
            decode_reserve_flash_fee_sf(&acc, &wsol()).unwrap(),
            mainnet_sf
        );
    }

    #[test]
    fn decode_rejects_wrong_mint() {
        let acc = fixture_reserve(wsol(), 0);
        let other = Pubkey::new_from_array([7u8; 32]);
        assert!(matches!(
            decode_reserve_flash_fee_sf(&acc, &other),
            Err(KaminoReadError::MintMismatch { .. })
        ));
    }

    #[test]
    fn decode_rejects_bad_discriminator() {
        let mut acc = fixture_reserve(wsol(), 0);
        acc[0] ^= 0xff;
        assert!(matches!(
            decode_reserve_flash_fee_sf(&acc, &wsol()),
            Err(KaminoReadError::BadDiscriminator)
        ));
    }

    #[test]
    fn decode_rejects_short_account() {
        assert!(matches!(
            decode_reserve_flash_fee_sf(&[0u8; 32], &wsol()),
            Err(KaminoReadError::AccountTooShort(_))
        ));
    }

    #[test]
    fn decode_rejects_inactive_reserve() {
        let mut acc = fixture_reserve(wsol(), 0);
        acc[8 + 4848] = 1; // not active
        assert!(matches!(
            decode_reserve_flash_fee_sf(&acc, &wsol()),
            Err(KaminoReadError::ReserveNotActive(1))
        ));
    }

    #[test]
    fn test_calculate_kamino_flash_fee() {
        // Zero rate / zero amount are both zero-fee.
        assert_eq!(calculate_kamino_flash_fee(10_000_000, 0), 0);
        assert_eq!(calculate_kamino_flash_fee(0, 11_529_215_046_068), 0);

        // 5% in U68F60 (60 fractional bits, integer-derived) on 1 SOL = 50M.
        let sf_5pct: u64 = ((1u128 << KAMINO_FRACTION_FRAC_BITS) / 20) as u64;
        assert_eq!(
            calculate_kamino_flash_fee(1_000_000_000, sf_5pct),
            50_000_000
        );

        // Mainnet WSOL reserve: flash_loan_fee_sf = 1e-5 exactly.
        // round(1e9 * 11529215046068 / 2^60) = 10_000 lamports per 1 SOL.
        assert_eq!(
            calculate_kamino_flash_fee(1_000_000_000, 11_529_215_046_068),
            10_000
        );
        // 0.5 SOL borrows half the fee.
        assert_eq!(
            calculate_kamino_flash_fee(500_000_000, 11_529_215_046_068),
            5_000
        );

        // The 1-unit minimum floor: a sub-1e-5 borrow still pays >= 1 unit.
        assert_eq!(calculate_kamino_flash_fee(1, 11_529_215_046_068), 1);

        // u64::MAX disables flash loans; the infallible helper fails closed by
        // returning u64::MAX (never a plausible-looking number, never 0).
        assert_eq!(
            calculate_kamino_flash_fee(1_000_000_000, u64::MAX),
            u64::MAX
        );
        assert_eq!(calculate_kamino_flash_fee(0, u64::MAX), u64::MAX);
    }

    #[test]
    fn test_calculate_kamino_flash_fee_checked_guards() {
        // Disabled sentinel.
        assert_eq!(
            calculate_kamino_flash_fee_checked(1_000_000_000, u64::MAX),
            Err(KaminoFeeError::FlashLoanDisabled)
        );
        // Zero rate is a valid zero fee.
        assert_eq!(calculate_kamino_flash_fee_checked(1_000_000, 0), Ok(0));
        // Mainnet rate on 1 SOL.
        assert_eq!(
            calculate_kamino_flash_fee_checked(1_000_000_000, 11_529_215_046_068),
            Ok(10_000)
        );
        // sf >= 2^60 (i.e. rate >= 1.0) makes the fee >= the principal.
        assert_eq!(
            calculate_kamino_flash_fee_checked(1_000_000_000, 1u64 << KAMINO_FRACTION_FRAC_BITS),
            Err(KaminoFeeError::BorrowTooSmall)
        );
        // A 1-lamport borrow always hits BorrowTooSmall (min fee == 1 == amount).
        assert_eq!(
            calculate_kamino_flash_fee_checked(1, 11_529_215_046_068),
            Err(KaminoFeeError::BorrowTooSmall)
        );
    }

    /// Independent re-derivation of the fee using 256-bit arithmetic and the
    /// literal klend formula, to catch any error in the fast path.
    #[test]
    fn flash_fee_matches_bigint_reference() {
        fn reference(amount: u64, sf: u64) -> u128 {
            if sf == 0 || amount == 0 {
                return 0;
            }
            let product = (amount as u128) * (sf as u128); // exact for the test inputs
            let bits = product.max(KAMINO_FRACTION_ONE_BITS);
            let q = bits >> KAMINO_FRACTION_FRAC_BITS;
            let r = bits & (KAMINO_FRACTION_ONE_BITS - 1);
            q + u128::from(r >= (KAMINO_FRACTION_ONE_BITS >> 1))
        }
        for &(amount, sf) in &[
            (1u64, 11_529_215_046_068u64),
            (999, 11_529_215_046_068),
            (1_000_000_000, 11_529_215_046_068),
            (1_000_000_000_000, 11_529_215_046_068),
            (123_456_789, 57_646_075_230_342_348),
            (1 << 40, 1 << 40),
        ] {
            assert_eq!(
                calculate_kamino_flash_fee(amount, sf) as u128,
                reference(amount, sf),
                "amount={amount} sf={sf}"
            );
        }
    }

    fn fixture_token_account(mint: Pubkey, owner: Pubkey, amount: u64) -> Vec<u8> {
        let mut v = vec![0u8; 165];
        v[0..32].copy_from_slice(mint.as_ref());
        v[32..64].copy_from_slice(owner.as_ref());
        v[64..72].copy_from_slice(&amount.to_le_bytes());
        v
    }

    #[test]
    fn supply_vault_liquidity_decodes_balance() {
        let mint = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let acct = fixture_token_account(mint, owner, 4_200_000_000);
        assert_eq!(
            decode_supply_vault_liquidity(&acct, &mint).unwrap(),
            4_200_000_000
        );
    }

    #[test]
    fn supply_vault_liquidity_rejects_wrong_mint() {
        let mint = Pubkey::new_unique();
        let other = Pubkey::new_unique();
        let acct = fixture_token_account(other, Pubkey::new_unique(), 1_000);
        match decode_supply_vault_liquidity(&acct, &mint) {
            Err(KaminoReadError::MintMismatch { expected, .. }) => {
                assert_eq!(expected, mint.to_string());
            }
            other => panic!("expected MintMismatch, got {other:?}"),
        }
    }

    #[test]
    fn supply_vault_liquidity_rejects_short_account() {
        let mint = Pubkey::new_unique();
        let acct = vec![0u8; 71];
        assert!(matches!(
            decode_supply_vault_liquidity(&acct, &mint),
            Err(KaminoReadError::TokenAccountTooShort(71))
        ));
    }

    #[test]
    fn supply_vault_liquidity_accepts_token_2022_base_layout() {
        let mint = Pubkey::new_unique();
        let mut acct = fixture_token_account(mint, Pubkey::new_unique(), u64::MAX);
        acct.extend_from_slice(&[0u8; 32]);
        assert_eq!(
            decode_supply_vault_liquidity(&acct, &mint).unwrap(),
            u64::MAX
        );
    }
}
