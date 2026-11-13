//! Raydium CLMM swap instruction builder and PDA derivation.

use super::*;

/// Complete Account Matrix for Raydium CLMM `swap` instruction (with 60-ticks/array tick arrays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumClmmAccounts {
    pub payer: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    pub token_program: Pubkey,
    pub tick_array_0: Pubkey,
    pub tick_array_1: Pubkey,
    pub tick_array_2: Pubkey,
}

impl RaydiumClmmAccounts {
    #[allow(clippy::too_many_arguments)]
    pub fn for_pool(
        pool_state: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        amm_config: Pubkey,
        tick_spacing: u16,
        tick_current: i32,
        zero_for_one: bool,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CLMM_OBSERVATION_SEED, pool_state.as_ref()],
            &program_id,
        );

        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = 60 * spacing;
        let start_tick = tick_current.div_euclid(ticks_in_array) * ticks_in_array;

        let derive_ta = |start: i32| {
            let start_bytes = start.to_be_bytes();
            let seeds: &[&[u8]] = &[
                RAYDIUM_CLMM_TICK_ARRAY_SEED,
                pool_state.as_ref(),
                &start_bytes,
            ];
            Pubkey::find_program_address(seeds, &program_id).0
        };

        let ta0 = derive_ta(start_tick);
        let (ta1, ta2) = if zero_for_one {
            (
                derive_ta(start_tick - ticks_in_array),
                derive_ta(start_tick - 2 * ticks_in_array),
            )
        } else {
            (
                derive_ta(start_tick + ticks_in_array),
                derive_ta(start_tick + 2 * ticks_in_array),
            )
        };

        Self {
            payer,
            amm_config,
            pool_state,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            observation_state,
            token_program,
            tick_array_0: ta0,
            tick_array_1: ta1,
            tick_array_2: ta2,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_tick_arrays(
        pool_state: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        amm_config: Pubkey,
        observation_state: Pubkey,
        tick_array_0: Pubkey,
        tick_array_1: Pubkey,
        tick_array_2: Pubkey,
    ) -> Self {
        Self {
            payer,
            amm_config,
            pool_state,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            observation_state,
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            tick_array_0,
            tick_array_1,
            tick_array_2,
        }
    }
}

/// Derive Raydium CLMM tick array PDA.
pub fn derive_raydium_clmm_tick_array_pda(pool_state: &Pubkey, start_index: i32) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    let start_bytes = start_index.to_be_bytes();
    let seeds: &[&[u8]] = &[
        RAYDIUM_CLMM_TICK_ARRAY_SEED,
        pool_state.as_ref(),
        &start_bytes,
    ];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Derive Raydium CLMM observation PDA.
pub fn derive_raydium_clmm_observation_pda(pool_state: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(
        &[RAYDIUM_CLMM_OBSERVATION_SEED, pool_state.as_ref()],
        &program_id,
    )
    .0
}

/// Derive Raydium CLMM vault PDA.
pub fn derive_raydium_clmm_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[b"pool_vault", pool_state.as_ref(), mint.as_ref()];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Build genuine Raydium CLMM `swap` instruction.
pub fn build_raydium_clmm_swap_instruction(
    accounts: &RaydiumClmmAccounts,
    amount_in: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(accounts.payer, true),
            AccountMeta::new_readonly(accounts.amm_config, false),
            AccountMeta::new(accounts.pool_state, false),
            AccountMeta::new(accounts.input_token_account, false),
            AccountMeta::new(accounts.output_token_account, false),
            AccountMeta::new(accounts.input_vault, false),
            AccountMeta::new(accounts.output_vault, false),
            AccountMeta::new(accounts.observation_state, false),
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new(accounts.tick_array_0, false),
            AccountMeta::new(accounts.tick_array_1, false),
            AccountMeta::new(accounts.tick_array_2, false),
        ],
        data,
    })
}
