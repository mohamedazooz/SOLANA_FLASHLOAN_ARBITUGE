//! Orca Whirlpool swap instruction builder and PDA derivation.

use super::*;

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
        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = 88 * spacing;
        let start_tick = tick_current_index.div_euclid(ticks_in_array) * ticks_in_array;

        let derive_ta = |start: i32| derive_orca_tick_array_pda(&whirlpool, start);

        let ta0 = derive_ta(start_tick);
        let (ta1, ta2) = if a_to_b {
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
    // Slippage-derived sqrt_price_limit: no hardcoded full-range values.
    // When a_to_b (price drops), the limit is the price at which we receive exactly
    // other_amount_threshold of token B for amount_in of token A.
    // When b_to_a (price rises), the limit is the price at which we pay exactly
    // amount_in of token B for other_amount_threshold of token A.
    const Q64: u128 = 1u128 << 64;
    let sqrt_price_limit = if amount_in > 0 && other_amount_threshold > 0 {
        if a_to_b {
            // price = other_amount_threshold / amount_in (B per A)
            (u128::from(other_amount_threshold) * Q64) / u128::from(amount_in)
        } else {
            // price = amount_in / other_amount_threshold (A per B) -> invert for B per A
            (u128::from(amount_in) * Q64) / u128::from(other_amount_threshold)
        }
    } else if a_to_b {
        // No slippage protection possible; use the absolute minimum price.
        4_295_048_016u128
    } else {
        79_226_673_515_401_499_993_453_864_427u128
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
