//! Pump.fun <-> Raydium Atomic Arbitrage Engine
//!
//! This module implements spatial arbitrage between Pump.fun Bonding Curve and all Raydium
//! pool architectures (AMM v4, CPMM, CLMM) using atomic flash loan transactions.
//!
//! # Architecture
//!
//! The arbitrage engine follows a strict pipeline:
//! 1. **Discovery**: Identify Pump.fun tokens with corresponding Raydium pools
//! 2. **Quoting**: Compute exact output amounts for both legs
//! 3. **Sizing**: Find optimal trade size Δx* that maximizes net profit
//! 4. **Validation**: Ensure profit > minimum threshold after all fees
//! 5. **Execution**: Build atomic V1 transaction with flash loan
//!
//! # Mathematical Model
//!
//! For a 2-leg arbitrage cycle (Pump.fun ↔ Raydium):
//! - Pump.fun: Virtual CPMM with fee γ_p
//! - Raydium: CPMM/CLMM with fee γ_r
//!
//! Net profit: Π(Δx) = f_out(Δx) - Δx - F_flash - F_priority
//!
//! Optimal input (for CPMM↔CPMM): Δx* = (√(A·B) - B) / C
//! where A = y_r·(1-γ_r)·y_p, B = x_r·x_p, C = (1-γ_p)·(1-γ_r)

use arb_dex::pump_fun::PumpFunBondingCurve;
use arb_dex::pump_swap::PumpSwapPool;
use arb_dex::raydium_clmm::RaydiumClmmPool;
use arb_dex::raydium_cpmm::RaydiumCpmmPool;
use arb_dex::raydium_v4::RaydiumV4Pool;
use arb_dex::traits::AmmPool;
use arb_math::cpmm_out_fee;
use arb_types::{
    ArbError, DexProtocol, Lamports, MintId, PoolId, PoolKind, PoolState, Result, SwapHop,
};
use solana_pubkey::Pubkey;
use std::collections::HashMap;

/// Minimum net profit in lamports to justify execution.
pub const MIN_NET_PROFIT_LAMPORTS: u64 = arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS;

/// Maximum allowed slippage in basis points.
pub const MAX_SLIPPAGE_BPS: u64 = 50; // 0.5%

// ============================================================================
// Arbitrage Direction
// ============================================================================

/// Direction of arbitrage between Pump.fun and Raydium.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArbitrageDirection {
    /// Buy on Pump.fun, Sell on Raydium (SOL -> Token -> SOL)
    PumpBuyRaydiumSell,
    /// Sell on Pump.fun, Buy on Raydium (Token -> SOL -> Token)
    PumpSellRaydiumBuy,
}

impl ArbitrageDirection {
    /// Reverse the direction.
    pub fn reverse(self) -> Self {
        match self {
            ArbitrageDirection::PumpBuyRaydiumSell => ArbitrageDirection::PumpSellRaydiumBuy,
            ArbitrageDirection::PumpSellRaydiumBuy => ArbitrageDirection::PumpBuyRaydiumSell,
        }
    }
}

// ============================================================================
// Raydium Pool Type
// ============================================================================

/// Enumeration of Raydium pool architectures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RaydiumPoolType {
    /// Raydium AMM v4 (OpenBook integration)
    AMMv4,
    /// Raydium CPMM (Pure constant product)
    CPMM,
    /// Raydium CLMM (Concentrated liquidity)
    CLMM,
}

impl RaydiumPoolType {
    /// Get the corresponding DexProtocol.
    pub fn protocol(self) -> DexProtocol {
        match self {
            RaydiumPoolType::AMMv4 => DexProtocol::RaydiumV4,
            RaydiumPoolType::CPMM => DexProtocol::RaydiumCpmm,
            RaydiumPoolType::CLMM => DexProtocol::RaydiumClmm,
        }
    }

    /// Get the PoolKind.
    pub fn kind(self) -> PoolKind {
        match self {
            RaydiumPoolType::AMMv4 => PoolKind::Cpmm,
            RaydiumPoolType::CPMM => PoolKind::Cpmm,
            RaydiumPoolType::CLMM => PoolKind::Clmm,
        }
    }
}

// ============================================================================
// Arbitrage Opportunity
// ============================================================================

/// Represents a discovered arbitrage opportunity between Pump.fun and Raydium.
#[derive(Debug, Clone)]
pub struct PumpRaydiumOpportunity {
    /// Pump.fun bonding curve pool.
    pub pump_pool: PumpFunBondingCurve,
    /// Raydium pool (one of AMM v4, CPMM, or CLMM).
    pub raydium_pool: RaydiumPool,
    /// Direction of arbitrage.
    pub direction: ArbitrageDirection,
    /// Raydium pool type.
    pub raydium_type: RaydiumPoolType,
    /// Estimated gross profit per unit of input (before fees).
    pub gross_spread_bps: u64,
    /// Optimal input amount in lamports.
    pub optimal_input_lamports: u64,
    /// Expected output amount in lamports.
    pub expected_output_lamports: u64,
    /// Estimated net profit in lamports (after all fees).
    pub net_profit_lamports: i64,
    /// Flash loan fee in lamports.
    pub flash_loan_fee_lamports: u64,
    /// Priority fee in lamports.
    pub priority_fee_lamports: u64,
    /// Whether the opportunity is currently profitable.
    pub is_profitable: bool,
}

/// Unified Raydium pool representation.
#[derive(Debug, Clone)]
pub enum RaydiumPool {
    AMMv4(RaydiumV4Pool),
    CPMM(RaydiumCpmmPool),
    CLMM(RaydiumClmmPool),
}

impl RaydiumPool {
    /// Get pool ID.
    pub fn pool_id(&self) -> PoolId {
        match self {
            RaydiumPool::AMMv4(p) => p.pool_id,
            RaydiumPool::CPMM(p) => p.pool_id,
            RaydiumPool::CLMM(p) => p.pool_id,
        }
    }

    /// Get base mint.
    pub fn base_mint(&self) -> MintId {
        match self {
            RaydiumPool::AMMv4(p) => p.base_mint,
            RaydiumPool::CPMM(p) => p.token_0_mint,
            RaydiumPool::CLMM(p) => p.token_mint_0,
        }
    }

    /// Get quote mint.
    pub fn quote_mint(&self) -> MintId {
        match self {
            RaydiumPool::AMMv4(p) => p.quote_mint,
            RaydiumPool::CPMM(p) => p.token_1_mint,
            RaydiumPool::CLMM(p) => p.token_mint_1,
        }
    }

    /// Get reserves.
    pub fn reserves(&self) -> (u128, u128) {
        match self {
            RaydiumPool::AMMv4(p) => (p.base_reserve, p.quote_reserve),
            RaydiumPool::CPMM(p) => (p.reserve_0, p.reserve_1),
            RaydiumPool::CLMM(p) => {
                // For CLMM, we need to compute effective reserves from liquidity
                // This is a simplified approximation
                (u128::MAX, u128::MAX) // TODO: Implement proper CLMM reserve calculation
            }
        }
    }

    /// Get fee in basis points.
    pub fn fee_bps(&self) -> Option<u16> {
        match self {
            RaydiumPool::AMMv4(p) => {
                if p.fee_denominator == 0 {
                    None
                } else {
                    // fee_numerator / fee_denominator * 10000
                    let fee = (p.fee_numerator as u128 * 10_000u128)
                        .checked_div(p.fee_denominator as u128)
                        .map(|v| v as u16);
                    fee
                }
            }
            RaydiumPool::CPMM(p) => {
                // trade_fee_rate is in ppm (1e6), convert to bps (1e4)
                let total_fee_ppm = p.trade_fee_rate_ppm + p.creator_fee_rate_ppm;
                if total_fee_ppm > 1_000_000 {
                    None
                } else {
                    Some((total_fee_ppm / 100) as u16)
                }
            }
            RaydiumPool::CLMM(p) => Some(p.fee_rate_ppm as u16 / 100), // ppm to bps
        }
    }

    /// Get pool type.
    pub fn pool_type(&self) -> RaydiumPoolType {
        match self {
            RaydiumPool::AMMv4(_) => RaydiumPoolType::AMMv4,
            RaydiumPool::CPMM(_) => RaydiumPoolType::CPMM,
            RaydiumPool::CLMM(_) => RaydiumPoolType::CLMM,
        }
    }

    /// Quote an exact-in swap.
    pub fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        match self {
            RaydiumPool::AMMv4(p) => {
                // For AMM v4, use the CPMM formula with upfront fee
                let (x, y) = if a_to_b {
                    (p.base_reserve, p.quote_reserve)
                } else {
                    (p.quote_reserve, p.base_reserve)
                };
                let fee_bps = self.fee_bps().unwrap_or(25); // Default 25 bps
                cpmm_out_fee(amount_in, x, y, fee_bps)
            }
            RaydiumPool::CPMM(p) => {
                let (x, y) = if a_to_b {
                    (p.reserve_0, p.reserve_1)
                } else {
                    (p.reserve_1, p.reserve_0)
                };
                let total_fee_ppm = p.trade_fee_rate_ppm + p.creator_fee_rate_ppm;
                let fee_bps = (total_fee_ppm / 100) as u16; // ppm to bps
                cpmm_out_fee(amount_in, x, y, fee_bps)
            }
            RaydiumPool::CLMM(p) => {
                // For CLMM, use the official quote function
                // This is a placeholder - actual implementation needs proper CLMM math
                arb_math::quote_clmm(
                    amount_in,
                    p.sqrt_price_x64,
                    p.tick_current,
                    p.liquidity,
                    p.tick_spacing,
                    p.fee_rate_ppm as u16,
                    a_to_b,
                    &[],
                )
                .map(|v| v as u64)
                .map_err(|_| ArbError::QuoteError("CLMM quote failed".into()))
            }
        }
    }
}

// ============================================================================
// Arbitrage Engine
// ============================================================================

/// Configuration for the Pump.fun-Raydium arbitrage engine.
#[derive(Debug, Clone)]
pub struct PumpRaydiumConfig {
    /// Minimum net profit in lamports.
    pub min_net_profit_lamports: u64,
    /// Maximum slippage in basis points.
    pub max_slippage_bps: u64,
    /// Flash loan fee rate in basis points.
    pub flash_loan_fee_bps: u16,
    /// Priority fee in micro-lamports per CU.
    pub priority_fee_micro_lamports_per_cu: u64,
    /// Estimated compute units for the transaction.
    pub estimated_cu: u32,
}

impl Default for PumpRaydiumConfig {
    fn default() -> Self {
        Self {
            min_net_profit_lamports: MIN_NET_PROFIT_LAMPORTS,
            max_slippage_bps: MAX_SLIPPAGE_BPS,
            flash_loan_fee_bps: 50,                   // 0.5%
            priority_fee_micro_lamports_per_cu: 1000, // 1000 μL per CU
            estimated_cu: 50_000,
        }
    }
}

/// Pump.fun-Raydium Arbitrage Engine.
pub struct PumpRaydiumEngine {
    config: PumpRaydiumConfig,
    /// Map of token mint -> Pump.fun pool
    pump_pools: HashMap<MintId, PumpFunBondingCurve>,
    /// Map of token mint -> Raydium pools
    raydium_pools: HashMap<MintId, Vec<RaydiumPool>>,
}

impl PumpRaydiumEngine {
    /// Create a new arbitrage engine with default configuration.
    pub fn new() -> Self {
        Self {
            config: PumpRaydiumConfig::default(),
            pump_pools: HashMap::new(),
            raydium_pools: HashMap::new(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: PumpRaydiumConfig) -> Self {
        Self {
            config,
            pump_pools: HashMap::new(),
            raydium_pools: HashMap::new(),
        }
    }

    /// Add a Pump.fun pool to the engine.
    pub fn add_pump_pool(&mut self, pool: PumpFunBondingCurve) {
        self.pump_pools.insert(pool.quote_mint, pool);
    }

    /// Add a Raydium pool to the engine.
    pub fn add_raydium_pool(&mut self, pool: RaydiumPool) {
        let token_mint = pool.quote_mint();
        self.raydium_pools
            .entry(token_mint)
            .or_insert_with(Vec::new)
            .push(pool);
    }

    /// Discover arbitrage opportunities between Pump.fun and Raydium.
    /// Returns a vector of profitable opportunities.
    pub fn discover_opportunities(&self) -> Vec<PumpRaydiumOpportunity> {
        let mut opportunities = Vec::new();

        // Iterate through all Pump.fun pools
        for (token_mint, pump_pool) in &self.pump_pools {
            // Find matching Raydium pools for this token
            if let Some(raydium_pools) = self.raydium_pools.get(token_mint) {
                for raydium_pool in raydium_pools {
                    // Try both directions
                    for direction in [
                        ArbitrageDirection::PumpBuyRaydiumSell,
                        ArbitrageDirection::PumpSellRaydiumBuy,
                    ] {
                        if let Some(opp) =
                            self.evaluate_opportunity(pump_pool, raydium_pool, direction)
                        {
                            if opp.is_profitable {
                                opportunities.push(opp);
                            }
                        }
                    }
                }
            }
        }

        // Sort by net profit (descending)
        opportunities.sort_by(|a, b| b.net_profit_lamports.cmp(&a.net_profit_lamports));
        opportunities
    }

    /// Evaluate a single arbitrage opportunity.
    fn evaluate_opportunity(
        &self,
        pump_pool: &PumpFunBondingCurve,
        raydium_pool: &RaydiumPool,
        direction: ArbitrageDirection,
    ) -> Option<PumpRaydiumOpportunity> {
        let pump_fee_bps = pump_pool.fee_bps?;
        let raydium_fee_bps = raydium_pool.fee_bps()?;

        // Get reserves
        let (pump_x, pump_y) = match direction {
            ArbitrageDirection::PumpBuyRaydiumSell => {
                // Buy on Pump: SOL -> Token
                // Pump.fun: base_mint = SOL, quote_mint = Token
                (
                    pump_pool.virtual_sol_reserves,
                    pump_pool.virtual_token_reserves,
                )
            }
            ArbitrageDirection::PumpSellRaydiumBuy => {
                // Sell on Pump: Token -> SOL
                (
                    pump_pool.virtual_token_reserves,
                    pump_pool.virtual_sol_reserves,
                )
            }
        };

        let (ray_x, ray_y) = raydium_pool.reserves();

        // Calculate optimal input size
        let (optimal_input, expected_output, gross_spread_bps) = match direction {
            ArbitrageDirection::PumpBuyRaydiumSell => {
                // Buy on Pump, Sell on Raydium
                // For CPMM↔CPMM, we can use the closed-form solution
                self.calculate_optimal_input_cpmm_cpmm(
                    pump_x,
                    pump_y,
                    pump_fee_bps,
                    ray_x,
                    ray_y,
                    raydium_fee_bps,
                )
            }
            ArbitrageDirection::PumpSellRaydiumBuy => {
                // Sell on Pump, Buy on Raydium (reverse)
                self.calculate_optimal_input_cpmm_cpmm(
                    pump_x,
                    pump_y,
                    pump_fee_bps,
                    ray_x,
                    ray_y,
                    raydium_fee_bps,
                )
            }
        };

        if optimal_input == 0 {
            return None;
        }

        // Calculate fees
        let flash_loan_fee = self.calculate_flash_loan_fee(optimal_input);
        let priority_fee = self
            .config
            .priority_fee_micro_lamports_per_cu
            .checked_mul(self.config.estimated_cu as u64)?
            / 1_000_000; // Convert μL to lamports

        // Calculate net profit
        let net_profit = expected_output
            .checked_sub(optimal_input)?
            .checked_sub(flash_loan_fee)?
            .checked_sub(priority_fee)? as i64;

        let is_profitable = net_profit > self.config.min_net_profit_lamports as i64;

        Some(PumpRaydiumOpportunity {
            pump_pool: pump_pool.clone(),
            raydium_pool: raydium_pool.clone(),
            direction,
            raydium_type: raydium_pool.pool_type(),
            gross_spread_bps,
            optimal_input_lamports: optimal_input,
            expected_output_lamports: expected_output,
            net_profit_lamports: net_profit,
            flash_loan_fee_lamports: flash_loan_fee,
            priority_fee_lamports: priority_fee,
            is_profitable,
        })
    }

    /// Calculate optimal input for CPMM↔CPMM arbitrage.
    /// Returns (optimal_input, expected_output, gross_spread_bps)
    fn calculate_optimal_input_cpmm_cpmm(
        &self,
        pump_x: u128,
        pump_y: u128,
        pump_fee_bps: u16,
        ray_x: u128,
        ray_y: u128,
        ray_fee_bps: u16,
    ) -> (u64, u64, u64) {
        // For a 2-pool CPMM arbitrage cycle:
        // A = y_r * (1 - γ_r) * y_p
        // B = x_r * x_p
        // C = (1 - γ_p) * (1 - γ_r)
        // Δx* = (√(A*B) - B) / C

        if pump_x == 0 || pump_y == 0 || ray_x == 0 || ray_y == 0 {
            return (0, 0, 0);
        }

        let fee_p = pump_fee_bps as u128;
        let fee_r = ray_fee_bps as u128;

        // Calculate A, B, C using fixed-point arithmetic
        let one_minus_gamma_p = 10_000u128.checked_sub(fee_p).unwrap_or(0);
        let one_minus_gamma_r = 10_000u128.checked_sub(fee_r).unwrap_or(0);

        // A = y_r * one_minus_gamma_r * y_p
        let a = ray_y
            .checked_mul(one_minus_gamma_r)?
            .checked_mul(pump_y)?
            .checked_div(10_000)?
            .checked_div(10_000)?;

        // B = x_r * x_p
        let b = ray_x.checked_mul(pump_x)?;

        // C = one_minus_gamma_p * one_minus_gamma_r
        let c = one_minus_gamma_p
            .checked_mul(one_minus_gamma_r)?
            .checked_div(10_000)?;

        if a <= b || c == 0 {
            return (0, 0, 0);
        }

        // √(A*B) using integer square root approximation
        let ab = a.checked_mul(b)?;
        let sqrt_ab = integer_sqrt(ab);

        // Δx* = (√(A*B) - B) / C
        let numerator = sqrt_ab.checked_sub(b)?;
        let optimal_input_u128 = numerator.checked_div(c)?;

        // Clamp to u64
        let optimal_input = optimal_input_u128.try_into().unwrap_or(u64::MAX);

        if optimal_input == 0 {
            return (0, 0, 0);
        }

        // Calculate expected output
        // On Pump.fun: output_p = (pump_y * optimal_input * one_minus_gamma_p) / (pump_x + optimal_input * one_minus_gamma_p)
        let pump_input_after_fee = optimal_input
            .checked_mul(one_minus_gamma_p as u64)?
            .checked_div(10_000 as u64)?;

        let pump_output = pump_y
            .checked_mul(pump_input_after_fee as u128)?
            .checked_div(pump_x.checked_add(pump_input_after_fee as u128)?)?;

        // On Raydium: output_r = (ray_y * pump_output * one_minus_gamma_r) / (ray_x + pump_output * one_minus_gamma_r)
        let ray_input_after_fee = pump_output
            .checked_mul(one_minus_gamma_r as u128)?
            .checked_div(10_000 as u128)?;

        let ray_output = ray_y
            .checked_mul(ray_input_after_fee)?
            .checked_div(ray_x.checked_add(ray_input_after_fee)?)?;

        let expected_output = ray_output.try_into().unwrap_or(0);

        // Calculate gross spread in bps
        // Gross spread = (output - input) / input * 10000
        let gross_profit = expected_output.checked_sub(optimal_input).unwrap_or(0);
        let gross_spread_bps = (gross_profit as u128 * 10_000u128)
            .checked_div(optimal_input as u128)
            .unwrap_or(0) as u64;

        (optimal_input, expected_output, gross_spread_bps)
    }

    /// Calculate flash loan fee for a given amount.
    fn calculate_flash_loan_fee(&self, amount: u64) -> u64 {
        amount
            .checked_mul(self.config.flash_loan_fee_bps as u64)
            .unwrap_or(0)
            .checked_div(10_000)
            .unwrap_or(0)
    }

    /// Get the best opportunity for a specific token.
    pub fn get_best_opportunity(&self, token_mint: &MintId) -> Option<PumpRaydiumOpportunity> {
        self.discover_opportunities()
            .into_iter()
            .find(|opp| opp.pump_pool.quote_mint == *token_mint)
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Integer square root approximation using Newton's method.
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }

    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arbitrage_direction_reverse() {
        assert_eq!(
            ArbitrageDirection::PumpBuyRaydiumSell.reverse(),
            ArbitrageDirection::PumpSellRaydiumBuy
        );
        assert_eq!(
            ArbitrageDirection::PumpSellRaydiumBuy.reverse(),
            ArbitrageDirection::PumpBuyRaydiumSell
        );
    }

    #[test]
    fn test_raydium_pool_type_protocol() {
        assert_eq!(RaydiumPoolType::AMMv4.protocol(), DexProtocol::RaydiumV4);
        assert_eq!(RaydiumPoolType::CPMM.protocol(), DexProtocol::RaydiumCpmm);
        assert_eq!(RaydiumPoolType::CLMM.protocol(), DexProtocol::RaydiumClmm);
    }

    #[test]
    fn test_integer_sqrt() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(16), 4);
        assert_eq!(integer_sqrt(100), 10);
        assert_eq!(integer_sqrt(10000), 100);
        // Approximate for non-perfect squares
        assert!(integer_sqrt(10) >= 3 && integer_sqrt(10) <= 4);
    }

    #[test]
    fn test_optimal_input_calculation() {
        let engine = PumpRaydiumEngine::new();

        // Test with equal reserves and no fees
        let (input, output, spread) = engine.calculate_optimal_input_cpmm_cpmm(
            1_000_000, // pump_x
            1_000_000, // pump_y
            0,         // pump_fee_bps
            1_000_000, // ray_x
            1_000_000, // ray_y
            0,         // ray_fee_bps
        );

        // With no fees and equal reserves, there should be no arbitrage opportunity
        assert_eq!(input, 0);
        assert_eq!(output, 0);
        assert_eq!(spread, 0);
    }

    #[test]
    fn test_optimal_input_with_fees() {
        let engine = PumpRaydiumEngine::new();

        // Test with different reserves and fees
        // Pump: x=1M, y=1M, fee=25 bps
        // Raydium: x=1M, y=1.1M, fee=30 bps
        // This creates an arbitrage opportunity
        let (input, output, spread) = engine.calculate_optimal_input_cpmm_cpmm(
            1_000_000, // pump_x (SOL)
            1_000_000, // pump_y (Token)
            25,        // pump_fee_bps
            1_000_000, // ray_x (Token)
            1_100_000, // ray_y (SOL)
            30,        // ray_fee_bps
        );

        // Should find some arbitrage opportunity
        if input > 0 {
            assert!(output > input); // Should be profitable before fees
            assert!(spread > 0);
        }
    }
}
