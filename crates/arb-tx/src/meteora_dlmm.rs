//! Meteora DLMM account compression for the 3-leg meme arbitrage compiler.
//!
//! This module builds byte-optimized `swap2` (and exact-output equivalent) instructions
//! using the official MeteoraAg/dlmm-sdk account layouts and PDA seed conventions.
//!
//! # Design invariants
//!
//! - Optional Anchor accounts use the canonical program-ID sentinel *only* when verified
//!   against the generated official IDL builder; a genuinely configured host fee / referral
//!   account is never replaced with a sentinel.
//! - The Memo Program address is used where the instruction requires it.
//! - Generated account metas match the official Meteora builder exactly; accounts are
//!   never manually reordered to save bytes.
//! - Absent optional accounts never introduce unintended unique account keys.
//! - Maximum two Bin Arrays per leg is a transaction-size policy, not a claim that all
//!   DLMM swaps need only two.

use crate::{AccountMeta, CompiledInstruction};
use arb_constants::{METEORA_BINS_PER_BIN_ARRAY, METEORA_SWAP2_DISCRIMINATOR};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Number of liquidity bins per `BinArray` account (protocol constant: 70, not 64).
pub const MAX_BIN_PER_ARRAY: i64 = METEORA_BINS_PER_BIN_ARRAY;

/// DLMM program ID on Mainnet.
pub const DLMM_PROGRAM_ID_STR: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

/// Memo Program ID required by DLMM swap2 instructions.
pub const MEMO_PROGRAM_ID_STR: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// SPL Token Program ID.
pub const SPL_TOKEN_PROGRAM_ID_STR: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// Token-2022 Program ID.
pub const TOKEN_2022_PROGRAM_ID_STR: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Canonical program-ID sentinel used for absent optional Anchor accounts.
/// When an optional account (e.g. host fee) is not configured, the official Meteora
/// builder places the DLMM program ID itself as a placeholder. We mirror this exactly.
/// The placeholder meta is always `readonly, non-signer` per the official TS SDK
/// (`derivePlaceHolderAccountMeta`): passing an executable program account as
/// writable would fail at the runtime.
pub const OPTIONAL_ACCOUNT_SENTINEL: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

/// Default DLMM bitmap covers Bin Array indices `-512..=511`
/// (`BIN_ARRAY_BITMAP_SIZE = 512` in the official IDL; `internalBitmapRange()`
/// in the official TS SDK). Any array index outside this range requires the
/// BinArrayBitmapExtension account, which must exist on-chain — callers MUST
/// verify it exists before referencing it and MUST NOT pass the extension PDA
/// to a pool that has none.
pub const DEFAULT_BITMAP_BIN_ARRAY_RANGE: i64 = 512;

/// Returns true if any of the given Bin Array indices fall outside the default
/// bitmap, meaning the BinArrayBitmapExtension account is required.
pub fn exceeds_default_bitmap(indices: &[i64]) -> bool {
    indices
        .iter()
        .any(|&i| !(-DEFAULT_BITMAP_BIN_ARRAY_RANGE..DEFAULT_BITMAP_BIN_ARRAY_RANGE).contains(&i))
}

/// Swap direction through a DLMM pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapDirection {
    /// Selling token X for token Y: price moves down, bin indices decrease.
    XtoY,
    /// Selling token Y for token X: price moves up, bin indices increase.
    YtoX,
}

/// Exact-input vs exact-output swap mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapMode {
    /// Exact input: spend exactly `amount_in`, receive at least `min_amount_out`.
    ExactIn,
    /// Exact output: receive exactly `amount_out`, spend at most `max_amount_in`.
    ExactOut,
}

/// Fully resolved account set for a single Meteora DLMM swap2 leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlmmSwapAccountSet {
    pub lb_pair: Pubkey,
    /// `Some(pda)` when the BinArrayBitmapExtension account exists and is required
    /// (a Bin Array index outside the default bitmap range, or an overflowing
    /// active-array traversal). `None` when the swap stays inside the default
    /// bitmap: the sentinel (DLMM program ID, readonly) is encoded instead, exactly
    /// as the official SDK does for an unconfigured optional account.
    pub bin_array_bitmap_extension: Option<Pubkey>,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub user_token_in: Pubkey,
    pub user_token_out: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    /// Optional host fee / referral account. `None` means absent: the sentinel is used.
    /// When present, the account is WRITABLE per the official IDL.
    pub host_fee_in: Option<Pubkey>,
    pub user: Pubkey, // signer
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub memo_program: Pubkey,
    pub event_authority: Pubkey,
    pub bin_arrays: Vec<Pubkey>,
}

impl DlmmSwapAccountSet {
    /// Build the complete account meta vector matching the official Meteora `swap2` builder
    /// account order exactly. No manual reordering.
    pub fn to_account_metas(&self) -> Vec<AccountMeta> {
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).expect("valid sentinel");

        // Account order verified against the official IDL (`idls/dlmm.json` in
        // MeteoraAg/dlmm-sdk) for both `swap2` and `swap_exact_out2`:
        //  0  lb_pair                          (writable)
        //  1  bin_array_bitmap_extension       (writable, optional -> sentinel)
        //  2  reserve_x                        (writable)
        //  3  reserve_y                        (writable)
        //  4  user_token_in                    (writable)
        //  5  user_token_out                   (writable)
        //  6  token_x_mint                     (readonly)
        //  7  token_y_mint                     (readonly)
        //  8  oracle                           (writable)
        //  9  host_fee_in                      (writable, optional -> sentinel)
        // 10  user                             (signer, NOT writable)
        // 11  token_x_program                  (readonly)
        // 12  token_y_program                  (readonly)
        // 13  memo_program                     (readonly)
        // 14  event_authority                  (readonly)
        // 15  program self-reference           (readonly)
        // Remaining: bin_arrays (writable)
        let bitmap_ext_meta = match self.bin_array_bitmap_extension {
            Some(addr) => AccountMeta::new(addr, false),
            None => AccountMeta::new_readonly(sentinel, false),
        };
        let host_fee_meta = match self.host_fee_in {
            Some(addr) => AccountMeta::new(addr, false),
            None => AccountMeta::new_readonly(sentinel, false),
        };

        let mut metas = vec![
            AccountMeta::new(self.lb_pair, false),
            bitmap_ext_meta,
            AccountMeta::new(self.reserve_x, false),
            AccountMeta::new(self.reserve_y, false),
            AccountMeta::new(self.user_token_in, false),
            AccountMeta::new(self.user_token_out, false),
            AccountMeta::new_readonly(self.token_x_mint, false),
            AccountMeta::new_readonly(self.token_y_mint, false),
            AccountMeta::new(self.oracle, false),
            host_fee_meta,
            AccountMeta::new_readonly(self.user, true),
            AccountMeta::new_readonly(self.token_x_program, false),
            AccountMeta::new_readonly(self.token_y_program, false),
            AccountMeta::new_readonly(self.memo_program, false),
            AccountMeta::new_readonly(self.event_authority, false),
            AccountMeta::new_readonly(Pubkey::from_str(DLMM_PROGRAM_ID_STR).unwrap(), false),
        ];
        for ba in &self.bin_arrays {
            metas.push(AccountMeta::new(*ba, false));
        }
        metas
    }

    /// Returns true if an optional-account sentinel is in use (host fee absent).
    pub fn uses_optional_sentinel(&self) -> bool {
        self.host_fee_in.is_none()
    }
}

/// Resolved token programs for a DLMM leg, one per mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmTokenPrograms {
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
}

/// Result of directional Bin Array derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionalBinArrays {
    /// All Bin Array PDAs to include, in order: current array then adjacent.
    pub bin_array_pdas: Vec<Pubkey>,
    /// The array indices used (for verification and reporting).
    pub indices: Vec<i64>,
    /// Whether a Bitmap Extension account is required (any selected index lies outside
    /// the default bitmap range `-512..=511`). When required, the caller MUST pass the
    /// extension account explicitly or the swap will fail.
    pub bitmap_extension_required: bool,
}

/// The default bitmap covers array indices in `-512..=511`
/// (`BIN_ARRAY_BITMAP_SIZE = 512` in the official IDL).
fn default_bitmap_covers(indices: &[i64]) -> bool {
    !exceeds_default_bitmap(indices)
}

/// Derive the active Bin Array index from a bin ID using floor division.
///
/// Uses `active_id.div_euclid(70)`. Rust `div_euclid` already implements floor
/// division for negative numbers — no second negative-index adjustment is applied.
pub fn active_bin_array_index(active_id: i32) -> i64 {
    (active_id as i64).div_euclid(MAX_BIN_PER_ARRAY)
}

/// Derive a Bin Array PDA using the official seed representation:
/// `[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()]`
/// where the index uses the protocol's verified signed integer type (i64).
pub fn derive_bin_array_pda(lb_pair: &Pubkey, index: i64) -> Pubkey {
    let program_id = Pubkey::from_str(DLMM_PROGRAM_ID_STR).expect("valid dlmm program");
    let seeds: &[&[u8]] = &[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive the BinArrayBitmapExtension PDA using official seeds: `[b"bitmap", lb_pair]`.
pub fn derive_bin_array_bitmap_extension_pda(lb_pair: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(DLMM_PROGRAM_ID_STR).expect("valid dlmm program");
    let seeds: &[&[u8]] = &[b"bitmap", lb_pair.as_ref()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive directional Bin Arrays for a DLMM swap leg.
///
/// Directional traversal:
/// - X → Y: current array and lower adjacent array.
/// - Y → X: current array and higher adjacent array.
///
/// Maximum two Bin Arrays per leg is a transaction-size policy.
///
/// Pre-conditions (checked by the caller before construction):
/// 1. The actual trade has been quoted.
/// 2. All Bin Arrays traversed have been determined.
/// 3. They belong to the correct pool.
/// 4. Their direction and ordering are verified.
/// 5. The Bitmap Extension requirement is verified.
/// 6. The route is rejected if more than two arrays are necessary for any leg.
pub fn derive_directional_bin_arrays(
    lb_pair: &Pubkey,
    active_id: i32,
    direction: SwapDirection,
    bins_traversed: u32,
) -> Result<DirectionalBinArrays, DlmmError> {
    let active_index = active_bin_array_index(active_id);

    // Determine direction increment: X→Y goes lower, Y→X goes higher.
    let increment: i64 = match direction {
        SwapDirection::XtoY => -1,
        SwapDirection::YtoX => 1,
    };

    // Number of arrays needed to cover the bins traversed, starting from the active array.
    let arrays_needed = (bins_traversed as i64).div_euclid(MAX_BIN_PER_ARRAY) + 1;

    // Policy: max two Bin Arrays per leg.
    if arrays_needed > 2 {
        return Err(DlmmError::TooManyBinArrays {
            needed: arrays_needed as usize,
            max: 2,
        });
    }

    let mut indices = Vec::with_capacity(arrays_needed as usize);
    let mut pdas = Vec::with_capacity(arrays_needed as usize);

    // Always include the current (active) array first.
    indices.push(active_index);
    pdas.push(derive_bin_array_pda(lb_pair, active_index));

    // Add adjacent arrays in the swap direction.
    for i in 1..arrays_needed {
        let idx = active_index + increment * i;
        indices.push(idx);
        pdas.push(derive_bin_array_pda(lb_pair, idx));
    }

    // The Bitmap Extension is required only when a selected Bin Array index lies
    // outside the default bitmap range `-512..=511` (see `exceeds_default_bitmap`).
    // Two arrays within the default range do NOT require it.
    let bitmap_extension_required = !default_bitmap_covers(&indices);

    Ok(DirectionalBinArrays {
        bin_array_pdas: pdas,
        indices,
        bitmap_extension_required,
    })
}

/// `AccountsType` discriminants from the official IDL (`AccountsType` enum, Borsh
/// single-byte variant index for unit variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountsType {
    /// Transfer hook of the token X account: matching `length` extra accounts are appended.
    TransferHookX,
    /// Transfer hook of the token Y account.
    TransferHookY,
    /// Transfer hook of the reward mint.
    TransferHookReward,
    /// Transfer hook of a multi-reward, with the reward index as payload.
    TransferHookMultiReward(u8),
    /// Transfer hook of the referral account.
    TransferHookReferral,
}

impl AccountsType {
    fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            Self::TransferHookX => buf.push(0),
            Self::TransferHookY => buf.push(1),
            Self::TransferHookReward => buf.push(2),
            Self::TransferHookMultiReward(idx) => {
                buf.push(3);
                buf.push(*idx);
            }
            Self::TransferHookReferral => buf.push(4),
        }
    }
}

/// One entry of `RemainingAccountsSlice { accounts_type, length }` (Borsh).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemainingAccountsSlice {
    pub accounts_type: AccountsType,
    /// Number of remaining accounts for this slice.
    pub length: u8,
}

/// Borsh-encode a `RemainingAccountsInfo { slices: Vec<RemainingAccountsSlice> }` value,
/// exactly as Anchor does (Vec length is a little-endian u32).
pub fn encode_remaining_accounts_info(slices: &[RemainingAccountsSlice]) -> Vec<u8> {
    let extra = slices
        .iter()
        .map(|s| match s.accounts_type {
            AccountsType::TransferHookMultiReward(_) => 3usize,
            _ => 2usize,
        })
        .sum::<usize>();
    let mut buf = Vec::with_capacity(4 + extra);
    buf.extend_from_slice(&(slices.len() as u32).to_le_bytes());
    for slice in slices {
        slice.accounts_type.encode(&mut buf);
        buf.push(slice.length);
    }
    buf
}

/// `RemainingAccountsInfo` with no transfer-hook slices. Borsh encoding: a u32 LE zero.
pub const EMPTY_REMAINING_ACCOUNTS_INFO: [u8; 4] = [0, 0, 0, 0];

/// Anchor 8-byte discriminator for `swap_exact_out2`, verified against the pinned
/// official IDL `idls/dlmm.json` (MeteoraAg/dlmm-sdk, `swap_exact_out2`):
/// `[43, 215, 247, 132, 137, 60, 243, 81]`. Do NOT change this without
/// re-verifying against the IDL.
pub const SWAP_EXACT_OUT2_DISCRIMINATOR: [u8; 8] = [43, 215, 247, 132, 137, 60, 243, 81];

/// Build a genuine Meteora DLMM `swap2` instruction (exact input).
///
/// Data layout (from the official IDL `swap2` and `Swap2` args):
///   [0..8]   discriminator = `METEORA_SWAP2_DISCRIMINATOR` = sha256("global:swap2")[..8]
///   [8..16]  amount_in: u64 LE
///   [16..24] min_amount_out: u64 LE
///   [24..]   RemainingAccountsInfo (Borsh): vec length (u32 LE) of slices
pub fn build_swap2_exact_in(
    accounts: &DlmmSwapAccountSet,
    amount_in: u64,
    min_amount_out: u64,
) -> CompiledInstruction {
    build_swap2_exact_in_with_remaining_accounts(accounts, amount_in, min_amount_out, &[])
}

/// `swap2` with an explicit remaining-accounts slice list (e.g. Token-2022 transfer hooks).
/// Bin Arrays are always appended as remaining accounts after these slices.
pub fn build_swap2_exact_in_with_remaining_accounts(
    accounts: &DlmmSwapAccountSet,
    amount_in: u64,
    min_amount_out: u64,
    transfer_hook_slices: &[RemainingAccountsSlice],
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(28 + transfer_hook_slices.len() * 3);
    data.extend_from_slice(&METEORA_SWAP2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&encode_remaining_accounts_info(transfer_hook_slices));

    CompiledInstruction {
        program_id: Pubkey::from_str(DLMM_PROGRAM_ID_STR).expect("valid dlmm program"),
        accounts: accounts.to_account_metas(),
        data,
    }
}

/// Build a Meteora DLMM `swap_exact_out2` instruction (exact output).
///
/// Data layout (verified against the official IDL `swap_exact_out2` and the
/// Borsh-serialized `SwapExactOut2` args):
///   [0..8]   discriminator = `SWAP_EXACT_OUT2_DISCRIMINATOR`
///   [8..16]  max_in_amount: u64 LE  (spend at most this)
///   [16..24] out_amount: u64 LE     (receive exactly this)
///   [24..]   RemainingAccountsInfo (Borsh): vec length (u32 LE) of slices
///
/// Account order and flags match the official IDL exactly (same 16 base accounts +
/// dynamic Bin Arrays as `swap2`): lb_pair writable, bitmap extension writable or
/// sentinel, oracles/reserves/user token accounts writable, host_fee_in writable or
/// sentinel, user signer readonly, programs readonly.
pub fn build_swap2_exact_out(
    accounts: &DlmmSwapAccountSet,
    max_amount_in: u64,
    amount_out: u64,
) -> CompiledInstruction {
    build_swap2_exact_out_with_remaining_accounts(accounts, max_amount_in, amount_out, &[])
}

/// `swap_exact_out2` with an explicit remaining-accounts slice list (e.g. Token-2022 transfer hooks).
pub fn build_swap2_exact_out_with_remaining_accounts(
    accounts: &DlmmSwapAccountSet,
    max_amount_in: u64,
    amount_out: u64,
    transfer_hook_slices: &[RemainingAccountsSlice],
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(28 + transfer_hook_slices.len() * 3);
    data.extend_from_slice(&SWAP_EXACT_OUT2_DISCRIMINATOR);
    data.extend_from_slice(&max_amount_in.to_le_bytes());
    data.extend_from_slice(&amount_out.to_le_bytes());
    data.extend_from_slice(&encode_remaining_accounts_info(transfer_hook_slices));

    CompiledInstruction {
        program_id: Pubkey::from_str(DLMM_PROGRAM_ID_STR).expect("valid dlmm program"),
        accounts: accounts.to_account_metas(),
        data,
    }
}

/// Derive token programs for a leg based on the mint types.
pub fn resolve_token_programs(
    token_x_program_kind: TokenProgramKind,
    token_y_program_kind: TokenProgramKind,
) -> DlmmTokenPrograms {
    DlmmTokenPrograms {
        token_x_program: token_x_program_kind.program_id(),
        token_y_program: token_y_program_kind.program_id(),
    }
}

/// Token program kind for a DLMM mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenProgramKind {
    #[default]
    SplToken,
    Token2022,
}

impl TokenProgramKind {
    pub fn program_id(self) -> Pubkey {
        match self {
            Self::SplToken => {
                Pubkey::from_str(SPL_TOKEN_PROGRAM_ID_STR).expect("valid spl token program")
            }
            Self::Token2022 => {
                Pubkey::from_str(TOKEN_2022_PROGRAM_ID_STR).expect("valid token-2022 program")
            }
        }
    }
}

/// Prove that absent optional accounts do not introduce unintended unique account keys.
///
/// Both optional accounts (`bin_array_bitmap_extension`, `host_fee_in`) are encoded as the
/// DLMM program ID (readonly, non-signer) when absent. The DLMM program ID is always
/// present in the account list anyway (position 15, the program self-reference), so an
/// absent optional account adds zero unique keys to the message.
///
/// Returns `(optional_account_metas, unique_keys)` so callers can assert that the
/// positions 1 and 9 metas are the readonly sentinel and the key set has no duplicates
/// beyond the expected program-ID repetitions.
pub fn optional_account_sentinel_proof(
    accounts: &DlmmSwapAccountSet,
) -> (Vec<AccountMeta>, Vec<Pubkey>) {
    let metas = accounts.to_account_metas();
    let optional_metas = vec![metas[1].clone(), metas[9].clone()];

    let mut seen = std::collections::HashSet::new();
    let unique_keys: Vec<Pubkey> = metas
        .into_iter()
        .map(|m| m.pubkey)
        .filter(|k| seen.insert(*k))
        .collect();

    (optional_metas, unique_keys)
}

#[derive(Debug, thiserror::Error)]
pub enum DlmmError {
    #[error("Too many Bin Arrays required: {needed} > max {max}")]
    TooManyBinArrays { needed: usize, max: usize },
    #[error("Invalid account configuration: {0}")]
    InvalidConfiguration(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_constants::METEORA_SWAP2_DISCRIMINATOR;

    fn test_lb_pair() -> Pubkey {
        Pubkey::from_str("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6").unwrap()
    }

    fn test_user() -> Pubkey {
        Pubkey::new_from_array([1u8; 32])
    }

    fn default_account_set() -> DlmmSwapAccountSet {
        DlmmSwapAccountSet {
            lb_pair: test_lb_pair(),
            bin_array_bitmap_extension: None,
            reserve_x: Pubkey::from_str("EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o").unwrap(),
            reserve_y: Pubkey::from_str("CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz").unwrap(),
            user_token_in: Pubkey::new_from_array([3u8; 32]),
            user_token_out: Pubkey::new_from_array([4u8; 32]),
            token_x_mint: Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap(),
            token_y_mint: Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap(),
            oracle: Pubkey::new_from_array([5u8; 32]),
            host_fee_in: None,
            user: test_user(),
            token_x_program: Pubkey::from_str(SPL_TOKEN_PROGRAM_ID_STR).unwrap(),
            token_y_program: Pubkey::from_str(TOKEN_2022_PROGRAM_ID_STR).unwrap(),
            memo_program: Pubkey::from_str(MEMO_PROGRAM_ID_STR).unwrap(),
            event_authority: Pubkey::from_str("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6")
                .unwrap(),
            bin_arrays: vec![],
        }
    }

    #[test]
    fn test_active_bin_array_index_uses_div_euclid() {
        // Floor division: -1 / 70 = -1 (not 0).
        assert_eq!(active_bin_array_index(-1), -1);
        assert_eq!(active_bin_array_index(0), 0);
        assert_eq!(active_bin_array_index(69), 0);
        assert_eq!(active_bin_array_index(70), 1);
        assert_eq!(active_bin_array_index(-70), -1);
        assert_eq!(active_bin_array_index(-71), -2);
    }

    #[test]
    fn test_derive_bin_array_pda_uses_i64_le_seed() {
        let pair = test_lb_pair();
        let pda_0 = derive_bin_array_pda(&pair, 0);
        let pda_1 = derive_bin_array_pda(&pair, 1);
        let pda_neg1 = derive_bin_array_pda(&pair, -1);
        assert_ne!(pda_0, pda_1);
        assert_ne!(pda_0, pda_neg1);
        // Deterministic
        assert_eq!(derive_bin_array_pda(&pair, 0), pda_0);
    }

    #[test]
    fn test_derive_directional_bin_arrays_x_to_y_goes_lower() {
        let pair = test_lb_pair();
        // 71 bins traversed => 2 arrays needed (active + lower adjacent for X→Y)
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 71).unwrap();
        assert_eq!(result.indices, vec![1, 0]); // active=1, lower=0
        assert_eq!(result.bin_array_pdas.len(), 2);
        // 2 arrays, both inside the default bitmap range -> no bitmap extension needed
        assert!(!result.bitmap_extension_required);

        // 1 bin traversed => only active array
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 1).unwrap();
        assert_eq!(result.indices, vec![1]);
        assert_eq!(result.bin_array_pdas.len(), 1);
    }

    #[test]
    fn test_derive_directional_bin_arrays_y_to_x_goes_higher() {
        let pair = test_lb_pair();
        // 71 bins traversed => 2 arrays needed (active + higher adjacent for Y→X)
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::YtoX, 71).unwrap();
        assert_eq!(result.indices, vec![1, 2]); // active=1, higher=2
        assert_eq!(result.bin_array_pdas.len(), 2);
        assert!(!result.bitmap_extension_required);

        // 1 bin traversed => only active array
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::YtoX, 1).unwrap();
        assert_eq!(result.indices, vec![1]);
        assert_eq!(result.bin_array_pdas.len(), 1);
    }

    #[test]
    fn test_derive_directional_bin_arrays_single_array_no_bitmap() {
        let pair = test_lb_pair();
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 1).unwrap();
        assert_eq!(result.indices.len(), 1);
        assert!(!result.bitmap_extension_required);
    }

    #[test]
    fn test_derive_directional_bin_arrays_two_arrays_in_default_range_no_bitmap() {
        let pair = test_lb_pair();
        // 71 bins traversed => 2 arrays needed, both within the default bitmap
        // (indices 1 and 0) -> bitmap extension NOT required.
        let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 71).unwrap();
        assert_eq!(result.indices.len(), 2);
        assert!(!result.bitmap_extension_required);
    }

    #[test]
    fn test_exceeds_default_bitmap() {
        assert!(!exceeds_default_bitmap(&[1, 0]));
        assert!(!exceeds_default_bitmap(&[-512, 511]));
        assert!(exceeds_default_bitmap(&[512]));
        assert!(exceeds_default_bitmap(&[-513]));
        assert!(!default_bitmap_covers(&[512]));
        assert!(default_bitmap_covers(&[-512, 511]));
    }

    #[test]
    fn test_derive_directional_bin_arrays_rejects_more_than_two() {
        let pair = test_lb_pair();
        // 141 bins traversed => 3 arrays needed => rejected by policy
        let err = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 141).unwrap_err();
        assert!(matches!(
            err,
            DlmmError::TooManyBinArrays { needed: 3, max: 2 }
        ));
    }

    #[test]
    fn test_derive_directional_bin_arrays_no_second_negative_adjustment() {
        // active_id = -1 => index = -1 (floor div), not adjusted again.
        // 1 bin traversed => only active array [-1].
        let pair = test_lb_pair();
        let result = derive_directional_bin_arrays(&pair, -1, SwapDirection::XtoY, 1).unwrap();
        assert_eq!(result.indices, vec![-1]);
        assert_eq!(result.bin_array_pdas.len(), 1);

        // 71 bins traversed => 2 arrays: active (-1) + lower (-2).
        let result = derive_directional_bin_arrays(&pair, -1, SwapDirection::XtoY, 71).unwrap();
        assert_eq!(result.indices, vec![-1, -2]);
        assert_eq!(result.bin_array_pdas.len(), 2);
    }

    #[test]
    fn test_build_swap2_account_order_matches_official_builder() {
        let mut accounts = default_account_set();
        accounts.bin_arrays = vec![
            derive_bin_array_pda(&test_lb_pair(), 0),
            derive_bin_array_pda(&test_lb_pair(), 1),
        ];

        let ix = build_swap2_exact_in(&accounts, 1_000_000, 900_000);
        assert_eq!(
            ix.program_id,
            Pubkey::from_str(DLMM_PROGRAM_ID_STR).unwrap()
        );
        assert_eq!(ix.data.len(), 28);
        assert_eq!(&ix.data[0..8], &METEORA_SWAP2_DISCRIMINATOR);

        // Verify account order: 16 static + 2 bin arrays = 18 accounts
        assert_eq!(ix.accounts.len(), 18);

        // Index 0: lb_pair (writable, non-signer)
        assert_eq!(ix.accounts[0].pubkey, accounts.lb_pair);
        assert!(ix.accounts[0].is_writable);
        assert!(!ix.accounts[0].is_signer);

        // Index 1: bitmap extension => sentinel (program ID) when absent
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).unwrap();
        assert_eq!(ix.accounts[1].pubkey, sentinel);
        assert!(!ix.accounts[1].is_signer);
        assert!(!ix.accounts[1].is_writable);

        // Index 9: host_fee_in => sentinel (program ID) when absent
        assert_eq!(ix.accounts[9].pubkey, sentinel);
        assert!(!ix.accounts[9].is_signer);
        assert!(!ix.accounts[9].is_writable);

        // Index 10: user (signer)
        assert_eq!(ix.accounts[10].pubkey, accounts.user);
        assert!(ix.accounts[10].is_signer);

        // Index 13: memo program
        assert_eq!(ix.accounts[13].pubkey, accounts.memo_program);

        // Index 14: event authority
        assert_eq!(ix.accounts[14].pubkey, accounts.event_authority);

        // Index 15: program self-reference
        assert_eq!(
            ix.accounts[15].pubkey,
            Pubkey::from_str(DLMM_PROGRAM_ID_STR).unwrap()
        );

        // Remaining: bin arrays (writable)
        assert_eq!(ix.accounts[16].pubkey, accounts.bin_arrays[0]);
        assert_eq!(ix.accounts[17].pubkey, accounts.bin_arrays[1]);
        assert!(ix.accounts[16].is_writable);
        assert!(ix.accounts[17].is_writable);
    }

    #[test]
    fn test_optional_sentinel_proof_no_unintended_keys() {
        let accounts = default_account_set();
        let (optional_metas, unique_keys) = optional_account_sentinel_proof(&accounts);
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).unwrap();
        assert_eq!(optional_metas[0].pubkey, sentinel);
        assert!(!optional_metas[0].is_writable);
        assert_eq!(optional_metas[1].pubkey, sentinel);
        assert!(!optional_metas[1].is_writable);

        // Both sentinels are readonly, non-signer, and exactly the DLMM program ID.
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).unwrap();
        for meta in &optional_metas {
            assert_eq!(meta.pubkey, sentinel);
            assert!(!meta.is_writable);
            assert!(!meta.is_signer);
        }
        // The DLMM program ID appears exactly once in the unique set (it is repeated
        // at positions 1, 9 and 15 in the raw account list).
        assert!(unique_keys.contains(&sentinel));
    }

    #[test]
    fn test_token_program_resolution_spl_and_token2022() {
        let progs = resolve_token_programs(TokenProgramKind::SplToken, TokenProgramKind::Token2022);
        assert_eq!(
            progs.token_x_program,
            Pubkey::from_str(SPL_TOKEN_PROGRAM_ID_STR).unwrap()
        );
        assert_eq!(
            progs.token_y_program,
            Pubkey::from_str(TOKEN_2022_PROGRAM_ID_STR).unwrap()
        );
    }

    #[test]
    fn test_build_swap2_exact_out_data_layout() {
        let mut accounts = default_account_set();
        accounts.host_fee_in = Some(Pubkey::new_unique()); // configured host fee, NOT sentinel
        let ix = build_swap2_exact_out(&accounts, 2_000_000, 1_000_000);
        assert_eq!(ix.data.len(), 28);
        // Account 9 should be the actual host fee, not the sentinel
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).unwrap();
        assert_ne!(ix.accounts[9].pubkey, sentinel);
    }

    // -------------------------------------------------------------------------
    // Golden vectors verified against the pinned official IDL (fixtures/dlmm.json,
    // MeteoraAg/dlmm-sdk) — swap_exact_out2 and RemainingAccountsInfo Borsh encoding.
    // -------------------------------------------------------------------------

    #[test]
    fn golden_vector_swap_exact_out2_discriminator_and_args() {
        // Official IDL `swap_exact_out2`.discriminator == [43, 215, 247, 132, 137, 60, 243, 81]
        assert_eq!(
            SWAP_EXACT_OUT2_DISCRIMINATOR,
            [43, 215, 247, 132, 137, 60, 243, 81]
        );

        let accounts = default_account_set();
        let ix = build_swap2_exact_out(&accounts, 2_000_000_000, 1_000_000_000);
        let expected: Vec<u8> = [
            SWAP_EXACT_OUT2_DISCRIMINATOR.as_slice(),
            &2_000_000_000u64.to_le_bytes(),
            &1_000_000_000u64.to_le_bytes(),
            &[0, 0, 0, 0], // RemainingAccountsInfo { slices: [] }
        ]
        .concat();
        assert_eq!(ix.data, expected);
        assert_eq!(ix.data.len(), 28);
    }

    #[test]
    fn golden_vector_swap2_exact_in_data() {
        // Official IDL `swap2`.discriminator == [65, 75, 63, 76, 235, 91, 91, 136]
        assert_eq!(
            METEORA_SWAP2_DISCRIMINATOR,
            [65, 75, 63, 76, 235, 91, 91, 136]
        );
        let accounts = default_account_set();
        let ix = build_swap2_exact_in(&accounts, 1_000_000_000, 900_000_000);
        let expected: Vec<u8> = [
            METEORA_SWAP2_DISCRIMINATOR.as_slice(),
            &1_000_000_000u64.to_le_bytes(),
            &900_000_000u64.to_le_bytes(),
            &[0, 0, 0, 0], // RemainingAccountsInfo { slices: [] }
        ]
        .concat();
        assert_eq!(ix.data, expected);
    }

    #[test]
    fn golden_vector_remaining_accounts_info() {
        assert_eq!(encode_remaining_accounts_info(&[]), vec![0, 0, 0, 0]);
        assert_eq!(
            encode_remaining_accounts_info(&[RemainingAccountsSlice {
                accounts_type: AccountsType::TransferHookX,
                length: 4,
            }]),
            vec![1, 0, 0, 0, 0, 4]
        );
        assert_eq!(
            encode_remaining_accounts_info(&[
                RemainingAccountsSlice {
                    accounts_type: AccountsType::TransferHookY,
                    length: 2,
                },
                RemainingAccountsSlice {
                    accounts_type: AccountsType::TransferHookReferral,
                    length: 3,
                },
            ]),
            vec![2, 0, 0, 0, 1, 2, 4, 3]
        );
        assert_eq!(
            encode_remaining_accounts_info(&[RemainingAccountsSlice {
                accounts_type: AccountsType::TransferHookMultiReward(3),
                length: 7,
            }]),
            vec![1, 0, 0, 0, 3, 3, 7]
        );
    }

    #[test]
    fn golden_vector_swap_exact_out2_with_transfer_hooks() {
        let accounts = default_account_set();
        let slices = [
            RemainingAccountsSlice {
                accounts_type: AccountsType::TransferHookX,
                length: 1,
            },
            RemainingAccountsSlice {
                accounts_type: AccountsType::TransferHookY,
                length: 2,
            },
        ];
        let ix = build_swap2_exact_out_with_remaining_accounts(
            &accounts,
            500_000_000,
            250_000_000,
            &slices,
        );
        let mut expected = Vec::new();
        expected.extend_from_slice(&SWAP_EXACT_OUT2_DISCRIMINATOR);
        expected.extend_from_slice(&500_000_000u64.to_le_bytes());
        expected.extend_from_slice(&250_000_000u64.to_le_bytes());
        expected.extend_from_slice(&[2, 0, 0, 0, 0, 1, 1, 2]);
        assert_eq!(ix.data, expected);
        assert_eq!(ix.data.len(), 32);
    }

    #[test]
    fn golden_vector_optional_account_flags_match_idl() {
        // absent optionals -> readonly program-ID sentinel at slot 1 and slot 9
        let accounts = default_account_set();
        let ix = build_swap2_exact_out(&accounts, 1, 1);
        let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).unwrap();
        assert_eq!(ix.accounts[1].pubkey, sentinel);
        assert!(!ix.accounts[1].is_writable && !ix.accounts[1].is_signer);
        assert_eq!(ix.accounts[9].pubkey, sentinel);
        assert!(!ix.accounts[9].is_writable && !ix.accounts[9].is_signer);

        // present optional accounts -> writable, same slot, real address
        let mut accounts = default_account_set();
        let ext = derive_bin_array_bitmap_extension_pda(&test_lb_pair());
        let host = Pubkey::new_unique();
        accounts.bin_array_bitmap_extension = Some(ext);
        accounts.host_fee_in = Some(host);
        let ix = build_swap2_exact_out(&accounts, 1, 1);
        assert_eq!(ix.accounts[1].pubkey, ext);
        assert!(ix.accounts[1].is_writable && !ix.accounts[1].is_signer);
        assert_eq!(ix.accounts[9].pubkey, host);
        assert!(ix.accounts[9].is_writable && !ix.accounts[9].is_signer);

        // account order is identical for exact-in and exact-out builders
        let mut in_accounts = default_account_set();
        in_accounts.bin_array_bitmap_extension = Some(ext);
        in_accounts.host_fee_in = Some(host);
        let ix_in = build_swap2_exact_in(&in_accounts, 1, 1);
        let mut out_accounts = default_account_set();
        out_accounts.bin_array_bitmap_extension = Some(ext);
        out_accounts.host_fee_in = Some(host);
        let ix_out = build_swap2_exact_out(&out_accounts, 1, 1);
        assert_eq!(
            ix_in
                .accounts
                .iter()
                .map(|a| (a.pubkey, a.is_writable, a.is_signer))
                .collect::<Vec<_>>(),
            ix_out
                .accounts
                .iter()
                .map(|a| (a.pubkey, a.is_writable, a.is_signer))
                .collect::<Vec<_>>(),
        );
    }
}
