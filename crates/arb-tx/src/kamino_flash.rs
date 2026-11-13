//! Kamino flash-loan parameter struct and borrow/repay instruction builders.

use super::*;

/// Kamino Flash Loan configuration parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaminoFlashParams {
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_source_liquidity: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
}

impl Default for KaminoFlashParams {
    fn default() -> Self {
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: Pubkey::default(),
            reserve_liquidity_fee_receiver: Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
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
        }
    }

    /// Resolve full 12-account matrix for Kamino flash loan operations.
    pub fn to_flash_accounts(&self, user_authority: Pubkey) -> KaminoFlashAccounts {
        let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
        let (lma, _) = derive_lending_market_authority(&self.lending_market, &klend);
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let sysvar = Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let user_ata = if self.user_destination_liquidity == Pubkey::default()
            || self.user_destination_liquidity == user_authority
        {
            get_associated_token_address(&user_authority, &wsol)
        } else {
            self.user_destination_liquidity
        };

        KaminoFlashAccounts {
            user_authority,
            lending_market: self.lending_market,
            lending_market_authority: lma,
            reserve: self.reserve,
            reserve_liquidity_mint: wsol,
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
