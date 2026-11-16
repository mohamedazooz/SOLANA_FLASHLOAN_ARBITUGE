//! Unified per-hop swap instruction dispatch across all supported protocols.

use crate::*;

/// Build genuine DEX swap instruction based on protocol hop with complete accounts matrix.
pub fn build_swap_hop_instruction(
    hop: &SwapHop,
    user_authority: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let exec_accounts = hop.execution_accounts.as_ref();

    let token_program_in = match hop.token_programs[0] {
        arb_types::TokenProgramKind::Token2022 => {
            Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
        }
        arb_types::TokenProgramKind::SplToken => {
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
        }
    };
    let token_program_out = match hop.token_programs[1] {
        arb_types::TokenProgramKind::Token2022 => {
            Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
        }
        arb_types::TokenProgramKind::SplToken => {
            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
        }
    };

    match hop.protocol {
        DexProtocol::PumpSwap => {
            if let Some(PoolExecutionAccounts::PumpSwap {
                global_config,
                pool_base_vault,
                pool_quote_vault,
                protocol_fee_recipient,
                protocol_fee_recipient_ata,
                coin_creator_vault_authority,
                coin_creator_vault_ata,
                fee_config,
                pool_version,
            }) = exec_accounts
            {
                let wsol = Pubkey::from_str(WSOL_MINT).unwrap_or_default();
                let usdc = Pubkey::from_str(USDC_MINT).unwrap_or_default();
                let is_buy =
                    hop.mint_out.0 != wsol && (hop.mint_in.0 == wsol || hop.mint_in.0 == usdc);
                let (base_mint, quote_mint) = if is_buy {
                    (hop.mint_out.0, hop.mint_in.0)
                } else {
                    (hop.mint_in.0, hop.mint_out.0)
                };
                let (token_prog_base, token_prog_quote) = if is_buy {
                    (token_program_out, token_program_in)
                } else {
                    (token_program_in, token_program_out)
                };

                match pool_version {
                    arb_types::PumpSwapPoolVersion::V1Legacy => {
                        // Explicit backward-compatibility path for non-V2-compatible pools.
                        build_pump_swap_instruction(
                            hop.pool_id.0,
                            user_authority,
                            base_mint,
                            quote_mint,
                            *pool_base_vault,
                            *pool_quote_vault,
                            *global_config,
                            *protocol_fee_recipient,
                            *protocol_fee_recipient_ata,
                            *fee_config,
                            *coin_creator_vault_authority,
                            *coin_creator_vault_ata,
                            token_prog_base,
                            token_prog_quote,
                            amount_in,
                            min_amount_out,
                            is_buy,
                        )
                    }
                    arb_types::PumpSwapPoolVersion::V2 => {
                        // Canonical V2 path (17 accounts).
                        let event_authority = Pubkey::find_program_address(
                            &[b"__event_authority"],
                            &Pubkey::from_str(PUMP_SWAP_PROGRAM_ID).unwrap(),
                        )
                        .0;

                        let trade_mode = if is_buy {
                            PumpSwapTradeMode::BuyExactQuoteInV2
                        } else {
                            PumpSwapTradeMode::SellExactBaseInV2
                        };

                        build_pump_swap_instruction_v2(
                            hop.pool_id.0,
                            user_authority,
                            base_mint,
                            quote_mint,
                            *pool_base_vault,
                            *pool_quote_vault,
                            *global_config,
                            *protocol_fee_recipient,
                            *protocol_fee_recipient_ata,
                            *coin_creator_vault_authority,
                            *coin_creator_vault_ata,
                            token_prog_base,
                            token_prog_quote,
                            event_authority,
                            amount_in,
                            min_amount_out,
                            trade_mode,
                        )
                    }
                }
            } else {
                Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "PumpSwap requires execution_accounts from registry".into(),
                })
            }
        }
        DexProtocol::RaydiumV4 => {
            let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
            let user_source = get_associated_token_address_with_program_id(
                &user_authority,
                &hop.mint_in.0,
                &token_program,
            );
            let user_destination = get_associated_token_address_with_program_id(
                &user_authority,
                &hop.mint_out.0,
                &token_program,
            );

            if let Some(PoolExecutionAccounts::RaydiumV4 {
                amm_authority,
                open_orders,
                target_orders,
                base_vault,
                quote_vault,
                serum_program,
                serum_market,
                serum_bids,
                serum_asks,
                serum_event_queue,
                serum_coin_vault,
                serum_pc_vault,
                serum_vault_signer,
                swap_version,
            }) = exec_accounts
            {
                let accounts = RaydiumV4Accounts {
                    token_program,
                    amm_pool: hop.pool_id.0,
                    amm_authority: *amm_authority,
                    amm_open_orders: *open_orders,
                    amm_target_orders: *target_orders,
                    pool_coin_vault: *base_vault,
                    pool_pc_vault: *quote_vault,
                    serum_program: *serum_program,
                    serum_market: *serum_market,
                    serum_bids: *serum_bids,
                    serum_asks: *serum_asks,
                    serum_event_queue: *serum_event_queue,
                    serum_coin_vault: *serum_coin_vault,
                    serum_pc_vault: *serum_pc_vault,
                    serum_vault_signer: *serum_vault_signer,
                };
                build_raydium_v4_swap_instruction_versioned(
                    &accounts,
                    *swap_version,
                    user_authority,
                    user_source,
                    user_destination,
                    amount_in,
                    min_amount_out,
                )
            } else {
                // Fail-closed: previously a default/unknown pool_id silently
                // swapped against the hardcoded RAYDIUM_SOL_USDC_POOL with
                // fabricated accounts — executing against a pool the quote
                // never priced.
                Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Raydium V4 pool requires verified execution accounts from registry"
                        .into(),
                })
            }
        }
        DexProtocol::OrcaWhirlpool => {
            // Fail-closed: no hardcoded pool fallback. A default `pool_id` or
            // missing execution accounts previously swapped against the
            // hardcoded ORCA_SOL_USDC_POOL with fabricated accounts — a
            // transaction that could execute against a pool the quote never
            // priced.
            if hop.pool_id.0 == Pubkey::default() {
                return Err(TxError::InvalidExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Orca hop has default pool_id; refusing hardcoded-pool fallback".into(),
                });
            }

            let Some(PoolExecutionAccounts::OrcaWhirlpool {
                vault_a,
                vault_b,
                oracle,
                tick_array_pdas,
                pool_mint_a,
            }) = exec_accounts
            else {
                return Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Orca Whirlpool requires execution_accounts from registry".into(),
                });
            };

            // Direction from the pool's on-chain `token_mint_a` (@101..133),
            // NOT from a WSOL comparison: non-WSOL pools would always derive
            // `a_to_b = false`, silently inverting the swap direction.
            if *pool_mint_a == Pubkey::default() {
                return Err(TxError::InvalidExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Orca execution accounts missing pool_mint_a (on-chain @101..133)"
                        .into(),
                });
            }
            let a_to_b = hop.mint_in.0 == *pool_mint_a;

            if tick_array_pdas.is_empty() {
                return Err(TxError::InvalidExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason:
                        "Orca tick_array_pdas empty; zero-pubkey tick arrays would revert on-chain"
                            .into(),
                });
            }
            // Remaining tick arrays beyond index 0 are duplicate-filled:
            // Whirlpool ignores unused remaining tick-array accounts, and the
            // registry supplies the full traversal sequence for crossing swaps.
            let ta0 = tick_array_pdas[0];
            let ta1 = tick_array_pdas.get(1).copied().unwrap_or(ta0);
            let ta2 = tick_array_pdas.get(2).copied().unwrap_or(ta1);
            let accounts = OrcaWhirlpoolAccounts::with_tick_arrays(
                hop.pool_id.0,
                *vault_a,
                *vault_b,
                *oracle,
                ta0,
                ta1,
                ta2,
            );
            let is_adaptive_fee = false;

            let (token_owner_a, token_owner_b) = if a_to_b {
                (
                    get_associated_token_address(&user_authority, &hop.mint_in.0),
                    get_associated_token_address(&user_authority, &hop.mint_out.0),
                )
            } else {
                (
                    get_associated_token_address(&user_authority, &hop.mint_out.0),
                    get_associated_token_address(&user_authority, &hop.mint_in.0),
                )
            };
            build_orca_whirlpool_swap_instruction_11(
                &accounts,
                user_authority,
                token_owner_a,
                token_owner_b,
                amount_in,
                min_amount_out,
                a_to_b,
                is_adaptive_fee,
            )
        }
        DexProtocol::MeteoraDlmm => {
            if let Some(PoolExecutionAccounts::MeteoraDlmm {
                reserve_x,
                reserve_y,
                oracle,
                bin_array_pdas,
                bitmap_extension,
                pool_token_x,
                pool_token_y,
            }) = exec_accounts
            {
                // The DLMM program validates token_x_mint/token_y_mint against
                // the LbPair (official SDK passes pool.tokenX/tokenY in the
                // fixed x/y slots, never inToken/outToken). Passing mint_in as
                // token_x_mint fails on-chain for every Y->X hop.
                if *pool_token_x == Pubkey::default() || *pool_token_y == Pubkey::default() {
                    return Err(TxError::InvalidExecutionAccounts {
                        pool_id: hop.pool_id.0,
                        reason: "DLMM execution accounts missing pool_token_x/pool_token_y (on-chain @88..152)"
                            .into(),
                    });
                }
                let swap_for_y = hop.mint_in.0 == *pool_token_x;
                let mint_out_matches = if swap_for_y {
                    hop.mint_out.0 == *pool_token_y
                } else {
                    hop.mint_out.0 == *pool_token_x
                };
                if !mint_out_matches {
                    return Err(TxError::InvalidExecutionAccounts {
                        pool_id: hop.pool_id.0,
                        reason: "DLMM hop mints do not match the pool's on-chain token_x/token_y"
                            .into(),
                    });
                }
                // PR24 §5: `hop.token_programs` is (mint_in, mint_out) per the
                // SwapHop contract, but the swap2 instruction requires the
                // POOL's (token_x_program, token_y_program) in fixed slots.
                // Mapping [in,out] straight into [x,y] inverts the programs for
                // every Y->X hop — fatal for any Token-2022 side.
                let program_of = |kind: arb_types::TokenProgramKind| -> Pubkey {
                    match kind {
                        arb_types::TokenProgramKind::SplToken => {
                            Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
                        }
                        arb_types::TokenProgramKind::Token2022 => {
                            Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID)
                                .unwrap_or_default()
                        }
                    }
                };
                let in_program = program_of(hop.token_programs[0]);
                let out_program = program_of(hop.token_programs[1]);
                let (token_x_program, token_y_program) = if swap_for_y {
                    (in_program, out_program)
                } else {
                    (out_program, in_program)
                };
                let accounts = MeteoraDlmmAccounts {
                    lb_pair: hop.pool_id.0,
                    reserve_x: *reserve_x,
                    reserve_y: *reserve_y,
                    token_x_mint: *pool_token_x,
                    token_y_mint: *pool_token_y,
                    oracle: *oracle,
                    event_authority: derive_meteora_event_authority(),
                    program_id: Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap(),
                };
                Ok(build_meteora_dlmm_swap_instruction_with_bin_arrays(
                    &accounts,
                    user_authority,
                    get_associated_token_address(&user_authority, &hop.mint_in.0),
                    get_associated_token_address(&user_authority, &hop.mint_out.0),
                    token_x_program,
                    token_y_program,
                    amount_in,
                    min_amount_out,
                    bin_array_pdas,
                    *bitmap_extension,
                ))
            } else {
                Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Meteora DLMM requires execution_accounts from registry".into(),
                })
            }
        }
        DexProtocol::RaydiumCpmm => {
            let user_token_in = get_associated_token_address(&user_authority, &hop.mint_in.0);
            let user_token_out = get_associated_token_address(&user_authority, &hop.mint_out.0);

            let accounts = if let Some(PoolExecutionAccounts::RaydiumCpmm {
                amm_config,
                authority: _,
                vault_0,
                vault_1,
                observation_state: _,
            }) = exec_accounts
            {
                // VERIFIED (Raydium official docs + raydium-cp-swap source):
                // CPMM pool creation REQUIRES token_0_mint < token_1_mint by
                // byte order ("InvalidInput — Mints unsorted or identical"),
                // so lexicographic direction is provably correct here — unlike
                // CLMM/Orca, which do NOT enforce ordering on live pools.
                let a_to_b = hop.mint_in.0.to_bytes() < hop.mint_out.0.to_bytes();
                let (input_vault, output_vault) = if a_to_b {
                    (*vault_0, *vault_1)
                } else {
                    (*vault_1, *vault_0)
                };
                let mut accs = RaydiumCpmmAccounts::for_pool(
                    hop.pool_id.0,
                    user_authority,
                    user_token_in,
                    user_token_out,
                    hop.mint_in.0,
                    hop.mint_out.0,
                    input_vault,
                    output_vault,
                    *amm_config,
                );
                accs.input_token_program = token_program_in;
                accs.output_token_program = token_program_out;
                accs
            } else {
                return Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Raydium CPMM requires execution_accounts from registry".into(),
                });
            };
            build_raydium_cpmm_swap_instruction(&accounts, amount_in, min_amount_out)
        }
        DexProtocol::RaydiumClmm => {
            let pool = hop.pool_id.0;
            let user_token_in = get_associated_token_address(&user_authority, &hop.mint_in.0);
            let user_token_out = get_associated_token_address(&user_authority, &hop.mint_out.0);

            let Some(PoolExecutionAccounts::RaydiumClmm {
                amm_config,
                vault_0,
                vault_1,
                observation_state,
                tick_array_pdas,
                bitmap_extension: _,
                pool_mint_0,
            }) = exec_accounts
            else {
                return Err(TxError::MissingExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Raydium CLMM requires execution_accounts from registry".into(),
                });
            };

            // Direction from the pool's on-chain `token_mint_0` (@73..105).
            // The program determines zero_for_one by matching the input vault's
            // mint against token_mint_0 (raydium-clmm swap.rs); it does NOT
            // enforce mint_0 < mint_1 on live pools (probe-verified), so the
            // previous lexicographic rule inverted direction on such pools.
            if *pool_mint_0 == Pubkey::default() {
                return Err(TxError::InvalidExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason:
                        "Raydium CLMM execution accounts missing pool_mint_0 (on-chain @73..105)"
                            .into(),
                });
            }
            let zero_for_one = hop.mint_in.0 == *pool_mint_0;

            if tick_array_pdas.is_empty() {
                return Err(TxError::InvalidExecutionAccounts {
                    pool_id: hop.pool_id.0,
                    reason: "Raydium CLMM tick_array_pdas empty; zero-pubkey tick arrays would revert on-chain"
                        .into(),
                });
            }
            let accounts = {
                let (input_vault, output_vault) = if zero_for_one {
                    (*vault_0, *vault_1)
                } else {
                    (*vault_1, *vault_0)
                };
                let ta0 = tick_array_pdas[0];
                let ta1 = tick_array_pdas.get(1).copied().unwrap_or(ta0);
                let ta2 = tick_array_pdas.get(2).copied().unwrap_or(ta1);
                RaydiumClmmAccounts::with_tick_arrays(
                    pool,
                    user_authority,
                    user_token_in,
                    user_token_out,
                    input_vault,
                    output_vault,
                    *amm_config,
                    *observation_state,
                    ta0,
                    ta1,
                    ta2,
                )
            };

            const Q64: u128 = 1u128 << 64;
            let sqrt_price_limit = if amount_in > 0 && min_amount_out > 0 {
                if zero_for_one {
                    (u128::from(min_amount_out) * Q64) / u128::from(amount_in)
                } else {
                    (u128::from(amount_in) * Q64) / u128::from(min_amount_out)
                }
            } else if zero_for_one {
                4_295_048_016u128
            } else {
                79_226_673_515_401_499_993_453_864_427u128
            };
            build_raydium_clmm_swap_instruction(
                &accounts,
                amount_in,
                min_amount_out,
                sqrt_price_limit,
                true,
            )
        }
        DexProtocol::PumpFun => {
            let wsol = Pubkey::from_str(WSOL_MINT).unwrap_or_default();
            let is_buy = hop.mint_in.0 == wsol;
            let meme_mint = if is_buy {
                hop.mint_out.0
            } else {
                hop.mint_in.0
            };
            let accounts = PumpFunAccounts::for_mint(meme_mint, user_authority);
            build_pump_fun_swap_instruction(&accounts, amount_in, min_amount_out, is_buy)
        }
        DexProtocol::MeteoraDammV2 => {
            if let Some(arb_types::PoolExecutionAccounts::MeteoraDammV2 {
                config,
                authority,
                vault_a,
                vault_b,
                event_authority,
            }) = hop.execution_accounts
            {
                let program_id = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID)
                    .map_err(|e| TxError::Serialization(e.to_string()))?;
                let user_source = get_associated_token_address(&user_authority, &hop.mint_in.0);
                let user_destination =
                    get_associated_token_address(&user_authority, &hop.mint_out.0);
                let a_to_b = hop.mint_in.0.to_bytes() < hop.mint_out.0.to_bytes();
                let (input_vault, output_vault) = if a_to_b {
                    (vault_a, vault_b)
                } else {
                    (vault_b, vault_a)
                };
                let token_program_in = match hop.token_programs[0] {
                    arb_types::TokenProgramKind::Token2022 => {
                        Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
                    }
                    arb_types::TokenProgramKind::SplToken => {
                        Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
                    }
                };
                let token_program_out = match hop.token_programs[1] {
                    arb_types::TokenProgramKind::Token2022 => {
                        Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
                    }
                    arb_types::TokenProgramKind::SplToken => {
                        Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
                    }
                };

                let mut data = Vec::with_capacity(24);
                data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]); // swap discriminator
                data.extend_from_slice(&amount_in.to_le_bytes());
                data.extend_from_slice(&min_amount_out.to_le_bytes());

                let accounts_meta = vec![
                    AccountMeta::new_readonly(config, false),
                    AccountMeta::new(hop.pool_id.0, false),
                    AccountMeta::new_readonly(authority, false),
                    AccountMeta::new(user_source, false),
                    AccountMeta::new(user_destination, false),
                    AccountMeta::new(input_vault, false),
                    AccountMeta::new(output_vault, false),
                    AccountMeta::new_readonly(hop.mint_in.0, false),
                    AccountMeta::new_readonly(hop.mint_out.0, false),
                    AccountMeta::new_readonly(user_authority, true),
                    AccountMeta::new_readonly(token_program_in, false),
                    AccountMeta::new_readonly(token_program_out, false),
                    AccountMeta::new_readonly(event_authority, false),
                    AccountMeta::new_readonly(program_id, false),
                ];
                Ok(CompiledInstruction {
                    program_id,
                    accounts: accounts_meta,
                    data,
                })
            } else {
                Err(TxError::Serialization(
                    "Meteora DAMM v2 missing execution accounts".into(),
                ))
            }
        }
        DexProtocol::RaydiumLaunchLab => {
            if let Some(arb_types::PoolExecutionAccounts::RaydiumLaunchLab {
                authority,
                global_config,
                platform_config,
                base_vault,
                quote_vault,
                event_authority,
            }) = hop.execution_accounts
            {
                let program_id = Pubkey::from_str(arb_constants::RAYDIUM_LAUNCHLAB_PROGRAM_ID)
                    .map_err(|e| TxError::Serialization(e.to_string()))?;
                let user_source = get_associated_token_address(&user_authority, &hop.mint_in.0);
                let user_destination =
                    get_associated_token_address(&user_authority, &hop.mint_out.0);
                let is_buy = hop.mint_in.0 == Pubkey::from_str(WSOL_MINT).unwrap_or(hop.mint_in.0);

                let mut data = Vec::with_capacity(24);
                let disc = if is_buy {
                    [102, 6, 61, 18, 1, 218, 235, 234] // buy exact in
                } else {
                    [51, 230, 133, 164, 1, 127, 131, 173] // sell exact in
                };
                data.extend_from_slice(&disc);
                data.extend_from_slice(&amount_in.to_le_bytes());
                data.extend_from_slice(&min_amount_out.to_le_bytes());

                let token_program_in = match hop.token_programs[0] {
                    arb_types::TokenProgramKind::Token2022 => {
                        Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
                    }
                    arb_types::TokenProgramKind::SplToken => {
                        Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
                    }
                };
                let token_program_out = match hop.token_programs[1] {
                    arb_types::TokenProgramKind::Token2022 => {
                        Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default()
                    }
                    arb_types::TokenProgramKind::SplToken => {
                        Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default()
                    }
                };

                let accounts_meta = vec![
                    AccountMeta::new_readonly(user_authority, true),
                    AccountMeta::new_readonly(authority, false),
                    AccountMeta::new_readonly(global_config, false),
                    AccountMeta::new_readonly(platform_config, false),
                    AccountMeta::new(hop.pool_id.0, false),
                    AccountMeta::new(user_source, false),
                    AccountMeta::new(user_destination, false),
                    AccountMeta::new(base_vault, false),
                    AccountMeta::new(quote_vault, false),
                    AccountMeta::new_readonly(hop.mint_in.0, false),
                    AccountMeta::new_readonly(hop.mint_out.0, false),
                    AccountMeta::new_readonly(token_program_in, false),
                    AccountMeta::new_readonly(token_program_out, false),
                    AccountMeta::new_readonly(event_authority, false),
                    AccountMeta::new_readonly(program_id, false),
                ];
                Ok(CompiledInstruction {
                    program_id,
                    accounts: accounts_meta,
                    data,
                })
            } else {
                Err(TxError::Serialization(
                    "Raydium LaunchLab missing execution accounts".into(),
                ))
            }
        }
    }
}
