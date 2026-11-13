//! Three-leg Meteora DLMM meme arbitrage pipeline (`ThreeLegMemeMemePipeline`).
//!
//! Wires the meme three-leg route `WSOL -> Meme A -> Meme B -> WSOL` through the
//! connected engine pipeline instead of testing only the standalone transaction
//! compiler:
//!
//!   Discovery (pool scan + chain resolution)
//!     -> Route Graph (mint-ordered WSOL -> A -> B -> WSOL orientation)
//!     -> Quote Engine (discrete DLMM math, per-leg)
//!     -> Trade Size Optimizer (3-leg-native coarse-to-fine search)
//!     -> Account Resolver (per-leg execution accounts -> compiler context)
//!     -> Compiler (`arb_tx::compile_3leg_meme_dlmm_arb`, outside this module)
//!     -> Simulation / Profitability Gate (`simulate_profitability_gate`)
//!
//! The full `Discovery -> ... -> Paper Journal` connection is exercised by
//! `tests/three_leg_meme_pipeline_tests.rs`. This module is the reusable
//! production half: discovery, route resolution, quoting, sizing, and context
//! construction. The Paper Journal writer is the existing
//! `arb_sim::opportunity_recorder::OpportunityHistoryRecorder`.
//!
//! NOTE: the dedicated `MeteoraDlmmTradeSizeOptimizer` is 2-pool scoped by design
//! and does not apply to a three-leg chain. Sizing is performed by the
//! 3-leg-native `ThreeLegMemeSizeSearchResult`, which uses the same coarse-to-fine
//! basin refinement philosophy as the 2-pool optimizer.

use arb_constants::WSOL_MINT;
use arb_tx::meteora_dlmm::{SwapDirection, SwapMode, derive_directional_bin_arrays};
use arb_tx::{LegExecutionAccounts, LegSwapParams, ThreeLegMemeArbitrageContext};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, PoolState, TokenProgramKind};
use solana_pubkey::Pubkey;

use crate::meteora_pair_discovery::MeteoraDlmmPoolPairDiscovery;
use crate::meteora_quote_engine::MeteoraDlmmCrossPoolQuoteEngine;

/// Knobs the caller must provide to build an executable three-leg context.
#[derive(Debug, Clone)]
pub struct ThreeLegMemeContextParams {
    pub payer: Pubkey,
    pub meme_a_ata: Pubkey,
    pub meme_b_ata: Pubkey,
    pub meme_a_ata_exists: bool,
    pub meme_b_ata_exists: bool,
    pub meme_a_ata_balance: u64,
    pub meme_b_ata_balance: u64,
    pub meme_a_is_token_2022: bool,
    pub meme_b_is_token_2022: bool,
    pub meme_a_withheld_fees: u64,
    pub meme_b_withheld_fees: u64,
    pub wsol_token_program: Pubkey,
    pub meme_a_token_program: Pubkey,
    pub meme_b_token_program: Pubkey,
    pub snapshot_slot: u64,
    pub cu_limit: u32,
    pub priority_fee_micro_lamports: u64,
    pub jito_tip_account: Pubkey,
    pub jito_tip_lamports: u64,
    pub recent_blockhash: [u8; 32],
    pub v1_transport_supported: bool,
    /// Swap slippage applied per leg (basis points, 100 = 1%).
    pub slippage_bps: u64,
    /// Hard minimal net profit (lamports) for the profitability gate.
    pub min_profit_lamports: u64,
    /// Kamino flash-loan fee as the raw on-chain U68F60 value
    /// (`ReserveConfig.fees.flash_loan_fee_sf`). `0` disables the fee term
    /// (non-flash-funded evaluation only).
    pub flash_loan_fee_sf: u64,
    /// Conservative flat network cost per transaction (base fee + priority
    /// fee + Jito tip) in lamports, deducted from every size candidate.
    pub network_cost_lamports: u64,
    /// Actual available Kamino flash liquidity decoded from the reserve
    /// supply-vault token account. `None` means the snapshot was unavailable;
    /// the borrow ceiling then falls back to the configured size ladder, and
    /// the live path must re-validate before dispatch.
    pub kamino_available_liquidity: Option<u64>,
    /// Initial input amount candidate for discovery.
    pub input_size: u64,
    /// Candidate input sizes for the 3-leg trade size search. The first entry
    /// is also used as the primary discovery size. An empty vec falls back to
    /// `[input_size]`.
    pub size_search_sizes: Vec<u64>,
}

impl Default for ThreeLegMemeContextParams {
    fn default() -> Self {
        use std::str::FromStr;
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        Self {
            payer: Pubkey::new_unique(),
            meme_a_ata: Pubkey::new_unique(),
            meme_b_ata: Pubkey::new_unique(),
            meme_a_ata_exists: false,
            meme_b_ata_exists: false,
            meme_a_ata_balance: 0,
            meme_b_ata_balance: 0,
            meme_a_is_token_2022: false,
            meme_b_is_token_2022: false,
            meme_a_withheld_fees: 0,
            meme_b_withheld_fees: 0,
            wsol_token_program: spl,
            meme_a_token_program: spl,
            meme_b_token_program: spl,
            snapshot_slot: 0,
            cu_limit: arb_constants::COMPUTE_BUDGET_LIMIT_3HOP,
            priority_fee_micro_lamports: 1_000,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 10_000,
            recent_blockhash: [42u8; 32],
            v1_transport_supported: false,
            slippage_bps: 100,
            min_profit_lamports: 30_000,
            flash_loan_fee_sf: 0,
            network_cost_lamports: 0,
            kamino_available_liquidity: None,
            input_size: 100_000_000,
            size_search_sizes: Vec::new(),
        }
    }
}

impl ThreeLegMemeContextParams {
    /// Discovery-mode constructor: the ATAs are deterministic placeholders
    /// because the compiler context built by the pipeline is not used on the
    /// capture path — per-hop `SwapHop.execution_accounts` come from live
    /// pool states instead. All account-existence/balance probes are zeroed.
    /// Callers override the live fields (slot, CU, fees, sizing) afterwards.
    pub fn for_discovery(payer: Pubkey) -> Self {
        use std::str::FromStr;
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        Self {
            payer,
            meme_a_ata: arb_tx::master_alt::derive_ata(
                &payer,
                &Pubkey::new_from_array([0u8; 32]),
                &spl,
            ),
            meme_b_ata: arb_tx::master_alt::derive_ata(
                &payer,
                &Pubkey::new_from_array([1u8; 32]),
                &spl,
            ),
            meme_a_ata_exists: false,
            meme_b_ata_exists: false,
            meme_a_ata_balance: 0,
            meme_b_ata_balance: 0,
            meme_a_is_token_2022: false,
            meme_b_is_token_2022: false,
            meme_a_withheld_fees: 0,
            meme_b_withheld_fees: 0,
            wsol_token_program: spl,
            meme_a_token_program: spl,
            meme_b_token_program: spl,
            ..Self::default()
        }
    }
}

/// Per-leg quote record for the chosen three-leg orientation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeLegMemeLegQuote {
    pub leg: usize,
    pub pool_id: PoolId,
    pub from_mint: MintId,
    pub to_mint: MintId,
    /// true when the swap moves X -> Y.
    pub swap_for_y: bool,
    pub amount_in: u64,
    pub amount_out: u64,
    pub fee: u64,
    pub bins_traversed: u32,
}

/// Result of the 3-leg-native trade size search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeLegMemeSizeSearchResult {
    /// (input size, NET profit in lamports) for every candidate evaluated.
    /// Net = gross - Kamino flash fee - configured network cost.
    pub tested_sizes: Vec<(u64, i64)>,
    pub best_input: u64,
    pub best_net_profit_lamports: i64,
}

/// Fully assembled result of the connected pipeline, before transaction compilation.
#[derive(Debug)]
pub struct ThreeLegMemePipelineResult {
    /// The three pools in WSOL -> A -> B -> WSOL order.
    pub pool_ids: [Pubkey; 3],
    /// The meme mints in order (A, B).
    pub meme_mints: [MintId; 2],
    /// Per-leg quote records.
    pub legs: [ThreeLegMemeLegQuote; 3],
    pub size_search: ThreeLegMemeSizeSearchResult,
    /// The executable compiler context, ready for `arb_tx::compile_3leg_meme_dlmm_arb`.
    pub context: ThreeLegMemeArbitrageContext,
    /// Discovery report (kept for journaling and rejection accounting).
    pub discovery: crate::meteora_pair_discovery::MeteoraDlmmPairDiscoveryReport,
    /// Final expected output of the chain at the chosen size (before slippage haircut).
    pub expected_final_out: u64,
    /// Gross profit (quote final out - input) in lamports.
    pub gross_profit_lamports: i64,
    /// Kamino flash fee at the chosen size (lamports, flash mint = WSOL).
    pub flash_fee_lamports: u64,
    /// Net profit after flash fee and network cost, in lamports.
    pub net_profit_lamports: i64,
    /// Conservative slippage-floor final output at the chosen size. Downstream
    /// gates must use this (not `expected_final_out`) for repayment checks.
    pub expected_min_final_output: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ThreeLegMemePipelineError {
    #[error("no 3-leg WSOL -> A -> B -> WSOL cycle found among {pools} candidate pools")]
    NoCycleFound { pools: usize },
    #[error("pools must be protocol MeteoraDlmm and active")]
    NoEligiblePools,
    #[error("quote failed on leg {leg}")]
    QuoteFailed { leg: usize },
    #[error(
        "requested trade would traverse {bins} bins on leg {leg}, exceeding the 2-array compiler policy"
    )]
    TooManyBinArrays { leg: usize, bins: u32 },
    #[error("leg {leg}: pool is missing MeteoraDlmm execution accounts")]
    MissingExecutionAccounts { leg: usize },
    #[error("leg {leg}: unsupported execution accounts for MeteoraDlmm")]
    UnsupportedExecutionAccounts { leg: usize },
    #[error(
        "every candidate borrow size exceeds the available Kamino flash liquidity ({available})"
    )]
    InsufficientFlashLiquidity { available: u64 },
    #[error("best net profit {best_net} is below the required floor {floor}")]
    BelowNetProfitFloor { best_net: i64, floor: u64 },
    #[error(
        "slippage floor of the final output ({min_final_output}) cannot cover principal + flash fee ({required})"
    )]
    RepaymentNotGuaranteed {
        min_final_output: u64,
        required: u64,
    },
    #[error("internal: {0}")]
    Internal(String),
}

fn wsol_mint() -> MintId {
    use std::str::FromStr;
    MintId(Pubkey::from_str(WSOL_MINT).expect("valid wsol mint"))
}

fn valid_meteora_pools(pools: &[PoolState]) -> Vec<PoolState> {
    pools
        .iter()
        .filter(|p| {
            p.protocol == DexProtocol::MeteoraDlmm
                && p.is_active
                && p.dlmm.is_some()
                && p.execution_accounts.is_some()
        })
        .cloned()
        .collect()
}

/// Internal representation of the selected chain (pool indexes into `candidates`).
#[derive(Clone, Copy)]
struct Chain {
    i: usize,
    j: usize,
    k: usize,
    a: MintId,
    b: MintId,
    profit: i128,
}

/// One fully-evaluated 3-leg orientation at a given input size.
#[derive(Clone, Copy)]
struct ChainQuote {
    out: [u64; 3],
    fee: [u64; 3],
    bins: [u32; 3],
    swap_for_y: [bool; 3],
}

impl ChainQuote {
    fn gross(&self, input: u64) -> i64 {
        self.out[2] as i64 - input as i64
    }
}

/// Chain a quote through the three pools of `chain` at `input` size.
fn quote_chain(
    pools: &[PoolState],
    pool_indexes: (usize, usize, usize),
    a: MintId,
    b: MintId,
    input: u64,
) -> Option<ChainQuote> {
    let wsol = wsol_mint();
    let (i, j, k) = pool_indexes;
    if i >= pools.len() || j >= pools.len() || k >= pools.len() {
        return None;
    }
    let (p1, p2, p3) = (&pools[i], &pools[j], &pools[k]);
    let (out1, q1, d1) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p1, input, wsol, a)?;
    let (out2, q2, d2) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p2, out1, a, b)?;
    let (out3, q3, d3) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p3, out2, b, wsol)?;
    if out3 == 0 {
        return None;
    }
    Some(ChainQuote {
        out: [out1, out2, out3],
        fee: [q1.fee_amount, q2.fee_amount, q3.fee_amount],
        bins: [q1.bins_crossed, q2.bins_crossed, q3.bins_crossed],
        swap_for_y: [d1, d2, d3],
    })
}

/// Enumerate all ordered distinct (p1, p2, p3) triples that form a
/// WSOL -> A -> B -> WSOL cycle. Returns every feasible chain with its quote
/// at the discovery input size.
fn enumerate_cycles(pools: &[PoolState], input_size: u64) -> Vec<(Chain, ChainQuote)> {
    let wsol = wsol_mint();
    let n = pools.len();
    let mut out = Vec::new();

    for i in 0..n {
        for j in 0..n {
            if j == i {
                continue;
            }
            for k in 0..n {
                if k == i || k == j {
                    continue;
                }
                let (p1, p2, p3) = (&pools[i], &pools[j], &pools[k]);

                // p1: WSOL <-> A, A != WSOL.
                let (a1, a2) = (p1.base_mint, p1.quote_mint);
                let a = if a1 == wsol {
                    a2
                } else if a2 == wsol {
                    a1
                } else {
                    continue;
                };
                if a == wsol {
                    continue;
                }
                // p3: B <-> WSOL, B != WSOL, B != A.
                let (b1, b2) = (p3.base_mint, p3.quote_mint);
                let b = if b1 == wsol {
                    b2
                } else if b2 == wsol {
                    b1
                } else {
                    continue;
                };
                if b == wsol || b == a {
                    continue;
                }
                // p2: must bridge A <-> B.
                let p2_mints = [p2.base_mint, p2.quote_mint];
                if !p2_mints.contains(&a) || !p2_mints.contains(&b) {
                    continue;
                }
                let Some(quote) = quote_chain(pools, (i, j, k), a, b, input_size) else {
                    continue;
                };
                let profit = quote.gross(input_size) as i128;
                out.push((
                    Chain {
                        i,
                        j,
                        k,
                        a,
                        b,
                        profit,
                    },
                    quote,
                ));
            }
        }
    }
    out
}

/// Run the integrated pipeline.
///
/// 1. Scans the provided pools and finds the best distinct 3-leg
///    `WSOL -> A -> B -> WSOL` cycle by chained quote profitability.
/// 2. Re-sizes the route across `params.size_search_sizes` (default: `[input_size]`).
/// 3. Resolves per-leg execution accounts into `ThreeLegMemeArbitrageContext`.
pub fn build_three_leg_meme_pipeline(
    pools: &[PoolState],
    params: &ThreeLegMemeContextParams,
) -> Result<ThreeLegMemePipelineResult, ThreeLegMemePipelineError> {
    let candidates = valid_meteora_pools(pools);
    if candidates.is_empty() {
        return Err(ThreeLegMemePipelineError::NoEligiblePools);
    }
    if candidates.len() < 3 {
        return Err(ThreeLegMemePipelineError::NoCycleFound {
            pools: candidates.len(),
        });
    }

    // Discovery report (kept for journaling).
    let discovery = MeteoraDlmmPoolPairDiscovery::discover_pairs(&candidates);

    // Route graph: pick the most profitable feasible triple at the discovery size.
    let wsol = wsol_mint();
    let discovery_size = params.input_size;
    let cycles = enumerate_cycles(&candidates, discovery_size);
    if cycles.is_empty() {
        return Err(ThreeLegMemePipelineError::NoCycleFound {
            pools: candidates.len(),
        });
    }
    let best_chain = cycles
        .iter()
        .max_by_key(|(chain, _)| chain.profit)
        .map(|(chain, _)| *chain)
        .expect("cycles is non-empty");

    // Trade size optimizer: 3-leg-native evaluation over the candidate set.
    // The objective is NET profit (gross - Kamino flash fee - network cost),
    // never gross output. Candidates above the actual available flash
    // liquidity (when known) are dropped, not silently clamped.
    let sizes: Vec<u64> = {
        let all: Vec<u64> = if params.size_search_sizes.is_empty() {
            vec![discovery_size]
        } else {
            params.size_search_sizes.clone()
        };
        match params.kamino_available_liquidity {
            Some(available) => {
                let filtered: Vec<u64> = all.into_iter().filter(|s| *s <= available).collect();
                if filtered.is_empty() {
                    return Err(ThreeLegMemePipelineError::InsufficientFlashLiquidity {
                        available,
                    });
                }
                filtered
            }
            None => all,
        }
    };

    let mut tested: Vec<(u64, i64)> = Vec::with_capacity(sizes.len());
    let mut best_size = discovery_size;
    let mut best_profit: i64 = i64::MIN;
    let mut best_quote: Option<ChainQuote> = None;
    for size in &sizes {
        let quote = quote_chain(
            &candidates,
            (best_chain.i, best_chain.j, best_chain.k),
            best_chain.a,
            best_chain.b,
            *size,
        );
        let profit = quote.map_or(i64::MIN, |q| net_profit_of(q, *size, params));
        tested.push((*size, profit));
        if profit > best_profit {
            best_profit = profit;
            best_size = *size;
            best_quote = quote;
        }
    }
    if best_profit < params.min_profit_lamports as i64 {
        return Err(ThreeLegMemePipelineError::BelowNetProfitFloor {
            best_net: best_profit,
            floor: params.min_profit_lamports,
        });
    }
    let chosen_quote = best_quote.unwrap_or_else(|| {
        quote_chain(
            &candidates,
            (best_chain.i, best_chain.j, best_chain.k),
            best_chain.a,
            best_chain.b,
            best_size,
        )
        .expect("discovery size must quote")
    });
    let size_search = ThreeLegMemeSizeSearchResult {
        tested_sizes: tested,
        best_input: best_size,
        best_net_profit_lamports: best_profit,
    };
    let flash_fee_lamports =
        crate::flash_loan_sizing::calculate_kamino_flash_fee(best_size, params.flash_loan_fee_sf);

    // Per-leg quote records at the chosen size.
    let p1 = &candidates[best_chain.i];
    let p2 = &candidates[best_chain.j];
    let p3 = &candidates[best_chain.k];
    let legs = build_leg_records(
        p1,
        p2,
        p3,
        best_size,
        wsol,
        &chosen_quote,
        best_chain.a,
        best_chain.b,
    )?;

    let input = best_size;
    let final_out = legs[2].amount_out;
    let gross = final_out as i128 - input as i128;

    // Account resolution: build the executable context.
    let context = build_context(p1, p2, p3, &legs, params, best_chain.a, best_chain.b, wsol)?;

    // Repayment guarantee: even at the slippage floor of the final leg, the
    // route must still cover principal + flash fee, otherwise the flash loan
    // cannot be repaid atomically and the whole transaction would revert.
    let required_repayment = input.saturating_add(flash_fee_lamports);
    if context.expected_min_final_output < required_repayment {
        return Err(ThreeLegMemePipelineError::RepaymentNotGuaranteed {
            min_final_output: context.expected_min_final_output,
            required: required_repayment,
        });
    }

    let expected_min_final_output = context.expected_min_final_output;
    Ok(ThreeLegMemePipelineResult {
        pool_ids: [p1.pool_id.0, p2.pool_id.0, p3.pool_id.0],
        meme_mints: [best_chain.a, best_chain.b],
        legs,
        size_search,
        context,
        discovery,
        expected_final_out: final_out,
        gross_profit_lamports: gross as i64,
        flash_fee_lamports,
        net_profit_lamports: best_profit,
        expected_min_final_output,
    })
}

/// NET profit of a fully-quoted chain at `input`: gross output minus input,
/// minus the Kamino flash fee at the on-chain U68F60 rate, minus the
/// configured flat network cost. All integer, all checked.
fn net_profit_of(quote: ChainQuote, input: u64, params: &ThreeLegMemeContextParams) -> i64 {
    // Delegates to the canonical accounting in flash_loan_sizing (single
    // source of truth for net profit across the optimizer and this pipeline).
    crate::flash_loan_sizing::net_profit_lamports(
        quote.out[2],
        input,
        params.flash_loan_fee_sf,
        params.network_cost_lamports,
    )
}

fn build_leg_records(
    p1: &PoolState,
    p2: &PoolState,
    p3: &PoolState,
    input: u64,
    wsol: MintId,
    quote: &ChainQuote,
    a: MintId,
    b: MintId,
) -> Result<[ThreeLegMemeLegQuote; 3], ThreeLegMemePipelineError> {
    let pool_ids = [p1.pool_id, p2.pool_id, p3.pool_id];
    let from_to = [(wsol, a), (a, b), (b, wsol)];
    let mut legs: Vec<ThreeLegMemeLegQuote> = Vec::with_capacity(3);
    let mut amount = input;
    for (i, ((from, to), pool_id)) in from_to.iter().zip(pool_ids.iter()).enumerate() {
        legs.push(ThreeLegMemeLegQuote {
            leg: i,
            pool_id: *pool_id,
            from_mint: *from,
            to_mint: *to,
            swap_for_y: quote.swap_for_y[i],
            amount_in: amount,
            amount_out: quote.out[i],
            fee: quote.fee[i],
            bins_traversed: quote.bins[i],
        });
        amount = quote.out[i];
    }
    Ok([legs.remove(0), legs.remove(0), legs.remove(0)])
}

fn token_program_pubkey(program: TokenProgramKind) -> Pubkey {
    use std::str::FromStr;
    match program {
        TokenProgramKind::SplToken => Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
        TokenProgramKind::Token2022 => {
            Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap()
        }
    }
}

fn direction_from_swap_for_y(swap_for_y: bool) -> SwapDirection {
    if swap_for_y {
        SwapDirection::XtoY
    } else {
        SwapDirection::YtoX
    }
}

#[allow(clippy::too_many_arguments)]
fn build_context(
    p1: &PoolState,
    p2: &PoolState,
    p3: &PoolState,
    legs: &[ThreeLegMemeLegQuote; 3],
    params: &ThreeLegMemeContextParams,
    a: MintId,
    b: MintId,
    wsol: MintId,
) -> Result<ThreeLegMemeArbitrageContext, ThreeLegMemePipelineError> {
    let wsol_ata = arb_tx::get_associated_token_address_with_program_id(
        &params.payer,
        &wsol.0,
        &params.wsol_token_program,
    );

    let slippage_mul = |amount: u64| -> u64 {
        amount.saturating_mul(10_000u64.saturating_sub(params.slippage_bps)) / 10_000
    };

    // Floor for the whole chain: the final leg's quoted output minus slippage.
    let expected_min_final_output = slippage_mul(legs[2].amount_out);

    let pools = [p1, p2, p3];
    let mut leg_accounts: Vec<LegExecutionAccounts> = Vec::with_capacity(3);
    for (idx, (pool, leg_quote)) in pools.iter().zip(legs.iter()).enumerate() {
        let exec = pool
            .execution_accounts
            .clone()
            .ok_or(ThreeLegMemePipelineError::MissingExecutionAccounts { leg: idx })?;
        let PoolExecutionAccounts::MeteoraDlmm {
            token_x_mint: _,
            token_y_mint: _,
            reserve_x,
            reserve_y,
            oracle,
            bin_array_pdas: _,
            bitmap_extension,
        } = exec
        else {
            return Err(ThreeLegMemePipelineError::UnsupportedExecutionAccounts { leg: idx });
        };
        let active_id = pool.dlmm.as_ref().map(|d| d.active_bin_id).unwrap_or(0);
        let direction = direction_from_swap_for_y(leg_quote.swap_for_y);
        let derived = derive_directional_bin_arrays(
            &pool.pool_id.0,
            active_id,
            direction,
            leg_quote.bins_traversed,
        )
        .map_err(|_| ThreeLegMemePipelineError::TooManyBinArrays {
            leg: idx,
            bins: leg_quote.bins_traversed,
        })?;

        let token_x_mint = pool.base_mint;
        let token_y_mint = pool.quote_mint;
        let token_x_program = token_program_pubkey(pool.token_programs[0]);
        let token_y_program = token_program_pubkey(pool.token_programs[1]);

        let (user_token_in, user_token_out) = match idx {
            0 => (wsol_ata, params.meme_a_ata),
            1 => (params.meme_a_ata, params.meme_b_ata),
            2 => (params.meme_b_ata, wsol_ata),
            _ => unreachable!(),
        };

        leg_accounts.push(LegExecutionAccounts {
            lb_pair: pool.pool_id.0,
            bin_array_pdas: derived.bin_array_pdas,
            bitmap_extension,
            reserve_x,
            reserve_y,
            token_x_mint: token_x_mint.0,
            token_y_mint: token_y_mint.0,
            token_x_program,
            token_y_program,
            oracle,
            host_fee_in: None,
            user_token_in,
            user_token_out,
        });
    }

    let [leg_1_accounts, leg_2_accounts, leg_3_accounts] = [
        leg_accounts.remove(0),
        leg_accounts.remove(0),
        leg_accounts.remove(0),
    ];

    let mk_params = |leg_quote: &ThreeLegMemeLegQuote| -> LegSwapParams {
        LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: leg_quote.amount_in,
            min_amount_out: slippage_mul(leg_quote.amount_out),
            max_amount_in: 0,
            amount_out: 0,
        }
    };

    Ok(ThreeLegMemeArbitrageContext {
        payer: params.payer,
        wsol_mint: wsol.0,
        meme_a_mint: a,
        meme_b_mint: b,
        pool_1: p1.pool_id,
        pool_2: p2.pool_id,
        pool_3: p3.pool_id,
        leg_1_direction: direction_from_swap_for_y(legs[0].swap_for_y),
        leg_2_direction: direction_from_swap_for_y(legs[1].swap_for_y),
        leg_3_direction: direction_from_swap_for_y(legs[2].swap_for_y),
        leg_1_accounts,
        leg_2_accounts,
        leg_3_accounts,
        leg_1_params: mk_params(&legs[0]),
        leg_2_params: mk_params(&legs[1]),
        leg_3_params: mk_params(&legs[2]),
        wsol_token_program: params.wsol_token_program,
        meme_a_token_program: params.meme_a_token_program,
        meme_b_token_program: params.meme_b_token_program,
        meme_a_ata: params.meme_a_ata,
        meme_b_ata: params.meme_b_ata,
        meme_a_ata_exists: params.meme_a_ata_exists,
        meme_b_ata_exists: params.meme_b_ata_exists,
        meme_a_ata_balance: params.meme_a_ata_balance,
        meme_b_ata_balance: params.meme_b_ata_balance,
        meme_a_is_token_2022: params.meme_a_is_token_2022,
        meme_b_is_token_2022: params.meme_b_is_token_2022,
        meme_a_withheld_fees: params.meme_a_withheld_fees,
        meme_b_withheld_fees: params.meme_b_withheld_fees,
        snapshot_slot: params.snapshot_slot,
        instruction_version: 0,
        cu_limit: params.cu_limit,
        priority_fee_micro_lamports: params.priority_fee_micro_lamports,
        expected_min_final_output,
        jito_tip_account: params.jito_tip_account,
        jito_tip_lamports: params.jito_tip_lamports,
        recent_blockhash: params.recent_blockhash,
        v1_transport_supported: params.v1_transport_supported,
    })
}

/// Profitability gate: final output must exceed the input by `min_profit_lamports`.
///
/// This is the final math gate before any (currently disallowed) live submission.
/// Returns the gross profit in lamports when it passes.
pub fn simulate_profitability_gate(
    result: &ThreeLegMemePipelineResult,
    min_profit_lamports: u64,
) -> Result<i64, ThreeLegMemePipelineError> {
    let net = result.net_profit_lamports;
    if net < min_profit_lamports as i64 {
        return Err(ThreeLegMemePipelineError::Internal(format!(
            "profitability gate: net profit {net} (gross {}) < floor {min_profit_lamports}",
            result.gross_profit_lamports
        )));
    }
    Ok(net)
}
