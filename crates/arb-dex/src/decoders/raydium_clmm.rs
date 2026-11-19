//! Raydium Concentrated Liquidity (CLMM) PoolState Account Decoder.
//! Program ID: `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`
//! Discriminator: `[247, 237, 227, 245, 215, 195, 222, 70]`

use arb_types::{ArbError, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

pub const RAYDIUM_CLMM_POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const RAYDIUM_CLMM_POOL_STATE_SIZE: usize = 353;

/// Decoded Raydium CLMM Pool State.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRaydiumClmmPool {
    pub pool_id: PoolId,
    pub amm_config: Pubkey,
    pub token_mint_0: MintId,
    pub token_mint_1: MintId,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub status: u8,
}

impl DecodedRaydiumClmmPool {
    pub fn decode(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        if data.len() < RAYDIUM_CLMM_POOL_STATE_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CLMM data too short: {} < {RAYDIUM_CLMM_POOL_STATE_SIZE}",
                data.len()
            )));
        }

        if data[0..8] != RAYDIUM_CLMM_POOL_STATE_DISCRIMINATOR {
            return Err(ArbError::DeserializationError(
                "Invalid Raydium CLMM PoolState discriminator".into(),
            ));
        }

        let amm_config = Pubkey::new_from_array(data[8..40].try_into().unwrap());
        let token_mint_0 = MintId(Pubkey::new_from_array(data[72..104].try_into().unwrap()));
        let token_mint_1 = MintId(Pubkey::new_from_array(data[104..136].try_into().unwrap()));
        let token_vault_0 = Pubkey::new_from_array(data[136..168].try_into().unwrap());
        let token_vault_1 = Pubkey::new_from_array(data[168..200].try_into().unwrap());
        let observation_key = Pubkey::new_from_array(data[200..232].try_into().unwrap());

        let mint_decimals_0 = data[232];
        let mint_decimals_1 = data[233];
        let tick_spacing = u16::from_le_bytes(data[234..236].try_into().unwrap());

        let liquidity = u128::from_le_bytes(data[236..252].try_into().unwrap());
        let sqrt_price_x64 = u128::from_le_bytes(data[252..268].try_into().unwrap());
        let tick_current = i32::from_le_bytes(data[268..272].try_into().unwrap());

        let status = data[352];

        Ok(Self {
            pool_id,
            amm_config,
            token_mint_0,
            token_mint_1,
            token_vault_0,
            token_vault_1,
            observation_key,
            mint_decimals_0,
            mint_decimals_1,
            tick_spacing,
            liquidity,
            sqrt_price_x64,
            tick_current,
            status,
        })
    }
}
