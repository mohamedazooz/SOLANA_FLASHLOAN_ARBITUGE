//! Meteora DLMM Pro metadata adapter.
//!
//! This is deliberately NOT a quote engine. DLMM Pro uses dynamic-tail Pool,
//! TickArray, Oracle, PositionArray and LimitOrder accounts. Until the official
//! Meteora `dlmm-pro-commons` parser and quote path are wired to complete,
//! slot-consistent snapshots, all quotes and execution-account requests fail closed.

use crate::traits::AmmPool;
use arb_types::{ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result};
use solana_pubkey::Pubkey;

#[derive(Debug, Clone)]
pub struct MeteoraDlmmProPool {
    pub pool_id: PoolId,
    pub token_x_mint: MintId,
    pub token_y_mint: MintId,
    pub token_x_vault: Pubkey,
    pub token_y_vault: Pubkey,
    pub oracle: Pubkey,
}

impl MeteoraDlmmProPool {
    /// Construct a metadata-only adapter. No decimals, token programs, reserves,
    /// fee schedule or tick state are inferred from pool addresses.
    pub fn new_unverified(
        pool_id: PoolId,
        token_x_mint: MintId,
        token_y_mint: MintId,
        token_x_vault: Pubkey,
        token_y_vault: Pubkey,
        oracle: Pubkey,
    ) -> Self {
        Self {
            pool_id,
            token_x_mint,
            token_y_mint,
            token_x_vault,
            token_y_vault,
            oracle,
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

    /// Reserves are unknown until the official dynamic Pool/TickArray snapshot is decoded.
    fn reserves(&self) -> (u128, u128) {
        (0, 0)
    }

    fn fee_bps(&self) -> Option<u16> {
        None
    }

    fn quote(&self, _amount_in: u64, _a_to_b: bool) -> Result<u64> {
        Err(ArbError::UnsupportedOperation(
            "Meteora DLMM Pro quote requires official DynamicPool/DynamicTickArray parsing, bitmap extension, oracle, limit-order state, mint extensions and Clock snapshot".into(),
        ))
    }

    /// Unknown mint extensions/token programs are treated as risky until verified.
    fn has_risky_token(&self) -> bool {
        true
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        None
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        vec![
            self.token_x_mint.0,
            self.token_y_mint.0,
            self.token_x_vault,
            self.token_y_vault,
            self.oracle,
        ]
        .into_iter()
        .filter(|key| *key != Pubkey::default())
        .collect()
    }

    /// Fixed offsets are intentionally forbidden: Pool has extension-gated dynamic tails.
    fn update_from_account(&mut self, _data: &[u8]) -> Result<()> {
        Err(ArbError::UnsupportedOperation(
            "DLMM Pro account decoding must use Meteora's official DynamicPool parser; fixed-offset decoding is disabled".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlmm_pro_metadata_adapter_never_invents_reserves_or_execution_readiness() {
        let mut pool = MeteoraDlmmProPool::new_unverified(
            PoolId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );

        assert_eq!(pool.reserves(), (0, 0));
        assert!(pool.fee_bps().is_none());
        assert!(pool.has_risky_token());
        assert!(pool.execution_accounts().is_none());
        assert!(matches!(
            pool.quote(1_000_000, true),
            Err(ArbError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            pool.update_from_account(&vec![0; 1024]),
            Err(ArbError::UnsupportedOperation(_))
        ));
    }
}
