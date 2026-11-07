//! Verified on-chain economic inputs for the authoritative `EconomicConfig`.
//!
//! Every on-chain value is read from Yellowstone account snapshots and carries
//! provenance + freshness. The Jito cost input is the configured conservative tip
//! ceiling. Any unverifiable account fails closed; no JSON-RPC fallback is used.

// This module is included by several binary targets (main + bins) via `#[path]`;
// each target uses a different subset of the public API, so unused-in-one-target
// items are expected and not dead code.
#![allow(dead_code)]

use arb_math::economic::EconomicConfig;
use arb_types::{Lamports, MintId, Slot, TokenAmount};
use arb_ingest::AccountSnapshotStore;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// A single verified input with provenance and freshness against a quoting slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedValue<T> {
    pub value: T,
    /// Where the value came from (e.g. "kamino-reserve-flashLoanFeeSf").
    pub source: &'static str,
    /// Slot at which the account was observed on-chain.
    pub observed_slot: Slot,
    /// Slots elapsed between observation and the quoting slot.
    pub age_slots: u64,
}

impl<T> VerifiedValue<T> {
    pub fn new(value: T, source: &'static str, observed_slot: Slot, quoting_slot: Slot) -> Self {
        let age_slots = quoting_slot.0.saturating_sub(observed_slot.0);
        Self {
            value,
            source,
            observed_slot,
            age_slots,
        }
    }
}

/// All verified inputs needed to build the authoritative `EconomicConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedQuoteInputs {
    /// Kamino flash-loan fee as the raw Q64.64 on-chain value
    /// (`ReserveConfig.fees.flash_loan_fee_sf`).
    pub flash_loan_fee_sf: VerifiedValue<u64>,
    /// Base mint decimals, read from the verified SPL Mint account.
    pub base_decimals: VerifiedValue<u8>,
    /// Base-mint raw units per lamport numerator.
    pub lamports_per_base_raw: VerifiedValue<u128>,
    /// Base-mint raw units per lamport denominator (1 for WSOL).
    pub lamports_per_base_denom: VerifiedValue<u128>,
    /// Configured Jito tip ceiling, lamports. This deliberately overstates the
    /// expected cost until a fresh tip-floor feed is wired into the same slot-aware
    /// input contract.
    pub tip_lamports: VerifiedValue<Lamports>,
}

impl VerifiedQuoteInputs {
    /// Fail closed: every input must be fresh within `max_age_slots`.
    pub fn is_fresh(&self, max_age_slots: u64) -> bool {
        self.flash_loan_fee_sf.age_slots <= max_age_slots
            && self.base_decimals.age_slots <= max_age_slots
            && self.lamports_per_base_raw.age_slots <= max_age_slots
            && self.lamports_per_base_denom.age_slots <= max_age_slots
            && self.tip_lamports.age_slots <= max_age_slots
    }

    /// Build the authoritative economic config. `min_profit_raw` is the
    /// denomination-aware floor supplied by the strategy config.
    pub fn into_economic_config(self, min_profit_raw: TokenAmount) -> EconomicConfig {
        EconomicConfig {
            flash_loan_fee_sf: self.flash_loan_fee_sf.value,
            jito_tip_lamports: self.tip_lamports.value,
            base_decimals: self.base_decimals.value,
            lamports_to_base_mint_rate: self.lamports_per_base_raw.value,
            lamports_to_base_rate_denom: self.lamports_per_base_denom.value,
            min_profit_raw,
        }
    }
}

/// Derive the rational conversion rate `(numerator, denominator)` to convert
/// lamports into raw atomic base-mint units without loss of precision.
///
/// WSOL (9 decimals) -> (1, 1).
/// 18-decimal tokens -> (10^9, 1).
/// 6-decimal tokens (e.g. USDC) -> converts using live SOL price in USD:
/// `numerator = round(sol_price * 10^decimals)`, `denominator = 10^9`.
pub fn derive_rational_rate(decimals: u8, sol_price_usd: Option<f64>) -> Option<(u128, u128)> {
    if decimals > 18 {
        return None;
    }
    if decimals == 9 {
        return Some((1, 1));
    }
    if decimals > 9 {
        let factor = 10u128.checked_pow((decimals - 9) as u32)?;
        return Some((factor, 1));
    }
    // decimals < 9 (e.g. USDC with 6 decimals)
    let price = sol_price_usd?;
    if price <= 0.0 {
        return None;
    }
    let base_units_per_sol = (price * 10f64.powi(decimals as i32)).round();
    if base_units_per_sol <= 0.0 {
        return None;
    }
    Some((base_units_per_sol as u128, 1_000_000_000u128))
}

/// Derive `10^decimals / 10^9` as an integer ratio, or `None` when the ratio is
/// not integral (the lamport conversion would lose precision — fail closed).
/// Kept for backward compatibility with tests expecting integer-only ratio.
pub fn lamports_per_base_raw(decimals: u8) -> Option<u128> {
    if decimals > 18 {
        return None;
    }
    let base = 10u128.checked_pow(decimals as u32)?;
    if base % 1_000_000_000 == 0 {
        Some(base / 1_000_000_000)
    } else {
        None
    }
}

/// Decode the `decimals` field (offset 44) of a classic SPL Token mint account.
/// The SPL Mint layout is: COption<Pubkey> mint_authority (36), supply u64 (8),
/// then `decimals: u8` at offset 44 and `is_initialized: u8` at 45. Token-2022
/// mints keep the same base layout at the start. Fail closed: too-short,
/// uninitialized, or implausible decimals are errors.
pub fn decode_mint_decimals(account: &[u8]) -> Result<u8, String> {
    if account.len() < 82 {
        return Err(format!("mint account too short: {} bytes", account.len()));
    }
    let is_initialized = account[45];
    if is_initialized != 1 {
        return Err(format!(
            "mint account not initialized (flag {is_initialized})"
        ));
    }
    let decimals = account[44];
    if decimals > 18 {
        return Err(format!("implausible mint decimals: {decimals}"));
    }
    Ok(decimals)
}

/// Legacy API retained only to produce an explicit error for old diagnostic callers.
/// It never opens a JSON-RPC connection.
#[deprecated(note = "HTTP JSON-RPC reads are disabled; use build_verified_inputs_from_snapshots")]
pub async fn build_verified_inputs(
    _rpc_url: &str,
    _base_mint: &MintId,
    _reserve: &Pubkey,
    _expected_reserve_mint: &Pubkey,
    _tip_lamports: Lamports,
    _current_slot: Slot,
    _sol_price_usd: Option<f64>,
) -> Result<VerifiedQuoteInputs, String> {
    Err("HTTP JSON-RPC verified-input reads are disabled; Yellowstone snapshots are required".into())
}

/// Build authoritative economic inputs only from the current Yellowstone account
/// journal. No HTTP JSON-RPC, stale cache fallback, or guessed fee/decimal value is used.
pub fn build_verified_inputs_from_snapshots(
    store: &AccountSnapshotStore,
    base_mint: &MintId,
    reserve: &Pubkey,
    expected_reserve_mint: &Pubkey,
    tip_lamports: Lamports,
    current_slot: Slot,
    sol_price_usd: Option<f64>,
) -> Result<VerifiedQuoteInputs, String> {
    let reserve_snapshot = store.get(reserve)
        .ok_or_else(|| format!("missing Yellowstone Kamino reserve snapshot: {reserve}"))?;
    let klend_program = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID)
        .map_err(|e| format!("invalid Kamino program id: {e}"))?;
    if reserve_snapshot.owner != klend_program {
        return Err(format!("Kamino reserve owner mismatch: {}", reserve_snapshot.owner));
    }
    if reserve_snapshot.slot > current_slot.0 {
        return Err("Kamino reserve snapshot is newer than the quoting slot".into());
    }
    let fee_sf = arb_tx::kamino::decode_reserve_flash_fee_sf(
        &reserve_snapshot.data,
        expected_reserve_mint,
    ).map_err(|e| format!("Kamino reserve decode failed: {e}"))?;

    let mint_snapshot = store.get(&base_mint.0)
        .ok_or_else(|| format!("missing Yellowstone base mint snapshot: {base_mint}"))?;
    let token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)
        .map_err(|e| format!("invalid SPL Token program id: {e}"))?;
    let token_2022_program = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID)
        .map_err(|e| format!("invalid Token-2022 program id: {e}"))?;
    if mint_snapshot.owner != token_program && mint_snapshot.owner != token_2022_program {
        return Err(format!("base mint has unsupported owner: {}", mint_snapshot.owner));
    }
    if mint_snapshot.slot > current_slot.0 {
        return Err("base mint snapshot is newer than the quoting slot".into());
    }
    let decimals = decode_mint_decimals(&mint_snapshot.data)?;
    let (rate_num, rate_denom) = derive_rational_rate(decimals, sol_price_usd).ok_or_else(|| {
        format!("base mint {base_mint} has {decimals} decimals; cannot derive conversion rate (SOL price: {sol_price_usd:?})")
    })?;
    let observed = reserve_snapshot.slot.max(mint_snapshot.slot);

    Ok(VerifiedQuoteInputs {
        flash_loan_fee_sf: VerifiedValue::new(
            fee_sf,
            "yellowstone-kamino-reserve-flashLoanFeeSf",
            Slot(reserve_snapshot.slot),
            current_slot,
        ),
        base_decimals: VerifiedValue::new(
            decimals,
            "yellowstone-spl-mint-decimals",
            Slot(mint_snapshot.slot),
            current_slot,
        ),
        lamports_per_base_raw: VerifiedValue::new(
            rate_num,
            "lamports-to-base-rate-numerator",
            Slot(observed),
            current_slot,
        ),
        lamports_per_base_denom: VerifiedValue::new(
            rate_denom,
            "lamports-to-base-rate-denominator",
            Slot(observed),
            current_slot,
        ),
        tip_lamports: VerifiedValue::new(
            tip_lamports,
            "configured-jito-tip-ceiling",
            current_slot,
            current_slot,
        ),
    })
}

/// Well-known mints used by the WSOL-first execution path.
pub fn wsol_mint() -> Pubkey {
    Pubkey::from_str(arb_tx::WSOL_MINT).expect("valid WSOL mint constant")
}

pub fn wsol_reserve() -> Pubkey {
    Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE).expect("valid WSOL reserve constant")
}

pub fn usdc_mint() -> Pubkey {
    Pubkey::from_str(arb_constants::USDC_MINT).expect("valid USDC mint constant")
}

pub fn usdc_reserve() -> Pubkey {
    Pubkey::from_str("D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25")
        .expect("valid USDC reserve constant")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_inputs_use_only_yellowstone_reserve_and_mint_snapshots() {
        use std::sync::Arc;
        use arb_ingest::{AccountSnapshot, AccountSnapshotStore};

        let store = AccountSnapshotStore::default();
        let klend = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID).unwrap();
        let token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let mint = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
        let reserve = Pubkey::new_unique();

        let mut reserve_data = vec![0u8; arb_tx::kamino::KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET + 8];
        reserve_data[..8].copy_from_slice(&arb_tx::kamino::KAMINO_RESERVE_DISCRIMINATOR);
        let mint_start = arb_tx::kamino::KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET;
        reserve_data[mint_start..mint_start + 32].copy_from_slice(mint.as_ref());
        reserve_data[arb_tx::kamino::KAMINO_RESERVE_STATUS_OFFSET] = arb_tx::kamino::KAMINO_RESERVE_STATUS_ACTIVE;
        reserve_data[arb_tx::kamino::KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET..][..8]
            .copy_from_slice(&12_345u64.to_le_bytes());
        store.apply(AccountSnapshot {
            pubkey: reserve,
            owner: klend,
            slot: 99,
            write_version: 1,
            is_startup: false,
            provider_id: arb_ingest::ProviderId(1), bank_id: Some(1), generation: 0,
            data: Arc::new(reserve_data),
        });

        let mut mint_data = vec![0u8; 82];
        mint_data[44] = 9;
        mint_data[45] = 1;
        store.apply(AccountSnapshot {
            pubkey: mint,
            owner: token_program,
            slot: 100,
            write_version: 1,
            is_startup: false,
            provider_id: arb_ingest::ProviderId(1), bank_id: Some(1), generation: 0,
            data: Arc::new(mint_data),
        });

        let inputs = build_verified_inputs_from_snapshots(
            &store,
            &MintId(mint),
            &reserve,
            &mint,
            Lamports(50_000),
            Slot(100),
            None,
        ).unwrap();
        assert_eq!(inputs.flash_loan_fee_sf.value, 12_345);
        assert_eq!(inputs.base_decimals.value, 9);
        assert_eq!(inputs.lamports_per_base_raw.value, 1);
        assert_eq!(inputs.lamports_per_base_denom.value, 1);
        assert_eq!(inputs.tip_lamports.value, Lamports(50_000));
    }

    #[test]
    fn rate_wsol_is_one() {
        assert_eq!(lamports_per_base_raw(9), Some(1));
        assert_eq!(derive_rational_rate(9, None), Some((1, 1)));
        assert_eq!(derive_rational_rate(9, Some(180.0)), Some((1, 1)));
    }

    #[test]
    fn rate_usdc_is_rejected_integral_only() {
        // 6 decimals → 10^6/10^9 = 0.001, not integral → fail closed.
        assert_eq!(lamports_per_base_raw(6), None);
        // Rational rate without price also fails closed
        assert_eq!(derive_rational_rate(6, None), None);
        assert_eq!(derive_rational_rate(6, Some(0.0)), None);
    }

    #[test]
    fn rate_usdc_rational_with_price() {
        // 6 decimals at SOL price $185.50
        assert_eq!(
            derive_rational_rate(6, Some(185.50)),
            Some((185_500_000, 1_000_000_000))
        );
    }

    #[test]
    fn rate_eighteen_is_integral() {
        assert_eq!(lamports_per_base_raw(18), Some(1_000_000_000));
        assert_eq!(derive_rational_rate(18, None), Some((1_000_000_000, 1)));
    }

    #[test]
    fn decode_mint_decimals_wsol_fixture() {
        // Minimal classic SPL Mint: authority COption (36) + supply (8) +
        // decimals at 44 + initialized at 45 + freeze authority (36) = 82.
        let mut acc = vec![0u8; 82];
        acc[44] = 9;
        acc[45] = 1;
        assert_eq!(decode_mint_decimals(&acc).unwrap(), 9);
    }

    #[test]
    fn decode_mint_decimals_rejects_short_and_uninitialized() {
        assert!(decode_mint_decimals(&[0u8; 40]).is_err());
        let mut acc = vec![0u8; 82];
        acc[44] = 6;
        acc[45] = 0; // not initialized
        assert!(decode_mint_decimals(&acc).is_err());
    }
}
