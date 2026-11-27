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
    InvalidFeeRatio,
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
            Self::InvalidFeeRatio => "INVALID_FEE_RATIO",
        }
    }
}

/// Conservative, ceiling-rounded fee metadata shared with all graph paths.
/// Exact quote math continues to use the PoolState numerator/denominator pair.
fn pool_fee_bps(pool: &PoolState) -> Option<u16> {
    pool.fee_bps()
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
                // 1 (LiquidityMining) is quote-equivalent to 0; 2+ fail-closed,
                // matching the LbPair-validation gate in arb-dex.
                .is_none_or(|params| params.function_type > 1)
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

                    let (Some(pool_a_fee_bps), Some(pool_b_fee_bps)) =
                        (pool_fee_bps(pool_a), pool_fee_bps(pool_b))
                    else {
                        *report
                            .rejections_by_reason
                            .entry(PoolRejectionReason::InvalidFeeRatio.as_str())
                            .or_insert(0) += 1;
                        continue;
                    };

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
                        pool_a_fee_bps,
                        pool_b_fee_bps,
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

/// Cross-protocol pair of two distinct pools sharing the same underlying token pair.
/// Connects Meteora (DLMM, DAMM v2) with Raydium (AMM v4, CPMM, CLMM), Pump.fun / PumpSwap,
/// and internal Meteora cross-pool pairs (DLMM <-> DAMM v2).
#[derive(Debug, Clone)]
pub struct CrossProtocolEligiblePair {
    pub pool_a: PoolId,
    pub protocol_a: DexProtocol,
    pub pool_b: PoolId,
    pub protocol_b: DexProtocol,
    pub canonical_mint_x: MintId,
    pub canonical_mint_y: MintId,
    pub token_programs: [TokenProgramKind; 2],
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

#[derive(Debug, Clone, Default)]
pub struct CrossProtocolPairDiscoveryReport {
    pub total_pools_evaluated: usize,
    pub active_and_valid_pools: usize,
    pub unique_token_pairs: usize,
    pub distinct_eligible_pairs: usize,
    pub rejections_by_reason: HashMap<&'static str, usize>,
    pub eligible_pairs: Vec<CrossProtocolEligiblePair>,
}

/// Cross-protocol pair discovery connecting Meteora with the entire DEX ecosystem.
pub struct CrossProtocolPairDiscovery;

impl CrossProtocolPairDiscovery {
    pub fn discover_pairs(pools: &[PoolState]) -> CrossProtocolPairDiscoveryReport {
        let mut report = CrossProtocolPairDiscoveryReport::default();
        let mut valid_pools_by_pair: HashMap<(MintId, MintId), Vec<PoolState>> = HashMap::new();

        for pool in pools {
            report.total_pools_evaluated += 1;
            if !pool.is_active || pool.has_risky_token || pool.transfer_fee_bps != 0 {
                continue;
            }
            if pool.execution_accounts.is_none() {
                continue;
            }
            if pool.reserve_in == 0 && pool.reserve_out == 0 {
                continue;
            }

            report.active_and_valid_pools += 1;
            let pair_key =
                MeteoraDlmmPoolPairDiscovery::canonical_pair_key(pool.base_mint, pool.quote_mint);
            valid_pools_by_pair
                .entry(pair_key)
                .or_default()
                .push(pool.clone());
        }

        report.unique_token_pairs = valid_pools_by_pair.len();

        for ((mint_x, mint_y), candidate_pools) in &valid_pools_by_pair {
            if candidate_pools.len() < 2 {
                continue;
            }

            for i in 0..candidate_pools.len() {
                for j in (i + 1)..candidate_pools.len() {
                    let pool_a = &candidate_pools[i];
                    let pool_b = &candidate_pools[j];

                    if pool_a.pool_id == pool_b.pool_id {
                        continue;
                    }

                    // Must involve at least one Meteora pool (DLMM or DAMM v2)
                    let is_meteora_a = matches!(
                        pool_a.protocol,
                        DexProtocol::MeteoraDlmm | DexProtocol::MeteoraDammV2
                    );
                    let is_meteora_b = matches!(
                        pool_b.protocol,
                        DexProtocol::MeteoraDlmm | DexProtocol::MeteoraDammV2
                    );
                    if !is_meteora_a && !is_meteora_b {
                        continue;
                    }

                    let prog_a = pool_a.token_programs;
                    let prog_b = pool_b.token_programs;
                    let prog_x_a = if pool_a.base_mint == *mint_x {
                        prog_a[0]
                    } else {
                        prog_a[1]
                    };
                    let prog_x_b = if pool_b.base_mint == *mint_x {
                        prog_b[0]
                    } else {
                        prog_b[1]
                    };
                    let prog_y_a = if pool_a.quote_mint == *mint_y {
                        prog_a[1]
                    } else {
                        prog_a[0]
                    };
                    let prog_y_b = if pool_b.quote_mint == *mint_y {
                        prog_b[1]
                    } else {
                        prog_b[0]
                    };

                    if prog_x_a != prog_x_b || prog_y_a != prog_y_b {
                        continue;
                    }

                    let exec_a = pool_a.execution_accounts.as_ref().unwrap().clone();
                    let exec_b = pool_b.execution_accounts.as_ref().unwrap().clone();

                    let (Some(pool_a_fee_bps), Some(pool_b_fee_bps)) =
                        (pool_fee_bps(pool_a), pool_fee_bps(pool_b))
                    else {
                        *report
                            .rejections_by_reason
                            .entry(PoolRejectionReason::InvalidFeeRatio.as_str())
                            .or_insert(0) += 1;
                        continue;
                    };

                    report.eligible_pairs.push(CrossProtocolEligiblePair {
                        pool_a: pool_a.pool_id,
                        protocol_a: pool_a.protocol,
                        pool_b: pool_b.pool_id,
                        protocol_b: pool_b.protocol,
                        canonical_mint_x: *mint_x,
                        canonical_mint_y: *mint_y,
                        token_programs: [prog_x_a, prog_y_a],
                        pool_a_fee_bps,
                        pool_b_fee_bps,
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

        report.distinct_eligible_pairs = report.eligible_pairs.len();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{Slot, TokenProgramKind};
    use solana_pubkey::Pubkey;

    #[test]
    fn test_cross_protocol_pair_discovery_matches_meteora_with_raydium() {
        let mint_x = MintId(Pubkey::new_unique());
        let mint_y = MintId(Pubkey::new_unique());

        let pool_dlmm = PoolState {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(100),
            write_version: 1,
            base_mint: mint_x,
            quote_mint: mint_y,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 10_000_000_000,
            reserve_out: 50_000_000_000,
            fee_numerator: 2_500,
            fee_denominator: 1_000_000,
            raydium_cpmm_fee: Some(arb_types::RaydiumCpmmFeeState {
                trade_fee_rate_ppm: 2_500,
                creator_fee_rate_ppm: 0,
                protocol_fee_rate: 120_000,
                fund_fee_rate: 40_000,
                enable_creator_fee: false,
                creator_fee_on: 0,
            }),
            is_active: true,
            has_risky_token: false,
            kind: arb_types::PoolKind::Dlmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            token2022_fee_context: None,
            execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                bin_array_pdas: vec![Pubkey::new_unique()],
                bitmap_extension: None,
                pool_token_x: mint_x.0,
                pool_token_y: mint_y.0,
            }),
            last_update_slot: 100,
            last_update_ms: 1000,
        };

        let pool_raydium = PoolState {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::RaydiumCpmm,
            slot: Slot(100),
            write_version: 1,
            base_mint: mint_x,
            quote_mint: mint_y,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 12_000_000_000,
            reserve_out: 48_000_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            token2022_fee_context: None,
            execution_accounts: Some(PoolExecutionAccounts::RaydiumCpmm {
                amm_config: Pubkey::new_unique(),
                authority: Pubkey::new_unique(),
                vault_0: Pubkey::new_unique(),
                vault_1: Pubkey::new_unique(),
                observation_state: Pubkey::new_unique(),
            }),
            last_update_slot: 100,
            last_update_ms: 1000,
        };

        let report = CrossProtocolPairDiscovery::discover_pairs(&[pool_dlmm, pool_raydium]);
        assert_eq!(report.distinct_eligible_pairs, 1);
        let pair = &report.eligible_pairs[0];
        assert_eq!(pair.protocol_a, DexProtocol::MeteoraDlmm);
        assert_eq!(pair.protocol_b, DexProtocol::RaydiumCpmm);
    }

    // DATA-03 regression: fee_bps must be derived from the fee RATIO
    // (numerator/denominator scaled to bps), not from the raw numerator.
    fn dlmm_pool_with_fee(
        pool_id: PoolId,
        mint_x: MintId,
        mint_y: MintId,
        fee_numerator: u64,
        fee_denominator: u64,
    ) -> PoolState {
        PoolState {
            pool_id,
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(100),
            write_version: 1,
            base_mint: mint_x,
            quote_mint: mint_y,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 10_000_000_000,
            reserve_out: 50_000_000_000,
            fee_numerator,
            fee_denominator,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: arb_types::PoolKind::Dlmm,
            clmm: None,
            dlmm: Some(arb_types::DlmmState {
                active_bin_id: 0,
                status: 0,
                bin_step_bps: 25,
                params: Some(arb_types::DlmmFeeParams {
                    function_type: 0,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            token2022_fee_context: None,
            execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                bin_array_pdas: vec![Pubkey::new_unique()],
                bitmap_extension: None,
                pool_token_x: mint_x.0,
                pool_token_y: mint_y.0,
            }),
            last_update_slot: 100,
            last_update_ms: 1000,
        }
    }

    // A museum-style ppm fee (2,500 / 1,000,000 = 0.25% = 25 bps) previously
    // surfaced as 2,500 "bps" (25%) from the raw `as u16` cast of the numerator.
    #[test]
    fn dlmm_pair_fee_bps_scales_with_denominator() {
        let mint_x = MintId(Pubkey::new_unique());
        let mint_y = MintId(Pubkey::new_unique());
        let pool_a = dlmm_pool_with_fee(
            PoolId(Pubkey::new_unique()),
            mint_x,
            mint_y,
            2_500,
            1_000_000,
        );
        let pool_b = dlmm_pool_with_fee(PoolId(Pubkey::new_unique()), mint_x, mint_y, 25, 10_000);

        let dlmm_report =
            MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool_a.clone(), pool_b.clone()]);
        assert_eq!(dlmm_report.distinct_eligible_pool_pairs, 1);
        let pair = &dlmm_report.eligible_pairs[0];
        assert_eq!(pair.pool_a_fee_bps, 25);
        assert_eq!(pair.pool_b_fee_bps, 25);
        assert_eq!(
            dlmm_report
                .rejections_by_reason
                .get("INVALID_FEE_RATIO")
                .copied()
                .unwrap_or(0),
            0
        );

        let cp_report = CrossProtocolPairDiscovery::discover_pairs(&[pool_a, pool_b]);
        assert_eq!(cp_report.distinct_eligible_pairs, 1);
        assert_eq!(cp_report.eligible_pairs[0].pool_a_fee_bps, 25);
        assert_eq!(cp_report.eligible_pairs[0].pool_b_fee_bps, 25);
    }

    #[test]
    fn dlmm_pair_sub_basis_point_fee_rounds_up_for_graph_metadata() {
        let mint_x = MintId(Pubkey::new_unique());
        let mint_y = MintId(Pubkey::new_unique());
        let pool_a = dlmm_pool_with_fee(
            PoolId(Pubkey::new_from_array([1; 32])),
            mint_x,
            mint_y,
            50,
            1_000_000,
        );
        let pool_b = dlmm_pool_with_fee(
            PoolId(Pubkey::new_from_array([2; 32])),
            mint_x,
            mint_y,
            25,
            10_000,
        );
        let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool_a, pool_b]);
        assert_eq!(report.distinct_eligible_pool_pairs, 1);
        assert_eq!(report.eligible_pairs[0].pool_a_fee_bps, 1);
        assert_eq!(report.eligible_pairs[0].pool_b_fee_bps, 25);
    }
    // A zero (or unrepresentable) fee denominator must fail closed: no pair,
    // and the rejection is counted under INVALID_FEE_RATIO.
    #[test]
    fn dlmm_pair_with_zero_fee_denominator_is_rejected() {
        let mint_x = MintId(Pubkey::new_unique());
        let mint_y = MintId(Pubkey::new_unique());
        let pool_a = dlmm_pool_with_fee(PoolId(Pubkey::new_unique()), mint_x, mint_y, 25, 0);
        let pool_b = dlmm_pool_with_fee(PoolId(Pubkey::new_unique()), mint_x, mint_y, 25, 10_000);

        let dlmm_report =
            MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool_a.clone(), pool_b.clone()]);
        assert_eq!(dlmm_report.distinct_eligible_pool_pairs, 0);
        assert_eq!(
            dlmm_report
                .rejections_by_reason
                .get("INVALID_FEE_RATIO")
                .copied()
                .unwrap_or(0),
            1
        );

        let cp_report = CrossProtocolPairDiscovery::discover_pairs(&[pool_a, pool_b]);
        assert_eq!(cp_report.distinct_eligible_pairs, 0);
        assert_eq!(
            cp_report
                .rejections_by_reason
                .get("INVALID_FEE_RATIO")
                .copied()
                .unwrap_or(0),
            1
        );
    }
}
