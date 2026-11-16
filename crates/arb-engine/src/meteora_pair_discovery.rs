//! Meteora DLMM Pool Pair Discovery (`MeteoraDlmmPoolPairDiscovery`).
//!
//! Canonicalizes token pairs so that Token X / Token Y and Token Y / Token X
//! are recognized as the same underlying pair, while preserving the real direction
//! of each pool.
//!
//! Enforces:
//! - `pool_a.pubkey != pool_b.pubkey`
//! - Valid program ownership (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`)
//! - Active pools (`is_active == true`)
//! - Non-zero liquidity and valid DLMM state
//! - Token program compatibility across pools
//! - Resolved execution accounts (`execution_accounts.is_some()`)

use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, PoolState, TokenProgramKind};
use std::collections::{HashMap, HashSet};

/// Pair of two distinct Meteora DLMM pools sharing the same underlying token pair.
#[derive(Debug, Clone)]
pub struct MeteoraDlmmEligiblePair {
    pub pool_a: PoolId,
    pub pool_b: PoolId,
    pub canonical_mint_x: MintId,
    pub canonical_mint_y: MintId,
    pub token_programs: [TokenProgramKind; 2],
    pub pool_a_active_id: i32,
    pub pool_b_active_id: i32,
    pub pool_a_bin_step: u16,
    pub pool_b_bin_step: u16,
    pub pool_a_fee_bps: u16,
    pub pool_b_fee_bps: u16,
    pub pool_a_reserve_in: u128,
    pub pool_a_reserve_out: u128,
    pub pool_b_reserve_in: u128,
    pub pool_b_reserve_out: u128,
    pub pool_a_execution_accounts: PoolExecutionAccounts,
    pub pool_b_execution_accounts: PoolExecutionAccounts,
    pub pool_a_state: PoolState,
    pub pool_b_state: PoolState,
}

/// Rejection reason for a pool during pair discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoolRejectionReason {
    NotMeteoraDlmm,
    InactivePool,
    MissingDlmmState,
    MissingExecutionAccounts,
    ZeroLiquidity,
    TokenProgramMismatch,
    RiskyTokenOrTransferFee,
}

impl PoolRejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotMeteoraDlmm => "NOT_METEORA_DLMM",
            Self::InactivePool => "INACTIVE_POOL",
            Self::MissingDlmmState => "MISSING_DLMM_STATE",
            Self::MissingExecutionAccounts => "MISSING_EXECUTION_ACCOUNTS",
            Self::ZeroLiquidity => "ZERO_LIQUIDITY",
            Self::TokenProgramMismatch => "TOKEN_PROGRAM_MISMATCH",
            Self::RiskyTokenOrTransferFee => "RISKY_TOKEN_OR_TRANSFER_FEE",
        }
    }
}

/// Aggregate report of the Meteora DLMM pool pair discovery process.
#[derive(Debug, Clone, Default)]
pub struct MeteoraDlmmPairDiscoveryReport {
    pub total_meteora_pools_discovered: usize,
    pub active_and_valid_pools: usize,
    pub unique_token_pairs: usize,
    pub distinct_eligible_pool_pairs: usize,
    pub rejections_by_reason: HashMap<&'static str, usize>,
    pub eligible_pairs: Vec<MeteoraDlmmEligiblePair>,
}

/// Core engine for canonical pool pair discovery.
pub struct MeteoraDlmmPoolPairDiscovery;

impl MeteoraDlmmPoolPairDiscovery {
    /// Canonicalize a token pair into a deterministic unordered key `(min, max)`.
    pub fn canonical_pair_key(mint_a: MintId, mint_b: MintId) -> (MintId, MintId) {
        if mint_a.0 <= mint_b.0 {
            (mint_a, mint_b)
        } else {
            (mint_b, mint_a)
        }
    }

    /// Discover and pair distinct Meteora DLMM pools sharing the same underlying token pair.
    pub fn discover_pairs(pools: &[PoolState]) -> MeteoraDlmmPairDiscoveryReport {
        let mut report = MeteoraDlmmPairDiscoveryReport::default();
        let mut valid_pools_by_pair: HashMap<(MintId, MintId), Vec<PoolState>> = HashMap::new();

        for pool in pools {
            if pool.protocol != DexProtocol::MeteoraDlmm {
                continue;
            }
            report.total_meteora_pools_discovered += 1;

            if !pool.is_active {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::InactivePool.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            if pool.dlmm.is_none() {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::MissingDlmmState.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            if pool.has_risky_token || pool.transfer_fee_bps != 0 {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::RiskyTokenOrTransferFee.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            if pool
                .dlmm
                .as_ref()
                .and_then(|d| d.params.as_ref())
                .is_none_or(|params| params.function_type != 0)
            {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::MissingDlmmState.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            if pool.execution_accounts.is_none() {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::MissingExecutionAccounts.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            if pool.reserve_in == 0 && pool.reserve_out == 0 {
                *report
                    .rejections_by_reason
                    .entry(PoolRejectionReason::ZeroLiquidity.as_str())
                    .or_insert(0) += 1;
                continue;
            }

            report.active_and_valid_pools += 1;
            let pair_key = Self::canonical_pair_key(pool.base_mint, pool.quote_mint);
            valid_pools_by_pair
                .entry(pair_key)
                .or_default()
                .push(pool.clone());
        }

        report.unique_token_pairs = valid_pools_by_pair.len();

        let mut seen_pairs: HashSet<(PoolId, PoolId)> = HashSet::new();

        for ((mint_x, mint_y), pool_list) in valid_pools_by_pair {
            if pool_list.len() < 2 {
                continue;
            }

            for i in 0..pool_list.len() {
                for j in (i + 1)..pool_list.len() {
                    let pool_a = &pool_list[i];
                    let pool_b = &pool_list[j];

                    if pool_a.pool_id == pool_b.pool_id {
                        continue;
                    }

                    // Enforce deterministic unordered pair deduplication
                    let pair_id_key = if pool_a.pool_id.0 < pool_b.pool_id.0 {
                        (pool_a.pool_id, pool_b.pool_id)
                    } else {
                        (pool_b.pool_id, pool_a.pool_id)
                    };
                    if !seen_pairs.insert(pair_id_key) {
                        continue;
                    }

                    // Validate token program compatibility
                    let prog_a = pool_a.token_programs;
                    let prog_b = pool_b.token_programs;
                    // Token programs for same mint must match across pools
                    let prog_x_a = if pool_a.base_mint == mint_x {
                        prog_a[0]
                    } else {
                        prog_a[1]
                    };
                    let prog_x_b = if pool_b.base_mint == mint_x {
                        prog_b[0]
                    } else {
                        prog_b[1]
                    };
                    let prog_y_a = if pool_a.quote_mint == mint_y {
                        prog_a[1]
                    } else {
                        prog_a[0]
                    };
                    let prog_y_b = if pool_b.quote_mint == mint_y {
                        prog_b[1]
                    } else {
                        prog_b[0]
                    };

                    if prog_x_a != prog_x_b || prog_y_a != prog_y_b {
                        *report
                            .rejections_by_reason
                            .entry(PoolRejectionReason::TokenProgramMismatch.as_str())
                            .or_insert(0) += 1;
                        continue;
                    }

                    let dlmm_a = pool_a.dlmm.as_ref().unwrap();
                    let dlmm_b = pool_b.dlmm.as_ref().unwrap();
                    let exec_a = pool_a.execution_accounts.as_ref().unwrap().clone();
                    let exec_b = pool_b.execution_accounts.as_ref().unwrap().clone();

                    report.eligible_pairs.push(MeteoraDlmmEligiblePair {
                        pool_a: pool_a.pool_id,
                        pool_b: pool_b.pool_id,
                        canonical_mint_x: mint_x,
                        canonical_mint_y: mint_y,
                        token_programs: [prog_x_a, prog_y_a],
                        pool_a_active_id: dlmm_a.active_bin_id,
                        pool_b_active_id: dlmm_b.active_bin_id,
                        pool_a_bin_step: dlmm_a.bin_step_bps,
                        pool_b_bin_step: dlmm_b.bin_step_bps,
                        pool_a_fee_bps: pool_a.fee_numerator as u16,
                        pool_b_fee_bps: pool_b.fee_numerator as u16,
                        pool_a_reserve_in: pool_a.reserve_in,
                        pool_a_reserve_out: pool_a.reserve_out,
                        pool_b_reserve_in: pool_b.reserve_in,
                        pool_b_reserve_out: pool_b.reserve_out,
                        pool_a_execution_accounts: exec_a,
                        pool_b_execution_accounts: exec_b,
                        pool_a_state: pool_a.clone(),
                        pool_b_state: pool_b.clone(),
                    });
                }
            }
        }

        report.distinct_eligible_pool_pairs = report.eligible_pairs.len();
        report
    }
}
