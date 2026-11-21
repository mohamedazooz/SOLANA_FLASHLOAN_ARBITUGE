//! Kamino flash-loan parameter struct and borrow/repay instruction builders.

use crate::*;

/// Kamino Flash Loan configuration parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaminoFlashParams {
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_source_liquidity: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    /// Mint the borrowed liquidity is denominated in. Defaults to WSOL; a
    /// non-WSOL reserve selected via `from_candidate` must set it so account
    /// assembly (ATA derivation, reserve mint meta) matches the borrow asset.
    pub liquidity_mint: Pubkey,
    /// Token program that owns `liquidity_mint`; used for ATA derivation and
    /// the Kamino token-program account meta.
    pub liquidity_token_program: Pubkey,
}

impl Default for KaminoFlashParams {
    fn default() -> Self {
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: Pubkey::default(),
            reserve_liquidity_fee_receiver: Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
            liquidity_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
            liquidity_token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
        }
    }
}

impl KaminoFlashParams {
    /// WSOL main-market flash-loan parameters (verified mainnet addresses).
    /// The user destination starts as the zero pubkey and must be set with
    /// `for_user` / overwritten before assembly.
    pub fn default_wsol() -> Self {
        Self::default()
    }

    pub fn for_user(user: Pubkey) -> Self {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let user_wsol_ata = get_associated_token_address(&user, &wsol);
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: user_wsol_ata,
            reserve_liquidity_fee_receiver: Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
            liquidity_mint: wsol,
            liquidity_token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
        }
    }

    /// Parameters for a live-selected reserve candidate: the user's
    /// destination is the ATA of the candidate's mint, and the mint itself is
    /// carried into account assembly.
    /// Construct parameters for a reserve known to use the legacy SPL Token program.
    /// For Token-2022 reserves, call `from_candidate_with_token_program` with
    /// the actual mint account owner.
    pub fn from_candidate(user: Pubkey, c: &KaminoReserveCandidate) -> Self {
        Self::from_candidate_with_token_program(
            user,
            c,
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
        )
    }

    /// Construct parameters using the token program that owns the candidate mint.
    pub fn from_candidate_with_token_program(
        user: Pubkey,
        c: &KaminoReserveCandidate,
        token_program: Pubkey,
    ) -> Self {
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: c.reserve,
            reserve_source_liquidity: c.supply_vault,
            user_destination_liquidity: get_associated_token_address_with_program_id(
                &user,
                &c.mint,
                &token_program,
            ),
            reserve_liquidity_fee_receiver: c.fee_vault,
            liquidity_mint: c.mint,
            liquidity_token_program: token_program,
        }
    }

    /// Resolve full 12-account matrix for Kamino flash loan operations.
    pub fn to_flash_accounts(&self, user_authority: Pubkey) -> KaminoFlashAccounts {
        let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
        let (lma, _) = derive_lending_market_authority(&self.lending_market, &klend);
        let liquidity_mint = self.liquidity_mint;
        let sysvar = Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap();
        let token_program = self.liquidity_token_program;
        let user_ata = if self.user_destination_liquidity == Pubkey::default()
            || self.user_destination_liquidity == user_authority
        {
            get_associated_token_address_with_program_id(
                &user_authority,
                &liquidity_mint,
                &token_program,
            )
        } else {
            self.user_destination_liquidity
        };

        KaminoFlashAccounts {
            user_authority,
            lending_market: self.lending_market,
            lending_market_authority: lma,
            reserve: self.reserve,
            reserve_liquidity_mint: liquidity_mint,
            reserve_supply_vault: self.reserve_source_liquidity,
            user_liquidity_ata: user_ata,
            reserve_fee_vault: self.reserve_liquidity_fee_receiver,
            referrer_token_state: None,
            referrer_account: None,
            sysvar_instructions: sysvar,
            token_program,
            klend_program_id: klend,
        }
    }
}

/// Build Kamino Flash Borrow instruction (12 accounts, 16-byte data payload).
pub fn build_kamino_flash_borrow(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
) -> CompiledInstruction {
    let accounts = params.to_flash_accounts(user_authority);
    kamino::build_kamino_flash_borrow(&accounts, amount)
}

/// Build Kamino Flash Repay instruction with default borrow_instruction_index = 2 (12 accounts, 17-byte data payload).
pub fn build_kamino_flash_repay(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
) -> CompiledInstruction {
    build_kamino_flash_repay_with_index(params, user_authority, amount, 2)
}

/// Build Kamino Flash Repay instruction with explicit borrow_instruction_index (12 accounts, 17-byte data payload).
pub fn build_kamino_flash_repay_with_index(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
    borrow_instruction_index: u8,
) -> CompiledInstruction {
    let accounts = params.to_flash_accounts(user_authority);
    kamino::build_kamino_flash_repay(&accounts, amount, borrow_instruction_index)
}

#[cfg(test)]
mod token_program_tests {
    use super::*;

    #[test]
    fn candidate_flash_ata_uses_supplied_token_program() {
        let candidate = KaminoReserveCandidate {
            reserve: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            decimals: 6,
            active: true,
            flash_loan_fee_sf: 0,
            supply_vault: Pubkey::new_unique(),
            fee_vault: Pubkey::new_unique(),
            available_liquidity: 1_000_000,
        };
        let user = Pubkey::new_unique();
        let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
        let params =
            KaminoFlashParams::from_candidate_with_token_program(user, &candidate, token_2022);
        assert_eq!(params.liquidity_token_program, token_2022);
        assert_eq!(
            params.user_destination_liquidity,
            get_associated_token_address_with_program_id(&user, &candidate.mint, &token_2022)
        );
        assert_eq!(params.to_flash_accounts(user).token_program, token_2022);
    }
}
