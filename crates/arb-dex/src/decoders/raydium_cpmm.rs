//! Raydium CPMM (CP-Swap) PoolState Account Decoder.
//! Program ID: `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`
//! Discriminator: `[247, 237, 227, 245, 215, 195, 222, 70]`

use arb_types::{ArbError, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

pub const RAYDIUM_CPMM_POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const RAYDIUM_CPMM_POOL_STATE_SIZE: usize = 637;

/// Decoded Raydium CPMM Pool State.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRaydiumCpmmPool {
    pub pool_id: PoolId,
    pub amm_config: Pubkey,
    pub pool_creator: Pubkey,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub lp_mint: Pubkey,
    pub token_0_mint: MintId,
    pub token_1_mint: MintId,
    pub token_0_program: Pubkey,
    pub token_1_program: Pubkey,
    pub observation_key: Pubkey,
    pub status: u8,
    pub mint_0_decimals: u8,
    pub mint_1_decimals: u8,
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub creator_fees_token_0: u64,
    pub creator_fees_token_1: u64,
    pub creator_fee_on: u8,
    pub enable_creator_fee: bool,
}

impl DecodedRaydiumCpmmPool {
    pub fn decode(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        if data.len() < RAYDIUM_CPMM_POOL_STATE_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CPMM data too short: {} < {RAYDIUM_CPMM_POOL_STATE_SIZE}",
                data.len()
            )));
        }

        if data[0..8] != RAYDIUM_CPMM_POOL_STATE_DISCRIMINATOR {
            return Err(ArbError::DeserializationError(
                "Invalid Raydium CPMM PoolState discriminator".into(),
            ));
        }

        let read_key = |offset: usize| -> Result<Pubkey> {
            let key = Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap());
            if key == Pubkey::default() {
                return Err(ArbError::DeserializationError(format!(
                    "Raydium CPMM PoolState contains a default pubkey at offset {offset}"
                )));
            }
            Ok(key)
        };
        let amm_config = read_key(8)?;
        let pool_creator = read_key(40)?;
        let token_0_vault = read_key(72)?;
        let token_1_vault = read_key(104)?;
        let lp_mint = read_key(136)?;
        let token_0_mint = MintId(read_key(168)?);
        let token_1_mint = MintId(read_key(200)?);
        let token_0_program = read_key(232)?;
        let token_1_program = read_key(264)?;
        let spl_token = Pubkey::from_str_const(arb_constants::TOKEN_PROGRAM_ID);
        let token_2022 = Pubkey::from_str_const(arb_constants::TOKEN_2022_PROGRAM_ID);
        if ![spl_token, token_2022].contains(&token_0_program)
            || ![spl_token, token_2022].contains(&token_1_program)
        {
            return Err(ArbError::DeserializationError(
                "Raydium CPMM PoolState has unsupported token program id".into(),
            ));
        }
        let observation_key = read_key(296)?;
        let status = data[329];
        let mint_0_decimals = data[331];
        let mint_1_decimals = data[332];
        if mint_0_decimals > 18 || mint_1_decimals > 18 {
            return Err(ArbError::DeserializationError(
                "invalid Raydium CPMM mint decimals".into(),
            ));
        }
        let protocol_fees_token_0 = u64::from_le_bytes(data[341..349].try_into().unwrap());
        let protocol_fees_token_1 = u64::from_le_bytes(data[349..357].try_into().unwrap());
        let fund_fees_token_0 = u64::from_le_bytes(data[357..365].try_into().unwrap());
        let fund_fees_token_1 = u64::from_le_bytes(data[365..373].try_into().unwrap());
        let creator_fee_on = data[389];
        let enable_creator_fee = match data[390] {
            0 => false,
            1 => true,
            _ => {
                return Err(ArbError::DeserializationError(
                    "invalid enable_creator_fee flag".into(),
                ));
            }
        };
        if creator_fee_on > 2 {
            return Err(ArbError::DeserializationError(
                "invalid creator_fee_on mode".into(),
            ));
        }
        let creator_fees_token_0 = u64::from_le_bytes(data[397..405].try_into().unwrap());
        let creator_fees_token_1 = u64::from_le_bytes(data[405..413].try_into().unwrap());
        Ok(Self {
            pool_id,
            amm_config,
            pool_creator,
            token_0_vault,
            token_1_vault,
            lp_mint,
            token_0_mint,
            token_1_mint,
            token_0_program,
            token_1_program,
            observation_key,
            status,
            mint_0_decimals,
            mint_1_decimals,
            protocol_fees_token_0,
            protocol_fees_token_1,
            fund_fees_token_0,
            fund_fees_token_1,
            creator_fees_token_0,
            creator_fees_token_1,
            creator_fee_on,
            enable_creator_fee,
        })
    }
}
