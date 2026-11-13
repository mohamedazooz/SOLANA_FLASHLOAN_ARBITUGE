//! Pump.fun bonding-curve swap instruction builder.

use super::*;

/// Complete 12-Account Matrix for Pump.fun Buy and Sell instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpFunAccounts {
    pub global: Pubkey,
    pub fee_recipient: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub associated_bonding_curve: Pubkey,
    pub associated_user: Pubkey,
    pub user: Pubkey,
    pub system_program: Pubkey,
    pub token_program: Pubkey,
    pub rent: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
}

impl PumpFunAccounts {
    pub fn for_mint(mint: Pubkey, user: Pubkey) -> Self {
        let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
        let fee_recipient = Pubkey::from_str(PUMP_FUN_FEE_RECIPIENT).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
        let rent = Pubkey::from_str(SYSVAR_RENT_ID).unwrap();

        let (global, _) = Pubkey::find_program_address(&[PUMP_FUN_GLOBAL_SEED], &program);
        let (bonding_curve, _) =
            Pubkey::find_program_address(&[PUMP_FUN_BONDING_CURVE_SEED, mint.as_ref()], &program);
        let (event_authority, _) =
            Pubkey::find_program_address(&[PUMP_FUN_EVENT_AUTHORITY_SEED], &program);

        let associated_bonding_curve = get_associated_token_address(&bonding_curve, &mint);
        let associated_user = get_associated_token_address(&user, &mint);

        Self {
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            associated_user,
            user,
            system_program,
            token_program,
            rent,
            event_authority,
            program,
        }
    }
}

/// Derive Pump.fun global PDA.
pub fn derive_pump_fun_global_pda() -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_GLOBAL_SEED], &program).0
}

/// Derive Pump.fun bonding curve PDA for a token mint.
pub fn derive_pump_fun_bonding_curve_pda(mint: &Pubkey) -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_BONDING_CURVE_SEED, mint.as_ref()], &program).0
}

/// Derive Pump.fun event authority PDA.
pub fn derive_pump_fun_event_authority() -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_EVENT_AUTHORITY_SEED], &program).0
}

/// Build genuine Pump.fun swap instruction (Buy or Sell) with the official 12-account matrix.
pub fn build_pump_fun_swap_instruction(
    accounts: &PumpFunAccounts,
    amount_in: u64,
    min_amount_out: u64,
    is_buy: bool,
) -> Result<CompiledInstruction, TxError> {
    let mut data = Vec::with_capacity(24);
    if is_buy {
        data.extend_from_slice(&PUMP_FUN_BUY_DISCRIMINATOR);
        data.extend_from_slice(&min_amount_out.to_le_bytes()); // amount of tokens to buy
        data.extend_from_slice(&amount_in.to_le_bytes()); // max_sol_cost
    } else {
        data.extend_from_slice(&PUMP_FUN_SELL_DISCRIMINATOR);
        data.extend_from_slice(&amount_in.to_le_bytes()); // amount of tokens to sell
        data.extend_from_slice(&min_amount_out.to_le_bytes()); // min_sol_output
    }

    Ok(CompiledInstruction {
        program_id: accounts.program,
        accounts: vec![
            AccountMeta::new_readonly(accounts.global, false),
            AccountMeta::new(accounts.fee_recipient, false),
            AccountMeta::new_readonly(accounts.mint, false),
            AccountMeta::new(accounts.bonding_curve, false),
            AccountMeta::new(accounts.associated_bonding_curve, false),
            AccountMeta::new(accounts.associated_user, false),
            AccountMeta::new(accounts.user, true),
            AccountMeta::new_readonly(accounts.system_program, false),
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new_readonly(accounts.rent, false),
            AccountMeta::new_readonly(accounts.event_authority, false),
            AccountMeta::new_readonly(accounts.program, false),
        ],
        data,
    })
}
