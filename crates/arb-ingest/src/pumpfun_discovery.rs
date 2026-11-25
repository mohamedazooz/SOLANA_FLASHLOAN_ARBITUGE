//! Dynamic Pump.fun Bonding Curve Discovery via the public frontend endpoints.
//!
//! Queries pump.fun's high-speed frontend endpoints to discover, rank, and track the
//! top 30 most active and liquid bonding curves before and during graduation.
//!
//! PROVENANCE (DATA-06): this is an unofficial, undocumented candidate source —
//! Cloudflare-protected, rate-limited, and free to change shape without notice.
//! It may ONLY propose candidate mints for on-chain validation: every pool
//! registered from it is subsequently hydrated from live chain state
//! (bonding-curve decode), and an API failure here degrades to an empty
//! candidate list. It must never feed authoritative market data nor cause a
//! stale-cache fallback.

use crate::discovery::DiscoveredPoolState;
use arb_constants::{PUMP_FUN_BONDING_CURVE_SEED, PUMP_FUN_PROGRAM_ID, WSOL_MINT};
use arb_types::{DexProtocol, MintId, PoolId, Result};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Public frontend API endpoint cataloged in BankkRoll/pumpfun-apis.
pub const PUMP_FUN_API_BASE: &str = "https://frontend-api.pump.fun";

/// Deserialized Pump.fun coin metadata from the frontend API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PumpFunCoin {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub bonding_curve: Option<String>,
    pub associated_bonding_curve: Option<String>,
    pub virtual_sol_reserves: Option<u64>,
    pub virtual_token_reserves: Option<u64>,
    pub total_supply: Option<u64>,
    #[serde(default)]
    pub complete: bool,
    pub market_cap: Option<f64>,
    pub usd_market_cap: Option<f64>,
    pub reply_count: Option<u64>,
    pub last_trade_timestamp: Option<u64>,
    pub raydium_pool: Option<String>,
}

/// Dynamic fetcher and ranker for Pump.fun bonding curves.
pub struct PumpFunDiscoveryClient {
    client: reqwest::Client,
    base_url: String,
    pump_program: Pubkey,
    wsol_mint: Pubkey,
}

impl Default for PumpFunDiscoveryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PumpFunDiscoveryClient {
    /// Create a new discovery client with standard timeout and user agent headers.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();

        let pump_program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID)
            .unwrap_or_else(|_| Pubkey::new_from_array([0u8; 32]));
        let wsol_mint =
            Pubkey::from_str(WSOL_MINT).unwrap_or_else(|_| Pubkey::new_from_array([0u8; 32]));

        Self {
            client,
            base_url: PUMP_FUN_API_BASE.to_string(),
            pump_program,
            wsol_mint,
        }
    }

    /// Fetch the top active coins from Pump.fun using both recent trade activity and market cap.
    pub async fn fetch_top_coins(&self, limit: usize) -> Result<Vec<PumpFunCoin>> {
        let fetch_limit = limit.clamp(10, 100);

        // Fetch two cohorts:
        // 1. Most recently active (by last_trade_timestamp)
        // 2. Highest market cap (closer to graduation / deepest liquidity)
        let url_recent = format!(
            "{}/coins?offset=0&limit={}&sort=last_trade_timestamp&order=DESC&includeNsfw=false",
            self.base_url, fetch_limit
        );
        let url_mcap = format!(
            "{}/coins?offset=0&limit={}&sort=market_cap&order=DESC&includeNsfw=false",
            self.base_url, fetch_limit
        );

        let (res_recent, res_mcap) = tokio::join!(
            self.client.get(&url_recent).send(),
            self.client.get(&url_mcap).send()
        );

        let mut all_coins = Vec::with_capacity(fetch_limit * 2);
        let mut seen_mints = HashSet::new();

        if let Ok(res) = res_recent {
            if res.status().is_success() {
                if let Ok(coins) = res.json::<Vec<PumpFunCoin>>().await {
                    for c in coins {
                        if seen_mints.insert(c.mint.clone()) {
                            all_coins.push(c);
                        }
                    }
                }
            } else {
                warn!(
                    "Pump.fun recent coins API returned status: {}",
                    res.status()
                );
            }
        }

        if let Ok(res) = res_mcap {
            if res.status().is_success() {
                if let Ok(coins) = res.json::<Vec<PumpFunCoin>>().await {
                    for c in coins {
                        if seen_mints.insert(c.mint.clone()) {
                            all_coins.push(c);
                        }
                    }
                }
            } else {
                debug!("Pump.fun mcap coins API returned status: {}", res.status());
            }
        }

        if all_coins.is_empty() {
            // Candidate tier only: an empty result degrades discovery, it is
            // never papered over with cached snapshots or synthetic entries.
            debug!(
                "Pump.fun discovery returned empty candidate set from public endpoint; \
                 continuing without pump candidates (fail-open on tier, fail-closed on data)"
            );
        }

        Ok(all_coins)
    }

    /// Convert raw PumpFunCoin metadata to DiscoveredPoolState instances,
    /// ranking by liquidity and filtering for the top 30 active curves.
    pub fn select_top_30(&self, coins: &[PumpFunCoin]) -> Vec<DiscoveredPoolState> {
        self.select_top_n(coins, 30)
    }

    /// Convert raw PumpFunCoin metadata to DiscoveredPoolState instances,
    /// ranking by liquidity and filtering for the top N active curves.
    pub fn select_top_n(&self, coins: &[PumpFunCoin], n: usize) -> Vec<DiscoveredPoolState> {
        let mut scored_pools: Vec<(f64, DiscoveredPoolState)> = Vec::new();

        for coin in coins {
            let mint_pubkey = match Pubkey::from_str(&coin.mint) {
                Ok(pk) => pk,
                Err(_) => continue,
            };

            // Derive or parse bonding curve address
            let bonding_curve_pk = if let Some(ref bc_str) = coin.bonding_curve {
                Pubkey::from_str(bc_str).unwrap_or_else(|_| {
                    Pubkey::find_program_address(
                        &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                        &self.pump_program,
                    )
                    .0
                })
            } else {
                Pubkey::find_program_address(
                    &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                    &self.pump_program,
                )
                .0
            };

            // Fail closed on unknown reserves (DATA-06): inventing the 30-SOL
            // launch value here would fabricate both ranking position and the
            // reported `liquidity_usd` of a pool we know nothing about.
            let Some(virtual_sol) = coin.virtual_sol_reserves else {
                debug!(
                    "Pump.fun candidate {} missing virtual_sol_reserves; dropped (fail closed)",
                    coin.mint
                );
                continue;
            };

            // RANKING-ONLY SOL price approximation (relative ordering across
            // pump candidates; never consumed by economics, sizing, or profit
            // math). usd_market_cap is the primary signal when published.
            const RANKING_SOL_USD: f64 = 180.0;
            let sol_tvl_est = (virtual_sol as f64 / 1e9) * RANKING_SOL_USD;
            let usd_tvl = coin.usd_market_cap.unwrap_or(0.0);
            let effective_tvl = usd_tvl.max(sol_tvl_est);

            // Favor curves that are active but not completed (between 20% and 98% progress)
            let completion_multiplier = if coin.complete {
                0.1 // Graduated coins are low priority on bonding curve
            } else if effective_tvl > 10_000.0 {
                1.5 // Active high liquidity curves
            } else {
                1.0
            };

            let score = effective_tvl * completion_multiplier;

            scored_pools.push((
                score,
                DiscoveredPoolState {
                    pool_id: PoolId(bonding_curve_pk),
                    protocol: DexProtocol::PumpFun,
                    base_mint: MintId(self.wsol_mint),
                    quote_mint: MintId(mint_pubkey),
                    base_vault: Pubkey::default(),
                    quote_vault: Pubkey::default(),
                    decimals_a: 9,
                    decimals_b: 6,
                    // Conservative discovery estimate for the documented SOL bonding-curve fee.
                    // The live quote path still requires a verified on-chain FeeConfig.
                    fee_bps: 125, // 1.25%; metadata only, not execution-authoritative
                    liquidity_usd: effective_tvl,
                },
            ));
        }

        // Sort descending by score
        scored_pools.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Take top n
        let top_n: Vec<DiscoveredPoolState> = scored_pools
            .into_iter()
            .take(n)
            .map(|(_, pool)| pool)
            .collect();

        info!(
            "Pump.fun discovery selected {} high-priority bonding curves for surveillance (requested up to {})",
            top_n.len(),
            n
        );

        top_n
    }

    /// Identify Pump.fun bonding curves approaching graduation within the BOOST sweet spot.
    ///
    /// Evaluates both Flash Loan execution mode (immediate atomic dump against opening liquidity)
    /// and Working Capital mode (TWAP absorption).
    pub fn find_boost_migration_targets(
        &self,
        coins: &[PumpFunCoin],
        flash_loan_fee_sf: u64,
        execution_tip_lamports: u64,
        min_net_profit_lamports: u64,
    ) -> Vec<PumpBoostTarget> {
        let mut targets = Vec::new();

        for coin in coins {
            // Must be active and not already migrated
            if coin.complete {
                continue;
            }

            let vsol = match coin.virtual_sol_reserves {
                Some(v) if v > 0 => v,
                _ => continue,
            };
            let vtok = match coin.virtual_token_reserves {
                Some(v) if v > 0 => v,
                _ => continue,
            };

            // Estimate real SOL reserves: standard curve launches with 30 SOL virtual reserve
            let real_sol_reserves = vsol.saturating_sub(30_000_000_000);

            // Sweet-spot filter: 40 SOL <= S <= 75 SOL (focusing around empirical peak ~66 SOL)
            if !(40_000_000_000..=75_000_000_000).contains(&real_sol_reserves) {
                continue;
            }

            let mint_pubkey = match Pubkey::from_str(&coin.mint) {
                Ok(pk) => pk,
                Err(_) => continue,
            };

            let bonding_curve = match coin.bonding_curve.as_ref() {
                Some(bc_str) => Pubkey::from_str(bc_str).unwrap_or_else(|_| {
                    let (pda, _) = Pubkey::find_program_address(
                        &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                        &self.pump_program,
                    );
                    pda
                }),
                None => {
                    let (pda, _) = Pubkey::find_program_address(
                        &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                        &self.pump_program,
                    );
                    pda
                }
            };

            // 1. Evaluate Flash Loan mode (conservative: 65% realized capture against immediate pool liquidity)
            let flash_loan_eval = arb_math::pump_boost_arb::evaluate_pump_boost_arb(
                real_sol_reserves,
                vsol,
                vtok,
                flash_loan_fee_sf,
                execution_tip_lamports,
                6_500, // 65% immediate execution efficiency
                min_net_profit_lamports,
            );

            // 2. Evaluate Working Capital mode (patient TWAP absorption: 90% capture efficiency)
            // sf = 0: wallet balance funds this mode, not a flash loan. The
            // canonical formula still applies its 1-lamport floor — a
            // conservative (overstated) cost, fail-closed direction.
            let working_capital_eval = arb_math::pump_boost_arb::evaluate_pump_boost_arb(
                real_sol_reserves,
                vsol,
                vtok,
                0, // 0% flash loan fee since using wallet balance
                execution_tip_lamports,
                9_000, // 90% TWAP absorption efficiency
                min_net_profit_lamports,
            );

            if let Some(wc_eval) = working_capital_eval {
                targets.push(PumpBoostTarget {
                    mint: MintId(mint_pubkey),
                    bonding_curve,
                    symbol: coin.symbol.clone().unwrap_or_else(|| "UNKNOWN".into()),
                    real_sol_reserves,
                    flash_loan_eval,
                    working_capital_eval: wc_eval,
                });
            }
        }

        targets.sort_by(|a, b| {
            b.working_capital_eval
                .net_profit_lamports
                .cmp(&a.working_capital_eval.net_profit_lamports)
        });

        targets
    }
}

/// Discovered target for Pump.fun BOOST Migration Arbitrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpBoostTarget {
    pub mint: MintId,
    pub bonding_curve: Pubkey,
    pub symbol: String,
    pub real_sol_reserves: u64,
    /// Evaluation if executed atomically via Flash Loan (same-block dump)
    pub flash_loan_eval: Option<arb_math::pump_boost_arb::BoostArbEvaluation>,
    /// Evaluation if executed via Working Capital (TWAP buyback absorption)
    pub working_capital_eval: arb_math::pump_boost_arb::BoostArbEvaluation,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coin(mint: &str, virtual_sol: Option<u64>) -> PumpFunCoin {
        PumpFunCoin {
            mint: mint.to_string(),
            name: Some("T".into()),
            symbol: Some("T".into()),
            description: None,
            bonding_curve: None,
            associated_bonding_curve: None,
            virtual_sol_reserves: virtual_sol,
            virtual_token_reserves: Some(1_073_000_000_000_000),
            total_supply: Some(1_000_000_000_000_000),
            complete: false,
            market_cap: None,
            usd_market_cap: None,
            reply_count: None,
            last_trade_timestamp: None,
            raydium_pool: None,
        }
    }

    /// (DATA-06 regression) Candidates with unreported reserves must be
    /// dropped, not silently scored against the 30-SOL launch default.
    #[test]
    fn test_missing_virtual_reserves_drop_fail_closed() {
        let client = PumpFunDiscoveryClient::new();
        let known = coin(&Pubkey::new_unique().to_string(), Some(96_000_000_000));
        let unknown_mint = Pubkey::new_unique().to_string();
        let unknown = coin(&unknown_mint, None);

        let pools = client.select_top_n(&[known, unknown], 30);
        assert_eq!(pools.len(), 1);
        assert_ne!(pools[0].quote_mint.0.to_string(), unknown_mint);
    }
    #[test]
    fn test_pumpfun_curve_derivation() {
        let client = PumpFunDiscoveryClient::new();
        let dummy_mint = Pubkey::new_unique();
        let (expected_pda, _) = Pubkey::find_program_address(
            &[PUMP_FUN_BONDING_CURVE_SEED, dummy_mint.as_ref()],
            &client.pump_program,
        );

        let coin = PumpFunCoin {
            mint: dummy_mint.to_string(),
            name: Some("Test Coin".into()),
            symbol: Some("TEST".into()),
            description: None,
            bonding_curve: None,
            associated_bonding_curve: None,
            virtual_sol_reserves: Some(30_000_000_000),
            virtual_token_reserves: Some(1_073_000_000_000_000),
            total_supply: Some(1_000_000_000_000_000),
            complete: false,
            market_cap: Some(15.0),
            usd_market_cap: Some(25_000.0),
            reply_count: Some(42),
            last_trade_timestamp: Some(1730870000),
            raydium_pool: None,
        };

        let pools = client.select_top_30(&[coin]);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].pool_id.0, expected_pda);
        assert_eq!(pools[0].protocol, DexProtocol::PumpFun);
        assert_eq!(pools[0].decimals_a, 9);
        assert_eq!(pools[0].decimals_b, 6);
        assert_eq!(pools[0].fee_bps, 125); // documented 1.25% SOL bonding-curve estimate
    }

    #[test]
    fn test_find_boost_migration_targets() {
        let client = PumpFunDiscoveryClient::new();
        let dummy_mint = Pubkey::new_unique();

        // Stalled curve in sweet spot (S = 66 SOL => vsol = 96 SOL)
        let coin = PumpFunCoin {
            mint: dummy_mint.to_string(),
            name: Some("Boost Gem".into()),
            symbol: Some("BOOST".into()),
            description: None,
            bonding_curve: None,
            associated_bonding_curve: None,
            virtual_sol_reserves: Some(96_000_000_000), // 30 + 66 SOL
            virtual_token_reserves: Some(335_000_000_000_000),
            total_supply: Some(1_000_000_000_000_000),
            complete: false,
            market_cap: Some(60.0),
            usd_market_cap: Some(100_000.0),
            reply_count: Some(100),
            last_trade_timestamp: Some(1730870000),
            raydium_pool: None,
        };

        let targets = client.find_boost_migration_targets(
            &[coin],
            3_458_764_513_820_540, // 30 bps as raw U68F60 sf (30 * 2^60/10000, truncated)
            100_000_000,           // 0.10 SOL Jito tip
            0,                     // min profit 0 for testing
        );

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].symbol, "BOOST");
        assert_eq!(targets[0].real_sol_reserves, 66_000_000_000);
        assert!(targets[0].working_capital_eval.acquired_tokens > 0);
        assert_eq!(
            targets[0].working_capital_eval.completion_cost_lamports,
            19_000_000_000
        );
    }
}
