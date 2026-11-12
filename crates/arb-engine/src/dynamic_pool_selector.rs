//! Dynamic Pool Selector — discovers and ranks Meteora DLMM pools
//! by on-chain liquidity, volume, and recent activity. Replaces static allowlist.

use arb_constants::METEORA_DLMM_PROGRAM_ID;
use arb_ingest::registry::{compute_dlmm_reserves, decode_dlmm_state};
use arb_math::dlmm::dlmm_active_bin_max_input;
use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, TokenProgramKind};
use async_trait::async_trait;
use solana_pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for dynamic pool selection
#[derive(Debug, Clone)]
pub struct DynamicPoolSelectorConfig {
    /// Maximum number of pools to track (top N by score)
    pub max_pools: usize,
    /// Minimum total liquidity (X + Y reserves) to consider a pool
    pub min_total_liquidity: u128,
    /// Minimum bin step (bps) - avoid ultra-wide bins with low precision
    pub min_bin_step_bps: u16,
    /// Maximum bin step (bps) - avoid ultra-narrow bins with high fee impact
    pub max_bin_step_bps: u16,
    /// Refresh interval in seconds
    pub refresh_interval_secs: u64,
    /// Weight for liquidity in scoring (0.0 - 1.0)
    pub liquidity_weight: f64,
    /// Weight for fee tier in scoring (0.0 - 1.0) - prefer lower fees
    pub fee_weight: f64,
    /// Whether to require WSOL as one of the mints
    pub require_wsol_pair: bool,
    /// WSOL mint address
    pub wsol_mint: MintId,
}

impl Default for DynamicPoolSelectorConfig {
    fn default() -> Self {
        use std::str::FromStr;
        Self {
            max_pools: 100,
            min_total_liquidity: 5_000_000_000, // 5 SOL equivalent
            min_bin_step_bps: 1,                // 0.01%
            max_bin_step_bps: 500,              // 5%
            refresh_interval_secs: 300,         // 5 minutes
            liquidity_weight: 0.7,
            fee_weight: 0.3,
            require_wsol_pair: true,
            wsol_mint: MintId(
                Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            ),
        }
    }
}

/// Scored pool entry for ranking
#[derive(Debug, Clone)]
pub struct ScoredPool {
    pub pool_id: PoolId,
    pub pool_state: PoolState,
    pub score: f64,
    pub total_liquidity: u128,
    pub bin_step_bps: u16,
    pub active_bin_max_input_wsol: u64,
    pub active_bin_max_input_other: u64,
}

/// Dynamic pool selector with periodic refresh
pub struct DynamicPoolSelector {
    config: DynamicPoolSelectorConfig,
    scored_pools: Arc<RwLock<Vec<ScoredPool>>>,
    last_refresh: Arc<RwLock<Option<u64>>>,
    rpc_client: Arc<dyn RpcPoolProvider>,
}

impl DynamicPoolSelector {
    pub fn new(config: DynamicPoolSelectorConfig, rpc_client: Arc<dyn RpcPoolProvider>) -> Self {
        Self {
            config,
            scored_pools: Arc::new(RwLock::new(Vec::new())),
            last_refresh: Arc::new(RwLock::new(None)),
            rpc_client,
        }
    }

    /// Get current top pools (triggers refresh if stale)
    pub async fn get_top_pools(&self) -> Vec<ScoredPool> {
        self.ensure_fresh().await;
        let pools = self.scored_pools.read().await;
        pools.clone()
    }

    /// Get pools suitable for 3-pool flash arb (WSOL -> A -> B -> WSOL)
    pub async fn get_flash_arb_candidates(
        &self,
        min_flash_liquidity: u64,
    ) -> Vec<FlashArbCandidate> {
        let pools = self.get_top_pools().await;
        let wsol = self.config.wsol_mint;

        // Group pools by their non-WSOL mint
        let mut pools_by_mint: HashMap<MintId, Vec<&ScoredPool>> = HashMap::new();
        for pool in &pools {
            let (mint_a, mint_b) = self.get_pool_mints(&pool.pool_state);
            let other_mint = if mint_a == wsol {
                mint_b
            } else if mint_b == wsol {
                mint_a
            } else {
                continue;
            };

            if pool.active_bin_max_input_wsol >= min_flash_liquidity {
                pools_by_mint.entry(other_mint).or_default().push(pool);
            }
        }

        // Build candidates: WSOL -> A -> B -> WSOL
        let mut candidates = Vec::new();
        let mints: Vec<MintId> = pools_by_mint.keys().cloned().collect();

        for (i, &mint_a) in mints.iter().enumerate() {
            for &mint_b in &mints[i + 1..] {
                let pools_a = pools_by_mint.get(&mint_a).unwrap_or(&Vec::new());
                let pools_b = pools_by_mint.get(&mint_b).unwrap_or(&Vec::new());

                for pool1 in pools_a {
                    for pool2 in pools_b {
                        // Check if pool2 can bridge A <-> B
                        let (mint_x, mint_y) = self.get_pool_mints(&pool2.pool_state);
                        if (mint_x == mint_a && mint_y == mint_b)
                            || (mint_x == mint_b && mint_y == mint_a)
                        {
                            // Find pool3: B -> WSOL
                            if let Some(pools_c) = pools_by_mint.get(&mint_b) {
                                for pool3 in pools_c {
                                    if pool1.pool_id != pool3.pool_id
                                        && pool2.pool_id != pool3.pool_id
                                    {
                                        let min_flash = pool1
                                            .active_bin_max_input_wsol
                                            .min(pool2.active_bin_max_input_wsol)
                                            .min(pool3.active_bin_max_input_wsol);

                                        if min_flash >= min_flash_liquidity {
                                            candidates.push(FlashArbCandidate {
                                                pool1: pool1.clone(),
                                                pool2: pool2.clone(),
                                                pool3: pool3.clone(),
                                                mint_a,
                                                mint_b,
                                                min_flash_liquidity: min_flash,
                                                estimated_profit_bps: self
                                                    .estimate_profit_bps(pool1, pool2, pool3),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by estimated profit
        candidates.sort_by(|a, b| {
            b.estimated_profit_bps
                .partial_cmp(&a.estimated_profit_bps)
                .unwrap()
        });
        candidates
    }

    /// Ensure pool data is fresh
    async fn ensure_fresh(&self) {
        let should_refresh = {
            let last = self.last_refresh.read().await;
            match *last {
                Some(t) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    now - t > self.config.refresh_interval_secs
                }
                None => true,
            }
        };

        if should_refresh {
            self.refresh_pools().await;
        }
    }

    /// Full pool discovery and scoring
    async fn refresh_pools(&self) {
        info!("Refreshing dynamic pool list...");

        // 1. Fetch all Meteora DLMM pool accounts from RPC
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
        let accounts = match self.rpc_client.get_program_accounts(program_id).await {
            Ok(accs) => accs,
            Err(e) => {
                warn!("Failed to fetch program accounts: {}", e);
                return;
            }
        };

        info!("Fetched {} Meteora DLMM accounts", accounts.len());

        // 2. Decode and score each pool
        let mut scored = Vec::new();
        let now_slot = self.get_current_slot().await;

        for (pubkey, account) in accounts {
            if account.data.len() < 200 {
                continue;
            }

            if let Some(state) = decode_dlmm_state(&account.data) {
                if state.status != 0 {
                    continue;
                }
                if state.bin_step_bps < self.config.min_bin_step_bps {
                    continue;
                }
                if state.bin_step_bps > self.config.max_bin_step_bps {
                    continue;
                }

                // Get reserves
                let reserves = compute_dlmm_reserves(
                    &state,
                    &account.data,
                    &Pubkey::default(),
                    &Pubkey::default(),
                );
                let total_liquidity = reserves.0.saturating_add(reserves.1);

                if total_liquidity < self.config.min_total_liquidity {
                    continue;
                }

                // Create pool state
                let pool_state = self.build_pool_state(pubkey, state, reserves);

                // Score the pool
                let score = self.score_pool(&pool_state, now_slot);

                // Calculate max flash input for WSOL
                let (wsol_max, other_max) = self.calculate_max_flash_inputs(&pool_state);

                scored.push(ScoredPool {
                    pool_id: PoolId(pubkey),
                    pool_state,
                    score,
                    total_liquidity,
                    bin_step_bps: state.bin_step_bps,
                    active_bin_max_input_wsol: wsol_max,
                    active_bin_max_input_other: other_max,
                });
            }
        }

        // Sort by score and take top N
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        scored.truncate(self.config.max_pools);

        info!("Selected top {} pools for trading", scored.len());

        *self.scored_pools.write().await = scored;
        *self.last_refresh.write().await = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
    }

    fn score_pool(&self, pool: &PoolState, _now_slot: u64) -> f64 {
        // Get total liquidity from reserves
        let total_liquidity = pool.reserve_in.saturating_add(pool.reserve_out) as f64;
        let liquidity_score = total_liquidity.ln().max(0.0) / 30.0; // ~1e13 max

        // Fee score - lower fees are better
        let fee_rate = pool.fee_numerator as f64 / pool.fee_denominator as f64;
        let fee_score = 1.0 - fee_rate.min(1.0);

        self.config.liquidity_weight * liquidity_score.min(1.0) + self.config.fee_weight * fee_score
    }

    fn get_pool_mints(&self, pool: &PoolState) -> (MintId, MintId) {
        // Use on-chain token_x/token_y if available, fall back to base/quote
        if let Some(dlmm) = &pool.dlmm {
            if let (Some(tx), Some(ty)) = (dlmm.token_x_mint, dlmm.token_y_mint) {
                return (tx, ty);
            }
        }
        (pool.base_mint, pool.quote_mint)
    }

    fn build_pool_state(
        &self,
        pubkey: Pubkey,
        state: arb_types::DlmmState,
        reserves: (u128, u128),
    ) -> PoolState {
        PoolState {
            pool_id: PoolId(pubkey),
            protocol: DexProtocol::MeteoraDlmm,
            slot: arb_types::Slot(0),
            write_version: 0,
            base_mint: state.token_x_mint.unwrap_or(MintId::default()),
            quote_mint: state.token_y_mint.unwrap_or(MintId::default()),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: reserves.0,
            reserve_out: reserves.1,
            fee_numerator: state.bin_step_bps as u64,
            fee_denominator: 10_000,
            is_active: state.status == 0,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(state),
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            execution_accounts: None,
            last_update_slot: 0,
            last_update_ms: 0,
        }
    }

    fn calculate_max_flash_inputs(&self, pool: &PoolState) -> (u64, u64) {
        if let Some(dlmm) = &pool.dlmm {
            if let Some(active_bin) = dlmm.bins.iter().find(|b| b.bin_id == dlmm.active_bin_id) {
                let wsol_max = dlmm_active_bin_max_input(
                    dlmm.active_bin_id,
                    dlmm.bin_step_bps,
                    u64::try_from(active_bin.amount_x).unwrap_or(0),
                    u64::try_from(active_bin.amount_y).unwrap_or(0),
                    true, // X -> Y
                )
                .unwrap_or(0);

                let other_max = dlmm_active_bin_max_input(
                    dlmm.active_bin_id,
                    dlmm.bin_step_bps,
                    u64::try_from(active_bin.amount_x).unwrap_or(0),
                    u64::try_from(active_bin.amount_y).unwrap_or(0),
                    false, // Y -> X
                )
                .unwrap_or(0);

                return (wsol_max, other_max);
            }
        }
        (0, 0)
    }

    fn estimate_profit_bps(&self, p1: &ScoredPool, p2: &ScoredPool, p3: &ScoredPool) -> f64 {
        // Simplified profit estimation based on fee tiers and liquidity
        let fee1 = p1.pool_state.fee_numerator as f64 / p1.pool_state.fee_denominator as f64;
        let fee2 = p2.pool_state.fee_numerator as f64 / p2.pool_state.fee_denominator as f64;
        let fee3 = p3.pool_state.fee_numerator as f64 / p3.pool_state.fee_denominator as f64;

        let total_fees = fee1 + fee2 + fee3;
        let liquidity_factor = (p1
            .total_liquidity
            .min(p2.total_liquidity)
            .min(p3.total_liquidity) as f64)
            .ln()
            .max(1.0);

        // Rough estimate: higher liquidity + lower fees = better profit potential
        (1.0 - total_fees) * liquidity_factor * 100.0
    }

    async fn get_current_slot(&self) -> u64 {
        self.rpc_client.get_slot().await.unwrap_or(0)
    }
}

/// Flash arb candidate with all three pools
#[derive(Debug, Clone)]
pub struct FlashArbCandidate {
    pub pool1: ScoredPool, // WSOL -> A
    pub pool2: ScoredPool, // A -> B
    pub pool3: ScoredPool, // B -> WSOL
    pub mint_a: MintId,
    pub mint_b: MintId,
    pub min_flash_liquidity: u64,
    pub estimated_profit_bps: f64,
}

/// Trait for RPC pool provider (allows mocking in tests)
#[async_trait]
pub trait RpcPoolProvider: Send + Sync {
    async fn get_program_accounts(
        &self,
        program_id: Pubkey,
    ) -> Result<Vec<(Pubkey, solana_account::Account)>, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_slot(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>;
    async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<Vec<Option<solana_account::Account>>, Box<dyn std::error::Error + Send + Sync>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DynamicPoolSelectorConfig::default();
        assert_eq!(config.max_pools, 100);
        assert!(config.require_wsol_pair);
    }
}
