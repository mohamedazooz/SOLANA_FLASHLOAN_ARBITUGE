//! Phase 15.3 — Strict dimensional accounting types.
//!
//! Provides strongly-typed monetary values that prevent cross-mint arithmetic:
//! * `TokenAmount` — raw units + mint + decimals (immutable, carry provenance)
//! * `ProfitAmount` — signed raw profit in a specific mint
//! * `TokenPrice` — base↔quote price with slot provenance
//!
//! All arithmetic is checked. Mixing mints is a compile-time or runtime error.
//! No implicit decimal conversions. No unverified default prices.

use crate::MintId;
use ruint::aliases::U256;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A quantity of tokens in raw (atomic) units, tagged with its mint and decimals.
///
/// This type carries full denomination metadata so that arithmetic operations
/// can enforce same-mint rules at runtime (and ideally at compile time via
/// wrapper types in the future).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenAmount {
    pub raw: u128,
    pub mint: MintId,
    pub decimals: u8,
}

impl TokenAmount {
    pub const ZERO: Self = Self {
        raw: 0,
        mint: MintId::new([0u8; 32]),
        decimals: 0,
    };

    pub fn new(raw: u64, mint: MintId, decimals: u8) -> Self {
        Self {
            raw: raw as u128,
            mint,
            decimals,
        }
    }

    pub fn from_u128(raw: u128, mint: MintId, decimals: u8) -> Self {
        Self {
            raw,
            mint,
            decimals,
        }
    }

    pub fn as_u64(self) -> Option<u64> {
        u64::try_from(self.raw).ok()
    }

    pub fn as_u128(self) -> u128 {
        self.raw
    }

    /// Convert raw units to a human-readable decimal value (e.g., USDC 1_000_000 → 1.0).
    /// Returns None if decimals > 38 (overflow in f64 scaling).
    pub fn to_decimal(self) -> Option<f64> {
        if self.decimals > 38 {
            return None;
        }
        let scale = 10f64.powi(self.decimals as i32);
        Some(self.raw as f64 / scale)
    }

    /// Check that this amount's mint matches `other`'s mint.
    pub fn same_mint_as(self, other: TokenAmount) -> bool {
        self.mint == other.mint
    }

    /// Check that this amount's decimals match `other`'s decimals.
    pub fn same_decimals_as(self, other: TokenAmount) -> bool {
        self.decimals == other.decimals
    }
}

impl fmt::Display for TokenAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let decimal = self.to_decimal().unwrap_or(0.0);
        write!(
            f,
            "{} {} (decimals={}, raw={})",
            decimal, self.mint, self.decimals, self.raw
        )
    }
}

/// A signed profit/loss quantity in a specific mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfitAmount {
    pub raw: i128,
    pub mint: MintId,
    pub decimals: u8,
}

impl ProfitAmount {
    pub const ZERO: Self = Self {
        raw: 0,
        mint: MintId::new([0u8; 32]),
        decimals: 0,
    };

    pub fn new(raw: i64, mint: MintId, decimals: u8) -> Self {
        Self {
            raw: raw as i128,
            mint,
            decimals,
        }
    }

    pub fn from_i128(raw: i128, mint: MintId, decimals: u8) -> Self {
        Self {
            raw,
            mint,
            decimals,
        }
    }

    pub fn is_positive(self) -> bool {
        self.raw > 0
    }

    pub fn is_negative(self) -> bool {
        self.raw < 0
    }

    pub fn is_zero(self) -> bool {
        self.raw == 0
    }

    /// Convert to decimal representation (e.g., +1.50 USDC).
    pub fn to_decimal(self) -> Option<f64> {
        if self.decimals > 38 {
            return None;
        }
        let scale = 10f64.powi(self.decimals as i32);
        Some(self.raw as f64 / scale)
    }

    /// Same-mint addition. Returns error if mints differ.
    pub fn checked_add(self, other: ProfitAmount) -> Result<ProfitAmount, DimensionalError> {
        if self.mint != other.mint {
            return Err(DimensionalError::MintMismatch {
                left: self.mint,
                right: other.mint,
            });
        }
        if self.decimals != other.decimals {
            return Err(DimensionalError::DecimalsMismatch {
                left: self.decimals,
                right: other.decimals,
            });
        }
        self.raw
            .checked_add(other.raw)
            .map(|r| ProfitAmount {
                raw: r,
                mint: self.mint,
                decimals: self.decimals,
            })
            .ok_or(DimensionalError::Overflow)
    }

    /// Same-mint subtraction. Returns error if mints differ.
    pub fn checked_sub(self, other: ProfitAmount) -> Result<ProfitAmount, DimensionalError> {
        if self.mint != other.mint {
            return Err(DimensionalError::MintMismatch {
                left: self.mint,
                right: other.mint,
            });
        }
        if self.decimals != other.decimals {
            return Err(DimensionalError::DecimalsMismatch {
                left: self.decimals,
                right: other.decimals,
            });
        }
        self.raw
            .checked_sub(other.raw)
            .map(|r| ProfitAmount {
                raw: r,
                mint: self.mint,
                decimals: self.decimals,
            })
            .ok_or(DimensionalError::Overflow)
    }
}

/// A price quote between two mints, with slot provenance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TokenPrice {
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Price of 1 base_mint unit in quote_mint raw units (Q64.64 or raw depending on convention).
    /// Stored as a rational numerator/denominator to avoid floating point.
    pub rate_num: u128,
    pub rate_denom: u128,
    pub slot: u64,
    pub source: PriceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceSource {
    LiveOracle,
    DexReserveRatio,
    ConfiguredConstant,
    Stale,
}

/// Errors from dimensional accounting violations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionalError {
    /// Attempted arithmetic between amounts of different mints.
    MintMismatch { left: MintId, right: MintId },
    /// Attempted arithmetic between amounts with different decimals.
    DecimalsMismatch { left: u8, right: u8 },
    /// Overflow/underflow in checked arithmetic.
    Overflow,
    /// Underflow (result would be negative for unsigned).
    Underflow,
    /// No valid price conversion available for the required mint pair.
    MissingConversion { from: MintId, to: MintId },
    /// The only available price is stale (slot too old).
    StalePrice {
        mint: MintId,
        price_slot: u64,
        current_slot: u64,
        max_age_slots: u64,
    },
    /// Zero denominator in a rate.
    ZeroRateDenominator,
}

impl fmt::Display for DimensionalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DimensionalError::MintMismatch { left, right } => {
                write!(f, "mint mismatch: {left} != {right}")
            }
            DimensionalError::DecimalsMismatch { left, right } => {
                write!(f, "decimals mismatch: {left} != {right}")
            }
            DimensionalError::Overflow => write!(f, "arithmetic overflow"),
            DimensionalError::Underflow => write!(f, "arithmetic underflow"),
            DimensionalError::MissingConversion { from, to } => {
                write!(f, "missing price conversion: {from} → {to}")
            }
            DimensionalError::StalePrice {
                mint,
                price_slot,
                current_slot,
                max_age_slots,
            } => write!(
                f,
                "stale price for {mint}: price_slot={price_slot} current_slot={current_slot} max_age={max_age_slots}"
            ),
            DimensionalError::ZeroRateDenominator => write!(f, "zero rate denominator"),
        }
    }
}

impl std::error::Error for DimensionalError {}

/// Result type for dimensional accounting operations.
pub type DimensionalResult<T> = Result<T, DimensionalError>;

/// Convert a SOL-denominated lamport cost into base-mint raw units using an
/// explicit, verified rational rate `rate_num / rate_denom`.
///
/// Returns `DimensionalError::StalePrice` if `max_age_slots` is provided and
/// the price slot is too old.
pub fn lamports_to_base_raw_checked(
    lamports: u64,
    rate_num: u128,
    rate_denom: u128,
    price_slot: u64,
    current_slot: u64,
    max_age_slots: u64,
) -> DimensionalResult<u128> {
    if rate_denom == 0 {
        return Err(DimensionalError::ZeroRateDenominator);
    }
    if current_slot.saturating_sub(price_slot) > max_age_slots {
        return Err(DimensionalError::StalePrice {
            mint: MintId::new([0u8; 32]),
            price_slot,
            current_slot,
            max_age_slots,
        });
    }
    let numerator = U256::from(lamports)
        .checked_mul(U256::from(rate_num))
        .ok_or(DimensionalError::Overflow)?;
    let denominator = U256::from(rate_denom);
    let (quotient, remainder) = numerator.div_rem(denominator);
    let rounded = if remainder.is_zero() {
        quotient
    } else {
        quotient
            .checked_add(U256::from(1))
            .ok_or(DimensionalError::Overflow)?
    };
    u128::try_from(rounded).map_err(|_| DimensionalError::Overflow)
}

/// Convert base-mint raw units to SOL-denominated lamports using `rate_num / rate_denom`.
pub fn base_raw_to_lamports_checked(
    base_raw: u128,
    rate_num: u128,
    rate_denom: u128,
) -> DimensionalResult<u64> {
    if rate_num == 0 {
        return Err(DimensionalError::ZeroRateDenominator);
    }
    let numerator = U256::from(base_raw)
        .checked_mul(U256::from(rate_denom))
        .ok_or(DimensionalError::Overflow)?;
    let denominator = U256::from(rate_num);
    let (quotient, remainder) = numerator.div_rem(denominator);
    let rounded = if remainder.is_zero() {
        quotient
    } else {
        quotient
            .checked_add(U256::from(1))
            .ok_or(DimensionalError::Overflow)?
    };
    u64::try_from(rounded).map_err(|_| DimensionalError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usdc_mint() -> MintId {
        MintId(
            solana_pubkey::Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
                .unwrap(),
        )
    }

    fn wsol_mint() -> MintId {
        MintId(
            solana_pubkey::Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
        )
    }

    #[test]
    fn token_amount_same_mint_check() {
        let a = TokenAmount::new(1_000_000, usdc_mint(), 6);
        let b = TokenAmount::new(500_000, usdc_mint(), 6);
        assert!(a.same_mint_as(b));

        let c = TokenAmount::new(1_000_000_000, wsol_mint(), 9);
        assert!(!a.same_mint_as(c));
    }

    #[test]
    fn profit_amount_checked_add_same_mint() {
        let a = ProfitAmount::new(1_000_000, usdc_mint(), 6);
        let b = ProfitAmount::new(500_000, usdc_mint(), 6);
        let c = a.checked_add(b).unwrap();
        assert_eq!(c.raw, 1_500_000);
        assert_eq!(c.mint, usdc_mint());
        assert_eq!(c.decimals, 6);
    }

    #[test]
    fn profit_amount_checked_add_cross_mint_rejected() {
        let a = ProfitAmount::new(1_000_000, usdc_mint(), 6);
        let b = ProfitAmount::new(500_000_000, wsol_mint(), 9);
        let err = a.checked_add(b).unwrap_err();
        assert!(matches!(err, DimensionalError::MintMismatch { .. }));
    }

    #[test]
    fn token_amount_to_decimal_usdc() {
        let a = TokenAmount::new(1_500_000, usdc_mint(), 6);
        assert!((a.to_decimal().unwrap() - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn lamports_to_base_raw_checked_stale_price_rejected() {
        let err = lamports_to_base_raw_checked(
            5000,
            180,
            1_000_000_000,
            100,
            500,
            2, // max_age_slots=2, price is 400 slots old
        )
        .unwrap_err();
        assert!(matches!(err, DimensionalError::StalePrice { .. }));
    }

    #[test]
    fn lamports_to_base_raw_checked_fresh_price_ok() {
        let result = lamports_to_base_raw_checked(
            5000,
            180,
            1_000_000_000,
            498,
            500,
            2, // max_age_slots=2, price is 2 slots old
        );
        assert!(result.is_ok());
    }
}
