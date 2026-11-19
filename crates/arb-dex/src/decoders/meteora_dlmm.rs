//! Meteora DLMM (lb_clmm) Account State Decoder.
//! Program ID: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
//! Discriminator: `[33, 110, 107, 169, 184, 112, 59, 189]`

use arb_math::dlmm::{DlmmParameters, DlmmVariableParameters};
use arb_types::{ArbError, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

pub const LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 110, 107, 169, 184, 112, 59, 189];
pub const LB_PAIR_MIN_SIZE: usize = 370;

/// Decoded LbPair State.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedLbPair {
    pub pool_id: PoolId,
    pub token_x_mint: MintId,
    pub token_y_mint: MintId,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
    pub active_id: i32,
    pub bin_step: u16,
    pub base_factor: u16,
    pub protocol_share: u16,
    pub volatility_accumulator: u32,
    pub parameters: DlmmParameters,
    pub v_parameters: DlmmVariableParameters,
}

impl DecodedLbPair {
    /// Decode LbPair account data bytes.
    pub fn decode(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        if data.len() < LB_PAIR_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "LbPair data too short: {} < {LB_PAIR_MIN_SIZE}",
                data.len()
            )));
        }

        if data[0..8] != LB_PAIR_DISCRIMINATOR {
            return Err(ArbError::DeserializationError(
                "Invalid Meteora DLMM LbPair discriminator".into(),
            ));
        }

        let bin_step = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let base_factor = u16::from_le_bytes(data[10..12].try_into().unwrap());
        let filter_period = u16::from_le_bytes(data[12..14].try_into().unwrap());
        let decay_period = u16::from_le_bytes(data[14..16].try_into().unwrap());
        let reduction_factor = u16::from_le_bytes(data[16..18].try_into().unwrap());
        let variable_fee_control = u32::from_le_bytes(data[18..22].try_into().unwrap());
        let max_volatility_accumulator = u32::from_le_bytes(data[22..26].try_into().unwrap());
        let protocol_share = u16::from_le_bytes(data[34..36].try_into().unwrap());

        let volatility_accumulator = u32::from_le_bytes(data[42..46].try_into().unwrap());
        let volatility_reference = u32::from_le_bytes(data[46..50].try_into().unwrap());
        let index_reference = i32::from_le_bytes(data[50..54].try_into().unwrap());
        let last_update_timestamp = i64::from_le_bytes(data[58..66].try_into().unwrap());

        let active_id = i32::from_le_bytes(data[78..82].try_into().unwrap());

        let reserve_x = Pubkey::new_from_array(data[210..242].try_into().unwrap());
        let reserve_y = Pubkey::new_from_array(data[242..274].try_into().unwrap());
        let token_x_mint = MintId(Pubkey::new_from_array(data[274..306].try_into().unwrap()));
        let token_y_mint = MintId(Pubkey::new_from_array(data[306..338].try_into().unwrap()));
        let oracle = Pubkey::new_from_array(data[338..370].try_into().unwrap());

        let parameters = DlmmParameters {
            bin_step,
            base_factor,
            filter_period,
            decay_period,
            reduction_factor,
            variable_fee_control,
            max_volatility_accumulator,
            protocol_share,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
        };

        let v_parameters = DlmmVariableParameters {
            volatility_accumulator,
            volatility_reference,
            index_reference,
            last_update_timestamp,
        };

        Ok(Self {
            pool_id,
            token_x_mint,
            token_y_mint,
            reserve_x,
            reserve_y,
            oracle,
            active_id,
            bin_step,
            base_factor,
            protocol_share,
            volatility_accumulator,
            parameters,
            v_parameters,
        })
    }
}
