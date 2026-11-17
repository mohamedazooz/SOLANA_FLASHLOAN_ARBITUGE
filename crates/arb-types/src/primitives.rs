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
