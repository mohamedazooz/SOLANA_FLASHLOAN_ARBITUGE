//! Unified AMM Trait for Solana Decentralized Exchanges.

use arb_types::{
    DexProtocol, MintId, PoolExecutionAccounts, PoolId, PoolKind, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;

/// Unified interface for AMM pools across all 9 supported Solana DEX protocols:
/// Raydium V4, Raydium CPMM, Raydium CLMM, Orca Whirlpool, Meteora DLMM,
/// Meteora DAMM v2, Raydium LaunchLab, Pump.fun, and PumpSwap.
pub trait AmmPool: Send + Sync {
    /// Canonical pool public key address.
    fn pool_id(&self) -> PoolId;

    /// Specific protocol identifier.
    fn protocol(&self) -> DexProtocol;

    /// Base token mint address.
    fn base_mint(&self) -> MintId;

    /// Quote token mint address.
    fn quote_mint(&self) -> MintId;

    /// Current vault or effective reserves `(reserve_in, reserve_out)`.
    fn reserves(&self) -> (u128, u128);

    /// Fee in basis points (e.g. 25 for 0.25%).
    /// Returns `None` if the pool has an invalid or unloaded fee configuration.
    fn fee_bps(&self) -> Option<u16>;

    /// Pricing family: determines which curve the quote model must use.
    fn kind(&self) -> PoolKind {
        self.protocol().family()
    }

    /// Concentrated-liquidity state, if this pool is a CLMM. `None` for CPMM/DLMM.
    fn clmm_state(&self) -> Option<arb_types::ClmmState> {
        let _ = self;
        None
    }

    /// Discrete-bin state, if this pool is a DLMM. `None` for CPMM/CLMM.
    fn dlmm_state(&self) -> Option<arb_types::DlmmState> {
        let _ = self;
        None
    }

    /// Pump.fun bonding curve state. `None` for CPMM/CLMM/DLMM.
    fn pump_fun_state(&self) -> Option<arb_types::PumpFunState> {
        let _ = self;
        None
    }

    /// Mint decimals `(decimals_a, decimals_b)` for `(base_mint, quote_mint)`.
    fn decimals(&self) -> (u8, u8) {
        let _ = self;
        (0, 0)
    }

    /// Token programs `(base_token_program, quote_token_program)`.
    fn token_programs(&self) -> [TokenProgramKind; 2] {
        let _ = self;
        [TokenProgramKind::SplToken; 2]
    }

    /// Token-2022 transfer fee bps `(base_fee_bps, quote_fee_bps)`.
    fn transfer_fees_bps(&self) -> (u16, u16) {
        let _ = self;
        (0, 0)
    }

    /// True when either mint has a risky/unsupported Token-2022 extension.
    fn has_risky_token(&self) -> bool {
        let _ = self;
        false
    }

    /// Resolved on-chain accounts needed by `arb-tx` to construct an instruction.
    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        let _ = self;
        None
    }

    /// List of dependency account pubkeys (vaults, tick arrays, bin arrays, configs,
    /// OpenBook markets) that must be tracked via WebSocket/RPC.
    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let _ = self;
        Vec::new()
    }

    /// Mutate pool state from a dependency account update (vault balance, TickArray,
    /// BinArray, AmmConfig, FeeConfig, OpenBook Market). Returns `Ok(true)` if the
    /// pubkey belonged to this pool and updated state.
    fn apply_dependency_account(&mut self, _pubkey: &Pubkey, _data: &[u8]) -> Result<bool> {
        Ok(false)
    }

    /// Update internal pool state from raw Solana pool account bytes.
    fn update_from_account(&mut self, data: &[u8]) -> Result<()>;

    /// Calculate expected output amount for a given input amount.
    /// `a_to_b`: true if swapping `base_mint -> quote_mint`; false if `quote_mint -> base_mint`.
    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64>;
}
