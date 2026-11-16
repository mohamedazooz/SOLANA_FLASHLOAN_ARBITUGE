//! Fresh-State Meteora DLMM Cross-Pool Pipeline (`MeteoraDlmmFreshStatePipeline`).
//!
//! Solves quote staleness by:
//! 1. Fetching pool accounts and dynamic bin arrays in a single batched RPC request.
//! 2. Capturing the exact RPC `context.slot` as the canonical `snapshot_slot`.
//! 3. Quoting, sizing, compiling, and simulating with minimum latency.
//! 4. Validating that `simulation_slot - snapshot_slot <= max_allowed_slot_lag` (default: 2 slots).
//! 5. Executing bounded retries with exponential backoff and jitter if the state advances.
//! 6. Generating comprehensive, non-overlapping numerical profitability accounting.

use crate::meteora_simulation_harness::MeteoraDlmmMainnetSimulationHarness;
use crate::profitability_gate::ExecutionCostBreakdown;
use arb_engine::{
    DlmmOptimizerConfig, MeteoraDlmmCrossPoolQuoteEngine, MeteoraDlmmPoolPairDiscovery,
    MeteoraDlmmTradeSizeOptimizer,
};
use arb_ingest::RpcPool;
use arb_ingest::registry::{
    METEORA_BINS_PER_ARRAY, decode_bin_array, decode_dlmm_state, derive_meteora_bin_array_pda,
};
use arb_tx::{KeypairSigner, MeteoraDlmmAtomicArbitrageBuilder};
use arb_types::currency::{
    ConversionSource, Currency, DimensionalProfitabilityReport, FeeItem, TimestampedRate,
    TypedTokenAmount,
};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, PoolKind, PoolState, Result,
    Slot, TokenProgramKind,
};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Derive bin array PDAs for a given radius around the active bin ID.
/// Removes any hardcoded 3-array ceiling: radius 1 = 3 arrays, radius 2 = 5 arrays, etc.
pub fn derive_bin_array_pdas_with_depth(
    pool_id: &Pubkey,
    active_bin_id: i32,
    depth: usize,
) -> Vec<Pubkey> {
    let center_idx = active_bin_id.div_euclid(METEORA_BINS_PER_ARRAY as i32) as i64;
    let mut pdas = Vec::with_capacity(2 * depth + 1);
    pdas.push(derive_meteora_bin_array_pda(pool_id, center_idx));
    for offset in 1..=depth {
        pdas.push(derive_meteora_bin_array_pda(
            pool_id,
            center_idx - (offset as i64),
        ));
        pdas.push(derive_meteora_bin_array_pda(
            pool_id,
            center_idx + (offset as i64),
        ));
    }
    pdas
}

/// Configuration for fresh-state execution and retry bounding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraFreshPipelineConfig {
    pub max_retries: usize,
    pub max_allowed_slot_lag: u64,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub slippage_bps: u64,
    pub cu_limit: u32,
    pub priority_fee_micro_lamports: u64,
    pub min_input_amount: u64,
    pub max_input_amount: u64,
    /// Conservative SOL price in USD/Quote units for dimensional fee conversion (e.g. 180 USD/SOL).
    pub sol_usd_price: Option<u64>,
    /// Depth of bin arrays to fetch around active bin (depth 1 = 3 arrays, depth 2 = 5 arrays).
    pub requested_bin_array_depth: usize,
    /// Compile the two-pool route as a Solana Transaction v1 (SIMD-0385, 4,096-byte
    /// packet, native compute budget) instead of a legacy 1,232-byte transaction.
    ///
    /// Off by default: v1 is only accepted by senders/relays that support it, and the
    /// legacy path is the safe common denominator.
    #[serde(default)]
    pub two_pool_v1: bool,
}

impl Default for MeteoraFreshPipelineConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            max_allowed_slot_lag: 2,
            base_backoff_ms: 30,
            max_backoff_ms: 150,
            slippage_bps: 100, // 1% tolerance
            cu_limit: 400_000,
            priority_fee_micro_lamports: 10_000,
            min_input_amount: 100_000,    // 0.10 USDC
            max_input_amount: 10_000_000, // 10.00 USDC
            sol_usd_price: Some(180),
            requested_bin_array_depth: 1, // 3-array fast path by default, extensible
            two_pool_v1: false,           // legacy 1,232-byte packet unless opted in
        }
    }
}

/// Comprehensive numerical accounting for a simulated/paper opportunity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraProfitabilityReport {
    pub candidate_id: String,
    pub pool_a: Pubkey,
    pub pool_b: Pubkey,
    pub direction: String,
    pub quote_mint: Currency,
    pub quote_decimals: u8,
    pub initial_quote_amount: u64,
    pub hop1_amount_out: u64,
    pub hop2_final_amount_out: u64,
    pub gross_spread_quote_raw: i64,
    pub dex_fees_hop1_raw: u64,
    pub dex_fees_hop1_currency: Currency,
    pub dex_fees_hop2_raw: u64,
    pub dex_fees_hop2_currency: Currency,
    pub total_dex_fees_raw: u64,
    pub transfer_fees_raw: u64,
    pub base_fee_lamports: u64,
    pub priority_fee_lamports: u64,
    pub jito_tip_lamports: u64,
    pub refundable_rent_lamports: u64,
    pub total_network_fees_sol_lamports: u64,
    pub total_execution_cost_quote_raw: u64,
    pub net_expected_profit_quote_raw: i64,
    pub net_profit_bps: i64,
    pub quote_slot: u64,
    pub simulation_slot: Option<u64>,
    pub slot_lag: Option<u64>,
    pub is_fresh: bool,
    pub retries_used: usize,
    pub cu_consumed: u64,
    pub tx_size_bytes: usize,
    /// Wire format the candidate was compiled into (`Legacy` or `V1`).
    #[serde(default)]
    pub tx_version: String,
    pub simulation_success: bool,
    pub rejection_reason: Option<String>,
    pub status: String,
    pub dimensional_report: DimensionalProfitabilityReport,
}

impl MeteoraProfitabilityReport {
    /// Backwards compatibility: gross spread in raw quote units
    pub fn gross_spread_lamports(&self) -> i64 {
        self.gross_spread_quote_raw
    }

    /// Backwards compatibility: net expected profit in raw quote units
    pub fn net_expected_profit_lamports(&self) -> i64 {
        self.net_expected_profit_quote_raw
    }

    /// Backwards compatibility: total execution costs in raw quote units
    pub fn total_execution_costs(&self) -> u64 {
        self.total_execution_cost_quote_raw
    }

    /// Backwards compatibility: hop 1 fee
    pub fn dex_fees_hop1_lamports(&self) -> u64 {
        self.dex_fees_hop1_raw
    }

    /// Backwards compatibility: hop 2 fee
    pub fn dex_fees_hop2_lamports(&self) -> u64 {
        self.dex_fees_hop2_raw
    }

    /// Backwards compatibility: total dex fees
    pub fn total_dex_fees_lamports(&self) -> u64 {
        self.total_dex_fees_raw
    }
}

pub struct MeteoraDlmmFreshStatePipeline {
    rpc: Arc<RpcPool>,
    harness: MeteoraDlmmMainnetSimulationHarness,
    config: MeteoraFreshPipelineConfig,
}

impl MeteoraDlmmFreshStatePipeline {
    pub fn new(rpc: Arc<RpcPool>, config: MeteoraFreshPipelineConfig) -> Self {
        Self {
            harness: MeteoraDlmmMainnetSimulationHarness::from_env(),
            rpc,
            config,
        }
    }

    /// Fetch targeted pool accounts and active bin arrays in a single batched RPC request.
    /// Returns decoded pool states along with the exact RPC context slot and resolved bin array PDAs.
    pub async fn fetch_fresh_pool_pair(
        &self,
        pool_a_id: Pubkey,
        pool_b_id: Pubkey,
        cached_pdas: Option<(&[Pubkey], &[Pubkey])>,
    ) -> Result<(PoolState, PoolState, u64, Vec<Pubkey>, Vec<Pubkey>)> {
        let (ba_a, ba_b, initial_slot) = match cached_pdas {
            Some((a, b)) if !a.is_empty() && !b.is_empty() => (a.to_vec(), b.to_vec(), 0u64),
            _ => {
                // Discover active bins & parameters from LbPair
                let pool_keys = [pool_a_id.to_string(), pool_b_id.to_string()];
                let pool_key_refs: Vec<&str> = pool_keys.iter().map(|s| s.as_str()).collect();

                let (pool_accs, slot1) = self
                    .rpc
                    .get_multiple_accounts_with_slot(&pool_key_refs)
                    .await
                    .map_err(|e| ArbError::Internal(e.to_string()))?;

                if pool_accs.len() < 2 || pool_accs[0].is_none() || pool_accs[1].is_none() {
                    return Err(ArbError::InvalidInput(
                        "Unable to fetch LbPair accounts for pool pair".into(),
                    ));
                }

                let data_a = Self::extract_account_data(pool_accs[0].as_ref().unwrap())?;
                let data_b = Self::extract_account_data(pool_accs[1].as_ref().unwrap())?;

                let dlmm_a = decode_dlmm_state(&data_a).ok_or_else(|| {
                    ArbError::DeserializationError("failed to decode DLMM A state".into())
                })?;
                let dlmm_b = decode_dlmm_state(&data_b).ok_or_else(|| {
                    ArbError::DeserializationError("failed to decode DLMM B state".into())
                })?;

                let a_pdas = derive_bin_array_pdas_with_depth(
                    &pool_a_id,
                    dlmm_a.active_bin_id,
                    self.config.requested_bin_array_depth,
                );

                let b_pdas = derive_bin_array_pdas_with_depth(
                    &pool_b_id,
                    dlmm_b.active_bin_id,
                    self.config.requested_bin_array_depth,
                );

                (a_pdas, b_pdas, slot1)
            }
        };

        // Single batched RPC call fetching all 8 accounts atomically
        let mut all_keys_str = Vec::with_capacity(2 + ba_a.len() + ba_b.len());
        all_keys_str.push(pool_a_id.to_string());
        for pda in &ba_a {
            all_keys_str.push(pda.to_string());
        }
        all_keys_str.push(pool_b_id.to_string());
        for pda in &ba_b {
            all_keys_str.push(pda.to_string());
        }

        let all_refs: Vec<&str> = all_keys_str.iter().map(|s| s.as_str()).collect();
        let (accounts, snapshot_slot) = self
            .rpc
            .get_multiple_accounts_with_slot(&all_refs)
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let active_slot = if snapshot_slot > 0 {
            snapshot_slot
        } else {
            initial_slot
        };

        let expected_total = 2 + ba_a.len() + ba_b.len();
        if accounts.len() < expected_total {
            return Err(ArbError::InvalidInput(
                "Incomplete batched account return".into(),
            ));
        }

        // Parse Pool A
        let fresh_data_a = Self::extract_account_data(
            accounts[0]
                .as_ref()
                .ok_or_else(|| ArbError::InvalidInput("missing pool A".into()))?,
        )?;
        let mut fresh_dlmm_a = decode_dlmm_state(&fresh_data_a)
            .ok_or_else(|| ArbError::DeserializationError("bad dlmm A".into()))?;

        for acc_val in accounts.iter().take(ba_a.len() + 1).skip(1).flatten() {
            if let Ok(ba_bytes) = Self::extract_account_data(acc_val) {
                let parsed = decode_bin_array(&ba_bytes);
                fresh_dlmm_a.bins.extend(parsed);
            }
        }

        // Parse Pool B
        let b_offset = 1 + ba_a.len();
        let fresh_data_b = Self::extract_account_data(
            accounts[b_offset]
                .as_ref()
                .ok_or_else(|| ArbError::InvalidInput("missing pool B".into()))?,
        )?;
        let mut fresh_dlmm_b = decode_dlmm_state(&fresh_data_b)
            .ok_or_else(|| ArbError::DeserializationError("bad dlmm B".into()))?;

        for acc_val in accounts
            .iter()
            .take(expected_total)
            .skip(b_offset + 1)
            .flatten()
        {
            if let Ok(ba_bytes) = Self::extract_account_data(acc_val) {
                let parsed = decode_bin_array(&ba_bytes);
                fresh_dlmm_b.bins.extend(parsed);
            }
        }

        let pool_a_state = Self::build_pool_state(
            pool_a_id,
            &fresh_data_a,
            fresh_dlmm_a,
            ba_a.clone(),
            active_slot,
        )?;
        let pool_b_state = Self::build_pool_state(
            pool_b_id,
            &fresh_data_b,
            fresh_dlmm_b,
            ba_b.clone(),
            active_slot,
        )?;

        Ok((pool_a_state, pool_b_state, active_slot, ba_a, ba_b))
    }

    /// Execute a fresh-state arbitrage evaluation and simulation cycle with bounded retries.
    pub async fn execute_fresh_arbitrage_cycle(
        &self,
        candidate_id: &str,
        signer: &KeypairSigner,
        pool_a_id: Pubkey,
        pool_b_id: Pubkey,
    ) -> Result<MeteoraProfitabilityReport> {
        let mut last_report: Option<MeteoraProfitabilityReport> = None;
        let mut cached_pdas_holder: Option<(Vec<Pubkey>, Vec<Pubkey>)> = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let jitter = (attempt as u64) * 15;
                let backoff_ms = (self.config.base_backoff_ms * (1 << attempt) + jitter)
                    .min(self.config.max_backoff_ms);
                debug!(
                    "Freshness retry {}/{} after {}ms backoff...",
                    attempt, self.config.max_retries, backoff_ms
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }

            // 1. Ingest fresh batched pool states (uses cached PDAs after attempt 0 for single-shot RPC)
            let cached_ref = cached_pdas_holder
                .as_ref()
                .map(|(a, b)| (a.as_slice(), b.as_slice()));

            let (pool_a_state, pool_b_state, quote_slot, new_ba_a, new_ba_b) = match self
                .fetch_fresh_pool_pair(pool_a_id, pool_b_id, cached_ref)
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    warn!("Failed to fetch fresh pool states: {e}");
                    continue;
                }
            };
            cached_pdas_holder = Some((new_ba_a, new_ba_b));

            // 2. Discover & canonicalize pair
            let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&[
                pool_a_state.clone(),
                pool_b_state.clone(),
            ]);

            if report.distinct_eligible_pool_pairs == 0 {
                return Err(ArbError::InvalidInput("Pool pair is not eligible".into()));
            }

            let pair = &report.eligible_pairs[0];

            // 3. Optimize trade size on fresh state
            let opt_cfg = DlmmOptimizerConfig {
                min_input: self.config.min_input_amount,
                max_input: self.config.max_input_amount,
                coarse_samples: 8,
                max_candidate_basins: 4,
                refine_steps_per_basin: 6,
                estimated_network_cost_lamports: 15_000,
                sol_usd_price: self.config.sol_usd_price.unwrap_or(180),
                quote_decimals: 6,
                is_quote_sol: false,
            };

            let opt_res = MeteoraDlmmTradeSizeOptimizer::find_best_opportunity(pair, &opt_cfg);

            let (amount_in, hop1_out, final_out, swap_a_first, direction_str, quote_details) =
                if let Some(opt) = opt_res {
                    let is_a = opt.direction == arb_engine::DlmmArbitrageDirection::DirectionA;
                    (
                        opt.optimal_amount_in,
                        opt.expected_hop1_out,
                        opt.expected_final_out,
                        is_a,
                        if is_a { "DirectionA" } else { "DirectionB" },
                        Some(opt.quote),
                    )
                } else {
                    let q_a = MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a(
                        pair,
                        self.config.min_input_amount,
                    );
                    let q = q_a.unwrap_or(arb_engine::TwoHopDlmmQuote {
                        initial_amount_in: self.config.min_input_amount,
                        hop1_amount_out: 1_160_000,
                        hop2_final_amount_out: 99_000,
                        gross_profit: -1_000,
                        spread_bps: 0,
                        hop1_bins_traversed: 0,
                        hop2_bins_traversed: 0,
                        hop1_fee: 400,
                        hop2_fee: 400,
                        hop1_swap_for_y: false,
                        hop2_swap_for_y: true,
                    });
                    (
                        self.config.min_input_amount,
                        q.hop1_amount_out,
                        q.hop2_final_amount_out,
                        true,
                        "DirectionA",
                        Some(q),
                    )
                };

            // 4. Build atomic transaction with proportional slippage floor
            // In a two-hop atomic route where Hop 2's input is constrained to hop1_min_out (hop1_out * (1 - s)),
            // the expected output from hop1_min_out under Hop 2 slippage tolerance is scaled by (1 - s)^2.
            let slippage_factor = 10_000u64.saturating_sub(self.config.slippage_bps.min(10_000));
            let min_final_out = ((final_out as u128 * slippage_factor as u128 / 10_000)
                * slippage_factor as u128
                / 10_000) as u64;

            // In simulation, replaceRecentBlockhash is enabled so any valid 32-byte hash works
            // without paying a blocking RPC get_latest_blockhash roundtrip.
            let dummy_blockhash = [1u8; 32];

            let tx = {
                let hop1_bins = quote_details
                    .as_ref()
                    .map(|q| q.hop1_bins_traversed)
                    .unwrap_or(0);
                let hop2_bins = quote_details
                    .as_ref()
                    .map(|q| q.hop2_bins_traversed)
                    .unwrap_or(0);

                // Opt-in Transaction v1 path: same route, 4,096-byte packet, compute
                // budget and priority fee carried in the signed message header. Any v1
                // build failure falls back to the legacy packet rather than dropping the
                // candidate, because v1 support is a transport property, not a route one.
                let v1 = if self.config.two_pool_v1 {
                    let priority_fee_lamports = ((self.config.priority_fee_micro_lamports as u128)
                        .saturating_mul(self.config.cu_limit as u128)
                        .saturating_add(999_999))
                        / 1_000_000;
                    let v1_config = arb_tx::MeteoraTwoPoolV1Config {
                        compute_unit_limit: self.config.cu_limit,
                        priority_fee_lamports: u64::try_from(priority_fee_lamports)
                            .unwrap_or(u64::MAX),
                        loaded_accounts_data_size_limit:
                            arb_tx::METEORA_TWO_POOL_V1_DEFAULT_LOADED_DATA_LIMIT,
                    };
                    match MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1(
                        signer,
                        &pair.pool_a_state,
                        &pair.pool_b_state,
                        swap_a_first,
                        amount_in,
                        hop1_out,
                        min_final_out.max(1),
                        self.config.slippage_bps,
                        v1_config,
                        dummy_blockhash,
                        hop1_bins,
                        hop2_bins,
                    ) {
                        Ok(t) => Some(t),
                        Err(e) => {
                            warn!("V1 two-pool build failed, falling back to legacy packet: {e}");
                            None
                        }
                    }
                } else {
                    None
                };

                match v1 {
                    Some(t) => t,
                    None => match MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_with_traversal(
                        signer,
                        &pair.pool_a_state,
                        &pair.pool_b_state,
                        swap_a_first,
                        amount_in,
                        hop1_out,
                        min_final_out.max(1),
                        self.config.slippage_bps,
                        self.config.cu_limit,
                        self.config.priority_fee_micro_lamports,
                        dummy_blockhash,
                        hop1_bins,
                        hop2_bins,
                    ) {
                        Ok(t) => t,
                        Err(e) => {
                            warn!("Failed to build atomic transaction: {e}");
                            continue;
                        }
                    },
                }
            };

            // 5. Simulate immediately against Mainnet (non-broadcast)
            let priority_fee_lamports = (self.config.priority_fee_micro_lamports
                * (self.config.cu_limit as u64))
                / 1_000_000;
            let costs = ExecutionCostBreakdown::standard(
                priority_fee_lamports,
                0, // no jito tip in simulation
                0, // DEX fees are reflected in quote token outputs
                0,
                0,
            );

            let sim_start = std::time::Instant::now();
            let sim_record = match self
                .harness
                .simulate_candidate(
                    candidate_id,
                    &tx.wire_bytes,
                    tx.hop1_pool,
                    tx.hop2_pool,
                    direction_str,
                    amount_in,
                    hop1_out,
                    final_out,
                    (final_out as i64) - (amount_in as i64),
                    costs,
                    quote_slot,
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Simulation call error: {e}");
                    continue;
                }
            };
            let sim_latency_ms = sim_start.elapsed().as_millis() as u64;

            let sim_slot = sim_record.simulation_context_slot;
            let slot_lag = sim_slot.map(|s| s.saturating_sub(quote_slot));
            let is_fresh = slot_lag.is_some_and(|lag| lag <= self.config.max_allowed_slot_lag);

            let (h1_fee, h2_fee, swap_for_y1, swap_for_y2) = quote_details
                .as_ref()
                .map(|q| (q.hop1_fee, q.hop2_fee, q.hop1_swap_for_y, q.hop2_swap_for_y))
                .unwrap_or((0, 0, true, false));

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let quote_currency = Currency::Token(pair.canonical_mint_y);
            let quote_decimals = if quote_currency.is_usdc() {
                6
            } else if quote_currency.is_sol() {
                9
            } else {
                pair.pool_a_state.decimals_b.max(6)
            };

            let hop1_currency = Currency::Token(pair.canonical_mint_x);
            let hop1_decimals = if hop1_currency.is_usdc() {
                6
            } else if hop1_currency.is_sol() {
                9
            } else {
                pair.pool_a_state.decimals_a.max(6)
            };

            let initial_input = TypedTokenAmount::new(
                amount_in,
                quote_currency,
                quote_decimals,
                ConversionSource::Native,
                quote_slot,
                now_ms,
            );
            let hop1_amount = TypedTokenAmount::new(
                hop1_out,
                hop1_currency,
                hop1_decimals,
                ConversionSource::Native,
                quote_slot,
                now_ms,
            );
            let final_amount = TypedTokenAmount::new(
                final_out,
                quote_currency,
                quote_decimals,
                ConversionSource::Native,
                quote_slot,
                now_ms,
            );

            // Determine fee denomination based on collect_fee_mode and swap direction
            // Mode 0 = InputOnly (Token X if swap_for_y else Token Y), Mode 1 = OnlyY (always Token Y)
            let mode_a = pair
                .pool_a_state
                .dlmm
                .as_ref()
                .and_then(|d| d.params.as_ref())
                .map(|p| p.collect_fee_mode)
                .unwrap_or(0);
            let mode_b = pair
                .pool_b_state
                .dlmm
                .as_ref()
                .and_then(|d| d.params.as_ref())
                .map(|p| p.collect_fee_mode)
                .unwrap_or(0);

            let fee1_currency = if mode_a == 1 {
                quote_currency
            } else if swap_for_y1 {
                hop1_currency
            } else {
                quote_currency
            };
            let fee1_decimals = if fee1_currency == quote_currency {
                quote_decimals
            } else {
                hop1_decimals
            };

            let fee2_currency = if mode_b == 1 {
                quote_currency
            } else if swap_for_y2 {
                hop1_currency
            } else {
                quote_currency
            };
            let fee2_decimals = if fee2_currency == quote_currency {
                quote_decimals
            } else {
                hop1_decimals
            };

            let mut fee_items = vec![
                FeeItem::network_base_fee(costs.base_fee_lamports, quote_slot, now_ms),
                FeeItem::priority_fee(costs.priority_fee_lamports, quote_slot, now_ms),
            ];
            if costs.jito_tip_lamports > 0 {
                fee_items.push(FeeItem::jito_tip(
                    costs.jito_tip_lamports,
                    quote_slot,
                    now_ms,
                ));
            }
            if h1_fee > 0 {
                fee_items.push(FeeItem::dex_swap_fee(
                    "Hop 1 DLMM Pool Fee",
                    h1_fee,
                    fee1_currency,
                    fee1_decimals,
                    quote_slot,
                    now_ms,
                ));
            }
            if h2_fee > 0 {
                fee_items.push(FeeItem::dex_swap_fee(
                    "Hop 2 DLMM Pool Fee",
                    h2_fee,
                    fee2_currency,
                    fee2_decimals,
                    quote_slot,
                    now_ms,
                ));
            }

            let sol_to_quote_rate = if quote_currency.is_sol() {
                None
            } else if quote_currency.is_usdc() {
                let sol_price = self.config.sol_usd_price.unwrap_or(180);
                Some(
                    TimestampedRate::sol_to_usdc(
                        sol_price,
                        1,
                        quote_slot,
                        now_ms,
                        ConversionSource::ConfiguredRate {
                            label: "Conservative Sol-USDC Rate".into(),
                        },
                    )
                    .map_err(|e| ArbError::Internal(e.to_string()))?,
                )
            } else {
                None
            };

            let dim_report = DimensionalProfitabilityReport::compute(
                candidate_id,
                &format!("Meteora DLMM Cross-Pool ({} -> {})", pool_a_id, pool_b_id),
                initial_input,
                hop1_amount,
                final_amount,
                fee_items,
                sol_to_quote_rate,
                quote_slot,
            )
            .map_err(|e| ArbError::Internal(e.to_string()))?;

            let gross_spread = dim_report.gross_profit_quote_raw;
            let total_costs = dim_report.total_execution_cost_quote_raw;
            let net_profit = dim_report.net_profit_quote_raw;
            let net_bps = dim_report.net_profit_bps;

            let status = if !sim_record.simulation_success {
                "REJECTED_SIM_FAIL".to_string()
            } else if !is_fresh {
                "STALE_QUOTE".to_string()
            } else if dim_report.is_profitable {
                "VERIFIED_PROFITABLE".to_string()
            } else {
                "SIMULATED_SUCCESS_NEGATIVE_EDGE".to_string()
            };

            let report = MeteoraProfitabilityReport {
                candidate_id: candidate_id.to_string(),
                pool_a: pool_a_id,
                pool_b: pool_b_id,
                direction: direction_str.to_string(),
                quote_mint: quote_currency,
                quote_decimals,
                initial_quote_amount: amount_in,
                hop1_amount_out: hop1_out,
                hop2_final_amount_out: final_out,
                gross_spread_quote_raw: gross_spread,
                dex_fees_hop1_raw: h1_fee,
                dex_fees_hop1_currency: fee1_currency,
                dex_fees_hop2_raw: h2_fee,
                dex_fees_hop2_currency: fee2_currency,
                total_dex_fees_raw: h1_fee.saturating_add(h2_fee),
                transfer_fees_raw: 0,
                base_fee_lamports: costs.base_fee_lamports,
                priority_fee_lamports: costs.priority_fee_lamports,
                jito_tip_lamports: costs.jito_tip_lamports,
                refundable_rent_lamports: 0,
                total_network_fees_sol_lamports: costs
                    .base_fee_lamports
                    .saturating_add(costs.priority_fee_lamports)
                    .saturating_add(costs.jito_tip_lamports),
                total_execution_cost_quote_raw: total_costs,
                net_expected_profit_quote_raw: net_profit,
                net_profit_bps: net_bps,
                quote_slot,
                simulation_slot: sim_slot,
                slot_lag,
                is_fresh,
                retries_used: attempt,
                cu_consumed: sim_record.compute_units_consumed,
                tx_size_bytes: tx.tx_size,
                tx_version: format!("{:?}", tx.version),
                simulation_success: sim_record.simulation_success,
                rejection_reason: if !sim_record.simulation_success {
                    sim_record.error_status
                } else if !is_fresh {
                    Some(format!(
                        "StaleQuote: quote_slot={}, sim_slot={:?}, lag={:?} > max={}",
                        quote_slot, sim_slot, slot_lag, self.config.max_allowed_slot_lag
                    ))
                } else {
                    None
                },
                status,
                dimensional_report: dim_report,
            };

            // Persist candidate opportunity record for historical tracking and shadow replay
            let recorder =
                crate::opportunity_recorder::OpportunityHistoryRecorder::new("logs/opportunities");
            let opp_record = crate::opportunity_recorder::CandidateOpportunityRecord {
                candidate_id: candidate_id.to_string(),
                detection_timestamp_ms: now_ms,
                pool_a_id,
                pool_b_id,
                pool_a_slot: pair.pool_a_state.slot.0,
                pool_b_slot: pair.pool_b_state.slot.0,
                quote_slot,
                pool_a_state: pair.pool_a_state.clone(),
                pool_b_state: pair.pool_b_state.clone(),
                direction: direction_str.to_string(),
                trade_size_in: amount_in,
                hop1_quote_out: hop1_out,
                hop2_final_out: final_out,
                fee_items: report.dimensional_report.fee_items.clone(),
                gross_profit_quote_raw: gross_spread,
                total_execution_cost_quote_raw: total_costs,
                net_profit_quote_raw: net_profit,
                net_profit_bps: net_bps,
                simulation_success: sim_record.simulation_success,
                simulation_slot: sim_slot,
                simulation_latency_ms: sim_latency_ms,
                quote_freshness_cleared: is_fresh,
                disappearance_time_ms: Some(now_ms.saturating_add(sim_latency_ms)),
                candidate_lifetime_ms: Some(sim_latency_ms),
                best_observed_net_profit: Some(net_profit),
                rejection_reason: report.rejection_reason.clone(),
                status: report.status.clone(),
            };
            let _ = recorder.record_candidate(&opp_record);

            if is_fresh || attempt == self.config.max_retries {
                return Ok(report);
            }

            last_report = Some(report);
        }

        last_report.ok_or_else(|| ArbError::SimulationFailed("Pipeline retries exhausted".into()))
    }

    fn extract_account_data(val: &serde_json::Value) -> Result<Vec<u8>> {
        let data_arr = val
            .pointer("/data")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ArbError::DeserializationError("missing data array".into()))?;

        let b64_str = data_arr[0]
            .as_str()
            .ok_or_else(|| ArbError::DeserializationError("missing base64 data string".into()))?;

        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(b64_str)
            .map_err(|e| ArbError::DeserializationError(format!("bad base64: {e}")))
    }

    fn build_pool_state(
        pool_id: Pubkey,
        data: &[u8],
        dlmm: arb_types::DlmmState,
        bin_array_pdas: Vec<Pubkey>,
        slot: u64,
    ) -> Result<PoolState> {
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey slice".into()))?;
            Ok(Pubkey::new_from_array(b))
        };

        // LbPair layout:
        // token_x_mint @88, token_y_mint @120, reserve_x @152, reserve_y @184, oracle @216
        let mint_x = read_pk(88)?;
        let mint_y = read_pk(120)?;
        let reserve_x = read_pk(152)?;
        let reserve_y = read_pk(184)?;
        let oracle = read_pk(216)?;

        let exec = PoolExecutionAccounts::MeteoraDlmm {
            reserve_x,
            reserve_y,
            oracle,
            bin_array_pdas,
            bitmap_extension: None,
            pool_token_x: mint_x,
            pool_token_y: mint_y,
        };

        Ok(PoolState {
            pool_id: PoolId(pool_id),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(slot),
            write_version: 1,
            base_mint: MintId(mint_x),
            quote_mint: MintId(mint_y),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 50_000_000_000,
            reserve_out: 50_000_000_000,
            fee_numerator: dlmm.bin_step_bps as u64,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(dlmm),
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            execution_accounts: Some(exec),
            last_update_slot: slot,
            last_update_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
    }
}
