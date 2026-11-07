//! Unified AMM Trait for Solana Decentralized Exchanges.

use arb_types::{DexProtocol, MintId, PoolId, PoolKind, Result};

/// Unified interface for AMM pools across protocols (Raydium v4, Raydium CLMM, Orca Whirlpools).
pub trait AmmPool: Send + Sync {
    /// Canonical pool public key address.
    fn pool_id(&self) -> PoolId;

    /// Specific protocol identifier.
    fn protocol(&self) -> DexProtocol;

    /// Base token mint address.
    fn base_mint(&self) -> MintId;

    /// Quote token mint address.
    fn quote_mint(&self) -> MintId;

    /// Current vault reserves (reserve_in, reserve_out).
    fn reserves(&self) -> (u128, u128);

    /// Fee in basis points (e.g. 25 for 0.25%).
    /// Returns `None` if the pool has an invalid fee configuration.
    fn fee_bps(&self) -> Option<u16>;

    /// Pricing family: determines which curve the quote model must use.
    fn kind(&self) -> PoolKind {
        match self.protocol() {
            DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm | DexProtocol::PumpFun | DexProtocol::PumpSwap => {
                PoolKind::Cpmm
            }
            DexProtocol::RaydiumClmm | DexProtocol::OrcaWhirlpool => PoolKind::Clmm,
            DexProtocol::MeteoraDlmm => PoolKind::Dlmm,
        }
    }

    /// Concentrated-liquidity state, if this pool is a CLMM. `None` for CPMM/DLMM.
    /// Default implementation returns `None`; CLMM adapters override.
    fn clmm_state(&self) -> Option<arb_types::ClmmState> {
        let _ = self;
        None
    }

    /// Discrete-bin state, if this pool is a DLMM. `None` for CPMM/CLMM.
    /// Default implementation returns `None`; DLMM adapters override.
    fn dlmm_state(&self) -> Option<arb_types::DlmmState> {
        let _ = self;
        None
    }

    /// Pump.fun bonding curve state. `None` for CPMM/CLMM/DLMM.
    /// Default implementation returns `None`; PumpFunBondingCurve overrides.
    fn pump_fun_state(&self) -> Option<arb_types::PumpFunState> {
        let _ = self;
        None
    }

    /// Mint decimals `(decimals_a, decimals_b)` for `(base_mint, quote_mint)`.
    /// `0` means unknown; pools with unknown decimals must not be priced.
    /// Default implementation returns `(0, 0)`.
    fn decimals(&self) -> (u8, u8) {
        let _ = self;
        (0, 0)
    }

    /// True when either mint has a risky Token-2022 extension. Default `false`.
    /// Adapters that can inspect mint data should override.
    fn has_risky_token(&self) -> bool {
        let _ = self;
        false
    }

    /// Update internal pool state from raw Solana account data bytes.
    fn update_from_account(&mut self, data: &[u8]) -> Result<()>;

    /// Calculate expected output amount for a given input amount.
    /// `a_to_b`: true if swapping base_mint -> quote_mint; false if quote_mint -> base_mint.
    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64>;
}
