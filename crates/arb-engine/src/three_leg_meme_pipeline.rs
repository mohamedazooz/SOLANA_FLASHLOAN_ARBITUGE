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
    /// Payer WSOL settlement ATA balance. Any non-zero value fails the
    /// pipeline closed: pre-existing WSOL could subsidize the flash-loan
    /// repayment, breaking the proceeds-only solvency invariant.
    pub wsol_ata_balance: u64,
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
            wsol_ata_balance: 0,
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
            wsol_ata_balance: 0,
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
    #[error("leg {leg}: pool is missing DLMM state (active bin unknown)")]
    MissingDlmmState { leg: usize },
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
    #[error(
        "pre-existing {token} ATA balance {balance} would subsidize execution (fail-closed provenance policy)"
    )]
    PreExistingInventorySubsidy { token: &'static str, balance: u64 },
    #[error(
        "leg {leg}: traversal leaves the default bitmap but no existence-proven bitmap extension is available"
    )]
    BitmapExtensionRequired { leg: usize },
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
///
/// Chaining is CONSERVATIVE (sequential min-out): leg N+1 is quoted at
/// `slippage_floor(leg N output)`, never at the optimistic expected output.
/// `inputs[i]` is therefore exactly the amount the on-chain leg-i
/// instruction can rely on receiving: leg i-1 carries
/// `min_amount_out = inputs[i]`, so a leg can never consume wallet
/// inventory beyond what the previous leg guarantees.
#[derive(Clone, Copy)]
struct ChainQuote {
    inputs: [u64; 3],
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

/// Chain a quote through the three pools of `chain` at `input` size using
/// CONSERVATIVE sequential chaining:
///
///   in1 = input
///   out1 = quote(p1, in1);   in2 = floor(out1 * (10000 - s) / 10000)
///   out2 = quote(p2, in2);   in3 = floor(out2 * (10000 - s) / 10000)
///   out3 = quote(p3, in3)
///
/// Each hop's input equals the previous hop's enforced `min_amount_out`,
/// so the on-chain chain never depends on optimistic outputs and can never
/// consume pre-existing wallet inventory to bridge a leg shortfall.
/// Slippage is applied exactly once per hop boundary (no double-count).
/// All arithmetic is checked/saturating on u64.
fn quote_chain(
    pools: &[PoolState],
    pool_indexes: (usize, usize, usize),
    a: MintId,
    b: MintId,
    input: u64,
    slippage_bps: u64,
) -> Option<ChainQuote> {
    let wsol = wsol_mint();
    let (i, j, k) = pool_indexes;
    if i >= pools.len() || j >= pools.len() || k >= pools.len() {
        return None;
    }
    let slippage_floor = |amount: u64| -> u64 {
        amount.saturating_mul(10_000u64.saturating_sub(slippage_bps)) / 10_000
    };
    let (p1, p2, p3) = (&pools[i], &pools[j], &pools[k]);
    let (out1, q1, d1) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p1, input, wsol, a)?;
    let in2 = slippage_floor(out1);
    let (out2, q2, d2) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p2, in2, a, b)?;
    let in3 = slippage_floor(out2);
    let (out3, q3, d3) = MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(p3, in3, b, wsol)?;
    if out3 == 0 {
        return None;
    }
    Some(ChainQuote {
        inputs: [input, in2, in3],
        out: [out1, out2, out3],
        fee: [q1.fee_amount, q2.fee_amount, q3.fee_amount],
        bins: [q1.bins_crossed, q2.bins_crossed, q3.bins_crossed],
        swap_for_y: [d1, d2, d3],
    })
}

/// Enumerate all ordered distinct (p1, p2, p3) triples that form a
/// WSOL -> A -> B -> WSOL cycle. Returns every feasible chain with its quote
/// at the discovery input size.
fn enumerate_cycles(
    pools: &[PoolState],
    input_size: u64,
    slippage_bps: u64,
) -> Vec<(Chain, ChainQuote)> {
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
                let Some(quote) = quote_chain(pools, (i, j, k), a, b, input_size, slippage_bps)
                else {
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
    let cycles = enumerate_cycles(&candidates, discovery_size, params.slippage_bps);
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
            params.slippage_bps,
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
            params.slippage_bps,
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
    let legs = build_leg_records(p1, p2, p3, wsol, &chosen_quote, best_chain.a, best_chain.b)?;

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
    wsol: MintId,
    quote: &ChainQuote,
    a: MintId,
    b: MintId,
) -> Result<[ThreeLegMemeLegQuote; 3], ThreeLegMemePipelineError> {
    let pool_ids = [p1.pool_id, p2.pool_id, p3.pool_id];
    let from_to = [(wsol, a), (a, b), (b, wsol)];
    let mut legs: Vec<ThreeLegMemeLegQuote> = Vec::with_capacity(3);
    for (i, ((from, to), pool_id)) in from_to.iter().zip(pool_ids.iter()).enumerate() {
        legs.push(ThreeLegMemeLegQuote {
            leg: i,
            pool_id: *pool_id,
            from_mint: *from,
            to_mint: *to,
            swap_for_y: quote.swap_for_y[i],
            amount_in: quote.inputs[i],
            amount_out: quote.out[i],
            fee: quote.fee[i],
            bins_traversed: quote.bins[i],
        });
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
    // Fail-closed inventory-provenance policy: any pre-existing payer token
    // balance in a settlement/intermediate ATA could subsidize a leg shortfall
    // or the flash-loan repayment. The chain must be solvent from trade
    // proceeds alone, so a non-zero balance rejects the context outright.
    if params.wsol_ata_balance > 0 {
        return Err(ThreeLegMemePipelineError::PreExistingInventorySubsidy {
            token: "wsol",
            balance: params.wsol_ata_balance,
        });
    }
    if params.meme_a_ata_balance > 0 {
        return Err(ThreeLegMemePipelineError::PreExistingInventorySubsidy {
            token: "meme_a",
            balance: params.meme_a_ata_balance,
        });
    }
    if params.meme_b_ata_balance > 0 {
        return Err(ThreeLegMemePipelineError::PreExistingInventorySubsidy {
            token: "meme_b",
            balance: params.meme_b_ata_balance,
        });
    }
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
            reserve_x,
            reserve_y,
            oracle,
            bin_array_pdas: _,
            bitmap_extension: bitmap_ext_pda,
            ..
        } = exec
        else {
            return Err(ThreeLegMemePipelineError::UnsupportedExecutionAccounts { leg: idx });
        };
        let active_id = pool
            .dlmm
            .as_ref()
            .ok_or(ThreeLegMemePipelineError::MissingDlmmState { leg: idx })?
            .active_bin_id;
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
        // Fail-closed bitmap-extension policy (PR24 §4): the extension account
        // is passed only when the traversal leaves the default bitmap AND the
        // registry has proven the account exists (fetched on-chain). A required
        // but unavailable extension rejects the leg; an unneeded extension is
        // encoded as the official program-ID sentinel.
        let bitmap_extension = if derived.bitmap_extension_required {
            Some(
                bitmap_ext_pda
                    .ok_or(ThreeLegMemePipelineError::BitmapExtensionRequired { leg: idx })?,
            )
        } else {
            bitmap_ext_pda
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use arb_types::{PoolKind, Slot};

    fn wsol() -> MintId {
        MintId(Pubkey::from_str(WSOL_MINT).unwrap())
    }

    fn pool_without_dlmm_state(pool_byte: u8, base_mint: MintId, quote_mint: MintId) -> PoolState {
        let exec = PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_from_array([pool_byte.wrapping_add(100); 32]),
            reserve_y: Pubkey::new_from_array([pool_byte.wrapping_add(150); 32]),
            oracle: Pubkey::new_from_array([pool_byte.wrapping_add(200); 32]),
            bin_array_pdas: vec![Pubkey::new_from_array([pool_byte.wrapping_add(50); 32])],
            bitmap_extension: None,
            pool_token_x: base_mint.0,
            pool_token_y: quote_mint.0,
        };
        PoolState {
            pool_id: PoolId(Pubkey::new_from_array([pool_byte; 32])),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(467_000_000),
            write_version: 1,
            base_mint,
            quote_mint,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 10_u128.pow(12),
            reserve_out: 10_u128.pow(12),
            fee_numerator: 1,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            execution_accounts: Some(exec),
            last_update_slot: 467_000_000,
            last_update_ms: 1_700_000_000_000,
        }
    }

    fn leg_quote(leg: usize, pool: &PoolState) -> ThreeLegMemeLegQuote {
        ThreeLegMemeLegQuote {
            leg,
            pool_id: pool.pool_id,
            from_mint: pool.base_mint,
            to_mint: pool.quote_mint,
            swap_for_y: true,
            amount_in: 1_000_000,
            amount_out: 1_100_000,
            fee: 1_000,
            bins_traversed: 0,
        }
    }

    fn context_params() -> ThreeLegMemeContextParams {
        let payer = Pubkey::new_from_array([1u8; 32]);
        ThreeLegMemeContextParams {
            payer,
            meme_a_ata: Pubkey::new_from_array([2u8; 32]),
            meme_b_ata: Pubkey::new_from_array([3u8; 32]),
            wsol_ata_balance: 0,
            meme_a_ata_exists: false,
            meme_b_ata_exists: false,
            meme_a_ata_balance: 0,
            meme_b_ata_balance: 0,
            meme_a_is_token_2022: false,
            meme_b_is_token_2022: false,
            meme_a_withheld_fees: 0,
            meme_b_withheld_fees: 0,
            wsol_token_program: Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            meme_a_token_program: Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            meme_b_token_program: Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
            snapshot_slot: 467_000_000,
            cu_limit: 400_000,
            priority_fee_micro_lamports: 1_000,
            jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
            jito_tip_lamports: 10_000,
            recent_blockhash: [42u8; 32],
            v1_transport_supported: false,
            slippage_bps: 100,
            min_profit_lamports: 0,
            flash_loan_fee_sf: 0,
            network_cost_lamports: 0,
            kamino_available_liquidity: None,
            input_size: 1_000_000,
            size_search_sizes: vec![1_000_000],
        }
    }

    /// A pool whose DLMM snapshot is absent must fail closed with
    /// `MissingDlmmState`, never silently derive bin-array 0 from a
    /// fabricated `active_bin_id = 0`.
    #[test]
    fn build_context_fails_closed_when_dlmm_state_missing() {
        let wsol = wsol();
        let a = MintId(Pubkey::new_from_array([90u8; 32]));
        let b = MintId(Pubkey::new_from_array([91u8; 32]));

        let p1 = pool_without_dlmm_state(1, wsol, a);
        let p2 = pool_without_dlmm_state(2, a, b);
        let p3 = pool_without_dlmm_state(3, b, wsol);

        let legs = [leg_quote(0, &p1), leg_quote(1, &p2), leg_quote(2, &p3)];

        let err = build_context(&p1, &p2, &p3, &legs, &context_params(), a, b, wsol)
            .expect_err("missing DLMM state must be an error, not bin-array 0");
        assert!(
            matches!(err, ThreeLegMemePipelineError::MissingDlmmState { leg: 0 }),
            "unexpected error: {err:?}"
        );
    }

    /// PR24 §2.2: any pre-existing payer balance in a settlement or
    /// intermediate ATA must fail the context closed — it could subsidize a
    /// leg shortfall or the flash-loan repayment.
    #[test]
    fn build_context_rejects_pre_existing_inventory() {
        let wsol = wsol();
        let a = MintId(Pubkey::new_from_array([92u8; 32]));
        let b = MintId(Pubkey::new_from_array([93u8; 32]));

        let p1 = pool_without_dlmm_state(1, wsol, a);
        let p2 = pool_without_dlmm_state(2, a, b);
        let p3 = pool_without_dlmm_state(3, b, wsol);
        let legs = [leg_quote(0, &p1), leg_quote(1, &p2), leg_quote(2, &p3)];

        for (token, field) in [("wsol", 0u64), ("meme_a", 1u64), ("meme_b", 2u64)] {
            let mut params = context_params();
            match field {
                0 => params.wsol_ata_balance = 123,
                1 => params.meme_a_ata_balance = 123,
                _ => params.meme_b_ata_balance = 123,
            }
            let err = build_context(&p1, &p2, &p3, &legs, &params, a, b, wsol)
                .expect_err("pre-existing inventory must be rejected");
            assert!(
                matches!(
                    err,
                    ThreeLegMemePipelineError::PreExistingInventorySubsidy { token: t, balance: 123 }
                        if t == token
                ),
                "unexpected error for {token}: {err:?}"
            );
        }

        // All-zero provenance passes the subsidy gate (DLMM state still
        // missing, so the error must be MissingDlmmState, not subsidy).
        let params = context_params();
        let err = build_context(&p1, &p2, &p3, &legs, &params, a, b, wsol)
            .expect_err("dlmm state still missing");
        assert!(matches!(
            err,
            ThreeLegMemePipelineError::MissingDlmmState { leg: 0 }
        ));
    }

    /// PR24 §4: traversal leaving the default bitmap (-512..=511) requires an
    /// existence-proven bitmap extension; without one the leg fails closed.
    #[test]
    fn build_context_fails_closed_when_bitmap_extension_required_but_absent() {
        let wsol = wsol();
        let a = MintId(Pubkey::new_from_array([96u8; 32]));
        let b = MintId(Pubkey::new_from_array([97u8; 32]));

        // Active bin 35_840 -> array index 512, one outside the default bitmap.
        let out_of_range_active = 35_840i32;
        let mut p1 = pool_without_dlmm_state(1, wsol, a);
        p1.dlmm = Some(arb_types::DlmmState {
            active_bin_id: out_of_range_active,
            status: 0,
            bin_step_bps: 10,
            bins: Vec::new(),
            params: None,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: out_of_range_active,
            last_update_timestamp: 1_700_000_000,
        });
        let p2 = pool_without_dlmm_state(2, a, b);
        let p3 = pool_without_dlmm_state(3, b, wsol);
        let legs = [leg_quote(0, &p1), leg_quote(1, &p2), leg_quote(2, &p3)];

        let err = build_context(&p1, &p2, &p3, &legs, &context_params(), a, b, wsol)
            .expect_err("required-but-absent bitmap extension must fail closed");
        assert!(
            matches!(
                err,
                ThreeLegMemePipelineError::BitmapExtensionRequired { leg: 0 }
            ),
            "unexpected error: {err:?}"
        );

        // With an existence-proven extension account the bitmap gate passes
        // and the leg proceeds to account assembly. All legs need DLMM state
        // so the context assembles fully.
        let ext = arb_tx::derive_meteora_bin_array_bitmap_extension_pda(&p1.pool_id.0);
        let mut p1_proven = p1.clone();
        let mut p2_proven = p2.clone();
        let mut p3_proven = p3.clone();
        for (pool, active) in [
            (&mut p1_proven, out_of_range_active),
            (&mut p2_proven, 0i32),
            (&mut p3_proven, 0i32),
        ] {
            pool.dlmm = Some(arb_types::DlmmState {
                active_bin_id: active,
                status: 0,
                bin_step_bps: 10,
                bins: Vec::new(),
                params: None,
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: active,
                last_update_timestamp: 1_700_000_000,
            });
        }
        if let Some(arb_types::PoolExecutionAccounts::MeteoraDlmm {
            bitmap_extension, ..
        }) = p1_proven.execution_accounts.as_mut()
        {
            *bitmap_extension = Some(ext);
        }
        let ok = build_context(
            &p1_proven,
            &p2_proven,
            &p3_proven,
            &legs,
            &context_params(),
            a,
            b,
            wsol,
        );
        assert!(ok.is_ok(), "proven extension must pass: {ok:?}");
    }

    /// PR24 §2.3: conservative sequential chaining — leg N+1's input is
    /// leg N's slippage-floor min output, strictly below the optimistic
    /// output whenever slippage_bps > 0.
    #[test]
    fn quote_chain_inputs_are_conservative_min_outs() {
        let slippage_bps = 100u64;
        let floor = |amt: u64| amt.saturating_mul(10_000 - slippage_bps) / 10_000;
        let wsol = wsol();
        let a = MintId(Pubkey::new_from_array([94u8; 32]));
        let b = MintId(Pubkey::new_from_array([95u8; 32]));
        let mk = |byte: u8, x: MintId, y: MintId| pool_without_dlmm_state(byte, x, y);
        let pools = [mk(1, wsol, a), mk(2, a, b), mk(3, b, wsol)];
        // quote_single_hop requires DLMM state; pool_without_dlmm_state has
        // dlmm: None, so quote_chain returns None here — the conservative
        // input contract is instead verified through the pipeline-level test
        // (three_leg_meme_pipeline_tests). This test pins the contract at the
        // leg-record level using a synthetic ChainQuote.
        let quote = ChainQuote {
            inputs: [1_000_000, floor(904_873_582), floor(808_080_808)],
            out: [904_873_582, 808_080_808, 700_000_000],
            fee: [1_000; 3],
            bins: [0; 3],
            swap_for_y: [true; 3],
        };
        let legs = build_leg_records(&pools[0], &pools[1], &pools[2], wsol, &quote, a, b)
            .expect("leg records");
        assert_eq!(legs[0].amount_in, 1_000_000);
        assert_eq!(legs[1].amount_in, floor(legs[0].amount_out));
        assert_eq!(legs[2].amount_in, floor(legs[1].amount_out));
        assert!(legs[1].amount_in < legs[0].amount_out);
        assert!(legs[2].amount_in < legs[1].amount_out);
    }
}
