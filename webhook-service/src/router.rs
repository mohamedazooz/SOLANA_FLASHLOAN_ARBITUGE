//! AMM and Account Redundancy Routing Engine.
//!
//! Indexes all configured Solana AMMs (Raydium V4/CPMM/CLMM, Orca Whirlpool,
//! Meteora DLMM/Dynamic, Jupiter, and Pump.fun) and resolves pool state modifications in O(1) time.

use crate::config::{AppConfig, PoolConfig};
use crate::models::{ResolvedPoolEvent, SolanaTransactionPayload};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// High-speed router mapping on-chain accounts and program IDs to monitored AMM pools.
#[derive(Debug, Clone)]
pub struct AmmRouter {
    /// Maps Solana Program ID to AMM label (e.g. "raydium_v4", "orca_whirlpool", "pump_fun").
    program_to_amm: Arc<HashMap<String, String>>,
    /// Maps any monitored pool address or related account to its corresponding PoolConfigs.
    account_to_pools: Arc<HashMap<String, Vec<Arc<PoolConfig>>>>,
    /// Maps Token Mint to all redundant pools facilitating trades for that mint.
    mint_to_pools: Arc<HashMap<String, Vec<Arc<PoolConfig>>>>,
    /// Set of all monitored Program IDs for fast early filtering.
    monitored_programs: Arc<HashSet<String>>,
}

impl AmmRouter {
    /// Constructs routing lookup tables from the parsed configuration.
    pub fn new(config: &AppConfig) -> Self {
        let program_to_amm = config.amms.build_program_map();
        let monitored_programs: HashSet<String> = program_to_amm.keys().cloned().collect();

        let mut account_to_pools: HashMap<String, Vec<Arc<PoolConfig>>> = HashMap::new();
        let mut mint_to_pools: HashMap<String, Vec<Arc<PoolConfig>>> = HashMap::new();

        for token in &config.tokens {
            let mut pools_for_mint = Vec::new();

            for pool in &token.redundant_pools {
                let pool_arc = Arc::new(pool.clone());
                pools_for_mint.push(Arc::clone(&pool_arc));

                // Map primary pool ID
                account_to_pools
                    .entry(pool.pool_id.clone())
                    .or_default()
                    .push(Arc::clone(&pool_arc));

                // Map all redundant accounts associated with this pool (e.g. vault, authority, state)
                for acc in &pool.accounts {
                    account_to_pools
                        .entry(acc.clone())
                        .or_default()
                        .push(Arc::clone(&pool_arc));
                }
            }

            mint_to_pools.insert(token.mint.clone(), pools_for_mint);
        }

        Self {
            program_to_amm: Arc::new(program_to_amm),
            account_to_pools: Arc::new(account_to_pools),
            mint_to_pools: Arc::new(mint_to_pools),
            monitored_programs: Arc::new(monitored_programs),
        }
    }

    /// Checks if a given program ID belongs to our monitored AMMs.
    #[inline]
    pub fn is_monitored_program(&self, program_id: &str) -> bool {
        self.monitored_programs.contains(program_id)
    }

    /// Resolves an AMM label from a Program ID.
    #[inline]
    pub fn get_amm_type(&self, program_id: &str) -> Option<&str> {
        self.program_to_amm.get(program_id).map(|s| s.as_str())
    }

    /// Returns all redundant pools for a given target token mint.
    pub fn get_pools_for_mint(&self, mint: &str) -> Option<&[Arc<PoolConfig>]> {
        self.mint_to_pools.get(mint).map(|v| v.as_slice())
    }

    /// Scans a Solana transaction payload and instantly resolves which monitored AMM pools were modified.
    ///
    /// Evaluates:
    /// - Outer instruction accounts and program IDs.
    /// - Inner instruction accounts and program IDs.
    /// - Account data balance changes.
    /// - Token transfers (source, destination, mint).
    pub fn resolve_modified_pools(
        &self,
        payload: &SolanaTransactionPayload,
    ) -> Vec<ResolvedPoolEvent> {
        let mut matched_pools: HashMap<String, Arc<PoolConfig>> = HashMap::new();
        let mut modified_accounts_set: HashSet<String> = HashSet::new();

        // 1. Inspect outer instructions
        for inst in &payload.instructions {
            for acc in &inst.accounts {
                modified_accounts_set.insert(acc.clone());
                if let Some(pools) = self.account_to_pools.get(acc) {
                    for p in pools {
                        matched_pools.entry(p.pool_id.clone()).or_insert_with(|| Arc::clone(p));
                    }
                }
            }
        }

        // 2. Inspect inner instructions (crucial for multi-leg swaps and routing through aggregator contracts)
        for group in &payload.inner_instructions {
            for inner in &group.instructions {
                for acc in &inner.accounts {
                    modified_accounts_set.insert(acc.clone());
                    if let Some(pools) = self.account_to_pools.get(acc) {
                        for p in pools {
                            matched_pools.entry(p.pool_id.clone()).or_insert_with(|| Arc::clone(p));
                        }
                    }
                }
            }
        }

        // 3. Inspect modified account data entries
        for acc_data in &payload.account_data {
            modified_accounts_set.insert(acc_data.account.clone());
            if let Some(pools) = self.account_to_pools.get(&acc_data.account) {
                for p in pools {
                    matched_pools.entry(p.pool_id.clone()).or_insert_with(|| Arc::clone(p));
                }
            }
        }

        // 4. Inspect token transfer mints and token accounts
        for transfer in &payload.token_transfers {
            if let Some(ref from_acc) = transfer.from_token_account {
                modified_accounts_set.insert(from_acc.clone());
                if let Some(pools) = self.account_to_pools.get(from_acc) {
                    for p in pools {
                        matched_pools.entry(p.pool_id.clone()).or_insert_with(|| Arc::clone(p));
                    }
                }
            }
            if let Some(ref to_acc) = transfer.to_token_account {
                modified_accounts_set.insert(to_acc.clone());
                if let Some(pools) = self.account_to_pools.get(to_acc) {
                    for p in pools {
                        matched_pools.entry(p.pool_id.clone()).or_insert_with(|| Arc::clone(p));
                    }
                }
            }
        }

        let modified_accounts: Vec<String> = modified_accounts_set.into_iter().collect();

        // Build resolved pool events
        matched_pools
            .into_iter()
            .map(|(pool_id, pool_config)| ResolvedPoolEvent {
                signature: payload.signature.clone(),
                slot: payload.slot,
                timestamp: payload.timestamp,
                pool_id,
                amm_type: pool_config.amm.clone(),
                quote_mint: pool_config.quote_mint.clone(),
                modified_accounts: modified_accounts.clone(),
                token_transfers: payload.token_transfers.clone(),
            })
            .collect()
    }
}
