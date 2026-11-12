//! Phase 17 — Flash-loan funded arbitrage between two distinct Meteora DLMM pools.
//!
//! The route is:
//!
//! ```text
//! Kamino flash borrow (flash mint)
//!   -> Meteora DLMM pool A : flash mint -> mid mint
//!   -> Meteora DLMM pool B : mid mint  -> flash mint
//!   -> Kamino flash repay  (principal + fee)
//! ```
//!
//! Everything in this module is integer / fixed point. No floating point value is
//! ever used to decide an amount, a fee or a profit.
//!
//! Design decisions that matter:
//!
//! * **Mint ordering is read from chain, never assumed.** A pool is described by its
//!   on-chain `(token_x_mint, token_y_mint)`; the hop direction is derived per pool.
//!   A pool whose on-chain ordering is (USDC, WSOL) is handled by the same code.
//! * **The intermediate amount is the real one.** Pool B is quoted with the amount
//!   Pool A actually produces, not a placeholder.
//! * **Profit is not assumed monotonic.** The size search is a coarse
//!   logarithmic/linear grid followed by multi-basin local-maximum detection and a
//!   bounded integer fine sweep.
//! * **Everything is fail closed.** A missing bin array, an over-sized transaction,
//!   an exceeded compute budget, a quote failure or a profit below the floor all
//!   reject the candidate; none of them is silently truncated.

use arb_math::dlmm::{
    DlmmBin as MathDlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
    dlmm_active_bin_max_input, quote_dlmm_view,
};
use arb_tx::dlmm_bin_discovery::{DlmmBinArrayPlan, plan_bin_arrays};
use arb_tx::dlmm_flash_arb::{
    FlashArbParams, FlashArbPoolLeg, assemble_flash_funded_two_pool, estimate_v0_transaction_size,
};
use arb_tx::{AddressLookupTable, KaminoFlashParams, estimate_minimal_cu};
use arb_types::{MintId, PoolState, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Per-bin-array compute-unit overhead used by the size estimate.
pub const CU_PER_BIN_ARRAY: u32 = 15_000;

/// Maximum number of addresses a single address lookup table can index.
pub const MAX_ALT_ADDRESSES: usize = 256;

/// Explicit allowlist of the Meteora DLMM pools the Phase 17 route may trade.
///
/// The route never discovers a pool on its own: only pools listed here are
/// considered, and each one must additionally pass on-chain validation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DlmmPoolAllowlist {
    pub pools: Vec<Pubkey>,
}

impl DlmmPoolAllowlist {
    /// Build an allowlist from pool addresses.
    pub fn new(pools: Vec<Pubkey>) -> Self {
        Self { pools }
    }

    /// Build an allowlist from base58 addresses, skipping unparsable entries.
    pub fn from_base58(list: &[&str]) -> Self {
        Self {
            pools: list
                .iter()
                .filter_map(|s| Pubkey::from_str(s.trim()).ok())
                .collect(),
        }
    }

    /// True when the pool is on the allowlist.
    pub fn contains(&self, pool: &Pubkey) -> bool {
        self.pools.contains(pool)
    }

    /// Number of allowlisted pools.
    pub fn len(&self) -> usize {
        self.pools.len()
    }

    /// True when the allowlist is empty.
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }
}

/// One Meteora DLMM pool prepared for the flash-funded route.
///
/// The pool carries its **on-chain** X/Y ordering explicitly. The canonical
/// `PoolState::base_mint` / `quote_mint` pair comes from a discovery source and
/// does not necessarily match the on-chain ordering, so it is never used to decide
/// a swap direction.
#[derive(Debug, Clone)]
pub struct DlmmFlashArbPool {
    pub pool_id: Pubkey,
    /// Canonical pool state (bins, fee parameters, volatility).
    pub state: PoolState,
    /// On-chain token X mint.
    pub token_x_mint: MintId,
    /// On-chain token Y mint.
    pub token_y_mint: MintId,
    /// On-chain token X reserve vault.
    pub reserve_x: Pubkey,
    /// On-chain token Y reserve vault.
    pub reserve_y: Pubkey,
    /// On-chain oracle PDA.
    pub oracle: Pubkey,
    /// Token program owning token X.
    pub token_x_program: TokenProgramKind,
    /// Token program owning token Y.
    pub token_y_program: TokenProgramKind,
    /// Token-2022 transfer fee in bps, `[token_x, token_y]`.
    pub transfer_fee_bps: [u16; 2],
}

/// Result of quoting one DLMM hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmHopQuote {
    /// Amount paid into the hop (before the input transfer fee).
    pub amount_in: u64,
    /// Amount delivered to the user (after the output transfer fee).
    pub amount_out: u64,
    /// Pool output before the output transfer fee.
    pub gross_out: u64,
    /// Trading fee charged by the pool.
    pub fee_amount: u64,
    /// Protocol fee portion of `fee_amount`.
    pub protocol_fee: u64,
    /// Bins crossed by the traversal.
    pub bins_crossed: u32,
    /// Active bin after the traversal.
    pub end_active_id: i32,
    /// `true` when the hop is X -> Y.
    pub swap_for_y: bool,
}

impl DlmmFlashArbPool {
    /// Token-2022 transfer fee for `mint`.
    pub fn transfer_fee_for(&self, mint: &MintId) -> u16 {
        if self.token_x_mint == *mint {
            self.transfer_fee_bps[0]
        } else if self.token_y_mint == *mint {
            self.transfer_fee_bps[1]
        } else {
            0
        }
    }

    /// Swap direction for `from -> to`, resolved against the on-chain ordering.
    ///
    /// Returns `None` when the pool does not hold `from`.
    pub fn swap_for_y(&self, from: &MintId, to: &MintId) -> Option<bool> {
        if self.token_x_mint == *from && self.token_y_mint == *to {
            Some(true)
        } else if self.token_y_mint == *from && self.token_x_mint == *to {
            Some(false)
        } else {
            None
        }
    }

    /// Maximum input the pool's active bin can absorb in the `from -> to` direction.
    pub fn active_bin_max_input(&self, from: &MintId, to: &MintId) -> Option<u64> {
        let swap_for_y = self.swap_for_y(from, to)?;
        let dlmm = self.state.dlmm.as_ref()?;
        let active = dlmm.bins.iter().find(|b| b.bin_id == dlmm.active_bin_id)?;
        dlmm_active_bin_max_input(
            dlmm.active_bin_id,
            dlmm.bin_step_bps,
            u64::try_from(active.amount_x).ok()?,
            u64::try_from(active.amount_y).ok()?,
            swap_for_y,
        )
    }

    /// Quote an exact-in hop `from -> to`.
    ///
    /// Requires legacy SPL mints until the complete epoch-aware Token-2022
    /// transfer-fee and transfer-hook context is part of the route snapshot.
    pub fn quote(&self, amount_in: u64, from: &MintId, to: &MintId) -> Option<DlmmHopQuote> {
        if amount_in == 0 {
            return None;
        }
        let swap_for_y = self.swap_for_y(from, to)?;
        let dlmm = self.state.dlmm.as_ref()?;
        let fp = dlmm.params.as_ref()?;
        if dlmm.status != 0
            || fp.function_type != 0
            || fp.collect_fee_mode > 1
            || self.state.has_risky_token
            || self.state.transfer_fee_bps != 0
            || self.transfer_fee_bps != [0, 0]
        {
            return None;
        }
        // Do not approximate Token-2022 with BPS and an unbounded maximum fee.
        // This route accepts legacy SPL mints only until epoch-aware mint configs
        // and transfer-hook account metadata are included in the snapshot.
        let net_in = amount_in;

        let pool_state = DlmmPoolState {
            active_id: dlmm.active_bin_id,
            bin_step: dlmm.bin_step_bps,
            status: dlmm.status,
            parameters: DlmmParameters {
                bin_step: dlmm.bin_step_bps,
                base_factor: fp.base_factor,
                filter_period: fp.filter_period,
                decay_period: fp.decay_period,
                reduction_factor: fp.reduction_factor,
                variable_fee_control: fp.variable_fee_control,
                max_volatility_accumulator: fp.max_volatility_accumulator,
                protocol_share: fp.protocol_share,
                base_fee_power_factor: fp.base_fee_power_factor,
                collect_fee_mode: fp.collect_fee_mode,
            },
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: dlmm.volatility_accumulator,
                volatility_reference: dlmm.volatility_reference,
                index_reference: dlmm.index_reference,
                last_update_timestamp: dlmm.last_update_timestamp,
            },
        };

        let mut bins: Vec<MathDlmmBin> = Vec::with_capacity(dlmm.bins.len());
        for b in &dlmm.bins {
            bins.push(MathDlmmBin {
                id: b.bin_id,
                amount_x: u64::try_from(b.amount_x).ok()?,
                amount_y: u64::try_from(b.amount_y).ok()?,
                price_q64: 0,
            });
        }
        if bins.is_empty() {
            return None;
        }

        // The volatility decay is intentionally *not* applied: the on-chain
        // `volatility_reference` is the freshest known value and skipping the
        // decay keeps the variable fee at its upper bound (conservative).
        let quote = quote_dlmm_view(&pool_state, &bins, net_in, swap_for_y, None).ok()?;
        if quote.amount_out == 0 {
            return None;
        }

        let amount_out = quote.amount_out;
        if amount_out == 0 {
            return None;
        }

        Some(DlmmHopQuote {
            amount_in,
            amount_out,
            gross_out: quote.amount_out,
            fee_amount: quote.fee_amount,
            protocol_fee: quote.protocol_fee,
            bins_crossed: quote.bins_crossed.max(1),
            end_active_id: quote.end_active_id,
            swap_for_y,
        })
    }
}

/// A two-pool route: flash mint -> mid mint -> flash mint.
#[derive(Debug, Clone)]
pub struct DlmmFlashArbRoute {
    /// Pool used for the first hop.
    pub pool_a: DlmmFlashArbPool,
    /// Pool used for the second hop.
    pub pool_b: DlmmFlashArbPool,
    /// Mint the flash loan is denominated in (WSOL on mainnet).
    pub flash_mint: MintId,
    /// Intermediate mint traded between the two pools.
    pub mid_mint: MintId,
}

/// Configuration of the flash-funded two-pool strategy.
#[derive(Debug, Clone)]
pub struct DlmmFlashArbConfig {
    /// Pools the route may trade.
    pub allowlist: DlmmPoolAllowlist,
    /// Mint the Kamino flash loan is denominated in.
    pub flash_mint: MintId,
    /// Decimals of the flash mint (9 for WSOL).
    pub flash_mint_decimals: u8,
    /// Smallest borrow amount considered, in flash-mint raw units.
    pub min_borrow: u64,
    /// Largest borrow amount considered, in flash-mint raw units.
    pub max_borrow: u64,
    /// Liquidity the Kamino reserve can actually lend, in flash-mint raw units.
    pub kamino_available_liquidity: u64,
    /// On-chain `flash_loan_fee_sf` (Q64.64) of the Kamino reserve.
    pub kamino_flash_fee_sf: u64,
    /// Minimum acceptable net profit, in flash-mint raw units.
    pub min_profit_raw: u64,
    /// Slippage applied to the Pool A minimum output, in bps.
    pub hop_slippage_bps: u64,
    /// Compute-unit limit for the assembled transaction.
    pub cu_limit: u32,
    /// Maximum number of bin arrays per hop.
    pub max_bin_arrays_per_hop: usize,
    /// Maximum serialized transaction size in bytes.
    pub max_tx_bytes: usize,
    /// Number of coarse grid points.
    pub coarse_samples: usize,
    /// Number of fine steps per candidate basin.
    pub refine_steps: usize,
    /// Network cost (rent + base fee) in lamports.
    pub network_cost_lamports: u64,
    /// Priority fee in lamports.
    pub priority_fee_lamports: u64,
    /// Priority fee in micro-lamports (instruction payload).
    pub priority_fee_micro_lamports: u64,
    /// Transaction authority.
    pub payer: Pubkey,
    /// Kamino flash-loan parameters.
    pub kamino: KaminoFlashParams,
}

impl Default for DlmmFlashArbConfig {
    fn default() -> Self {
        Self {
            allowlist: DlmmPoolAllowlist::default(),
            flash_mint: MintId(
                Pubkey::from_str(arb_constants::WSOL_MINT).expect("valid WSOL mint"),
            ),
            flash_mint_decimals: 9,
            min_borrow: 100_000,
            max_borrow: 1_000_000_000,
            kamino_available_liquidity: 1_000_000_000,
            kamino_flash_fee_sf: 11_529_215_046_068,
            min_profit_raw: 0,
            hop_slippage_bps: 5,
            cu_limit: 400_000,
            max_bin_arrays_per_hop: 4,
            max_tx_bytes: arb_constants::SOLANA_PACKET_MTU_LIMIT,
            coarse_samples: 24,
            refine_steps: 10,
            network_cost_lamports: 9_000,
            priority_fee_lamports: 4_000,
            priority_fee_micro_lamports: 10_000,
            payer: Pubkey::default(),
            kamino: KaminoFlashParams::default(),
        }
    }
}

impl DlmmFlashArbConfig {
    /// Convert a lamport-denominated cost into flash-mint raw units.
    ///
    /// The flash mint is WSOL (9 decimals) on mainnet, where one lamport is one
    /// raw unit. The conversion is explicit so a different flash mint cannot
    /// silently mix units.
    pub fn cost_in_flash_mint(&self, lamports: u64) -> u64 {
        let scale = 10u128.pow(u32::from(self.flash_mint_decimals));
        let lamport_scale = 10u128.pow(9);
        // Round costs UP: understating a cost would overstate net profit, so a
        // sub-unit cost is never truncated away.
        let numerator = (lamports as u128)
            .saturating_mul(scale)
            .saturating_add(lamport_scale.saturating_sub(1));
        u64::try_from(numerator / lamport_scale).unwrap_or(u64::MAX)
    }

    /// Total non-pool cost of one execution, in flash-mint raw units.
    pub fn execution_cost(&self) -> u64 {
        self.cost_in_flash_mint(
            self.network_cost_lamports
                .saturating_add(self.priority_fee_lamports),
        )
    }
}

/// Why a candidate borrow amount was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashArbRejection {
    BorrowBelowMinimum { borrow: u64, min: u64 },
    BorrowAboveMaximum { borrow: u64, max: u64 },
    BorrowAboveKaminoLiquidity { borrow: u64, available: u64 },
    BorrowAbovePoolLiquidity { borrow: u64, capacity: u64 },
    IntermediateAbovePoolLiquidity { amount: u64, capacity: u64 },
    QuoteFailed { hop: &'static str, detail: String },
    TooManyBinArrays { required: usize, budget: usize },
    TransactionTooLarge { estimated: usize, limit: usize },
    ComputeUnitsExceeded { estimated: u32, limit: u32 },
    TooManyAccounts { accounts: usize, limit: usize },
    ProfitBelowFloor { net: i64, floor: u64 },
    PoolNotAllowlisted { pool: Pubkey },
    MismatchedMints,
}

impl FlashArbRejection {
    /// Stable machine-readable name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BorrowBelowMinimum { .. } => "BORROW_BELOW_MINIMUM",
            Self::BorrowAboveMaximum { .. } => "BORROW_ABOVE_MAXIMUM",
            Self::BorrowAboveKaminoLiquidity { .. } => "BORROW_ABOVE_KAMINO_LIQUIDITY",
            Self::BorrowAbovePoolLiquidity { .. } => "BORROW_ABOVE_POOL_LIQUIDITY",
            Self::IntermediateAbovePoolLiquidity { .. } => "INTERMEDIATE_ABOVE_POOL_LIQUIDITY",
            Self::QuoteFailed { .. } => "QUOTE_FAILED",
            Self::TooManyBinArrays { .. } => "TOO_MANY_BIN_ARRAYS",
            Self::TransactionTooLarge { .. } => "TRANSACTION_TOO_LARGE",
            Self::ComputeUnitsExceeded { .. } => "COMPUTE_UNITS_EXCEEDED",
            Self::TooManyAccounts { .. } => "TOO_MANY_ACCOUNTS",
            Self::ProfitBelowFloor { .. } => "PROFIT_BELOW_FLOOR",
            Self::PoolNotAllowlisted { .. } => "POOL_NOT_ALLOWLISTED",
            Self::MismatchedMints => "MISMATCHED_MINTS",
        }
    }
}

/// Full evaluation of one candidate borrow amount.
#[derive(Debug, Clone)]
pub struct DlmmFlashArbEvaluation {
    pub borrow_amount: u64,
    pub flash_fee: u64,
    /// Pool A output at the quoted (unslipped) amount.
    pub hop_a_out: u64,
    /// Pool A conservative minimum output (after slippage).
    pub hop_a_min_out: u64,
    /// Pool B output when fed the unslipped Pool A output.
    pub hop_b_out: u64,
    /// Pool B output when fed the conservative Pool A output (the executed path).
    pub hop_b_min_out: u64,
    pub gross_profit: i64,
    pub total_cost: u64,
    pub net_profit: i64,
    pub net_profit_bps: i64,
    pub bins_a: u32,
    pub bins_b: u32,
    pub bin_arrays: usize,
    pub unique_accounts: usize,
    pub estimated_cu: u32,
    pub estimated_tx_bytes: usize,
    pub feasible: bool,
    pub rejection: Option<FlashArbRejection>,
}

impl DlmmFlashArbEvaluation {
    fn reject(borrow: u64, reason: FlashArbRejection) -> Self {
        Self {
            borrow_amount: borrow,
            flash_fee: 0,
            hop_a_out: 0,
            hop_a_min_out: 0,
            hop_b_out: 0,
            hop_b_min_out: 0,
            gross_profit: 0,
            total_cost: 0,
            net_profit: 0,
            net_profit_bps: 0,
            bins_a: 0,
            bins_b: 0,
            bin_arrays: 0,
            unique_accounts: 0,
            estimated_cu: 0,
            estimated_tx_bytes: 0,
            feasible: false,
            rejection: Some(reason),
        }
    }
}

/// Evaluator for the flash-funded two-pool route.
pub struct DlmmFlashArbEvaluator<'a> {
    pub config: &'a DlmmFlashArbConfig,
}

impl DlmmFlashArbEvaluator<'_> {
    /// Evaluate one borrow amount against every constraint.
    pub fn evaluate(&self, route: &DlmmFlashArbRoute, borrow: u64) -> DlmmFlashArbEvaluation {
        let cfg = self.config;

        if borrow == 0 {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::BorrowBelowMinimum {
                    borrow,
                    min: cfg.min_borrow,
                },
            );
        }
        if borrow < cfg.min_borrow {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::BorrowBelowMinimum {
                    borrow,
                    min: cfg.min_borrow,
                },
            );
        }
        if borrow > cfg.max_borrow {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::BorrowAboveMaximum {
                    borrow,
                    max: cfg.max_borrow,
                },
            );
        }
        if borrow > cfg.kamino_available_liquidity {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::BorrowAboveKaminoLiquidity {
                    borrow,
                    available: cfg.kamino_available_liquidity,
                },
            );
        }
        if !cfg.allowlist.contains(&route.pool_a.pool_id)
            || !cfg.allowlist.contains(&route.pool_b.pool_id)
        {
            let pool = if cfg.allowlist.contains(&route.pool_a.pool_id) {
                route.pool_b.pool_id
            } else {
                route.pool_a.pool_id
            };
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::PoolNotAllowlisted { pool },
            );
        }
        if route.pool_a.pool_id == route.pool_b.pool_id {
            return DlmmFlashArbEvaluation::reject(borrow, FlashArbRejection::MismatchedMints);
        }

        let flash_fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, cfg.kamino_flash_fee_sf);

        // ── Hop A ──
        let Some(capacity_a) = route
            .pool_a
            .active_bin_max_input(&route.flash_mint, &route.mid_mint)
        else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "A",
                    detail: "active bin capacity unavailable".into(),
                },
            );
        };
        if borrow > capacity_a {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::BorrowAbovePoolLiquidity {
                    borrow,
                    capacity: capacity_a,
                },
            );
        }

        let Some(quote_a) = route
            .pool_a
            .quote(borrow, &route.flash_mint, &route.mid_mint)
        else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "A",
                    detail: "DLMM quote failed".into(),
                },
            );
        };

        // Conservative Pool A output (the amount the chain is allowed to deliver).
        let slippage_factor =
            10_000u128.saturating_sub(u128::from(cfg.hop_slippage_bps.min(10_000)));
        let hop_a_min_out = ((quote_a.amount_out as u128 * slippage_factor) / 10_000) as u64;
        if hop_a_min_out == 0 {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "A",
                    detail: "conservative output rounds to zero".into(),
                },
            );
        }

        // ── Hop B (executed path: fed the conservative Pool A output) ──
        let Some(capacity_b) = route
            .pool_b
            .active_bin_max_input(&route.mid_mint, &route.flash_mint)
        else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "B",
                    detail: "active bin capacity unavailable".into(),
                },
            );
        };
        if hop_a_min_out > capacity_b {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::IntermediateAbovePoolLiquidity {
                    amount: hop_a_min_out,
                    capacity: capacity_b,
                },
            );
        }

        let Some(quote_b) = route
            .pool_b
            .quote(hop_a_min_out, &route.mid_mint, &route.flash_mint)
        else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "B",
                    detail: "DLMM quote failed".into(),
                },
            );
        };
        // Optimistic Pool B output, used only for reporting.
        let quote_b_optimistic = route
            .pool_b
            .quote(quote_a.amount_out, &route.mid_mint, &route.flash_mint)
            .map(|q| q.amount_out)
            .unwrap_or(quote_b.amount_out);

        // ── Bin arrays ──
        let Some(active_a) = route.pool_a.state.dlmm.as_ref().map(|d| d.active_bin_id) else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "A",
                    detail: "missing DLMM state".into(),
                },
            );
        };
        let Some(active_b) = route.pool_b.state.dlmm.as_ref().map(|d| d.active_bin_id) else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "B",
                    detail: "missing DLMM state".into(),
                },
            );
        };
        let Some(plan_a) = plan_bin_arrays(
            &route.pool_a.pool_id,
            active_a,
            quote_a.swap_for_y,
            quote_a.bins_crossed.max(1),
        )
        .ok() else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "A",
                    detail: "bin array plan failed".into(),
                },
            );
        };
        let Some(plan_b) = plan_bin_arrays(
            &route.pool_b.pool_id,
            active_b,
            quote_b.swap_for_y,
            quote_b.bins_crossed.max(1),
        )
        .ok() else {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::QuoteFailed {
                    hop: "B",
                    detail: "bin array plan failed".into(),
                },
            );
        };
        let bin_arrays = plan_a.account_count() + plan_b.account_count();
        let budget = cfg.max_bin_arrays_per_hop * 2;
        if bin_arrays > budget {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::TooManyBinArrays {
                    required: bin_arrays,
                    budget,
                },
            );
        }

        // ── Compute units ──
        let estimated_cu = estimate_minimal_cu(true, 2)
            .saturating_add(CU_PER_BIN_ARRAY.saturating_mul(bin_arrays as u32))
            .min(arb_constants::MAX_COMPUTE_UNIT_LIMIT);
        if estimated_cu > cfg.cu_limit {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::ComputeUnitsExceeded {
                    estimated: estimated_cu,
                    limit: cfg.cu_limit,
                },
            );
        }

        // ── Transaction size (v0 with every non-signer account in a lookup table) ──
        let (unique_accounts, estimated_tx_bytes) = match self.estimate_tx_bytes(
            route,
            borrow,
            flash_fee,
            hop_a_min_out,
            &plan_a,
            &plan_b,
            quote_a.swap_for_y,
            quote_b.swap_for_y,
        ) {
            Ok(v) => v,
            Err(reason) => return DlmmFlashArbEvaluation::reject(borrow, reason),
        };
        if estimated_tx_bytes > cfg.max_tx_bytes {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::TransactionTooLarge {
                    estimated: estimated_tx_bytes,
                    limit: cfg.max_tx_bytes,
                },
            );
        }
        if unique_accounts > MAX_ALT_ADDRESSES {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::TooManyAccounts {
                    accounts: unique_accounts,
                    limit: MAX_ALT_ADDRESSES,
                },
            );
        }

        // ── Economics ──
        let repay = borrow.saturating_add(flash_fee);
        let total_cost = flash_fee.saturating_add(cfg.execution_cost());
        let gross_profit = (quote_b.amount_out as i64) - (borrow as i64);
        let net_profit = gross_profit - (total_cost as i64);
        if net_profit < 0 || (net_profit as u64) < cfg.min_profit_raw {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::ProfitBelowFloor {
                    net: net_profit,
                    floor: cfg.min_profit_raw,
                },
            );
        }
        // The executed chain must be able to fund the repayment.
        if quote_b.amount_out < repay {
            return DlmmFlashArbEvaluation::reject(
                borrow,
                FlashArbRejection::ProfitBelowFloor {
                    net: net_profit,
                    floor: cfg.min_profit_raw,
                },
            );
        }

        let net_profit_bps = if borrow > 0 {
            (net_profit * 10_000) / (borrow as i64)
        } else {
            0
        };

        DlmmFlashArbEvaluation {
            borrow_amount: borrow,
            flash_fee,
            hop_a_out: quote_a.amount_out,
            hop_a_min_out,
            hop_b_out: quote_b_optimistic,
            hop_b_min_out: quote_b.amount_out,
            gross_profit,
            total_cost,
            net_profit,
            net_profit_bps,
            bins_a: quote_a.bins_crossed,
            bins_b: quote_b.bins_crossed,
            bin_arrays,
            unique_accounts,
            estimated_cu,
            estimated_tx_bytes,
            feasible: true,
            rejection: None,
        }
    }

    /// Assemble the real instruction list and measure its v0 size.
    #[allow(clippy::too_many_arguments)]
    fn estimate_tx_bytes(
        &self,
        route: &DlmmFlashArbRoute,
        borrow: u64,
        flash_fee: u64,
        hop_a_min_out: u64,
        plan_a: &DlmmBinArrayPlan,
        plan_b: &DlmmBinArrayPlan,
        swap_a_for_y: bool,
        swap_b_for_y: bool,
    ) -> Result<(usize, usize), FlashArbRejection> {
        let cfg = self.config;
        let repay = borrow.saturating_add(flash_fee);

        let leg_a = FlashArbPoolLeg {
            lb_pair: route.pool_a.pool_id,
            reserve_x: route.pool_a.reserve_x,
            reserve_y: route.pool_a.reserve_y,
            token_x_mint: route.pool_a.token_x_mint.0,
            token_y_mint: route.pool_a.token_y_mint.0,
            oracle: route.pool_a.oracle,
            token_x_program: route.pool_a.token_x_program.program_id(),
            token_y_program: route.pool_a.token_y_program.program_id(),
            swap_for_y: swap_a_for_y,
            amount_in: borrow,
            min_amount_out: hop_a_min_out,
            bin_array_pdas: plan_a.pdas.clone(),
            bitmap_extension: Some(plan_a.bitmap_extension_pda),
            host_fee_in: None,
        };
        let leg_b = FlashArbPoolLeg {
            lb_pair: route.pool_b.pool_id,
            reserve_x: route.pool_b.reserve_x,
            reserve_y: route.pool_b.reserve_y,
            token_x_mint: route.pool_b.token_x_mint.0,
            token_y_mint: route.pool_b.token_y_mint.0,
            oracle: route.pool_b.oracle,
            token_x_program: route.pool_b.token_x_program.program_id(),
            token_y_program: route.pool_b.token_y_program.program_id(),
            swap_for_y: swap_b_for_y,
            amount_in: hop_a_min_out,
            min_amount_out: repay,
            bin_array_pdas: plan_b.pdas.clone(),
            bitmap_extension: Some(plan_b.bitmap_extension_pda),
            host_fee_in: None,
        };

        let params = FlashArbParams {
            payer: cfg.payer,
            kamino: cfg.kamino.clone(),
            borrow_amount: borrow,
            flash_fee,
            leg_a,
            leg_b,
            cu_limit: cfg.cu_limit,
            priority_fee_micro_lamports: cfg.priority_fee_micro_lamports,
        };

        let (ixs, _, _) = assemble_flash_funded_two_pool(&params).map_err(|e| {
            FlashArbRejection::QuoteFailed {
                hop: "tx",
                detail: e.to_string(),
            }
        })?;

        let mut accounts: Vec<Pubkey> = Vec::new();
        for ix in &ixs {
            for acc in &ix.accounts {
                if !accounts.contains(&acc.pubkey) {
                    accounts.push(acc.pubkey);
                }
            }
            if !accounts.contains(&ix.program_id) {
                accounts.push(ix.program_id);
            }
        }

        // Best case: every account lives in an address lookup table, so each one
        // costs a single index byte instead of a 32-byte inline key.
        let table = AddressLookupTable::new(Pubkey::default(), accounts.clone());
        let size = estimate_v0_transaction_size(&ixs, &[table]);
        Ok((accounts.len(), size))
    }

    /// Optimize the borrow amount for `route`.
    ///
    /// Bounded coarse-to-fine search: a logarithmic/linear coarse grid, multi-basin
    /// local-maximum detection, then a bounded integer fine sweep inside every
    /// promising basin. Profit is **not** assumed monotonic.
    pub fn optimize(&self, route: &DlmmFlashArbRoute) -> Option<DlmmFlashArbEvaluation> {
        let lo = self.config.min_borrow;
        let hi = self.config.max_borrow;
        if lo == 0 || hi < lo {
            return None;
        }

        let mut candidates: Vec<u64> = Vec::new();
        candidates.push(lo);

        // Logarithmic ladder (growth 1.5) preserving coverage across bin arrays.
        let mut cur = lo;
        while cur < hi && candidates.len() < 128 {
            let next = ((cur as u128 * 15) / 10).max(cur as u128 + 1);
            let Ok(next) = u64::try_from(next) else { break };
            if next <= cur {
                break;
            }
            if next > hi {
                break;
            }
            candidates.push(next);
            cur = next;
        }
        if *candidates.last().unwrap_or(&0) != hi {
            candidates.push(hi);
        }

        // Uniform fill.
        let fill = self.config.coarse_samples.clamp(2, 64) as u64;
        for k in 1..fill {
            let point = lo as u128 + ((hi as u128 - lo as u128) * (k as u128 + 1)) / fill as u128;
            if let Ok(v) = u64::try_from(point) {
                candidates.push(v);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();

        let mut evaluated: Vec<(u64, i64)> = Vec::new();
        for size in &candidates {
            let eval = self.evaluate(route, *size);
            if eval.feasible {
                evaluated.push((*size, eval.net_profit));
            }
        }
        if evaluated.is_empty() {
            return None;
        }
        evaluated.sort_by_key(|(size, _)| *size);

        let sizes: Vec<u64> = evaluated.iter().map(|(s, _)| *s).collect();
        let profits: Vec<i64> = evaluated.iter().map(|(_, p)| *p).collect();

        // Multi-basin detection: the profit curve is not assumed unimodal.
        let mut basins: Vec<(u64, u64)> = Vec::new();
        for i in 0..evaluated.len() {
            let left = if i > 0 { profits[i - 1] } else { i64::MIN };
            let right = if i + 1 < evaluated.len() {
                profits[i + 1]
            } else {
                i64::MIN
            };
            if profits[i] >= left && profits[i] >= right {
                let b_lo = if i > 0 { sizes[i - 1] } else { sizes[i] };
                let b_hi = if i + 1 < evaluated.len() {
                    sizes[i + 1]
                } else {
                    sizes[i]
                };
                basins.push((b_lo, b_hi));
            }
        }
        if basins.is_empty() {
            // No strict local maximum: sweep the whole (bounded) range coarsely.
            basins.push((lo, hi));
        }

        let steps = self.config.refine_steps.max(2) as u64;
        let mut best: Option<DlmmFlashArbEvaluation> = None;
        for (b_lo, b_hi) in basins {
            if b_hi <= b_lo + 1 {
                continue;
            }
            let span = b_hi - b_lo;
            let step = (span / steps).max(1);
            let mut point = b_lo + step;
            while point < b_hi {
                let eval = self.evaluate(route, point);
                if eval.feasible
                    && best
                        .as_ref()
                        .map(|b| eval.net_profit > b.net_profit)
                        .unwrap_or(true)
                {
                    best = Some(eval);
                }
                point = point.saturating_add(step);
            }
        }

        // The coarse winner is always a candidate.
        let coarse_best = evaluated
            .iter()
            .max_by_key(|(_, p)| *p)
            .map(|(s, _)| *s)
            .unwrap_or(lo);
        let coarse_eval = self.evaluate(route, coarse_best);
        if coarse_eval.feasible
            && best
                .as_ref()
                .map(|b| coarse_eval.net_profit > b.net_profit)
                .unwrap_or(true)
        {
            best = Some(coarse_eval);
        }

        best
    }
}

/// Enumerate every admissible two-pool route over the allowlist.
///
/// Both directions are produced: (A then B) and (B then A).
pub fn enumerate_routes(
    pools: &[DlmmFlashArbPool],
    allowlist: &DlmmPoolAllowlist,
    flash_mint: MintId,
) -> Vec<DlmmFlashArbRoute> {
    let mut out = Vec::new();
    for a in pools {
        if !allowlist.contains(&a.pool_id) {
            continue;
        }
        for b in pools {
            if a.pool_id == b.pool_id {
                continue;
            }
            if !allowlist.contains(&b.pool_id) {
                continue;
            }
            // Both pools must hold the flash mint.
            if a.token_x_mint != flash_mint && a.token_y_mint != flash_mint {
                continue;
            }
            if b.token_x_mint != flash_mint && b.token_y_mint != flash_mint {
                continue;
            }
            // The intermediate mint must be the pool's other mint and shared.
            let Some(mid_a) = other_mint(a, flash_mint) else {
                continue;
            };
            let Some(mid_b) = other_mint(b, flash_mint) else {
                continue;
            };
            if mid_a != mid_b {
                continue;
            }
            out.push(DlmmFlashArbRoute {
                pool_a: a.clone(),
                pool_b: b.clone(),
                flash_mint,
                mid_mint: mid_a,
            });
        }
    }
    out
}

/// The pool's mint other than `mint`.
fn other_mint(pool: &DlmmFlashArbPool, mint: MintId) -> Option<MintId> {
    if pool.token_x_mint == mint {
        Some(pool.token_y_mint)
    } else if pool.token_y_mint == mint {
        Some(pool.token_x_mint)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_x() -> MintId {
        MintId(Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap())
    }
    fn pool_y() -> MintId {
        MintId(Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap())
    }

    #[test]
    fn cost_conversion_is_one_to_one_for_wsol() {
        let cfg = DlmmFlashArbConfig::default();
        assert_eq!(cfg.cost_in_flash_mint(9_000), 9_000);
    }

    #[test]
    fn cost_conversion_scales_for_a_six_decimal_flash_mint() {
        let cfg = DlmmFlashArbConfig {
            flash_mint_decimals: 6,
            ..Default::default()
        };
        // 1 lamport = 1e-3 units at 6 decimals; costs round UP so a sub-unit
        // cost is never truncated to zero.
        assert_eq!(cfg.cost_in_flash_mint(1), 1);
        // 1_000_000 lamports = 0.001 SOL = 1_000 units at 6 decimals.
        assert_eq!(cfg.cost_in_flash_mint(1_000_000), 1_000);
    }

    #[test]
    fn allowlist_membership_is_explicit() {
        let list = DlmmPoolAllowlist::from_base58(&[
            "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6",
            "not-a-pubkey",
        ]);
        assert_eq!(list.len(), 1);
        assert!(
            list.contains(
                &Pubkey::from_str("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6").unwrap()
            )
        );
    }

    #[test]
    fn swap_direction_follows_the_on_chain_ordering() {
        let pool = DlmmFlashArbPool {
            pool_id: Pubkey::new_unique(),
            state: PoolState {
                pool_id: arb_types::PoolId(Pubkey::new_unique()),
                protocol: arb_types::DexProtocol::MeteoraDlmm,
                slot: arb_types::Slot(0),
                write_version: 0,
                base_mint: pool_x(),
                quote_mint: pool_y(),
                decimals_a: 9,
                decimals_b: 6,
                reserve_in: 0,
                reserve_out: 0,
                fee_numerator: 0,
                fee_denominator: 1,
                is_active: true,
                has_risky_token: false,
                kind: arb_types::PoolKind::Dlmm,
                clmm: None,
                dlmm: None,
                pump_fun: None,
                transfer_fee_bps: 0,
                token_programs: [TokenProgramKind::SplToken; 2],
                execution_accounts: None,
                last_update_slot: 0,
                last_update_ms: 0,
            },
            token_x_mint: pool_y(),
            token_y_mint: pool_x(),
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            token_x_program: TokenProgramKind::SplToken,
            token_y_program: TokenProgramKind::SplToken,
            transfer_fee_bps: [0, 0],
        };
        // On-chain order is (USDC, WSOL): WSOL -> USDC is Y -> X.
        assert_eq!(pool.swap_for_y(&pool_x(), &pool_y()), Some(false));
        assert_eq!(pool.swap_for_y(&pool_y(), &pool_x()), Some(true));
        // The canonical base/quote ordering is deliberately ignored.
        assert_eq!(pool.state.base_mint, pool_x());
    }
}
