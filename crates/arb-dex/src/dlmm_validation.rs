//! Phase 17 — Meteora DLMM on-chain account validation.
//!
//! Validates the *authoritative on-chain state* of a DLMM pool before it may be
//! priced or traded. Every check is fail-closed: an unverified account is never
//! quoted, never sized and never built into a transaction.
//!
//! Covered:
//! - `LbPair` account ownership (Meteora DLMM program), length, `status`,
//!   `function_type`, `collect_fee_mode`, distinct mints, bin-step and bin-id ranges.
//! - PDA derivation of the oracle, the two reserve vaults, the `BinArray` accounts
//!   and the `BinArrayBitmapExtension` (official MeteoraAg/dlmm-sdk
//!   `commons/src/pda.rs` seeds).
//! - `BinArray` account ownership and the account's **own stored `array_index`**,
//!   which is compared against the requested index. This is a data-driven check and
//!   therefore needs no duplicated PDA arithmetic.
//! - `BinArrayBitmapExtension` ownership.
//! - Token-program identity per mint, and mint-extension (transfer fee) awareness.
//!
//! The canonical PDA derivations live here. `crates/arb-tx` keeps its own
//! `derive_meteora_*_pda` helpers for instruction building; the two are proven
//! identical by `crates/arb-engine/tests/phase17_pda_differential.rs`.

use crate::meteora_dlmm::MeteoraDlmmPool;
use arb_constants::{METEORA_BINS_PER_BIN_ARRAY, METEORA_DLMM_PROGRAM_ID};
use arb_types::{DlmmBin, DlmmFeeParams, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::fmt;

/// Meteora DLMM program (`lb_clmm` 0.12.0).
pub const METEORA_DLMM_PROGRAM: Pubkey = Pubkey::from_str_const(METEORA_DLMM_PROGRAM_ID);

/// Offset of the `BinArray`'s own `array_index` field (after the 8-byte Anchor
/// discriminator). The account stores the index it represents, so a fetched account
/// can be matched to the requested index without recomputing a PDA.
pub const BIN_ARRAY_INDEX_OFFSET: usize = 8;
/// Length of the `BinArray` index field.
pub const BIN_ARRAY_INDEX_LEN: usize = 8;

// ============================================================================
// Canonical PDA derivations (official MeteoraAg/dlmm-sdk seeds)
// ============================================================================

/// Oracle PDA — seeds `[b"oracle", lb_pair]`.
#[inline]
pub fn derive_oracle_pda(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"oracle", lb_pair.as_ref()], &METEORA_DLMM_PROGRAM).0
}

/// Reserve (token vault) PDA — seeds `[lb_pair, token_mint]`.
#[inline]
pub fn derive_reserve_pda(lb_pair: &Pubkey, token_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[lb_pair.as_ref(), token_mint.as_ref()],
        &METEORA_DLMM_PROGRAM,
    )
    .0
}

/// `BinArray` PDA — seeds `[b"bin_array", lb_pair, index_le_i64]`.
#[inline]
pub fn derive_bin_array_pda(lb_pair: &Pubkey, bin_array_index: i64) -> Pubkey {
    let idx = bin_array_index.to_le_bytes();
    Pubkey::find_program_address(
        &[b"bin_array", lb_pair.as_ref(), &idx],
        &METEORA_DLMM_PROGRAM,
    )
    .0
}

/// `BinArrayBitmapExtension` PDA — seeds `[b"bitmap", lb_pair]`.
#[inline]
pub fn derive_bitmap_extension_pda(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"bitmap", lb_pair.as_ref()], &METEORA_DLMM_PROGRAM).0
}

/// Anchor `__event_authority` PDA — seeds `[b"__event_authority"]`.
#[inline]
pub fn derive_event_authority() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &METEORA_DLMM_PROGRAM).0
}

/// Bin-array index that contains `bin_id`.
///
/// Floor division toward negative infinity, so negative bin IDs map to the array
/// below zero (the on-chain convention).
#[inline]
pub fn bin_array_index_for_bin(bin_id: i32) -> i64 {
    (i64::from(bin_id)).div_euclid(METEORA_BINS_PER_BIN_ARRAY)
}

// ============================================================================
// Validation error type
// ============================================================================

/// Every way a DLMM account can fail validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DlmmValidationError {
    /// Account data shorter than the minimum layout.
    TooShort { expected: usize, actual: usize },
    /// Account is not owned by the Meteora DLMM program.
    WrongOwner {
        account: Pubkey,
        expected: Pubkey,
        actual: Pubkey,
    },
    /// The `LbPair` account does not exist on chain.
    AccountMissing { account: Pubkey },
    /// Pool is disabled / tombstoned (`status != 0`).
    PoolDisabled { pool: Pubkey, status: u8 },
    /// The two pool mints are identical.
    IdenticalMints { pool: Pubkey },
    /// A derived PDA does not match the address stored on chain.
    PdaMismatch {
        label: &'static str,
        expected: Pubkey,
        actual: Pubkey,
    },
    /// `bin_step` outside the protocol range.
    InvalidBinStep { pool: Pubkey, bin_step: u16 },
    /// Active bin outside the pool's declared bin-id range.
    ActiveBinOutOfRange {
        pool: Pubkey,
        active_id: i32,
        min_bin_id: i32,
        max_bin_id: i32,
    },
    /// Unsupported `ConcreteFunctionType`.
    UnsupportedFunctionType { pool: Pubkey, function_type: u8 },
    /// Unsupported `CollectFeeMode`.
    UnsupportedCollectFeeMode { pool: Pubkey, collect_fee_mode: u8 },
    /// The `BinArray` account stores an index different from the requested one.
    BinArrayIndexMismatch { expected: i64, actual: i64 },
    /// A `BinArray` account required by the traversal is missing on chain.
    BinArrayMissing { lb_pair: Pubkey, index: i64 },
    /// The `BinArrayBitmapExtension` account is missing.
    BitmapExtensionMissing { lb_pair: Pubkey },
    /// The two pools disagree on the token program of a shared mint.
    TokenProgramMismatch {
        mint: Pubkey,
        left: TokenProgramKind,
        right: TokenProgramKind,
    },
    /// The two pools do not share the same unordered mint pair.
    MintPairMismatch { left: Pubkey, right: Pubkey },
    /// The flash-loan mint is not present in the pool.
    FlashMintMissing { pool: Pubkey, mint: Pubkey },
    /// A required field could not be decoded.
    Decode { reason: String },
}

impl fmt::Display for DlmmValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { expected, actual } => {
                write!(
                    f,
                    "account too short: {actual} bytes, need at least {expected}"
                )
            }
            Self::WrongOwner {
                account,
                expected,
                actual,
            } => write!(
                f,
                "account {account} owned by {actual}, expected {expected}"
            ),
            Self::AccountMissing { account } => write!(f, "account {account} not found on chain"),
            Self::PoolDisabled { pool, status } => {
                write!(f, "pool {pool} is disabled (status {status})")
            }
            Self::IdenticalMints { pool } => write!(f, "pool {pool} has identical token mints"),
            Self::PdaMismatch {
                label,
                expected,
                actual,
            } => write!(
                f,
                "{label} PDA mismatch: derived {expected}, on-chain {actual}"
            ),
            Self::InvalidBinStep { pool, bin_step } => {
                write!(f, "pool {pool} has invalid bin_step {bin_step}")
            }
            Self::ActiveBinOutOfRange {
                pool,
                active_id,
                min_bin_id,
                max_bin_id,
            } => write!(
                f,
                "pool {pool} active bin {active_id} outside [{min_bin_id}, {max_bin_id}]"
            ),
            Self::UnsupportedFunctionType {
                pool,
                function_type,
            } => write!(
                f,
                "pool {pool} has unsupported ConcreteFunctionType {function_type}"
            ),
            Self::UnsupportedCollectFeeMode {
                pool,
                collect_fee_mode,
            } => write!(
                f,
                "pool {pool} has unsupported CollectFeeMode {collect_fee_mode}"
            ),
            Self::BinArrayIndexMismatch { expected, actual } => write!(
                f,
                "bin array account stores index {actual}, expected {expected}"
            ),
            Self::BinArrayMissing { lb_pair, index } => {
                write!(
                    f,
                    "bin array {index} for pool {lb_pair} is missing on chain"
                )
            }
            Self::BitmapExtensionMissing { lb_pair } => {
                write!(f, "bitmap extension for pool {lb_pair} is missing on chain")
            }
            Self::TokenProgramMismatch { mint, left, right } => write!(
                f,
                "token program mismatch for mint {mint}: {left:?} vs {right:?}"
            ),
            Self::MintPairMismatch { left, right } => {
                write!(
                    f,
                    "pools do not share the same mint pair: {left} vs {right}"
                )
            }
            Self::FlashMintMissing { pool, mint } => {
                write!(f, "pool {pool} does not contain flash-loan mint {mint}")
            }
            Self::Decode { reason } => write!(f, "DLMM decode error: {reason}"),
        }
    }
}

impl std::error::Error for DlmmValidationError {}

/// Result alias for DLMM validation.
pub type DlmmValidationResult<T> = Result<T, DlmmValidationError>;

// ============================================================================
// Validated pool state
// ============================================================================

/// A `LbPair` whose on-chain state has been fully verified.
///
/// `token_x_mint` / `token_y_mint` are the *on-chain* ordering. Nothing in Phase 17
/// assumes that WSOL is token X — the hop direction is derived from this field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDlmmPool {
    pub pool_id: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
    pub active_id: i32,
    pub bin_step: u16,
    pub status: u8,
    pub function_type: u8,
    pub collect_fee_mode: u8,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub fee_params: DlmmFeeParams,
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub last_update_timestamp: i64,
    /// Raw `bin_array_bitmap [u64; 16]` from the `LbPair` — tells which arrays exist.
    pub bin_array_bitmap: [u64; 16],
}

impl ValidatedDlmmPool {
    /// True when the on-chain token X is `mint`.
    #[inline]
    pub fn is_token_x(&self, mint: &Pubkey) -> bool {
        &self.token_x_mint == mint
    }

    /// True when the on-chain token Y is `mint`.
    #[inline]
    pub fn is_token_y(&self, mint: &Pubkey) -> bool {
        &self.token_y_mint == mint
    }

    /// The swap direction flag for `swap_for_y` when trading `from_mint -> to_mint`.
    ///
    /// Returns `None` when the pool does not hold `from_mint`.
    pub fn swap_for_y_for(&self, from_mint: &Pubkey, to_mint: &Pubkey) -> Option<bool> {
        if self.is_token_x(from_mint) && self.is_token_y(to_mint) {
            Some(true)
        } else if self.is_token_y(from_mint) && self.is_token_x(to_mint) {
            Some(false)
        } else {
            None
        }
    }

    /// True when the pool's on-chain bitmap marks `index` as an existing bin array.
    ///
    /// The bitmap is `u64[16]` where bit `i` of word `w` covers array index
    /// `w * 64 + i` (the DLMM `BitmapExtension` layout).
    pub fn bin_array_exists(&self, index: i64) -> bool {
        if index < 0 {
            return false;
        }
        let idx = index as usize;
        let word = idx / 64;
        if word >= self.bin_array_bitmap.len() {
            return false;
        }
        let bit = idx % 64;
        (self.bin_array_bitmap[word] >> bit) & 1 == 1
    }
}

// ============================================================================
// LbPair validation
// ============================================================================

/// Minimum `LbPair` account length accepted by the decoder.
pub const LB_PAIR_MIN_SIZE: usize = 300;

/// Validate a raw `LbPair` account payload.
///
/// `owner` is the on-chain account owner. The account's own stored oracle / reserve
/// addresses are compared against the canonical PDA derivations, so a pool whose
/// stored accounts were tampered with (or a program fork) is rejected.
pub fn validate_lb_pair_account(
    pool_id: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> DlmmValidationResult<ValidatedDlmmPool> {
    if *owner != METEORA_DLMM_PROGRAM {
        return Err(DlmmValidationError::WrongOwner {
            account: *pool_id,
            expected: METEORA_DLMM_PROGRAM,
            actual: *owner,
        });
    }
    if data.len() < LB_PAIR_MIN_SIZE {
        return Err(DlmmValidationError::TooShort {
            expected: LB_PAIR_MIN_SIZE,
            actual: data.len(),
        });
    }

    let decoded =
        MeteoraDlmmPool::from_account(arb_types::PoolId(*pool_id), data).map_err(|e| {
            DlmmValidationError::Decode {
                reason: format!("{e:?}"),
            }
        })?;

    if decoded.status != 0 {
        return Err(DlmmValidationError::PoolDisabled {
            pool: *pool_id,
            status: decoded.status,
        });
    }
    if decoded.token_x_mint.0 == decoded.token_y_mint.0 {
        return Err(DlmmValidationError::IdenticalMints { pool: *pool_id });
    }
    // ConcreteFunctionType: 0 = Spot, 1 = Concentrated, 2 = Percentile. Only Spot
    // (0) is supported by the discrete-bin quote engine.
    if decoded.function_type != 0 {
        return Err(DlmmValidationError::UnsupportedFunctionType {
            pool: *pool_id,
            function_type: decoded.function_type,
        });
    }
    // CollectFeeMode: 0 = InputOnly, 1 = OnlyY.
    if decoded.collect_fee_mode > 1 {
        return Err(DlmmValidationError::UnsupportedCollectFeeMode {
            pool: *pool_id,
            collect_fee_mode: decoded.collect_fee_mode,
        });
    }
    if decoded.bin_step == 0 || decoded.bin_step > 500 {
        return Err(DlmmValidationError::InvalidBinStep {
            pool: *pool_id,
            bin_step: decoded.bin_step,
        });
    }

    let min_bin_id = i32::from_le_bytes(data[24..28].try_into().unwrap_or([0; 4]));
    let max_bin_id = i32::from_le_bytes(data[28..32].try_into().unwrap_or([0; 4]));
    if min_bin_id >= max_bin_id {
        return Err(DlmmValidationError::Decode {
            reason: format!("pool {pool_id} declares an empty bin-id range"),
        });
    }
    if decoded.active_id < min_bin_id || decoded.active_id > max_bin_id {
        return Err(DlmmValidationError::ActiveBinOutOfRange {
            pool: *pool_id,
            active_id: decoded.active_id,
            min_bin_id,
            max_bin_id,
        });
    }

    // ── PDA derivation checks against the addresses stored on chain ──
    let expected_oracle = derive_oracle_pda(pool_id);
    if decoded.oracle != expected_oracle {
        return Err(DlmmValidationError::PdaMismatch {
            label: "oracle",
            expected: expected_oracle,
            actual: decoded.oracle,
        });
    }
    let expected_reserve_x = derive_reserve_pda(pool_id, &decoded.token_x_mint.0);
    if decoded.reserve_x != expected_reserve_x {
        return Err(DlmmValidationError::PdaMismatch {
            label: "reserve_x",
            expected: expected_reserve_x,
            actual: decoded.reserve_x,
        });
    }
    let expected_reserve_y = derive_reserve_pda(pool_id, &decoded.token_y_mint.0);
    if decoded.reserve_y != expected_reserve_y {
        return Err(DlmmValidationError::PdaMismatch {
            label: "reserve_y",
            expected: expected_reserve_y,
            actual: decoded.reserve_y,
        });
    }

    let mut bitmap = [0u64; 16];
    for (i, word) in bitmap.iter_mut().enumerate() {
        *word = u64::from_le_bytes(data[584 + i * 8..592 + i * 8].try_into().unwrap_or([0; 8]));
    }

    let fee_params = decoded
        .fee_params
        .clone()
        .ok_or_else(|| DlmmValidationError::Decode {
            reason: format!("pool {pool_id} has no fee parameters"),
        })?;

    Ok(ValidatedDlmmPool {
        pool_id: *pool_id,
        token_x_mint: decoded.token_x_mint.0,
        token_y_mint: decoded.token_y_mint.0,
        reserve_x: decoded.reserve_x,
        reserve_y: decoded.reserve_y,
        oracle: decoded.oracle,
        active_id: decoded.active_id,
        bin_step: decoded.bin_step,
        status: decoded.status,
        function_type: decoded.function_type,
        collect_fee_mode: decoded.collect_fee_mode,
        min_bin_id,
        max_bin_id,
        fee_params,
        volatility_accumulator: decoded.volatility_accumulator,
        volatility_reference: decoded.volatility_reference,
        index_reference: i32::from_le_bytes(data[48..52].try_into().unwrap_or([0; 4])),
        last_update_timestamp: decoded.last_update_timestamp,
        bin_array_bitmap: bitmap,
    })
}

// ============================================================================
// BinArray validation
// ============================================================================

/// Validate a raw `BinArray` account payload against the requested array index.
///
/// The account stores its own `array_index`; a mismatch means the fetched account is
/// not the array the traversal asked for and must never be substituted.
pub fn validate_bin_array_account(
    lb_pair: &Pubkey,
    requested_index: i64,
    owner: &Pubkey,
    data: &[u8],
) -> DlmmValidationResult<Vec<DlmmBin>> {
    if *owner != METEORA_DLMM_PROGRAM {
        return Err(DlmmValidationError::WrongOwner {
            account: *lb_pair,
            expected: METEORA_DLMM_PROGRAM,
            actual: *owner,
        });
    }
    let end = BIN_ARRAY_INDEX_OFFSET + BIN_ARRAY_INDEX_LEN;
    if data.len() < end {
        return Err(DlmmValidationError::TooShort {
            expected: end,
            actual: data.len(),
        });
    }
    let stored = i64::from_le_bytes(
        data[BIN_ARRAY_INDEX_OFFSET..end]
            .try_into()
            .unwrap_or([0; 8]),
    );
    if stored != requested_index {
        return Err(DlmmValidationError::BinArrayIndexMismatch {
            expected: requested_index,
            actual: stored,
        });
    }
    MeteoraDlmmPool::parse_bin_array_account(data).map_err(|e| DlmmValidationError::Decode {
        reason: format!("{e:?}"),
    })
}

/// Validate a `BinArrayBitmapExtension` account payload.
pub fn validate_bitmap_extension_account(
    lb_pair: &Pubkey,
    owner: &Pubkey,
    data: &[u8],
) -> DlmmValidationResult<()> {
    if *owner != METEORA_DLMM_PROGRAM {
        return Err(DlmmValidationError::WrongOwner {
            account: *lb_pair,
            expected: METEORA_DLMM_PROGRAM,
            actual: *owner,
        });
    }
    if data.len() < 8 {
        return Err(DlmmValidationError::TooShort {
            expected: 8,
            actual: data.len(),
        });
    }
    Ok(())
}

// ============================================================================
// Cross-pool compatibility
// ============================================================================

/// Token programs of a validated pool, indexed by mint.
///
/// `[(mint, program)]` with one entry per pool mint.
pub fn pool_token_programs(pool: &ValidatedDlmmPool) -> [(Pubkey, TokenProgramKind); 2] {
    [
        (pool.token_x_mint, TokenProgramKind::SplToken),
        (pool.token_y_mint, TokenProgramKind::SplToken),
    ]
}

/// Assert that two pools trade the same unordered mint pair, that the token programs
/// agree per shared mint, and that the flash-loan mint is present in both.
///
/// `programs` supplies the observed token program per mint (WSOL/USDC on mainnet are
/// legacy SPL Token mints; Token-2022 mints are supported through the same path).
pub fn validate_pool_pair_compatibility(
    a: &ValidatedDlmmPool,
    b: &ValidatedDlmmPool,
    programs: &[(Pubkey, TokenProgramKind); 4],
    flash_mint: &Pubkey,
) -> DlmmValidationResult<()> {
    let a_pair = canonical_mint_pair(a.token_x_mint, a.token_y_mint);
    let b_pair = canonical_mint_pair(b.token_x_mint, b.token_y_mint);
    if a_pair != b_pair {
        return Err(DlmmValidationError::MintPairMismatch {
            left: a_pair.0,
            right: b_pair.0,
        });
    }

    if !a.is_token_x(flash_mint) && !a.is_token_y(flash_mint) {
        return Err(DlmmValidationError::FlashMintMissing {
            pool: a.pool_id,
            mint: *flash_mint,
        });
    }
    if !b.is_token_x(flash_mint) && !b.is_token_y(flash_mint) {
        return Err(DlmmValidationError::FlashMintMissing {
            pool: b.pool_id,
            mint: *flash_mint,
        });
    }

    // Program lookup per mint.
    let program_of = |mint: &Pubkey| -> Option<TokenProgramKind> {
        programs.iter().find(|(m, _)| m == mint).map(|(_, p)| *p)
    };

    for mint in [a.token_x_mint, a.token_y_mint] {
        let Some(pa) = program_of(&mint) else {
            continue;
        };
        let Some(pb) = program_of(&mint) else {
            continue;
        };
        if pa != pb {
            return Err(DlmmValidationError::TokenProgramMismatch {
                mint,
                left: pa,
                right: pb,
            });
        }
    }

    Ok(())
}

/// Canonical unordered mint-pair key.
#[inline]
pub fn canonical_mint_pair(x: Pubkey, y: Pubkey) -> (Pubkey, Pubkey) {
    if x.to_bytes() <= y.to_bytes() {
        (x, y)
    } else {
        (y, x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn wsol() -> Pubkey {
        Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap()
    }
    fn usdc() -> Pubkey {
        Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap()
    }
    fn pool_a() -> Pubkey {
        Pubkey::from_str("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6").unwrap()
    }

    #[test]
    fn bin_array_index_uses_floor_division() {
        assert_eq!(bin_array_index_for_bin(0), 0);
        assert_eq!(bin_array_index_for_bin(69), 0);
        assert_eq!(bin_array_index_for_bin(70), 1);
        assert_eq!(bin_array_index_for_bin(-1), -1);
        assert_eq!(bin_array_index_for_bin(-70), -1);
        assert_eq!(bin_array_index_for_bin(-71), -2);
        assert_eq!(bin_array_index_for_bin(-140), -2);
    }

    #[test]
    fn wrong_owner_is_rejected() {
        let data = vec![0u8; LB_PAIR_MIN_SIZE];
        let err = validate_lb_pair_account(&pool_a(), &Pubkey::new_unique(), &data).unwrap_err();
        assert!(matches!(err, DlmmValidationError::WrongOwner { .. }));
    }

    #[test]
    fn short_account_is_rejected() {
        let err =
            validate_lb_pair_account(&pool_a(), &METEORA_DLMM_PROGRAM, &[0u8; 10]).unwrap_err();
        assert!(matches!(err, DlmmValidationError::TooShort { .. }));
    }

    #[test]
    fn bin_array_index_mismatch_is_rejected() {
        let mut data = vec![0u8; 128];
        data[BIN_ARRAY_INDEX_OFFSET..BIN_ARRAY_INDEX_OFFSET + 8]
            .copy_from_slice(&7i64.to_le_bytes());
        let err =
            validate_bin_array_account(&pool_a(), 3, &METEORA_DLMM_PROGRAM, &data).unwrap_err();
        assert!(matches!(
            err,
            DlmmValidationError::BinArrayIndexMismatch {
                expected: 3,
                actual: 7
            }
        ));
    }

    #[test]
    fn bin_array_index_match_is_accepted() {
        let mut data = vec![0u8; 128];
        data[BIN_ARRAY_INDEX_OFFSET..BIN_ARRAY_INDEX_OFFSET + 8]
            .copy_from_slice(&(-4i64).to_le_bytes());
        // Too short for a full bin array payload -> decode error, but NOT an index error.
        let err =
            validate_bin_array_account(&pool_a(), -4, &METEORA_DLMM_PROGRAM, &data).unwrap_err();
        assert!(!matches!(
            err,
            DlmmValidationError::BinArrayIndexMismatch { .. }
        ));
    }

    #[test]
    fn pda_derivations_are_deterministic_and_distinct() {
        let p = pool_a();
        assert_ne!(derive_oracle_pda(&p), derive_reserve_pda(&p, &wsol()));
        assert_ne!(
            derive_reserve_pda(&p, &wsol()),
            derive_reserve_pda(&p, &usdc())
        );
        assert_ne!(derive_bin_array_pda(&p, 0), derive_bin_array_pda(&p, 1));
        assert_ne!(derive_bitmap_extension_pda(&p), derive_event_authority());
    }

    #[test]
    fn canonical_pair_is_order_independent() {
        assert_eq!(
            canonical_mint_pair(wsol(), usdc()),
            canonical_mint_pair(usdc(), wsol())
        );
    }
}
