//! Multi-DEX Active Pool Registry & Live Reserve Ingestion.
//!
//! Registers verified mainnet pools across Raydium V4, Orca Whirlpool, and
//! Meteora DLMM and batch-fetches their live reserve balances each tick. The
//! reserve VALUES (the live market data) are decoded directly from on-chain
//! token-vault account balances via `getMultipleAccounts` — nothing is
//! hardcoded. Only the permanent on-chain account ADDRESSES (pool + vault
//! pubkeys, which never change after pool creation) are recorded, matching
//! the existing pattern used for Raydium / Orca vaults.
//!
//! Feeding these live multi-pool states into `MarketGraph` ends the previous
//! single-pool starvation where `find_base_cycles` could never find a second
//! pool to form a 2-hop cross-DEX arbitrage.

use crate::{DependencyManifestRegistry, RpcPool};
use arb_types::{DexProtocol, MintId, PoolId};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// A verified mainnet pool with its two token-vault addresses for live balance reads.
#[derive(Debug, Clone)]
pub struct RegisteredPool {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Mint decimals for `(base_mint, quote_mint)`, read from a verified mint
    /// account. `0` means unknown; such pools are rejected by the engine.
    pub decimals_a: u8,
    pub decimals_b: u8,
    /// SPL token account holding the base-mint reserve (live balance read each tick).
    pub base_vault: Pubkey,
    /// SPL token account holding the quote-mint reserve (live balance read each tick).
    pub quote_vault: Pubkey,
    /// Protocol swap fee in basis points (used for graph edge weights).
    pub fee_bps: u16,
}

/// Registry of verified multi-DEX pools that streams live reserve states into the
/// `MarketStateCache` / `MarketGraph`.
///
/// Registry configuration is immutable during a fetch. The supervisor stores
/// replacement registries behind ArcSwap so refreshes do not mutate in-flight reads.
#[derive(Clone)]
pub struct MultiDexRegistry {
    pub(crate) pools: Vec<RegisteredPool>,
    pub(crate) rpc: RpcPool,
    pub dependency_manifest: DependencyManifestRegistry,
    /// Immutable mint-decimal cache for canonical DLMM token_x/token_y mints.
    /// Mint decimals never change on-chain, so entries live forever. `Arc`
    /// keeps clones of the registry (per-worker) sharing one cache.
    pub(crate) dlmm_decimals_cache:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<MintId, u8>>>,
}

impl MultiDexRegistry {
    /// Build a registry pre-loaded with verified mainnet SOL-USDC pools across
    /// Raydium V4, Orca Whirlpool, and Meteora DLMM, sharing the supplied RPC pool.
    pub fn mainnet_default(rpc: RpcPool) -> Self {
        // Addresses come from `arb-constants`; nothing is re-typed here.
        let pk = |s: &str| Pubkey::from_str(s).expect("static mainnet address is valid base58");
        let wsol = MintId(pk(arb_constants::WSOL_MINT));
        let usdc = MintId(pk(arb_constants::USDC_MINT));

        let pools = vec![
            // Raydium AMM v4 — SOL/USDC
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumV4,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_USDC_VAULT),
                fee_bps: 25,
            },
            // Orca Whirlpools — SOL/USDC (tick-spacing 4 pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_VAULT),
                quote_vault: pk(arb_constants::ORCA_USDC_VAULT),
                fee_bps: 5,
            },
            // Orca Whirlpools — SOL/USDC, additional fee tiers. These unlock the
            // LOW-FEE routes (tick 1 => 1 bps, tick 2 => 2 bps, tick 8 => 8 bps;
            // the real fee is read on-chain). Vaults are placeholders: CLMM
            // reserves come from the pool account (sqrt_price + liquidity), not
            // the vault balances, so the registry does not require them here.
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_1BPS_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_USDC_1BPS_POOL),
                quote_vault: pk(arb_constants::ORCA_SOL_USDC_1BPS_POOL),
                fee_bps: 1,
            },
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_2BPS_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_USDC_2BPS_POOL),
                quote_vault: pk(arb_constants::ORCA_SOL_USDC_2BPS_POOL),
                fee_bps: 2,
            },
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_8BPS_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_USDC_8BPS_POOL),
                quote_vault: pk(arb_constants::ORCA_SOL_USDC_8BPS_POOL),
                fee_bps: 8,
            },
            // Meteora DLMM — SOL/USDC (bin_step 4, verified live mainnet pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::METEORA_SOL_USDC_POOL)),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::METEORA_SOL_RESERVE),
                quote_vault: pk(arb_constants::METEORA_USDC_RESERVE),
                fee_bps: arb_constants::METEORA_SOL_USDC_BIN_STEP,
            },
            // Meteora DLMM — SOL/USDC Pool 2 (verified live mainnet pool with deep liquidity)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::METEORA_SOL_USDC_POOL_2)),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::METEORA_SOL_USDC_POOL_2),
                quote_vault: pk(arb_constants::METEORA_SOL_USDC_POOL_2),
                fee_bps: 10,
            },
            // Meteora DLMM — SOL/USDC Pool 3 (verified live mainnet pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::METEORA_SOL_USDC_POOL_3)),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::METEORA_SOL_USDC_POOL_3),
                quote_vault: pk(arb_constants::METEORA_SOL_USDC_POOL_3),
                fee_bps: 20,
            },
            // Raydium CPMM — SOL/USDC (1 bps fee tier, lowest on Solana)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_CPMM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumCpmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_CPMM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_CPMM_USDC_VAULT),
                fee_bps: 1,
            },
        ];

        Self {
            pools,
            rpc,
            dependency_manifest: Default::default(),
            dlmm_decimals_cache: std::sync::Arc::new(parking_lot::Mutex::new(Default::default())),
        }
    }

    /// Register an additional verified pool dynamically (e.g. a DexScreener-discovered
    /// trending token pool with >= 2 venues).
    pub fn register(&mut self, pool: RegisteredPool) {
        if !self.pools.iter().any(|p| p.pool_id == pool.pool_id) {
            self.pools.push(pool);
        }
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    pub fn pools(&self) -> &[RegisteredPool] {
        &self.pools
    }
}

pub use crate::decoders::*;
pub use crate::fetcher::PacedAccountFetcher;
