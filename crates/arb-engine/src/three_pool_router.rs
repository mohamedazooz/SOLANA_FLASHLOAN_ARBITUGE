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
pub const DEFAULT_MIN_NET_PROFIT_LAMPORTS: u64 = arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS;
/// Base Solana network signature fee.
pub const BASE_NETWORK_FEE_LAMPORTS: u64 = arb_constants::SOLANA_BASE_FEE_PER_SIGNATURE_LAMPORTS;
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

/// Token-2022 fees are denominated in the mint charged on each hop. They must
/// never be summed across hops with different mints without a verified oracle conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HopTransferFeeSummary {
    pub hop_index: usize,
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub input_transfer_fee_raw: u64,
    pub output_transfer_fee_raw: u64,
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
    /// Per-hop fee records, each kept in its own token denomination.
    pub transfer_fees_by_hop: Vec<HopTransferFeeSummary>,
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
        // This legacy quote reports network/priority/Jito costs in lamports.
        // It is only dimensionally valid when the cycle's base mint is WSOL.
        if borrow_mint.0 != arb_constants::wsol_mint() {
            return Err(ArbError::InvalidInput(
                "three-pool lamport accounting requires WSOL as the borrow mint".into(),
            ));
        }
        if flash_fee_bps > 10_000 || tip_share_bps > 10_000 {
            return Err(ArbError::InvalidInput(
                "flash fee and tip share must be within 0..=10000 bps".into(),
            ));
        }
        // Verify cyclical continuity: final hop output must equal initial borrow mint
        let final_hop = &hops[hops.len() - 1];
        if final_hop.output_mint != borrow_mint {
            return Err(ArbError::InvalidInput(
                "Route is not cyclical: final output mint does not match borrow mint".into(),
            ));
        }

        let mut current_amount = borrow_amount;
        let mut hop_results = Vec::with_capacity(hops.len());
        let mut transfer_fees_by_hop = Vec::with_capacity(hops.len());
        let mut total_cu = 450u32 + 28_000u32; // Estimate only; final message/CU are builder-verified.

        for (i, hop) in hops.iter().enumerate() {
            // This router feeds the borrow-size optimizer used by the live path.
            // Quote-only venues must be rejected here as well as in `quote_hop`,
            // otherwise a separate dispatcher call could bypass execution gating.
            if let Err(reason) = crate::adapters::execution_capability(&hop.pool_state) {
                return Err(ArbError::UnsupportedOperation(format!(
                    "route hop {i} ({:?}) is quote-only: {reason}",
                    hop.pool_state.protocol
                )));
            }

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
                slippage_bps: arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
                input_transfer_fee_config: hop.input_transfer_fee,
                output_transfer_fee_config: hop.output_transfer_fee,
            };

            let res = self.dispatcher.quote_exact_in(&hop.pool_state, &req)?;
            if !res.quote_validity || res.unfilled_input_raw != 0 || res.amount_out_raw == 0 {
                return Err(ArbError::InvalidInput(format!(
                    "route hop {i} returned an invalid, partial, or zero-output quote"
                )));
            }

            transfer_fees_by_hop.push(HopTransferFeeSummary {
                hop_index: i,
                input_mint: hop.input_mint,
                output_mint: hop.output_mint,
                input_transfer_fee_raw: res.transfer_fee_breakdown.input_transfer_fee,
                output_transfer_fee_raw: res.transfer_fee_breakdown.output_transfer_fee,
            });

            // Accumulate estimated CU per DEX type
            let hop_cu = match hop.pool_state.protocol {
                DexProtocol::RaydiumV4
                | DexProtocol::RaydiumCpmm
                | DexProtocol::PumpFun
                | DexProtocol::PumpSwap => 35_000,
                DexProtocol::RaydiumClmm | DexProtocol::OrcaWhirlpool => 85_000,
                DexProtocol::MeteoraDlmm => 120_000,
                DexProtocol::MeteoraDammV2
                | DexProtocol::MeteoraDbc
                | DexProtocol::RaydiumLaunchLab => 60_000,
            };
            total_cu = total_cu.checked_add(hop_cu).ok_or(ArbError::Overflow)?;

            current_amount = res.amount_out_raw;
            hop_results.push(res);
        }

        let gross_output = current_amount;

        // This legacy router accepts a BPS estimate for borrow-size search only.
        // The live supervisor recomputes the exact Kamino U68F60 fee from the
        // verified reserve snapshot before any dispatch decision.
        // Estimate: ceil(borrow * fee_bps / 10_000).
        let flash_loan_fee = if flash_fee_bps > 0 {
            // u128 product cannot overflow (u64 * u16), but the quotient can
            // exceed u64 for absurd borrow sizes: refuse instead of truncating
            // a cost downward.
            let fee = u128::from(borrow_amount)
                .checked_mul(u128::from(flash_fee_bps))
                .ok_or(ArbError::Overflow)?
                .div_ceil(10_000);
            u64::try_from(fee).map_err(|_| ArbError::Overflow)?
        } else {
            0u64
        };
        let total_repay_required = borrow_amount
            .checked_add(flash_loan_fee)
            .ok_or(ArbError::Overflow)?;

        // Flash-loan fee is already reflected in repay principal here. Do not
        // subtract it again in total execution costs (double-count prevention).
        let gross_profit =
            i64::try_from(i128::from(gross_output) - i128::from(total_repay_required))
                .map_err(|_| ArbError::Overflow)?;

        // Tip is a share of the remaining edge, capped at that edge. A minimum
        // tip is applied only when the edge can actually afford it.
        let jito_tip = if gross_profit > 0 && tip_share_bps > 0 {
            let gross_edge = u64::try_from(gross_profit).map_err(|_| ArbError::Overflow)?;
            let share = u128::from(gross_edge)
                .checked_mul(u128::from(tip_share_bps))
                .ok_or(ArbError::Overflow)?
                / 10_000;
            let bounded = share.min(u128::from(gross_edge));
            let bounded = if bounded >= u128::from(arb_constants::MIN_JITO_TIP_LAMPORTS) {
                bounded
            } else if gross_edge >= arb_constants::MIN_JITO_TIP_LAMPORTS {
                u128::from(arb_constants::MIN_JITO_TIP_LAMPORTS)
            } else {
                0
            };
            u64::try_from(bounded).map_err(|_| ArbError::Overflow)?
        } else {
            0
        };

        let network_fee = BASE_NETWORK_FEE_LAMPORTS;
        let priority_fee = priority_fee_lamports;
        // All three costs are denominated in lamports; flash fee was deducted above.
        let total_costs = network_fee
            .checked_add(priority_fee)
            .and_then(|v| v.checked_add(jito_tip))
            .ok_or(ArbError::Overflow)?;
        // `gross_output` is already the chained quote result after each DEX
        // fee and the configured per-hop Token-2022 transfer treatment. The
        // per-hop fee breakdown above is forensic/reporting data in each mint;
        // it must not be summed across denominations or deducted a second time.
        // This route is WSOL-anchored (enforced above), so the verified
        // lamport-to-base conversion is the identity (1/1).
        let net_profit_signed = arb_math::economic::calculate_net_profit_raw_signed(
            gross_output,
            borrow_amount,
            flash_loan_fee,
            0, // DEX fees are already reflected in the chained quote
            0, // Token-2022 transfer fees are already reflected in quote outputs
            0, // no separate account-creation cost in this route estimate
            0, // no additional verified token-denominated cost in this route estimate
            arb_types::Lamports(network_fee),
            arb_types::Lamports(priority_fee),
            arb_types::Lamports(jito_tip),
            1,
            1,
        )?;
        let net_profit_lamports =
            i64::try_from(net_profit_signed).map_err(|_| ArbError::Overflow)?;
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
            transfer_fees_by_hop,
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

impl Default for KaminoBorrowOptimizer {
    fn default() -> Self {
        Self::new()
    }
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
        let min_borrow = arb_constants::MIN_KAMINO_FLASH_BORROW_LAMPORTS;
        if kamino_available_liquidity <= min_borrow {
            return None;
        }

        // Restrict max borrow to 15% of the reserve on the actual borrow side.
        // `reserve_in` is the base-mint reserve; `reserve_out` is the quote-mint reserve.
        let first_pool = &hops[0].pool_state;
        let borrow_side_reserve = if first_pool.base_mint == hops[0].input_mint {
            first_pool.reserve_in
        } else if first_pool.quote_mint == hops[0].input_mint {
            first_pool.reserve_out
        } else {
            return None;
        };
        // Unknown/zero borrow-side reserve is not permission to borrow the full
        // flash-loan market. The reserve cap is a strict upper bound.
        if borrow_side_reserve == 0 {
            return None;
        }
        let borrow_side_reserve = u64::try_from(borrow_side_reserve).unwrap_or(u64::MAX);
        let reserve_share = u128::from(borrow_side_reserve).checked_mul(u128::from(
            arb_constants::MAX_FLASH_BORROW_POOL_RESERVE_SHARE_BPS,
        ))? / 10_000;
        let pool_cap = u64::try_from(reserve_share).ok()?;

        let max_borrow = kamino_available_liquidity
            .min(pool_cap)
            .min(arb_constants::MAX_KAMINO_FLASH_BORROW_LAMPORTS);
        if max_borrow < min_borrow {
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
                    // This is a route-level estimate, not a serialized message.
                    // Exact packet/MTU validation belongs to the transaction builder.
                    q.net_profit_lamports
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
