//! PumpSwap v2 swap instruction builders.

use crate::*;

/// PumpSwap trade mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpSwapTradeMode {
    /// Exact quote in: spend exact quote tokens to get min base tokens (buy_exact_quote_in_v2).
    BuyExactQuoteInV2,
    /// Exact base out: specify exact base tokens to get with max quote tokens (buy_v2).
    BuyExactBaseOutV2,
    /// Exact base in: sell exact base tokens to get min quote tokens (sell_v2).
    SellExactBaseInV2,
    /// Legacy V1 buy (for non-V2 pools like cashback pools).
    BuyV1Legacy,
    /// Legacy V1 sell (for non-V2 pools like cashback pools).
    SellV1Legacy,
}

impl std::fmt::Display for PumpSwapTradeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PumpSwapTradeMode::BuyExactQuoteInV2 => write!(f, "BuyExactQuoteInV2"),
            PumpSwapTradeMode::BuyExactBaseOutV2 => write!(f, "BuyExactBaseOutV2"),
            PumpSwapTradeMode::SellExactBaseInV2 => write!(f, "SellExactBaseInV2"),
            PumpSwapTradeMode::BuyV1Legacy => write!(f, "BuyV1Legacy"),
            PumpSwapTradeMode::SellV1Legacy => write!(f, "SellV1Legacy"),
        }
    }
}

/// Build PumpSwap V2 using the official 17-account buy_v2,
/// buy_exact_quote_in_v2, and sell_v2 account layout.
///
/// Unlike V1, V2 does not pass protocol-fee recipient/creator-vault accounts
/// and does not append inferred remaining accounts. Those fees remain in
/// the pool; only the buyback fee is sent to a listed recipient ATA.
///
/// Instruction argument order depends on the mode:
/// - buy_exact_quote_in_v2: spendable quote input, minimum base output
/// - buy_v2: exact base output, maximum quote input
/// - sell_v2: exact base input, minimum quote output.
#[allow(clippy::too_many_arguments)]
pub fn build_pump_swap_instruction_v2(
    pool: Pubkey,
    user_authority: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    pool_base_vault: Pubkey,
    pool_quote_vault: Pubkey,
    global_config: Pubkey,
    fee_config: Pubkey,
    token_program_base: Pubkey,
    token_program_quote: Pubkey,
    event_authority: Pubkey,
    amount_specified: u64,
    other_amount_threshold: u64,
    trade_mode: PumpSwapTradeMode,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(PUMP_SWAP_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    // Pump's V2 buyback recipient ATAs are published and pre-created for the
    // standard WSOL/USDC quote assets. For arbitrary quote mints the ATA may
    // not exist; creating it requires a rent-aware setup instruction and is
    // deliberately not guessed in this swap-only builder.
    let wsol = Pubkey::from_str(WSOL_MINT).map_err(|e| TxError::InstructionError(e.to_string()))?;
    let usdc = Pubkey::from_str(USDC_MINT).map_err(|e| TxError::InstructionError(e.to_string()))?;
    if quote_mint != wsol && quote_mint != usdc {
        return Err(TxError::SanitizationError(
            "PumpSwap V2 only builds for WSOL/USDC quote mints until the buyback ATA is resolved and rent-accounted"
                .into(),
        ));
    }

    let buyback_wallet = Pubkey::from_str(arb_constants::PUMP_SWAP_BUYBACK_FEE_RECIPIENTS[0])
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let buyback_fee_recipient = get_associated_token_address_with_program_id(
        &buyback_wallet,
        &quote_mint,
        &token_program_quote,
    );
    let user_base_token = get_associated_token_address_with_program_id(
        &user_authority,
        &base_mint,
        &token_program_base,
    );
    let user_quote_token = get_associated_token_address_with_program_id(
        &user_authority,
        &quote_mint,
        &token_program_quote,
    );
    let (user_volume_accumulator, _) = Pubkey::find_program_address(
        &[b"user_volume_accumulator", user_authority.as_ref()],
        &program_id,
    );

    let discriminator = match trade_mode {
        PumpSwapTradeMode::BuyExactQuoteInV2 => PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
        PumpSwapTradeMode::BuyExactBaseOutV2 => PUMP_SWAP_BUY_V2_DISCRIMINATOR,
        PumpSwapTradeMode::SellExactBaseInV2 => PUMP_SWAP_SELL_V2_DISCRIMINATOR,
        PumpSwapTradeMode::BuyV1Legacy | PumpSwapTradeMode::SellV1Legacy => {
            return Err(TxError::SanitizationError(
                "V1 trade modes must use the V1 PumpSwap instruction builder".into(),
            ));
        }
    };

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&amount_specified.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());

    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(pool, false),
            AccountMeta::new(user_authority, true),
            AccountMeta::new_readonly(global_config, false),
            AccountMeta::new_readonly(base_mint, false),
            AccountMeta::new_readonly(quote_mint, false),
            AccountMeta::new(user_base_token, false),
            AccountMeta::new(user_quote_token, false),
            AccountMeta::new(pool_base_vault, false),
            AccountMeta::new(pool_quote_vault, false),
            AccountMeta::new_readonly(token_program_base, false),
            AccountMeta::new_readonly(token_program_quote, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new(user_volume_accumulator, false),
            AccountMeta::new_readonly(fee_config, false),
            AccountMeta::new(buyback_fee_recipient, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(program_id, false),
        ],
        data,
    })
}

/// Legacy PumpSwap V1 is intentionally disabled until its complete, current
/// IDL account contract is wired into this builder.
///
/// The old implementation emitted a shortened account list. Pump's V1 trade
/// instructions require additional program, authority, volume and fee accounts;
/// using an incomplete list creates transactions that can only fail at CPI.
/// V2 is supported by [`build_pump_swap_instruction_v2`]. Cashback pools, which
/// still require V1 according to Pump's documentation, must remain unavailable
/// until a fully verified V1 builder is implemented.
#[allow(clippy::too_many_arguments)]
pub fn build_pump_swap_instruction(
    _pool: Pubkey,
    _user_authority: Pubkey,
    _base_mint: Pubkey,
    _quote_mint: Pubkey,
    _pool_base_vault: Pubkey,
    _pool_quote_vault: Pubkey,
    _global_config: Pubkey,
    _fee_config: Pubkey,
    _protocol_fee_recipient: Pubkey,
    _protocol_fee_recipient_ata: Pubkey,
    _coin_creator_vault_authority: Pubkey,
    _coin_creator_vault_ata: Pubkey,
    _token_program_base: Pubkey,
    _token_program_quote: Pubkey,
    _amount_in: u64,
    _min_amount_out: u64,
    _is_buy: bool,
) -> Result<CompiledInstruction, TxError> {
    Err(TxError::SanitizationError(
        "PumpSwap V1 execution is disabled: the legacy builder does not yet provide the complete official account matrix; use a verified V2 pool or exclude this pool".into(),
    ))
}

#[cfg(test)]
mod ata_program_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn pump_swap_v2_uses_official_fixed_17_account_layout() {
        let user = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::from_str(WSOL_MINT).unwrap();
        let pool = Pubkey::new_unique();
        let global_config = Pubkey::new_unique();
        let fee_config = Pubkey::new_unique();
        let event_authority = Pubkey::new_unique();
        let base_token_program = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
        let quote_token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();

        let ix = build_pump_swap_instruction_v2(
            pool,
            user,
            base_mint,
            quote_mint,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            global_config,
            fee_config,
            base_token_program,
            quote_token_program,
            event_authority,
            10_000,
            9_000,
            PumpSwapTradeMode::BuyExactQuoteInV2,
        )
        .unwrap();

        let buyback_wallet =
            Pubkey::from_str(arb_constants::PUMP_SWAP_BUYBACK_FEE_RECIPIENTS[0]).unwrap();
        let expected_buyback_ata = get_associated_token_address_with_program_id(
            &buyback_wallet,
            &quote_mint,
            &quote_token_program,
        );
        let expected_user_volume = Pubkey::find_program_address(
            &[b"user_volume_accumulator", user.as_ref()],
            &ix.program_id,
        )
        .0;

        assert_eq!(ix.accounts.len(), 17);
        assert_eq!(ix.accounts[0].pubkey, pool);
        assert_eq!(ix.accounts[1].pubkey, user);
        assert!(ix.accounts[1].is_signer);
        assert_eq!(ix.accounts[2].pubkey, global_config);
        assert_eq!(ix.accounts[3].pubkey, base_mint);
        assert_eq!(ix.accounts[4].pubkey, quote_mint);
        assert_eq!(
            ix.accounts[5].pubkey,
            get_associated_token_address_with_program_id(&user, &base_mint, &base_token_program)
        );
        assert_eq!(
            ix.accounts[6].pubkey,
            get_associated_token_address_with_program_id(&user, &quote_mint, &quote_token_program)
        );
        assert_eq!(ix.accounts[9].pubkey, base_token_program);
        assert_eq!(ix.accounts[10].pubkey, quote_token_program);
        assert_eq!(
            ix.accounts[11].pubkey,
            Pubkey::new_from_array(SYSTEM_PROGRAM_ID)
        );
        assert_eq!(ix.accounts[12].pubkey, expected_user_volume);
        assert!(ix.accounts[12].is_writable);
        assert_eq!(ix.accounts[13].pubkey, fee_config);
        assert_eq!(ix.accounts[14].pubkey, expected_buyback_ata);
        assert!(ix.accounts[14].is_writable);
        assert_eq!(ix.accounts[15].pubkey, event_authority);
        assert_eq!(ix.accounts[16].pubkey, ix.program_id);
        assert!(
            ix.accounts
                .iter()
                .enumerate()
                .all(|(index, meta)| index == 11 || meta.pubkey != Pubkey::default())
        );
        assert_eq!(
            &ix.data[..8],
            &PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR
        );
    }

    #[test]
    fn pump_swap_v2_rejects_unresolved_custom_quote_mint() {
        let result = build_pump_swap_instruction_v2(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::new_unique(),
            100,
            90,
            PumpSwapTradeMode::SellExactBaseInV2,
        );
        assert!(result.is_err());
    }

    #[test]
    fn pump_swap_v1_builder_fails_closed_until_full_idl_contract_is_implemented() {
        let result = build_pump_swap_instruction(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            10_000,
            9_000,
            true,
        );
        assert!(result.is_err());
    }
}
