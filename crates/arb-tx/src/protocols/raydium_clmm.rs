use crate::*;
use std::collections::HashSet;

/// Account matrix for Raydium CLMM swap. The first tick array is the fixed
/// Anchor account; optional bitmap extension and further arrays are remaining
/// accounts, in exactly the order consumed by the on-chain swap handler.
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
    /// Compatibility aliases for the first three arrays used by older callers.
    pub tick_array_0: Pubkey,
    pub tick_array_1: Pubkey,
    pub tick_array_2: Pubkey,
    pub tick_array_pdas: Vec<Pubkey>,
    pub bitmap_extension: Option<Pubkey>,
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
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let observation_state = derive_raydium_clmm_observation_pda(&pool_state);

        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = 60 * spacing;
        let start_tick = tick_current.div_euclid(ticks_in_array) * ticks_in_array;
        let ta0 = derive_raydium_clmm_tick_array_pda(&pool_state, start_tick);
        let (ta1, ta2) = if zero_for_one {
            (
                derive_raydium_clmm_tick_array_pda(&pool_state, start_tick - ticks_in_array),
                derive_raydium_clmm_tick_array_pda(&pool_state, start_tick - 2 * ticks_in_array),
            )
        } else {
            (
                derive_raydium_clmm_tick_array_pda(&pool_state, start_tick + ticks_in_array),
                derive_raydium_clmm_tick_array_pda(&pool_state, start_tick + 2 * ticks_in_array),
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
            tick_array_pdas: vec![ta0, ta1, ta2],
            // A PDA derivation is not proof that the optional account exists.
            // The dependency resolver must attach it only after verifying a live snapshot.
            bitmap_extension: None,
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
        tick_array_pdas: Vec<Pubkey>,
        bitmap_extension: Option<Pubkey>,
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
            tick_array_0: tick_array_pdas.first().copied().unwrap_or_default(),
            tick_array_1: tick_array_pdas.get(1).copied().unwrap_or_default(),
            tick_array_2: tick_array_pdas.get(2).copied().unwrap_or_default(),
            tick_array_pdas,
            bitmap_extension,
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

/// Derive Raydium CLMM tick-array bitmap extension PDA.
pub fn derive_raydium_clmm_tick_array_bitmap_extension_pda(pool_state: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(
        &[b"pool_tick_array_bitmap_extension", pool_state.as_ref()],
        &program_id,
    )
    .0
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

/// Build Raydium CLMM swap instruction using one required tick array, optional
/// bitmap extension, and every additional tick array required by the quote walk.
pub fn build_raydium_clmm_swap_instruction(
    accounts: &RaydiumClmmAccounts,
    amount_in: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    if accounts.tick_array_pdas.is_empty() {
        return Err(TxError::SanitizationError(
            "Raydium CLMM requires at least one tick array".into(),
        ));
    }

    let mut seen = HashSet::with_capacity(accounts.tick_array_pdas.len() + 1);
    for tick_array in &accounts.tick_array_pdas {
        if *tick_array == Pubkey::default() || !seen.insert(*tick_array) {
            return Err(TxError::SanitizationError(
                "Raydium CLMM tick arrays contain a default or duplicate account".into(),
            ));
        }
    }
    if let Some(bitmap_extension) = accounts.bitmap_extension {
        if bitmap_extension == Pubkey::default() || !seen.insert(bitmap_extension) {
            return Err(TxError::SanitizationError(
                "Raydium CLMM bitmap extension is default or aliases another account".into(),
            ));
        }
    }

    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    let mut instruction_accounts = vec![
        AccountMeta::new(accounts.payer, true),
        AccountMeta::new_readonly(accounts.amm_config, false),
        AccountMeta::new(accounts.pool_state, false),
        AccountMeta::new(accounts.input_token_account, false),
        AccountMeta::new(accounts.output_token_account, false),
        AccountMeta::new(accounts.input_vault, false),
        AccountMeta::new(accounts.output_vault, false),
        AccountMeta::new(accounts.observation_state, false),
        AccountMeta::new_readonly(accounts.token_program, false),
        AccountMeta::new(accounts.tick_array_pdas[0], false),
    ];

    // The on-chain legacy swap handler scans remaining accounts by account
    // data length; place the optional bitmap extension before additional arrays.
    if let Some(bitmap_extension) = accounts.bitmap_extension {
        instruction_accounts.push(AccountMeta::new(bitmap_extension, false));
    }
    instruction_accounts.extend(
        accounts
            .tick_array_pdas
            .iter()
            .skip(1)
            .copied()
            .map(|tick_array| AccountMeta::new(tick_array, false)),
    );

    Ok(CompiledInstruction {
        program_id,
        accounts: instruction_accounts,
        data,
    })
}
