//! Meteora Dynamic Bonding Curve (DBC) AMM Pool Adapter.
//!
//! Program: `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`
//! Implements universal 16-point piecewise bonding curve math and graduation state.

use crate::traits::AmmPool;
use arb_constants::METEORA_DBC_PROGRAM_ID;
use arb_math::dbc::DbcCurve;
use arb_types::{ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const METEORA_DBC_POOL_MIN_SIZE: usize = 256;

#[derive(Debug, Clone)]
pub struct MeteoraDbcPool {
    pub pool_id: PoolId,
    pub config: Pubkey,
    pub authority: Pubkey,
    pub event_authority: Pubkey,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub curve: DbcCurve,
    pub fee_bps: u16,
    pub decimals_base: u8,
    pub decimals_quote: u8,
}

impl MeteoraDbcPool {
    pub fn new(
        pool_id: PoolId,
        config: Pubkey,
        base_mint: MintId,
        quote_mint: MintId,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        curve: DbcCurve,
        fee_bps: u16,
    ) -> Self {
        let prog = Pubkey::from_str(METEORA_DBC_PROGRAM_ID).unwrap_or_default();
        let (authority, _) =
            Pubkey::find_program_address(&[b"pool_authority", pool_id.0.as_ref()], &prog);
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &prog);
        Self {
            pool_id,
            config,
            authority,
            event_authority,
            base_mint,
            quote_mint,
            base_vault,
            quote_vault,
            curve,
            fee_bps,
            decimals_base: 6,
            decimals_quote: 9,
        }
    }
}

impl AmmPool for MeteoraDbcPool {
    fn protocol(&self) -> DexProtocol {
        DexProtocol::MeteoraDbc
    }

    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn base_mint(&self) -> MintId {
        self.base_mint
    }

    fn quote_mint(&self) -> MintId {
        self.quote_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (
            u128::from(self.curve.base_reserve),
            u128::from(self.curve.quote_reserve),
        )
    }

    fn fee_bps(&self) -> Option<u16> {
        Some(self.fee_bps)
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if amount_in == 0 {
            return Ok(0);
        }
        if self.curve.migrated {
            return Err(ArbError::InvalidInput(
                "DBC pool has already migrated to DAMM v2".into(),
            ));
        }
        // a_to_b = true means base -> quote (selling base token for quote token)
        // In DbcCurve::quote_exact_in:
        //   a_to_b = true is spending quote to buy base (is_buy)
        //   a_to_b = false is spending base to sell for quote
        let is_buy = !a_to_b;
        let (amount_out, _) = self
            .curve
            .quote_exact_in(amount_in, is_buy)
            .ok_or(ArbError::Overflow)?;
        Ok(amount_out)
    }

    fn has_risky_token(&self) -> bool {
        false
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.base_vault == Pubkey::default() || self.quote_vault == Pubkey::default() {
            return None;
        }
        Some(PoolExecutionAccounts::MeteoraDbc {
            config: self.config,
            authority: self.authority,
            vault_base: self.base_vault,
            vault_quote: self.quote_vault,
            event_authority: self.event_authority,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        vec![
            self.base_vault,
            self.quote_vault,
            self.base_mint.0,
            self.quote_mint.0,
        ]
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < METEORA_DBC_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Meteora DBC pool len {} < {}",
                data.len(),
                METEORA_DBC_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        self.config = read_pk(8)?;
        self.base_mint = MintId(read_pk(40)?);
        self.quote_mint = MintId(read_pk(72)?);
        self.base_vault = read_pk(104)?;
        self.quote_vault = read_pk(136)?;
        self.curve.base_reserve = u64::from_le_bytes(data[168..176].try_into().unwrap());
        self.curve.quote_reserve = u64::from_le_bytes(data[176..184].try_into().unwrap());
        self.curve.current_sqrt_price_x64 = u128::from_le_bytes(data[184..200].try_into().unwrap());
        self.curve.migration_quote_threshold =
            u64::from_le_bytes(data[200..208].try_into().unwrap());
        self.curve.migrated = data[208] == 1;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_math::dbc::{DbcCurveSegment, ONE_Q64};

    #[test]
    fn test_dbc_pool_quote() {
        let pool_id = PoolId(Pubkey::new_unique());
        let config = Pubkey::new_unique();
        let base_mint = MintId(Pubkey::new_unique());
        let quote_mint = MintId(Pubkey::new_unique());
        let base_vault = Pubkey::new_unique();
        let quote_vault = Pubkey::new_unique();

        let seg = DbcCurveSegment::new(ONE_Q64, ONE_Q64 * 2, 1_000_000_000);
        let curve = DbcCurve {
            current_sqrt_price_x64: ONE_Q64,
            segments: vec![seg],
            fee_numerator: 25_000_000,
            migration_quote_threshold: 500_000_000,
            quote_reserve: 100_000_000,
            base_reserve: 1_000_000_000,
            migrated: false,
        };

        let pool = MeteoraDbcPool::new(
            pool_id,
            config,
            base_mint,
            quote_mint,
            base_vault,
            quote_vault,
            curve,
            25,
        );

        // a_to_b = false is buying base with quote
        let out = pool.quote(10_000_000, false).expect("quote works");
        assert!(out > 0);
        assert_eq!(pool.protocol(), DexProtocol::MeteoraDbc);
    }
}
