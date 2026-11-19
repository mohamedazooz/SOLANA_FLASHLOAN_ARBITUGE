//! Orca Whirlpool swap instruction builder and PDA derivation.

use crate::*;
use arb_math::clmm::orca_tick_array_start_indexes;

/// Derive Whirlpool TickArray PDA for a given start tick index.
pub fn derive_orca_tick_array_pda(whirlpool: &Pubkey, start_tick_index: i32) -> Pubkey {
    let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
    let start_str = start_tick_index.to_string();
    let seeds: &[&[u8]] = &[b"tick_array", whirlpool.as_ref(), start_str.as_bytes()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Complete 11-Account Matrix for Orca Whirlpool Swap instruction
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaWhirlpoolAccounts {
    pub token_program: Pubkey,
    pub whirlpool: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_vault_b: Pubkey,
    pub tick_array_0: Pubkey,
    pub tick_array_1: Pubkey,
    pub tick_array_2: Pubkey,
    pub oracle: Pubkey,
}

impl OrcaWhirlpoolAccounts {
    pub fn for_whirlpool(
        whirlpool: Pubkey,
        token_vault_a: Pubkey,
        token_vault_b: Pubkey,
        tick_spacing: u16,
        tick_current_index: i32,
        a_to_b: bool,
    ) -> Self {
        let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
        let starts = orca_tick_array_start_indexes(tick_current_index, tick_spacing, a_to_b);
        let base = arb_math::clmm::get_start_tick_index(tick_current_index, tick_spacing);
        let start_at = |index: usize| -> i32 { starts.get(index).copied().unwrap_or(base) };

        let derive_ta = |start: i32| derive_orca_tick_array_pda(&whirlpool, start);

        let ta0 = derive_ta(start_at(0));
        let ta1 = derive_ta(start_at(1));
        let ta2 = derive_ta(start_at(2));

        let (oracle, _) =
            Pubkey::find_program_address(&[b"oracle", whirlpool.as_ref()], &program_id);

        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            whirlpool,
            token_vault_a,
            token_vault_b,
            tick_array_0: ta0,
            tick_array_1: ta1,
            tick_array_2: ta2,
            oracle,
        }
    }

    /// Select the correct 3 arrays for the hop direction from the registry
    /// window. The adapter publishes `[a_to_b x3][b_to_a x3]` (6 PDAs). A
    /// legacy 3-PDA window is used as-is and assumed pre-ordered by the caller.
    pub fn select_tick_arrays(tick_array_pdas: &[Pubkey], a_to_b: bool) -> [Pubkey; 3] {
        let base = if tick_array_pdas.len() >= 6 && !a_to_b {
            3
        } else {
            0
        };
        let ta0 = tick_array_pdas
            .get(base)
            .copied()
            .unwrap_or(Pubkey::default());
        let ta1 = tick_array_pdas.get(base + 1).copied().unwrap_or(ta0);
        let ta2 = tick_array_pdas.get(base + 2).copied().unwrap_or(ta1);
        [ta0, ta1, ta2]
    }

    pub fn with_tick_arrays(
        whirlpool: Pubkey,
        token_vault_a: Pubkey,
        token_vault_b: Pubkey,
        oracle: Pubkey,
        tick_array_0: Pubkey,
        tick_array_1: Pubkey,
        tick_array_2: Pubkey,
    ) -> Self {
        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            whirlpool,
            token_vault_a,
            token_vault_b,
            tick_array_0,
            tick_array_1,
            tick_array_2,
            oracle,
        }
    }

    pub fn mainnet_sol_usdc() -> Self {
        Self::for_whirlpool(
            Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(ORCA_SOL_VAULT).unwrap(),
            Pubkey::from_str(ORCA_USDC_VAULT).unwrap(),
            4,
            -23621,
            true,
        )
    }
}

/// Build genuine Orca Whirlpool Swap Instruction with the complete 11-account matrix.
pub fn build_orca_whirlpool_swap_instruction_11(
    accounts: &OrcaWhirlpoolAccounts,
    user_authority: Pubkey,
    token_owner_account_a: Pubkey,
    token_owner_account_b: Pubkey,
    amount_in: u64,
    other_amount_threshold: u64,
    a_to_b: bool,
    is_adaptive_fee: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let mut data = Vec::with_capacity(34);
    // Anchor discriminator for `swap`
    data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    // `sqrt_price_limit` is a Q64.64 SQUARE-ROOT price bound in raw token
    // units, NOT a token-ratio. The previous implementation derived it as
    // `threshold / amount_in`, which is dimensionally wrong: for a_to_b it
    // usually landed far below spot (silently disabling the bound) and for
    // b_to_a it landed below spot, which the program rejects outright
    // (`SqrtPriceLimitBelowPrice`).
    //
    // Exact-input slippage is already enforced by `other_amount_threshold`
    // (the program requires `amount_out >= threshold`). We therefore pass the
    // official program envelope so the threshold is the binding guard:
    //   a_to_b -> MIN_SQRT_PRICE (price may fall, no additional floor)
    //   b_to_a -> MAX_SQRT_PRICE (price may rise, no additional ceiling)
    let sqrt_price_limit = if a_to_b {
        arb_constants::CLMM_MIN_SQRT_PRICE_X64
    } else {
        arb_constants::ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64
    };
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(1); // amount_specified_is_input = true
    data.push(a_to_b as u8);

    let oracle_meta = if is_adaptive_fee {
        AccountMeta::new(accounts.oracle, false)
    } else {
        AccountMeta::new_readonly(accounts.oracle, false)
    };

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new_readonly(user_authority, true),
            AccountMeta::new(accounts.whirlpool, false),
            AccountMeta::new(token_owner_account_a, false),
            AccountMeta::new(accounts.token_vault_a, false),
            AccountMeta::new(token_owner_account_b, false),
            AccountMeta::new(accounts.token_vault_b, false),
            AccountMeta::new(accounts.tick_array_0, false),
            AccountMeta::new(accounts.tick_array_1, false),
            AccountMeta::new(accounts.tick_array_2, false),
            oracle_meta,
        ],
        data,
    })
}
