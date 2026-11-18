//! Core value & identity primitives (Lamports, TokenAmount, Bps, Slot, Seconds, Ms, CU, RouteId, PoolId, MintId, UsdMicros).

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fmt;
use std::str::FromStr;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Lamports(pub u64);

impl Lamports {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_u128(self) -> u128 {
        self.0 as u128
    }
}

impl fmt::Display for Lamports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} lamports", self.0)
    }
}

/// Signed lamports for PnL (can be negative for losses).
/// 
/// This type is used exclusively for profit/loss calculations where
/// negative values represent losses. Unlike `Lamports` which is always
/// non-negative (u64), `SignedLamports` uses i64 to support both profits
/// and losses.
/// 
/// # Financial Integrity
/// - Expected PnL: Calculated before execution, can be positive or negative
/// - Realized PnL: Derived from actual on-chain settlement, can be positive or negative
/// - NEVER convert Expected to Realized without settlement evidence
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct SignedLamports(pub i64);

impl SignedLamports {
    pub const ZERO: Self = Self(0);

    /// Create a new SignedLamports from i64
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns true if this represents a profit (positive)
    pub fn is_profit(self) -> bool {
        self.0 > 0
    }

    /// Returns true if this represents a loss (negative)
    pub fn is_loss(self) -> bool {
        self.0 < 0
    }

    /// Returns true if this is break-even (zero)
    pub fn is_break_even(self) -> bool {
        self.0 == 0
    }

    /// Returns the absolute value as Lamports (non-negative)
    pub fn abs(self) -> Lamports {
        Lamports(self.0.unsigned_abs() as u64)
    }

    /// Returns the raw i64 value
    pub fn as_i64(self) -> i64 {
        self.0
    }

    /// Returns the value as u64 if positive, None if negative
    pub fn as_u64_if_positive(self) -> Option<u64> {
        if self.0 >= 0 {
            Some(self.0 as u64)
        } else {
            None
        }
    }

    /// Convert from Lamports (always non-negative)
    pub fn from_lamports(lamports: Lamports) -> Self {
        Self(lamports.0 as i64)
    }

    /// Checked addition
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    /// Checked subtraction
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    /// Saturating addition
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl fmt::Display for SignedLamports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 >= 0 {
            write!(f, "+{} lamports", self.0)
        } else {
            write!(f, "-{} lamports", self.0.abs())
        }
    }
}

impl From<i64> for SignedLamports {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<SignedLamports> for i64 {
    fn from(value: SignedLamports) -> Self {
        value.0
    }
}

impl From<Lamports> for SignedLamports {
    fn from(lamports: Lamports) -> Self {
        Self(lamports.0 as i64)
    }
}

impl std::default::Default for SignedLamports {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Token amount (in smallest atomic units, e.g. 1e6 for USDC).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct TokenAmount(pub u64);

impl TokenAmount {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn checked_mul(self, rhs: u64) -> Option<Self> {
        self.0.checked_mul(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_u128(self) -> u128 {
        self.0 as u128
    }
}

impl fmt::Display for TokenAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Basis points (1 bp = 0.01%, 10000 = 100%).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Bps(pub u16);

impl Bps {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(10_000);
}

/// Slot number on the Solana blockchain.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Slot(pub u64);

impl Slot {
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> u64 {
        self.0.saturating_sub(rhs.0)
    }
}

/// Duration in milliseconds. Distinct from seconds to avoid unit confusion
/// in timeouts, poll intervals, and freshness windows.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Milliseconds(pub u64);

impl Milliseconds {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Duration in seconds. Distinct from milliseconds to avoid unit confusion.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct Seconds(pub u64);

impl Seconds {
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Compute units consumed/requested by a transaction. Distinct from Lamports.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
pub struct ComputeUnits(pub u32);

impl ComputeUnits {
    pub const MAX: Self = Self(1_400_000); // current Solana compute budget cap
    pub const ZERO: Self = Self(0);

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }
}

/// Route identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RouteId(pub u32);

/// Pool address identifier wrapping Pubkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct PoolId(pub Pubkey);

impl PoolId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(Pubkey::new_from_array(bytes))
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_bytes()
    }
}

impl From<Pubkey> for PoolId {
    fn from(p: Pubkey) -> Self {
        Self(p)
    }
}

impl From<[u8; 32]> for PoolId {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Display for PoolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PoolId {
    type Err = solana_pubkey::ParsePubkeyError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Pubkey::from_str(s).map(Self)
    }
}

/// Token Mint address identifier wrapping Pubkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct MintId(pub Pubkey);

impl MintId {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(Pubkey::new_from_array(bytes))
    }

    pub fn to_bytes(self) -> [u8; 32] {
        self.0.to_bytes()
    }
}

impl From<Pubkey> for MintId {
    fn from(p: Pubkey) -> Self {
        Self(p)
    }
}

impl From<[u8; 32]> for MintId {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Display for MintId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MintId {
    type Err = solana_pubkey::ParsePubkeyError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Pubkey::from_str(s).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_lamports_creation() {
        let profit = SignedLamports::new(100_000);
        assert_eq!(profit.as_i64(), 100_000);
        assert!(profit.is_profit());
        assert!(!profit.is_loss());
        assert!(!profit.is_break_even());

        let loss = SignedLamports::new(-50_000);
        assert_eq!(loss.as_i64(), -50_000);
        assert!(loss.is_loss());
        assert!(!loss.is_profit());
        assert!(!loss.is_break_even());

        let zero = SignedLamports::new(0);
        assert_eq!(zero.as_i64(), 0);
        assert!(zero.is_break_even());
        assert!(!zero.is_profit());
        assert!(!zero.is_loss());
    }

    #[test]
    fn signed_lamports_abs() {
        let profit = SignedLamports::new(100_000);
        assert_eq!(profit.abs().as_u64(), 100_000);

        let loss = SignedLamports::new(-50_000);
        assert_eq!(loss.abs().as_u64(), 50_000);

        let zero = SignedLamports::new(0);
        assert_eq!(zero.abs().as_u64(), 0);
    }

    #[test]
    fn signed_lamports_as_u64_if_positive() {
        let profit = SignedLamports::new(100_000);
        assert_eq!(profit.as_u64_if_positive(), Some(100_000));

        let loss = SignedLamports::new(-50_000);
        assert_eq!(loss.as_u64_if_positive(), None);

        let zero = SignedLamports::new(0);
        assert_eq!(zero.as_u64_if_positive(), Some(0));
    }

    #[test]
    fn signed_lamports_from_lamports() {
        let lamports = Lamports(100_000);
        let signed = SignedLamports::from_lamports(lamports);
        assert_eq!(signed.as_i64(), 100_000);
    }

    #[test]
    fn signed_lamports_arithmetic() {
        let a = SignedLamports::new(100_000);
        let b = SignedLamports::new(50_000);
        
        // checked_add
        assert_eq!(a.checked_add(b), Some(SignedLamports::new(150_000)));
        
        // checked_sub
        assert_eq!(a.checked_sub(b), Some(SignedLamports::new(50_000)));
        
        // saturating_add
        assert_eq!(a.saturating_add(b), SignedLamports::new(150_000));
        
        // saturating_sub
        assert_eq!(a.saturating_sub(b), SignedLamports::new(50_000));
        
        // Test with negative
        let c = SignedLamports::new(-25_000);
        assert_eq!(a.checked_add(c), Some(SignedLamports::new(75_000)));
        assert_eq!(a.checked_sub(c), Some(SignedLamports::new(125_000)));
    }

    #[test]
    fn signed_lamports_overflow() {
        let max = SignedLamports::new(i64::MAX);
        assert_eq!(max.checked_add(SignedLamports::new(1)), None);
        
        let min = SignedLamports::new(i64::MIN);
        assert_eq!(min.checked_sub(SignedLamports::new(1)), None);
        
        // Saturating operations
        assert_eq!(max.saturating_add(SignedLamports::new(1)), max);
        assert_eq!(min.saturating_sub(SignedLamports::new(1)), min);
    }

    #[test]
    fn signed_lamports_display() {
        let profit = SignedLamports::new(100_000);
        assert_eq!(format!("{}", profit), "+100000 lamports");
        
        let loss = SignedLamports::new(-50_000);
        assert_eq!(format!("{}", loss), "-50000 lamports");
    }

    #[test]
    fn signed_lamports_from_traits() {
        let from_i64: SignedLamports = 100_000.into();
        assert_eq!(from_i64.as_i64(), 100_000);
        
        let from_lamports: SignedLamports = Lamports(50_000).into();
        assert_eq!(from_lamports.as_i64(), 50_000);
        
        let to_i64: i64 = SignedLamports::new(75_000).into();
        assert_eq!(to_i64, 75_000);
    }
}
