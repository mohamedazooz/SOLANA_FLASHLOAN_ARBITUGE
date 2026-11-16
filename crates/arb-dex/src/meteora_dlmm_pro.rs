//! Meteora DLMM Pro AMM Pool Adapter.
//!
//! Program: `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n`
//! Implements continuous tick concentrated liquidity with resting limit-order taker matching.

use crate::traits::AmmPool;
use arb_constants::METEORA_DLMM_PRO_PROGRAM_ID;
use arb_math::dlmm_pro::{DlmmProPoolState, quote_dlmm_pro_exact_in};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const METEORA_DLMM_PRO_POOL_MIN_SIZE: usize = 256;

#[derive(Debug, Clone)]
pub struct MeteoraDlmmProPool {
    pub pool_id: PoolId,
    pub pool_authority: Pubkey,
    pub token_x_mint: MintId,
    pub token_y_mint: MintId,
    pub token_x_vault: Pubkey,
    pub token_y_vault: Pubkey,
    pub oracle: Pubkey,
    pub event_authority: Pubkey,
    pub token_x_program: TokenProgramKind,
    pub token_y_program: TokenProgramKind,
    pub state: DlmmProPoolState,
    pub tick_arrays: Vec<Pubkey>,
    pub decimals_x: u8,
    pub decimals_y: u8,
}

impl MeteoraDlmmProPool {
    pub fn new(
        pool_id: PoolId,
        token_x_mint: MintId,
        token_y_mint: MintId,
        token_x_vault: Pubkey,
        token_y_vault: Pubkey,
        oracle: Pubkey,
        state: DlmmProPoolState,
        tick_arrays: Vec<Pubkey>,
    ) -> Self {
        let prog = Pubkey::from_str(METEORA_DLMM_PRO_PROGRAM_ID).unwrap_or_default();
        let (pool_authority, _) =
            Pubkey::find_program_address(&[b"pool_authority", pool_id.0.as_ref()], &prog);
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &prog);
        Self {
            pool_id,
            pool_authority,
            token_x_mint,
            token_y_mint,
            token_x_vault,
            token_y_vault,
            oracle,
            event_authority,
            token_x_program: TokenProgramKind::SplToken,
            token_y_program: TokenProgramKind::SplToken,
            state,
            tick_arrays,
            decimals_x: 6,
            decimals_y: 6,
        }
    }
}

impl AmmPool for MeteoraDlmmProPool {
    fn protocol(&self) -> DexProtocol {
        DexProtocol::MeteoraDlmmPro
    }

    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn base_mint(&self) -> MintId {
        self.token_x_mint
    }

    fn quote_mint(&self) -> MintId {
        self.token_y_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (self.state.active_liquidity, self.state.active_liquidity)
    }

    fn fee_bps(&self) -> Option<u16> {
        Some(self.state.fee_bps)
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if amount_in == 0 {
            return Ok(0);
        }
        quote_dlmm_pro_exact_in(&self.state, amount_in, a_to_b)
            .map(|(out, _fee, _resting)| out)
            .map_err(|e| ArbError::InvalidInput(e.to_string()))
    }

    fn has_risky_token(&self) -> bool {
        false
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.token_x_vault == Pubkey::default() || self.token_y_vault == Pubkey::default() {
            return None;
        }
        Some(PoolExecutionAccounts::MeteoraDlmmPro {
            pool_authority: self.pool_authority,
            vault_x: self.token_x_vault,
            vault_y: self.token_y_vault,
            tick_arrays: self.tick_arrays.clone(),
            oracle: self.oracle,
            event_authority: self.event_authority,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut deps = vec![
            self.token_x_vault,
            self.token_y_vault,
            self.token_x_mint.0,
            self.token_y_mint.0,
            self.oracle,
        ];
        deps.extend_from_slice(&self.tick_arrays);
        deps
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < METEORA_DLMM_PRO_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Meteora DLMM Pro pool len {} < {}",
                data.len(),
                METEORA_DLMM_PRO_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        self.token_x_mint = MintId(read_pk(8)?);
        self.token_y_mint = MintId(read_pk(40)?);
        self.token_x_vault = read_pk(72)?;
        self.token_y_vault = read_pk(104)?;
        self.oracle = read_pk(136)?;
        self.state.tick_spacing = u16::from_le_bytes(data[168..170].try_into().unwrap());
        self.state.fee_bps = u16::from_le_bytes(data[170..172].try_into().unwrap());
        self.state.current_tick = i32::from_le_bytes(data[172..176].try_into().unwrap());
        self.state.current_sqrt_price_x64 = u128::from_le_bytes(data[176..192].try_into().unwrap());
        self.state.active_liquidity = u128::from_le_bytes(data[192..208].try_into().unwrap());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_math::clmm::tick_to_sqrt_price_x64;

    #[test]
    fn test_dlmm_pro_pool_quote() {
        let pool_id = PoolId(Pubkey::new_unique());
        let mint_x = MintId(Pubkey::new_unique());
        let mint_y = MintId(Pubkey::new_unique());
        let vault_x = Pubkey::new_unique();
        let vault_y = Pubkey::new_unique();
        let oracle = Pubkey::new_unique();

        let sqrt_p_0 = tick_to_sqrt_price_x64(0).unwrap();
        let state = DlmmProPoolState::new(0, sqrt_p_0, 10_000_000_000, 60, 25);
        let pool = MeteoraDlmmProPool::new(
            pool_id,
            mint_x,
            mint_y,
            vault_x,
            vault_y,
            oracle,
            state,
            vec![],
        );

        let out = pool.quote(1_000_000, true).expect("quote works");
        assert!(out > 0);
        assert_eq!(pool.protocol(), DexProtocol::MeteoraDlmmPro);
    }
}
