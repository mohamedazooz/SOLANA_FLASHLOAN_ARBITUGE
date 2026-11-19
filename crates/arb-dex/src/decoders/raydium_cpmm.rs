//! Raydium CPMM (CP-Swap) PoolState Account Decoder.
//! Program ID: `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`
//! Discriminator: `[247, 237, 227, 245, 215, 195, 222, 70]`

use arb_types::{ArbError, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

pub const RAYDIUM_CPMM_POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const RAYDIUM_CPMM_POOL_STATE_SIZE: usize = 381;

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

        let amm_config = Pubkey::new_from_array(data[8..40].try_into().unwrap());
        let pool_creator = Pubkey::new_from_array(data[40..72].try_into().unwrap());
        let token_0_vault = Pubkey::new_from_array(data[72..104].try_into().unwrap());
        let token_1_vault = Pubkey::new_from_array(data[104..136].try_into().unwrap());
        let lp_mint = Pubkey::new_from_array(data[136..168].try_into().unwrap());
        let token_0_mint = MintId(Pubkey::new_from_array(data[168..200].try_into().unwrap()));
        let token_1_mint = MintId(Pubkey::new_from_array(data[200..232].try_into().unwrap()));
        let token_0_program = Pubkey::new_from_array(data[232..264].try_into().unwrap());
        let token_1_program = Pubkey::new_from_array(data[264..296].try_into().unwrap());
        let observation_key = Pubkey::new_from_array(data[296..328].try_into().unwrap());

        let status = data[329];
        let mint_0_decimals = data[331];
        let mint_1_decimals = data[332];

        let protocol_fees_token_0 = u64::from_le_bytes(data[341..349].try_into().unwrap());
        let protocol_fees_token_1 = u64::from_le_bytes(data[349..357].try_into().unwrap());

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
        })
    }
}
