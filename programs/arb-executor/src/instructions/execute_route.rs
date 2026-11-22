//! Multi-leg generic route execution instruction implementation (`execute_route`).
//!
//! Supports 2-leg and 3-leg arbitrary cross-DEX paths with zero-copy POD descriptors:
//! - Hop 1: Token X -> Token Y (e.g. WSOL -> Meme A)
//! - Hop 2: Token Y -> Token Z (e.g. Meme A -> WSOL or Meme A -> Meme B)
//! - Optional Hop 3: Token Z -> Token X (e.g. Meme B -> WSOL)
//!
//! Enforces:
//! - Hop count must be 2 or 3
//! - Sequential solvency between hops: `hops[i].min_amount_out >= hops[i+1].amount_in`
//! - Program ID allowlisting for every hop via compile-time constants
//! - Signer verification on payer (`is_signer == true`)
//! - Writable privilege verification on settlement and intermediate accounts
//! - Distinct pools verification across all hops (`p1 != p2`, `p2 != p3`, `p1 != p3`)
//! - Final balance delta check: `final_balance - initial_balance >= min_final_delta`

use crate::cpi::{
    DlmmSwapAccounts, invoke_meteora_swap2, invoke_orca_whirlpool_swap, invoke_pump_swap,
    invoke_raydium_clmm_swap, invoke_raydium_cpmm_swap,
};
use crate::error::ExecutorError;
use crate::validation::{
    DexProtocolTag, read_token_account_balance, validate_account_signer, validate_account_writable,
    validate_dex_program, validate_distinct_pools, validate_distinct_pools_2leg,
};
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_program_error::{ProgramError, ProgramResult};

/// Minimum and maximum sqrt price limits in Q64.64.
pub const CLMM_MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const CLMM_MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_279_992_447_579_055;

/// POD descriptor for a single swap leg within a multi-leg route.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HopDescriptor {
    /// DEX Protocol Tag (0: Dlmm, 1: RaydiumCpmm, 2: RaydiumClmm, 3: OrcaWhirlpool, 4: PumpSwap).
    pub protocol: u8,
    /// Protocol-specific flags (e.g., bit 0 = a_to_b / is_buy, bit 1 = is_base_input).
    pub flags: u8,
    /// Number of AccountInfos allocated to this hop.
    pub account_count: u8,
    /// Explicit padding for 8-byte alignment of subsequent u64 fields.
    pub _reserved: [u8; 5],
    /// Exact amount in for this hop.
    pub amount_in: u64,
    /// Minimum acceptable amount out from this hop.
    pub min_amount_out: u64,
}

impl HopDescriptor {
    /// Safe unaligned deserialization from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ExecutorError> {
        if bytes.len() < 24 {
            return Err(ExecutorError::InvalidInstructionData);
        }
        let protocol = bytes[0];
        let flags = bytes[1];
        let account_count = bytes[2];
        let mut reserved = [0u8; 5];
        reserved.copy_from_slice(&bytes[3..8]);
        let amount_in = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| ExecutorError::InvalidInstructionData)?,
        );
        let min_amount_out = u64::from_le_bytes(
            bytes[16..24]
                .try_into()
                .map_err(|_| ExecutorError::InvalidInstructionData)?,
        );
        Ok(Self {
            protocol,
            flags,
            account_count,
            _reserved: reserved,
            amount_in,
            min_amount_out,
        })
    }
}

/// POD arguments for the `execute_route` instruction.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExecuteRouteArgs {
    /// Number of hops in this route (must be 2 or 3).
    pub hop_count: u8,
    /// Explicit padding for 8-byte alignment.
    pub _reserved: [u8; 7],
    /// Minimum required final net balance increase in the settlement token account.
    pub min_final_delta: u64,
    /// Fixed-size array for up to 3 hops.
    pub hops: [HopDescriptor; 3],
}

impl ExecuteRouteArgs {
    /// Safe unaligned deserialization from a byte slice.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ExecutorError> {
        if bytes.len() < 88 {
            return Err(ExecutorError::InvalidInstructionData);
        }
        let hop_count = bytes[0];
        let mut reserved = [0u8; 7];
        reserved.copy_from_slice(&bytes[1..8]);
        let min_final_delta = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| ExecutorError::InvalidInstructionData)?,
        );
        let hop0 = HopDescriptor::from_slice(&bytes[16..40])?;
        let hop1 = HopDescriptor::from_slice(&bytes[40..64])?;
        let hop2 = HopDescriptor::from_slice(&bytes[64..88])?;
        Ok(Self {
            hop_count,
            _reserved: reserved,
            min_final_delta,
            hops: [hop0, hop1, hop2],
        })
    }
}

/// Extract the canonical pool address from a hop's account slice to verify pool distinctness.
fn extract_pool_address(
    tag: DexProtocolTag,
    hop_accounts: &[AccountInfo],
) -> Result<Address, ProgramError> {
    match tag {
        DexProtocolTag::MeteoraDlmm => {
            if hop_accounts.is_empty() {
                return Err(ExecutorError::NotEnoughAccountInfos.into());
            }
            Ok(*hop_accounts[0].key) // lb_pair
        }
        DexProtocolTag::RaydiumCpmm => {
            if hop_accounts.len() < 4 {
                return Err(ExecutorError::NotEnoughAccountInfos.into());
            }
            Ok(*hop_accounts[3].key) // pool_state
        }
        DexProtocolTag::RaydiumClmm => {
            if hop_accounts.len() < 3 {
                return Err(ExecutorError::NotEnoughAccountInfos.into());
            }
            Ok(*hop_accounts[2].key) // pool_state
        }
        DexProtocolTag::OrcaWhirlpool => {
            if hop_accounts.len() < 3 {
                return Err(ExecutorError::NotEnoughAccountInfos.into());
            }
            Ok(*hop_accounts[2].key) // whirlpool
        }
        DexProtocolTag::PumpSwap => {
            if hop_accounts.is_empty() {
                return Err(ExecutorError::NotEnoughAccountInfos.into());
            }
            Ok(*hop_accounts[0].key) // pool / bonding curve
        }
    }
}

/// Execute a multi-leg route across allowlisted DEX protocols.
///
/// Header Accounts:
/// - `[0]` `user` (signer)
/// - `[1]` `settlement_account` (user WSOL / settlement ATA, writable)
/// - `[2]` `intermediate_account_1` (writable)
/// - `[3]` `intermediate_account_2` (writable, present only if hop_count == 3)
///
/// Followed by per-hop account slices as specified by `hop.account_count`.
pub fn process_execute_route<'a>(
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    let args = ExecuteRouteArgs::from_slice(instruction_data)?;

    // 1. Validate hop count (must be 2 or 3)
    if args.hop_count < 2 || args.hop_count > 3 {
        return Err(ExecutorError::InvalidHopCount.into());
    }

    // 2. Validate sequential solvency invariants
    if args.hops[0].min_amount_out < args.hops[1].amount_in {
        return Err(ExecutorError::Leg2InputExceedsLeg1Output.into());
    }
    if args.hop_count == 3 {
        if args.hops[1].min_amount_out < args.hops[2].amount_in {
            return Err(ExecutorError::Leg3InputExceedsLeg2Output.into());
        }
        if args.hops[2].min_amount_out < args.min_final_delta {
            return Err(ExecutorError::FinalWsolDeltaBelowMinimum.into());
        }
    } else if args.hops[1].min_amount_out < args.min_final_delta {
        return Err(ExecutorError::FinalWsolDeltaBelowMinimum.into());
    }

    // 3. Header accounts validation
    let header_len = if args.hop_count == 2 { 3 } else { 4 };
    if accounts.len() < header_len {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    let user = &accounts[0];
    validate_account_signer(user)?;

    let settlement_account = &accounts[1];
    validate_account_writable(settlement_account)?;

    let intermediate_1 = &accounts[2];
    validate_account_writable(intermediate_1)?;

    if args.hop_count == 3 {
        let intermediate_2 = &accounts[3];
        validate_account_writable(intermediate_2)?;
    }

    // 4. Measure initial settlement balance
    let initial_balance = read_token_account_balance(settlement_account)?;

    // 5. Unpack per-hop account slices, validate allowlisting & pool distinctness
    let mut offset = header_len;
    let mut pool_addresses = [Address::default(); 3];

    for (i, pool_addr) in pool_addresses
        .iter_mut()
        .enumerate()
        .take(args.hop_count as usize)
    {
        let hop = &args.hops[i];
        let count = hop.account_count as usize;
        if accounts.len() < offset + count {
            return Err(ExecutorError::NotEnoughAccountInfos.into());
        }
        let hop_accounts = &accounts[offset..offset + count];
        offset += count;

        let tag = DexProtocolTag::try_from(hop.protocol)?;
        *pool_addr = extract_pool_address(tag, hop_accounts)?;
    }

    // Enforce distinct pools
    if args.hop_count == 2 {
        validate_distinct_pools_2leg(&pool_addresses[0], &pool_addresses[1])?;
    } else {
        validate_distinct_pools(&pool_addresses[0], &pool_addresses[1], &pool_addresses[2])?;
    }

    // 6. Execute each hop via CPI
    offset = header_len;
    for i in 0..args.hop_count as usize {
        let hop = &args.hops[i];
        let count = hop.account_count as usize;
        let hop_accounts = &accounts[offset..offset + count];
        offset += count;

        let tag = DexProtocolTag::try_from(hop.protocol)?;

        match tag {
            DexProtocolTag::MeteoraDlmm => {
                if hop_accounts.len() < 17 {
                    return Err(ExecutorError::NotEnoughAccountInfos.into());
                }
                validate_dex_program(hop_accounts[15].key)?;
                let dlmm_accounts = DlmmSwapAccounts {
                    lb_pair: &hop_accounts[0],
                    bin_array_bitmap_extension: &hop_accounts[1],
                    reserve_x: &hop_accounts[2],
                    reserve_y: &hop_accounts[3],
                    user_token_in: &hop_accounts[4],
                    user_token_out: &hop_accounts[5],
                    token_x_mint: &hop_accounts[6],
                    token_y_mint: &hop_accounts[7],
                    oracle: &hop_accounts[8],
                    host_fee_in: &hop_accounts[9],
                    user: &hop_accounts[10],
                    token_x_program: &hop_accounts[11],
                    token_y_program: &hop_accounts[12],
                    memo_program: &hop_accounts[13],
                    event_authority: &hop_accounts[14],
                    dlmm_program: &hop_accounts[15],
                    bin_arrays: &hop_accounts[16..],
                };
                invoke_meteora_swap2(&dlmm_accounts, hop.amount_in, hop.min_amount_out)?;
            }
            DexProtocolTag::RaydiumCpmm => {
                if hop_accounts.len() < 14 {
                    return Err(ExecutorError::NotEnoughAccountInfos.into());
                }
                validate_dex_program(hop_accounts[13].key)?;
                invoke_raydium_cpmm_swap(hop_accounts, hop.amount_in, hop.min_amount_out)?;
            }
            DexProtocolTag::RaydiumClmm => {
                if hop_accounts.len() < 13 {
                    return Err(ExecutorError::NotEnoughAccountInfos.into());
                }
                validate_dex_program(hop_accounts[12].key)?;
                let a_to_b = (hop.flags & 1) != 0;
                let sqrt_price_limit = if a_to_b {
                    CLMM_MIN_SQRT_PRICE_X64
                } else {
                    CLMM_MAX_SQRT_PRICE_X64
                };
                invoke_raydium_clmm_swap(
                    hop_accounts,
                    hop.amount_in,
                    hop.min_amount_out,
                    sqrt_price_limit,
                    true,
                )?;
            }
            DexProtocolTag::OrcaWhirlpool => {
                if hop_accounts.len() < 12 {
                    return Err(ExecutorError::NotEnoughAccountInfos.into());
                }
                validate_dex_program(hop_accounts[11].key)?;
                let a_to_b = (hop.flags & 1) != 0;
                let sqrt_price_limit = if a_to_b {
                    CLMM_MIN_SQRT_PRICE_X64
                } else {
                    CLMM_MAX_SQRT_PRICE_X64
                };
                invoke_orca_whirlpool_swap(
                    hop_accounts,
                    hop.amount_in,
                    hop.min_amount_out,
                    sqrt_price_limit,
                    a_to_b,
                )?;
            }
            DexProtocolTag::PumpSwap => {
                if hop_accounts.len() < 2 {
                    return Err(ExecutorError::NotEnoughAccountInfos.into());
                }
                let prog = hop_accounts.last().unwrap();
                validate_dex_program(prog.key)?;
                let is_buy = (hop.flags & 1) != 0;
                invoke_pump_swap(hop_accounts, hop.amount_in, hop.min_amount_out, is_buy)?;
            }
        }
    }

    // 7. Post-execution settlement balance check
    let final_balance = read_token_account_balance(settlement_account)?;
    let delta = final_balance
        .checked_sub(initial_balance)
        .ok_or(ExecutorError::FinalWsolDeltaBelowMinimum)?;

    if delta < args.min_final_delta {
        return Err(ExecutorError::FinalWsolDeltaBelowMinimum.into());
    }

    Ok(())
}
