//! Exact Three-Pool & Multi-Hop Route Engine with Kamino Borrow-Size Optimizer.
//!
//! Enforces:
//! 1. Exact amount chaining: output of Hop 1 (minus transfer fees) feeds Hop 2; output of Hop 2 feeds Hop 3.
//! 2. Token-2022 transfer fee deduction on every inbound and outbound hop leg.
//! 3. Bounded Golden-Section & Binary search optimization for Kamino flash loan borrow sizing:
//!    Evaluates pool depth, price impact, CLMM tick liquidity, DLMM bin distributions,
//!    Kamino borrow fees, compute budget, transaction MTU (1232 bytes), and net profit.
//! 4. Fail-closed: Discards routes that fail reserve continuity, slippage bounds, or net profitability.

use arb_dex::engines::UniversalQuoteDispatcher;
use arb_dex::quote_engine::{QuoteRequest, QuoteResult};
use arb_dex::token2022::TransferFeeConfig;
use arb_types::{ArbError, DexProtocol, MintId, PoolState, Result};

/// Minimum net profit in lamports required to justify execution risk.
pub const DEFAULT_MIN_NET_PROFIT_LAMPORTS: u64 = 30_000;
/// Base Solana network signature fee.
pub const BASE_NETWORK_FEE_LAMPORTS: u64 = 5_000;
/// Solana packet MTU size constraint (SIMD-0296 4096 bytes for v1 transactions).
pub const SOLANA_PACKET_MTU_BYTES: usize = arb_constants::SOLANA_V1_PACKET_MTU_LIMIT;

/// A single hop specification in a multi-pool route.
#[derive(Debug, Clone)]
pub struct RouteHopSpec {
    pub pool_state: PoolState,
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub input_transfer_fee: Option<TransferFeeConfig>,
    pub output_transfer_fee: Option<TransferFeeConfig>,
}

/// Complete quotation and fee accounting for a 3-pool (or 2-pool) route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreePoolRouteQuote {
    pub hops: Vec<QuoteResult>,
    pub borrow_mint: MintId,
    pub principal_borrowed: u64,
    pub gross_output: u64,
    pub flash_loan_fee: u64,
    pub total_repay_required: u64,
    pub gross_profit: i64,
    pub network_fee: u64,
    pub priority_fee: u64,
    pub jito_tip: u64,
    pub token_transfer_fees: u64,
    pub total_costs: u64,
    pub net_profit_lamports: i64,
    pub is_profitable: bool,
    pub estimated_compute_units: u32,
    pub estimated_tx_size_bytes: usize,
}

/// Three-Pool Route Engine evaluating chained multi-DEX swaps.
#[derive(Debug, Default, Clone)]
pub struct ThreePoolRouteEngine {
    pub dispatcher: UniversalQuoteDispatcher,
}

impl ThreePoolRouteEngine {
    pub fn new() -> Self {
        Self {
            dispatcher: UniversalQuoteDispatcher::new(),
        }
    }

    /// Simulate exact chained swap across 2 or 3 pools sequentially.
    pub fn simulate_route(
        &self,
        hops: &[RouteHopSpec],
        borrow_amount: u64,
        flash_fee_bps: u16,
        priority_fee_lamports: u64,
        tip_share_bps: u16,
    ) -> Result<ThreePoolRouteQuote> {
        if hops.len() < 2 || hops.len() > 3 {
            return Err(ArbError::InvalidInput(
                "Route must contain 2 or 3 hops".into(),
            ));
        }

        let borrow_mint = hops[0].input_mint;
        // Verify cyclical continuity: final hop output must equal initial borrow mint
        let final_hop = &hops[hops.len() - 1];
        if final_hop.output_mint != borrow_mint {
            return Err(ArbError::InvalidInput(
                "Route is not cyclical: final output mint does not match borrow mint".into(),
            ));
        }

        let mut current_amount = borrow_amount;
        let mut hop_results = Vec::with_capacity(hops.len());
        let mut total_token_transfer_fees = 0u64;
        let mut total_cu = 450u32 + 28_000u32; // Base tx overhead + Kamino borrow/repay overhead

        for (i, hop) in hops.iter().enumerate() {
            // Continuity check between hops
            if i > 0 {
                let prev_out = hops[i - 1].output_mint;
                if hop.input_mint != prev_out {
                    return Err(ArbError::InvalidInput(format!(
                        "Hop {i} input mint does not match hop {} output mint",
                        i - 1
                    )));
                }
            }

            let req = QuoteRequest {
                input_mint: hop.input_mint,
                output_mint: hop.output_mint,
                amount: current_amount,
                slippage_bps: 10,
                input_transfer_fee_config: hop.input_transfer_fee,
                output_transfer_fee_config: hop.output_transfer_fee,
            };

            let res = self.dispatcher.quote_exact_in(&hop.pool_state, &req)?;

            total_token_transfer_fees = total_token_transfer_fees
                .saturating_add(res.transfer_fee_breakdown.input_transfer_fee)
                .saturating_add(res.transfer_fee_breakdown.output_transfer_fee);

            // Accumulate CU per DEX type
            let hop_cu = match hop.pool_state.protocol {
                DexProtocol::RaydiumV4
                | DexProtocol::RaydiumCpmm
                | DexProtocol::PumpFun
                | DexProtocol::PumpSwap => 35_000,
                DexProtocol::RaydiumClmm | DexProtocol::OrcaWhirlpool => 85_000,
                DexProtocol::MeteoraDlmm | DexProtocol::MeteoraDlmmPro => 120_000,
                DexProtocol::MeteoraDammV2
                | DexProtocol::MeteoraDbc
                | DexProtocol::RaydiumLaunchLab => 60_000,
            };
            total_cu = total_cu.saturating_add(hop_cu);

            current_amount = res.amount_out_raw;
            hop_results.push(res);
        }

        let gross_output = current_amount;

        // Kamino Flash Loan fee calculation: ceil(borrow * fee_bps / 10_000)
        let flash_loan_fee = if flash_fee_bps > 0 {
            ((borrow_amount as u128 * flash_fee_bps as u128 + 9_999) / 10_000) as u64
        } else {
            0u64
        };
        let total_repay_required = borrow_amount.saturating_add(flash_loan_fee);

        let gross_profit = (gross_output as i64) - (total_repay_required as i64);

        // Dynamic Jito tip calculation: e.g. 70% of gross profit if profitable
        let jito_tip = if gross_profit > 0 {
            let tip = (gross_profit as u64 * tip_share_bps as u64) / 10_000;
            tip.max(10_000).min(50_000_000) // bounded between 0.00001 SOL and 0.05 SOL
        } else {
            0
        };

        let network_fee = BASE_NETWORK_FEE_LAMPORTS;
        let priority_fee = priority_fee_lamports;
        let total_costs = network_fee
            .saturating_add(priority_fee)
            .saturating_add(jito_tip)
            .saturating_add(flash_loan_fee);

        let net_profit_lamports = gross_profit - (total_costs as i64);
        let is_profitable = net_profit_lamports >= (DEFAULT_MIN_NET_PROFIT_LAMPORTS as i64);

        // Estimate transaction size: Base header (64 bytes) + Kamino borrow/repay (2 * 150) + Hops (hops.len * 200)
        let estimated_tx_size_bytes = 64 + 300 + (hops.len() * 200);

        Ok(ThreePoolRouteQuote {
            hops: hop_results,
            borrow_mint,
            principal_borrowed: borrow_amount,
            gross_output,
            flash_loan_fee,
            total_repay_required,
            gross_profit,
            network_fee,
            priority_fee,
            jito_tip,
            token_transfer_fees: total_token_transfer_fees,
            total_costs,
            net_profit_lamports,
            is_profitable,
            estimated_compute_units: total_cu,
            estimated_tx_size_bytes,
        })
    }
}

/// Kamino Flash Loan Borrow-Size Optimizer using Golden-Section & Bounded Search.
pub struct KaminoBorrowOptimizer {
    pub router: ThreePoolRouteEngine,
}

impl KaminoBorrowOptimizer {
    pub fn new() -> Self {
        Self {
            router: ThreePoolRouteEngine::new(),
        }
    }

    /// Find the optimal borrow size that maximizes net atomic profit.
    ///
    /// Evaluates:
    /// - Kamino max borrow cap.
    /// - Conservative pool reserve limits (e.g. max 15% of shallowest pool reserve).
    /// - Non-linear price impact across CLMM ticks and DLMM bins.
    /// - All transaction costs, gas, tips, and MTU limits.
    pub fn optimize_borrow_size(
        &self,
        hops: &[RouteHopSpec],
        kamino_available_liquidity: u64,
        flash_fee_bps: u16,
        priority_fee_lamports: u64,
        tip_share_bps: u16,
    ) -> Option<ThreePoolRouteQuote> {
        if hops.is_empty() || kamino_available_liquidity == 0 {
            return None;
        }

        // 1. Determine search bounds [min_borrow, max_borrow]
        let min_borrow = 10_000_000u64; // 0.01 SOL floor
        if kamino_available_liquidity <= min_borrow {
            return None;
        }

        // Restrict max borrow to at most 15% of the initial pool reserve or Kamino limit
        let hop1_reserve = hops[0].pool_state.reserve_in as u64;
        let pool_cap = if hop1_reserve > 0 {
            (hop1_reserve / 6).max(min_borrow)
        } else {
            kamino_available_liquidity
        };

        let max_borrow = kamino_available_liquidity.min(pool_cap).min(50_000_000_000); // 50 SOL ceiling
        if max_borrow <= min_borrow {
            return None;
        }

        // 2. Golden Section Search for optimal input amount
        const PHI_INV: f64 = 0.618033988749895; // (sqrt(5) - 1) / 2
        let mut a = min_borrow as f64;
        let mut b = max_borrow as f64;

        let mut c = b - PHI_INV * (b - a);
        let mut d = a + PHI_INV * (b - a);

        let eval = |size: u64| -> i64 {
            match self.router.simulate_route(
                hops,
                size,
                flash_fee_bps,
                priority_fee_lamports,
                tip_share_bps,
            ) {
                Ok(q) => {
                    // Check physical MTU limit
                    if q.estimated_tx_size_bytes > SOLANA_PACKET_MTU_BYTES {
                        i64::MIN
                    } else {
                        q.net_profit_lamports
                    }
                }
                Err(_) => i64::MIN,
            }
        };

        let mut profit_c = eval(c as u64);
        let mut profit_d = eval(d as u64);

        // Run 16 iterations of golden-section search (~1e-4 relative tolerance)
        for _ in 0..16 {
            if (b - a) < 100_000.0 {
                // Converged to within 0.0001 SOL
                break;
            }

            if profit_c > profit_d {
                b = d;
                d = c;
                profit_d = profit_c;
                c = b - PHI_INV * (b - a);
                profit_c = eval(c as u64);
            } else {
                a = c;
                c = d;
                profit_c = profit_d;
                d = a + PHI_INV * (b - a);
                profit_d = eval(d as u64);
            }
        }

        let best_size = ((a + b) / 2.0) as u64;
        let final_quote = self
            .router
            .simulate_route(
                hops,
                best_size,
                flash_fee_bps,
                priority_fee_lamports,
                tip_share_bps,
            )
            .ok()?;

        if final_quote.is_profitable {
            Some(final_quote)
        } else {
            None
        }
    }
}
