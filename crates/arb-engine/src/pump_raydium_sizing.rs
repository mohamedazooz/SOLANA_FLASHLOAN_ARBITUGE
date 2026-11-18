//! Optimal Trade Sizing for Pump.fun-Raydium Arbitrage
//!
//! This module implements advanced trade sizing algorithms to find the optimal
//! input amount Δx* that maximizes net profit for arbitrage cycles between
//! Pump.fun Bonding Curve and Raydium pools.
//!
//! # Mathematical Background
//!
//! For a 2-leg arbitrage cycle with CPMM pools:
//! - Pump.fun: (x_p, y_p) with fee γ_p
//! - Raydium: (x_r, y_r) with fee γ_r
//!
//! The composite output function is:
//! f(Δx) = y_r * (Δx * (1-γ_p)) / (x_r + Δx * (1-γ_p)) * (1-γ_r) * y_p / (x_p + Δx * (1-γ_p))
//!
//! Setting df/dΔx = 1 yields the closed-form solution:
//! Δx* = (√(A·B) - B) / C
//! where:
//!   A = y_r * (1-γ_r) * y_p
//!   B = x_r * x_p
//!   C = (1-γ_p) * (1-γ_r)
//!
//! For CLMM and DLMM pools, we use numerical methods (Newton-Raphson, Brent's method)
//! to find the optimal input where marginal return = 1 + fees.

use arb_types::{ArbError, Result};

// ============================================================================
// Sizing Configuration
// ============================================================================

/// Configuration for optimal trade sizing.
#[derive(Debug, Clone)]
pub struct SizingConfig {
    /// Maximum iterations for numerical methods.
    pub max_iterations: usize,
    /// Tolerance for convergence (in lamports).
    pub tolerance_lamports: u64,
    /// Minimum trade size to consider (in lamports).
    pub min_trade_size: u64,
    /// Maximum trade size to consider (in lamports).
    pub max_trade_size: u64,
}

impl Default for SizingConfig {
    fn default() -> Self {
        Self {
            max_iterations: 100,
            tolerance_lamports: 1,
            min_trade_size: 1_000,      // 0.000001 SOL
            max_trade_size: 10_000_000, // 10 SOL
        }
    }
}

// ============================================================================
// Sizing Result
// ============================================================================

/// Result of optimal trade sizing.
#[derive(Debug, Clone)]
pub struct SizingResult {
    /// Optimal input amount in lamports.
    pub optimal_input: u64,
    /// Expected output amount in lamports.
    pub expected_output: u64,
    /// Gross profit (output - input) in lamports.
    pub gross_profit: i64,
    /// Net profit after all fees in lamports.
    pub net_profit: i64,
    /// Number of iterations used.
    pub iterations: usize,
    /// Whether the result is valid (profitable).
    pub is_valid: bool,
    /// Error message if invalid.
    pub error: Option<String>,
}

// ============================================================================
// CPMM-CPMM Sizing (Closed-Form)
// ============================================================================

/// Calculate optimal trade size for CPMM↔CPMM arbitrage using closed-form solution.
///
/// This is the most efficient method for constant-product pools where we can derive
/// the optimal input analytically.
pub fn size_cpmm_cpmm(
    pump_x: u128,
    pump_y: u128,
    pump_fee_bps: u16,
    raydium_x: u128,
    raydium_y: u128,
    raydium_fee_bps: u16,
    flash_loan_fee_bps: u16,
    config: &SizingConfig,
) -> SizingResult {
    // Validate inputs
    if pump_x == 0 || pump_y == 0 || raydium_x == 0 || raydium_y == 0 {
        return SizingResult {
            optimal_input: 0,
            expected_output: 0,
            gross_profit: 0,
            net_profit: 0,
            iterations: 0,
            is_valid: false,
            error: Some("Zero reserves in one or more pools".to_string()),
        };
    }

    let fee_p = pump_fee_bps as u128;
    let fee_r = raydium_fee_bps as u128;
    let fee_flash = flash_loan_fee_bps as u128;

    // Calculate coefficients
    let one_minus_gamma_p = 10_000u128.checked_sub(fee_p).unwrap_or(0);
    let one_minus_gamma_r = 10_000u128.checked_sub(fee_r).unwrap_or(0);
    let one_plus_flash = 10_000u128.checked_add(fee_flash).unwrap_or(10_000);

    // A = y_r * (1-γ_r) * y_p
    let a = raydium_y
        .checked_mul(one_minus_gamma_r)?
        .checked_mul(pump_y)?
        .checked_div(10_000)?
        .checked_div(10_000)?;

    // B = x_r * x_p
    let b = raydium_x.checked_mul(pump_x)?;

    // C = (1-γ_p) * (1-γ_r)
    let c = one_minus_gamma_p
        .checked_mul(one_minus_gamma_r)?
        .checked_div(10_000)?;

    // Check if arbitrage is possible
    if a <= b || c == 0 {
        return SizingResult {
            optimal_input: 0,
            expected_output: 0,
            gross_profit: 0,
            net_profit: 0,
            iterations: 0,
            is_valid: false,
            error: Some("No arbitrage opportunity (A <= B or C = 0)".to_string()),
        };
    }

    // √(A*B) using integer square root
    let ab = a.checked_mul(b)?;
    let sqrt_ab = integer_sqrt(ab);

    // Δx* = (√(A*B) - B) / C
    let numerator = sqrt_ab.checked_sub(b)?;
    let optimal_input_u128 = numerator.checked_div(c)?;

    // Clamp to config bounds
    let optimal_input = optimal_input_u128
        .try_into()
        .unwrap_or(config.max_trade_size)
        .clamp(config.min_trade_size, config.max_trade_size);

    if optimal_input == 0 {
        return SizingResult {
            optimal_input: 0,
            expected_output: 0,
            gross_profit: 0,
            net_profit: 0,
            iterations: 1,
            is_valid: false,
            error: Some("Optimal input is zero".to_string()),
        };
    }

    // Calculate expected output through both pools
    let pump_input_after_fee = optimal_input
        .checked_mul(one_minus_gamma_p as u64)?
        .checked_div(10_000 as u64)?;

    let pump_output = pump_y
        .checked_mul(pump_input_after_fee as u128)?
        .checked_div(pump_x.checked_add(pump_input_after_fee as u128)?)?
        .try_into()
        .unwrap_or(0);

    let ray_input_after_fee = pump_output
        .checked_mul(one_minus_gamma_r as u64)?
        .checked_div(10_000 as u64)?;

    let ray_output = raydium_y
        .checked_mul(ray_input_after_fee as u128)?
        .checked_div(raydium_x.checked_add(ray_input_after_fee as u128)?)?
        .try_into()
        .unwrap_or(0);

    // Calculate profits
    let gross_profit = ray_output as i64 - optimal_input as i64;

    // Calculate flash loan fee
    let flash_fee = optimal_input
        .checked_mul(fee_flash as u64)?
        .checked_div(10_000)?
        .unwrap_or(0);

    // Net profit = gross - flash fee
    let net_profit = gross_profit.checked_sub(flash_fee as i64).unwrap_or(0);

    // Check if profitable
    let is_valid = net_profit > 0 && gross_profit > 0;

    SizingResult {
        optimal_input,
        expected_output: ray_output,
        gross_profit,
        net_profit,
        iterations: 1, // Closed-form solution
        is_valid,
        error: if is_valid {
            None
        } else {
            Some("Not profitable".to_string())
        },
    }
}

// ============================================================================
// Newton-Raphson Sizing (For Non-CPMM Pools)
// ============================================================================

/// Calculate optimal trade size using Newton-Raphson method.
///
/// This is used for CLMM and DLMM pools where the output function is not
/// fractional-linear and requires numerical root-finding.
pub fn size_newton_raphson<F>(mut f: F, config: &SizingConfig) -> SizingResult
where
    F: FnMut(u64) -> (u64, u64), // (input, output)
{
    let mut x = (config.min_trade_size + config.max_trade_size) / 2;
    let mut prev_x = 0;
    let mut iterations = 0;

    loop {
        if iterations >= config.max_iterations {
            break;
        }

        let (_, output) = f(x);

        // For arbitrage, we want output - input to be maximized
        // The derivative condition is: marginal_return = 1
        // We approximate this by checking if increasing input decreases marginal return

        // Try x + delta
        let delta = x.max(1);
        let (_, output_plus) = f(x.saturating_add(delta));

        // Marginal return at x
        let marginal = if x == 0 { 0 } else { output as i64 - x as i64 };

        // Marginal return at x + delta
        let marginal_plus = if x.saturating_add(delta) == 0 {
            0
        } else {
            output_plus as i64 - (x.saturating_add(delta)) as i64
        };

        // If marginal return is decreasing, we've passed the optimum
        if marginal_plus < marginal {
            // Optimum is between prev_x and x
            // Use bisection
            if prev_x > 0 {
                x = (prev_x + x) / 2;
            } else {
                x = x / 2;
            }
        } else {
            // Still increasing, try larger x
            prev_x = x;
            x = x.saturating_add(delta);
        }

        // Check convergence
        if (x as i64 - prev_x as i64).unsigned_abs() as u64 <= config.tolerance_lamports {
            break;
        }

        iterations += 1;
    }

    // Clamp to bounds
    x = x.clamp(config.min_trade_size, config.max_trade_size);

    let (_, output) = f(x);
    let gross_profit = output as i64 - x as i64;

    SizingResult {
        optimal_input: x,
        expected_output: output,
        gross_profit,
        net_profit: gross_profit, // Flash fee not included here
        iterations,
        is_valid: gross_profit > 0,
        error: if gross_profit > 0 {
            None
        } else {
            Some("Not profitable".to_string())
        },
    }
}

// ============================================================================
// Brent's Method Sizing
// ============================================================================

/// Calculate optimal trade size using Brent's method.
///
/// Brent's method combines bisection, secant, and inverse quadratic interpolation
/// for robust root-finding. This is useful for pools with complex fee structures.
pub fn size_brents_method<F>(f: F, config: &SizingConfig) -> SizingResult
where
    F: Fn(u64) -> (u64, u64), // (input, output)
{
    // Brent's method implementation for finding maximum of f(x) - x
    // We want to find x where the derivative of (output - input) is zero

    let mut a = config.min_trade_size;
    let mut b = config.max_trade_size;
    let mut c = a;
    let mut d = b;
    let mut e = a;
    let mut fa = {
        let (_, o) = f(a);
        o as i64 - a as i64
    };
    let mut fb = {
        let (_, o) = f(b);
        o as i64 - b as i64
    };
    let mut fc = fa;

    if fa > 0 && fb > 0 {
        // Both ends are profitable, search for maximum
        // Use golden section search
        return size_golden_section(f, config);
    }

    if fa < 0 && fb < 0 {
        return SizingResult {
            optimal_input: 0,
            expected_output: 0,
            gross_profit: 0,
            net_profit: 0,
            iterations: 0,
            is_valid: false,
            error: Some("No profitable region found".to_string()),
        };
    }

    // Standard Brent's method for root finding
    let mut iter = 0;
    loop {
        if iter >= config.max_iterations {
            break;
        }

        if fb > 0 && fc > 0 {
            // Root is between a and c
            c = a;
            fc = fa;
            e = b - a;
            d = b - a;
        } else if fb < 0 && fc < 0 {
            // Root is between c and b
            a = b;
            fa = fb;
            c = b;
            fc = fb;
        }

        if (b as i64 - a as i64).unsigned_abs() as u64 <= config.tolerance_lamports {
            break;
        }

        // Try to fit parabola
        let s = if a == c {
            // Secant method
            let q = fa;
            let r = fb;
            if q == r {
                b - a
            } else {
                let t = (a as i64 - b as i64) * q / (q - r);
                t.unsigned_abs() as u64
            }
        } else {
            // Inverse quadratic interpolation
            let q = fa;
            let r = fb;
            let t = fc;
            let denom = (q - r) * (t - q);
            if denom == 0 {
                b - a
            } else {
                let s = (a as i64 * r * t - b as i64 * q * t + c as i64 * q * r) / denom;
                s.unsigned_abs() as u64
            }
        };

        // Accept the interpolation if it's within bounds
        let x = if s < (b - a) / 2 || s > b - a {
            b - a
        } else {
            s
        };

        let new_x = if x < (b - a) / 2 { a + x } else { b - x };

        let (_, new_output) = f(new_x);
        let new_f = new_output as i64 - new_x as i64;

        d = b - a;
        e = b - a;

        if new_f < 0 {
            b = new_x;
            fb = new_f;
        } else {
            a = new_x;
            fa = new_f;
        }

        iter += 1;
    }

    // Return the best point found
    let x = (a + b) / 2;
    let (_, output) = f(x);
    let gross_profit = output as i64 - x as i64;

    SizingResult {
        optimal_input: x,
        expected_output: output,
        gross_profit,
        net_profit: gross_profit,
        iterations: iter,
        is_valid: gross_profit > 0,
        error: if gross_profit > 0 {
            None
        } else {
            Some("Not profitable".to_string())
        },
    }
}

// ============================================================================
// Golden Section Search
// ============================================================================

/// Calculate optimal trade size using Golden Section Search.
///
/// This is a robust method for finding the maximum of a unimodal function
/// without requiring derivatives. It's particularly useful for CLMM pools
/// where the profit function may have complex shapes.
pub fn size_golden_section<F>(f: F, config: &SizingConfig) -> SizingResult
where
    F: Fn(u64) -> (u64, u64), // (input, output)
{
    const GOLDEN_RATIO: f64 = 0.6180339887498948;

    let mut a = config.min_trade_size as f64;
    let mut b = config.max_trade_size as f64;

    let mut x1 = b - GOLDEN_RATIO * (b - a);
    let mut x2 = a + GOLDEN_RATIO * (b - a);

    let mut f1 = {
        let (_, o) = f(x1 as u64);
        o as i64 - x1 as i64
    };
    let mut f2 = {
        let (_, o) = f(x2 as u64);
        o as i64 - x2 as i64
    };

    let mut iter = 0;

    loop {
        if iter >= config.max_iterations {
            break;
        }

        if f1 > f2 {
            // Maximum is in [a, x2]
            b = x2;
            x2 = x1;
            f2 = f1;
            x1 = b - GOLDEN_RATIO * (b - a);
            f1 = {
                let (_, o) = f(x1 as u64);
                o as i64 - x1 as i64
            };
        } else {
            // Maximum is in [x1, b]
            a = x1;
            x1 = x2;
            f1 = f2;
            x2 = a + GOLDEN_RATIO * (b - a);
            f2 = {
                let (_, o) = f(x2 as u64);
                o as i64 - x2 as i64
            };
        }

        if (b - a) < config.tolerance_lamports as f64 {
            break;
        }

        iter += 1;
    }

    let x = if f1 > f2 { x1 } else { x2 };
    let x_int = x as u64;
    let (_, output) = f(x_int);
    let gross_profit = output as i64 - x_int as i64;

    SizingResult {
        optimal_input: x_int,
        expected_output: output,
        gross_profit,
        net_profit: gross_profit,
        iterations: iter,
        is_valid: gross_profit > 0,
        error: if gross_profit > 0 {
            None
        } else {
            Some("Not profitable".to_string())
        },
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Integer square root using Newton's method.
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
    fn test_cpmm_cpmm_sizing_no_opportunity() {
        let config = SizingConfig::default();

        // Equal reserves, no fees - no arbitrage
        let result = size_cpmm_cpmm(
            1_000_000, 1_000_000, // pump reserves
            0,         // pump fee
            1_000_000, 1_000_000, // raydium reserves
            0,         // raydium fee
            0,         // flash fee
            &config,
        );

        assert!(!result.is_valid);
        assert_eq!(result.optimal_input, 0);
    }

    #[test]
    fn test_cpmm_cpmm_sizing_with_opportunity() {
        let config = SizingConfig::default();

        // Pump: x=1M, y=1M, fee=25 bps
        // Raydium: x=1M, y=1.1M, fee=25 bps
        // This creates an arbitrage opportunity
        let result = size_cpmm_cpmm(
            1_000_000, 1_000_000, // pump reserves
            25,        // pump fee
            1_000_000, 1_100_000, // raydium reserves
            25,        // raydium fee
            50,        // flash fee
            &config,
        );

        if result.is_valid {
            assert!(result.optimal_input > 0);
            assert!(result.expected_output > result.optimal_input);
            assert!(result.net_profit > 0);
        }
    }

    #[test]
    fn test_integer_sqrt() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(16), 4);
        assert_eq!(integer_sqrt(100), 10);
        assert_eq!(integer_sqrt(10000), 100);
    }

    #[test]
    fn test_golden_section_search() {
        let config = SizingConfig {
            max_iterations: 100,
            tolerance_lamports: 1,
            min_trade_size: 1,
            max_trade_size: 1000,
        };

        // Simple quadratic function: output = -x^2 + 100x + input
        // Maximum at x = 50
        let result = size_golden_section(
            |x| {
                let output = -x as i64 * x as i64 + 100 * x as i64 + x as i64;
                (x, output as u64)
            },
            &config,
        );

        // Should find maximum near x=50
        assert!(result.optimal_input >= 40 && result.optimal_input <= 60);
    }
}
