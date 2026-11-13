//! Raydium CPMM swap instruction builder and PDA derivation.

use crate::*;

/// Complete 13-Account Matrix for Raydium CP-Swap (CPMM) `swap_base_input` instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmAccounts {
    pub payer: Pubkey,
    pub authority: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub input_token_program: Pubkey,
    pub output_token_program: Pubkey,
    pub input_token_mint: Pubkey,
    pub output_token_mint: Pubkey,
    pub observation_state: Pubkey,
}

impl RaydiumCpmmAccounts {
    /// Canonical mainnet SOL-USDC CPMM pool.
    pub fn mainnet_sol_usdc(
        user_authority: Pubkey,
        user_input_token: Pubkey,
        user_output_token: Pubkey,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
        let pool = Pubkey::from_str(RAYDIUM_CPMM_SOL_USDC_POOL).unwrap();
        let (authority, _) = Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id);
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CPMM_OBSERVATION_SEED, pool.as_ref()],
            &program_id,
        );
        let amm_config = Pubkey::from_str("D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ").unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

        Self {
            payer: user_authority,
            authority,
            amm_config,
            pool_state: pool,
            input_token_account: user_input_token,
            output_token_account: user_output_token,
            input_vault: Pubkey::from_str(RAYDIUM_CPMM_SOL_VAULT).unwrap(),
            output_vault: Pubkey::from_str(RAYDIUM_CPMM_USDC_VAULT).unwrap(),
            input_token_program: token_program,
            output_token_program: token_program,
            input_token_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
            output_token_mint: Pubkey::from_str(USDC_MINT).unwrap(),
            observation_state,
        }
    }

    /// Construct accounts for an arbitrary pool by deriving PDAs.
    #[allow(clippy::too_many_arguments)]
    pub fn for_pool(
        pool: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_token_mint: Pubkey,
        output_token_mint: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        amm_config: Pubkey,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
        let (authority, _) = Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id);
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CPMM_OBSERVATION_SEED, pool.as_ref()],
            &program_id,
        );
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

        Self {
            payer,
            authority,
            amm_config,
            pool_state: pool,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            input_token_program: token_program,
            output_token_program: token_program,
            input_token_mint,
            output_token_mint,
            observation_state,
        }
    }

    /// Construct accounts for an arbitrary CPMM pool with vaults automatically derived.
    pub fn for_pool_derived(
        pool: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_token_mint: Pubkey,
        output_token_mint: Pubkey,
    ) -> Self {
        let input_vault = derive_raydium_cpmm_vault_pda(&pool, &input_token_mint);
        let output_vault = derive_raydium_cpmm_vault_pda(&pool, &output_token_mint);
        let amm_config = Pubkey::from_str("D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ").unwrap();
        Self::for_pool(
            pool,
            payer,
            input_token_account,
            output_token_account,
            input_token_mint,
            output_token_mint,
            input_vault,
            output_vault,
            amm_config,
        )
    }
}

/// Derive Raydium CPMM pool vault PDA for a given pool state and token mint.
/// Seeds: `[b"pool_vault", pool_state.as_ref(), mint.as_ref()]`
pub fn derive_raydium_cpmm_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[
        RAYDIUM_CPMM_POOL_VAULT_SEED,
        pool_state.as_ref(),
        mint.as_ref(),
    ];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Derive Raydium CPMM authority PDA.
pub fn derive_raydium_cpmm_authority() -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id).0
}

/// Derive Raydium CPMM observation state PDA.
pub fn derive_raydium_cpmm_observation_pda(pool_state: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(
        &[RAYDIUM_CPMM_OBSERVATION_SEED, pool_state.as_ref()],
        &program_id,
    )
    .0
}

/// Build genuine Raydium CPMM `swap_base_input` instruction with the official 13-account matrix.
pub fn build_raydium_cpmm_swap_instruction(
    accounts: &RaydiumCpmmAccounts,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(accounts.payer, true),
            AccountMeta::new_readonly(accounts.authority, false),
            AccountMeta::new_readonly(accounts.amm_config, false),
            AccountMeta::new(accounts.pool_state, false),
            AccountMeta::new(accounts.input_token_account, false),
            AccountMeta::new(accounts.output_token_account, false),
            AccountMeta::new(accounts.input_vault, false),
            AccountMeta::new(accounts.output_vault, false),
            AccountMeta::new_readonly(accounts.input_token_program, false),
            AccountMeta::new_readonly(accounts.output_token_program, false),
            AccountMeta::new_readonly(accounts.input_token_mint, false),
            AccountMeta::new_readonly(accounts.output_token_mint, false),
            AccountMeta::new(accounts.observation_state, false),
        ],
        data,
    })
}
