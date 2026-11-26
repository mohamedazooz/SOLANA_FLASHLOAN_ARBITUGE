//! ATA derivation, idempotent ATA creation, token-account close, sync-native
//! and system transfer helpers.

use crate::*;

use arb_types::{DexProtocol, SwapHop};
use std::collections::HashMap;

/// ATA instruction/account shape shared by the transaction builder and CU model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RouteAtaInstructionCounts {
    pub unique_token_accounts: u16,
    pub intermediate_accounts: u16,
    pub idempotent_create_instructions: u16,
    pub close_token_account_instructions: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAtaPlan {
    pub wsol_ata: Pubkey,
    pub intermediate_atas: Vec<(Pubkey, Pubkey, Pubkey)>,
    pub counts: RouteAtaInstructionCounts,
}

fn route_intermediate_mints(hops: &[SwapHop]) -> Result<Vec<(Pubkey, Pubkey)>> {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let mut programs = HashMap::<Pubkey, Pubkey>::new();
    let mut intermediate = Vec::new();

    for hop in hops {
        for (mint, program_kind) in [
            (hop.mint_in.0, hop.token_programs[0]),
            (hop.mint_out.0, hop.token_programs[1]),
        ] {
            if mint == wsol || mint == usdc {
                continue;
            }
            let program = program_kind.program_id();
            if let Some(existing) = programs.get(&mint) {
                if *existing != program {
                    return Err(TxError::InvalidInput(format!(
                        "route assigns multiple token programs to mint {mint}"
                    ))
                    .into());
                }
            } else {
                programs.insert(mint, program);
                intermediate.push((mint, program));
            }
        }
    }
    Ok(intermediate)
}

/// Count ATA instructions emitted by the canonical atomic builder.
/// Intermediate ATAs are not closed in the atomic transaction; settlement
/// reclaims rent only after verifying a zero token balance.
pub fn route_ata_instruction_counts(hops: &[SwapHop]) -> Result<RouteAtaInstructionCounts> {
    let intermediate = route_intermediate_mints(hops)?;
    let first_is_pump_buy = hops
        .first()
        .is_some_and(|hop| hop.protocol == DexProtocol::PumpFun);
    let last_is_pump_sell = hops
        .last()
        .is_some_and(|hop| hop.protocol == DexProtocol::PumpFun);
    let wsol_recreated_after_first_pump = first_is_pump_buy && hops.len() > 1;

    let unique_accounts = 1usize.saturating_add(intermediate.len());
    let create_instructions = 1usize
        .saturating_add(intermediate.len())
        .saturating_add(usize::from(wsol_recreated_after_first_pump))
        .saturating_add(usize::from(last_is_pump_sell));

    Ok(RouteAtaInstructionCounts {
        unique_token_accounts: u16::try_from(unique_accounts).unwrap_or(u16::MAX),
        intermediate_accounts: u16::try_from(intermediate.len()).unwrap_or(u16::MAX),
        idempotent_create_instructions: u16::try_from(create_instructions).unwrap_or(u16::MAX),
        close_token_account_instructions: u16::from(first_is_pump_buy),
    })
}

/// Build the exact local ATA PDA plan used by the canonical transaction builder.
pub fn build_route_ata_plan(payer: Pubkey, hops: &[SwapHop]) -> Result<RouteAtaPlan> {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let wsol_ata = get_associated_token_address(&payer, &wsol);
    let intermediate_atas = route_intermediate_mints(hops)?
        .into_iter()
        .map(|(mint, program)| {
            let ata = get_associated_token_address_with_program_id(&payer, &mint, &program);
            (mint, ata, program)
        })
        .collect::<Vec<_>>();
    let counts = route_ata_instruction_counts(hops)?;
    Ok(RouteAtaPlan {
        wsol_ata,
        intermediate_atas,
        counts,
    })
}

/// Derive an SPL Token (legacy Token Program) ATA.
///
/// This helper is only correct when the mint is owned by the legacy SPL Token
/// program. For Token-2022 or mint-dynamic swap paths, use
/// [`get_associated_token_address_with_program_id`] with the mint's actual
/// token program.
pub fn get_associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Derive Associated Token Address (ATA) deterministically for a wallet, mint, and token program (SPL Token or Token-2022).
pub fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Build idempotent ATA creation instruction (CreateIdempotent opcode 1).
/// Safely initializes intermediate token account if missing, does not fail if it already exists.
pub fn build_create_ata_idempotent(
    payer: Pubkey,
    wallet_owner: Pubkey,
    token_mint: Pubkey,
    associated_token_account: Pubkey,
) -> CompiledInstruction {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    CompiledInstruction {
        program_id: ata_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(associated_token_account, false),
            AccountMeta::new_readonly(wallet_owner, false),
            AccountMeta::new_readonly(token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1], // CreateIdempotent opcode
    }
}

/// Build idempotent ATA creation instruction for a specific token program (SPL Token or Token-2022).
/// Data layout: [1] (CreateIdempotent opcode). Account order matches the ATA program.
pub fn build_create_ata_idempotent_for_program(
    payer: Pubkey,
    wallet_owner: Pubkey,
    token_mint: Pubkey,
    associated_token_account: Pubkey,
    token_program: Pubkey,
) -> CompiledInstruction {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    CompiledInstruction {
        program_id: ata_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(associated_token_account, false),
            AccountMeta::new_readonly(wallet_owner, false),
            AccountMeta::new_readonly(token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1],
    }
}

/// Build SPL Token / Token-2022 `CloseAccount` instruction for a specific token program.
/// Data layout: [9] (CloseAccount opcode).
pub fn build_close_token_account_for_program(
    account_to_close: Pubkey,
    destination_rent_receiver: Pubkey,
    owner_authority: Pubkey,
    token_program: Pubkey,
) -> CompiledInstruction {
    CompiledInstruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(account_to_close, false),
            AccountMeta::new(destination_rent_receiver, false),
            AccountMeta::new_readonly(owner_authority, true),
        ],
        data: vec![9],
    }
}

/// Build SPL Token close_account instruction (opcode 9) to reclaim 100% of the rent deposit back to payer.
pub fn build_close_token_account(
    account_to_close: Pubkey,
    destination_rent_receiver: Pubkey,
    owner_authority: Pubkey,
) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

    CompiledInstruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(account_to_close, false),
            AccountMeta::new(destination_rent_receiver, false),
            AccountMeta::new_readonly(owner_authority, true),
        ],
        data: vec![9], // CloseAccount opcode
    }
}

/// Build SPL Token sync_native instruction (opcode 17).
pub fn build_sync_native(account: Pubkey) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    CompiledInstruction {
        program_id: token_program,
        accounts: vec![AccountMeta::new(account, false)],
        data: vec![17], // SyncNative opcode
    }
}

/// Build standard System Program transfer instruction.
pub fn build_system_transfer(from: Pubkey, to: Pubkey, lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemProgram::Transfer instruction index = 2
    data.extend_from_slice(&lamports.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_ata_plan_counts_actual_instructions_not_settlement_closes() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let meme_a = Pubkey::new_unique();
        let meme_b = Pubkey::new_unique();
        let make_hop = |protocol, input, output| SwapHop {
            pool_id: arb_types::PoolId(Pubkey::new_unique()),
            protocol,
            mint_in: arb_types::MintId(input),
            mint_out: arb_types::MintId(output),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [arb_types::TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let hops = [
            make_hop(DexProtocol::PumpFun, wsol, meme_a),
            make_hop(DexProtocol::MeteoraDlmm, meme_a, meme_b),
            make_hop(DexProtocol::PumpFun, meme_b, wsol),
        ];
        let plan = build_route_ata_plan(Pubkey::new_unique(), &hops).unwrap();
        assert_eq!(plan.counts.intermediate_accounts, 2);
        assert_eq!(plan.counts.unique_token_accounts, 3);
        // Initial WSOL + two intermediate accounts + WSOL recreated after
        // the first Pump buy + idempotent create before the final Pump sell.
        assert_eq!(plan.counts.idempotent_create_instructions, 5);
        // Only the WSOL unwrap is an in-transaction close. Intermediate ATA
        // closes occur in the post-landing settlement transaction.
        assert_eq!(plan.counts.close_token_account_instructions, 1);
        assert_eq!(plan.intermediate_atas.len(), 2);
    }

    #[test]
    fn route_ata_plan_rejects_conflicting_token_program_for_same_mint() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let meme = Pubkey::new_unique();
        let first = SwapHop {
            pool_id: arb_types::PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::MeteoraDlmm,
            mint_in: arb_types::MintId(wsol),
            mint_out: arb_types::MintId(meme),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [
                arb_types::TokenProgramKind::SplToken,
                arb_types::TokenProgramKind::SplToken,
            ],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let second = SwapHop {
            pool_id: arb_types::PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::MeteoraDlmm,
            mint_in: arb_types::MintId(meme),
            mint_out: arb_types::MintId(usdc),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [
                arb_types::TokenProgramKind::Token2022,
                arb_types::TokenProgramKind::SplToken,
            ],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        assert!(route_ata_instruction_counts(&[first.clone(), second.clone()]).is_err());
        assert!(build_route_ata_plan(Pubkey::new_unique(), &[first, second]).is_err());
    }

    #[test]
    fn ata_derivation_is_token_program_specific() {
        let wallet = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
        let spl_ata = get_associated_token_address_with_program_id(&wallet, &mint, &spl);
        let token_2022_ata =
            get_associated_token_address_with_program_id(&wallet, &mint, &token_2022);

        assert_eq!(get_associated_token_address(&wallet, &mint), spl_ata);
        assert_ne!(spl_ata, token_2022_ata);
    }
}
