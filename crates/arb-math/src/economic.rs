//! Authoritative economic calculation model — single source of truth.
//!
//! Consumed by the opportunity engine, route optimizer, transaction builder, and
//! execution validator. No other crate may compute net profit independently.
//!
//! All monetary values are strongly typed. Token amounts are in the base mint's
//! raw atomic units. SOL-denominated costs (network, priority fee, Jito tip) are
//! in `Lamports` and are converted to base-mint units via an explicit rate before
//! comparison. No floating-point arithmetic is used for the final executable
//! amount or the profit guard.

use arb_types::{ArbError, Lamports, MintId, Result, Slot, TokenAmount};
use ruint::aliases::U256;

/// Version of the economic calculation model. Bump when the formula changes.
pub const ECONOMIC_CALCULATION_VERSION: u32 = 2;

/// Lamports in one SOL (WSOL has 9 decimals; 1 raw WSOL unit == 1 lamport).
pub const LAMPORTS_PER_SOL: u128 = 1_000_000_000;

/// Unique identifier for a cost ledger entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CostId(pub u64);

/// Scope of a cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CostScope {
    PerHop,
    PerLoan,
    PerTransaction,
    PerBundle,
    PerAccount,
}

/// Type of cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CostType {
    AmmSwapFee,
    TransferFee,
    ProtocolFee,
    DynamicFee,
    FlashLoanPremium,
    ProviderCharge,
    BaseSignatureFee,
    PriorityFee,
    OtherNetworkCharge,
    JitoTip,
    AtaCreation,
    AccountInitialization,
    RentDeposit,
    OtherVerifiedCost,
}

/// Unit of a cost amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CostUnit {
    Lamports,
    TokenRaw,
}

/// A single cost ledger entry. Every financial input records value, unit,
/// source, timestamp/slot, applicable protocol, calculation method, and whether
/// it is already included in or deducted from the quote — preventing double
/// counting and double deduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostEntry {
    pub id: CostId,
    pub cost_type: CostType,
    pub scope: CostScope,
    /// Amount in the cost's native units.
    pub amount_raw: u128,
    pub unit: CostUnit,
    /// Source of the value (e.g. "kamino-reserve-flashLoanFeeSf").
    pub source: &'static str,
    /// Applicable protocol (e.g. "kamino", "raydium-v4", "solana-runtime").
    pub protocol: &'static str,
    /// Slot at which the value was observed.
    pub observed_slot: Slot,
    /// Calculation method used to derive the amount.
    pub calculation_method: &'static str,
    /// True when this amount is already reflected in an upstream quote.
    pub included_in_quote: bool,
    /// True when this amount has already been deducted (prevents double deduction).
    pub deducted_already: bool,
}

/// Cost ledger with double-count and double-deduction prevention.
#[derive(Debug, Clone, Default)]
pub struct CostLedger {
    entries: Vec<CostEntry>,
    next_id: CostId,
}

impl CostLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a cost entry and return its id.
    pub fn add(&mut self, entry: CostEntry) -> CostId {
        let id = self.next_id;
        self.next_id.0 = self.next_id.0.saturating_add(1);
        let mut e = entry;
        e.id = id;
        self.entries.push(e);
        id
    }

    pub fn entries(&self) -> &[CostEntry] {
        &self.entries
    }

    /// Total of entries included in the quote but not yet deducted, in raw units.
    /// Used to reconcile the quote against the ledger.
    pub fn total_included_not_deducted(&self, unit: CostUnit) -> Result<u128> {
        self.entries
            .iter()
            .filter(|e| e.included_in_quote && !e.deducted_already && e.unit == unit)
            .try_fold(0u128, |sum, e| {
                sum.checked_add(e.amount_raw).ok_or(ArbError::Overflow)
            })
    }
}

/// Configuration for economic evaluation. Every field must be supplied by a
/// verified source; nothing here is hardcoded in the authoritative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicConfig {
    /// Kamino flash-loan fee as the raw Q64.64 value read from the verified
    /// reserve account (`ReserveConfig.fees.flash_loan_fee_sf`). The fee in raw
    /// units is `ceil(borrow * sf / 2^64)` — never assumed to be 0 or 5 bps.
    /// A bps integer cannot represent the on-chain value (e.g. the main-market
    /// WSOL/USDC reserves carry `sf = 11_529_215_046_068` ≈ 6.25e-7 ≈ 0.006 bps).
    pub flash_loan_fee_sf: u64,
    /// Jito tip in lamports, derived from current tip-floor market data and an
    /// explicit strategy policy — never a fabricated fraction of profit.
    pub jito_tip_lamports: Lamports,
    /// Base mint decimals, read from a verified mint account. Never hardcoded to 9.
    pub base_decimals: u8,
    /// Raw base-mint units per lamport (1 when the base mint is WSOL).
    /// Used to convert SOL-denominated network costs into base-mint units using a
    /// fresh, verified conversion rate.
    pub lamports_to_base_mint_rate: u128,
    /// Denominator for rational rate conversion (1 for integer ratios like WSOL).
    /// Effective conversion multiplier = lamports_to_base_mint_rate / lamports_to_base_rate_denom.
    pub lamports_to_base_rate_denom: u128,
    /// Minimum acceptable profit in base-mint raw units (NOT lamports), configurable.
    pub min_profit_raw: TokenAmount,
}

/// The authoritative economic quote for one opportunity. All token amounts are in
/// the base mint raw atomic units. Execution eligibility uses conservative min-out PnL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicQuote {
    pub base_mint: MintId,
    pub base_decimals: u8,
    pub borrow_amount_raw: TokenAmount,
    pub expected_final_amount_raw: TokenAmount,
    pub minimum_final_amount_raw: TokenAmount,
    pub flash_loan_fee_raw: TokenAmount,
    pub dex_fees_raw: TokenAmount,
    pub transfer_fees_raw: TokenAmount,
    pub account_creation_cost_raw: TokenAmount,
    pub other_verified_costs_raw: TokenAmount,
    pub network_fee_lamports: Lamports,
    pub priority_fee_lamports: Lamports,
    pub jito_tip_lamports: Lamports,
    pub lamports_to_base_mint_rate: u128,
    pub lamports_to_base_rate_denom: u128,
    /// Non-negative compatibility projection of expected net PnL. Use the signed fields for decisions.
    pub net_profit_raw: TokenAmount,
    /// Signed expected PnL in base-mint raw units; negative values represent losses.
    pub expected_net_profit_signed_raw: i128,
    /// Signed conservative PnL using minimum_final_amount_raw, not expected output.
    pub minimum_net_profit_signed_raw: i128,
    /// Gross profit (final received - borrowed principal) in base-mint raw units.
    pub gross_profit_raw: TokenAmount,
    /// Signed gross profit (final received - borrowed principal); negative values represent gross loss.
    pub gross_profit_signed_raw: i128,
    /// Minimum acceptable profit in base-mint raw units.
    pub min_profit_raw: TokenAmount,
    /// True when conservative net PnL clears `min_profit_raw`.
    pub meets_min_profit: bool,
    pub source_slot: Slot,
    pub quote_timestamp_ms: u64,
    pub calculation_version: u32,
}

impl EconomicQuote {
    /// True when the opportunity clears the minimum profit floor.
    pub fn is_profitable(&self) -> bool {
        self.meets_min_profit && self.minimum_net_profit_signed_raw > 0
    }
}

/// Convert a SOL-denominated lamport cost into base-mint raw units using an
/// explicit, verified rational rate `rate_num / rate_denom`. Checked arithmetic throughout —
/// overflow rejects the quote rather than silently saturating.
fn lamports_to_base_raw(lamports: Lamports, rate_num: u128, rate_denom: u128) -> Result<u128> {
    if rate_num == 0 || rate_denom == 0 {
        return Err(ArbError::InvalidInput(
            "zero conversion rate component".into(),
        ));
    }
    let numerator = U256::from(lamports.as_u64())
        .checked_mul(U256::from(rate_num))
        .ok_or(ArbError::Overflow)?;
    let denominator = U256::from(rate_denom);
    let (quotient, remainder) = numerator.div_rem(denominator);
    let rounded = if remainder.is_zero() {
        quotient
    } else {
        quotient
            .checked_add(U256::from(1))
            .ok_or(ArbError::Overflow)?
    };
    u128::try_from(rounded).map_err(|_| ArbError::Overflow)
}

fn signed_difference(lhs: U256, rhs: U256) -> Result<i128> {
    if lhs >= rhs {
        let magnitude = i128::try_from(lhs - rhs).map_err(|_| ArbError::Overflow)?;
        Ok(magnitude)
    } else {
        let magnitude = i128::try_from(rhs - lhs).map_err(|_| ArbError::Overflow)?;
        magnitude.checked_neg().ok_or(ArbError::Overflow)
    }
}

fn positive_projection(value: i128) -> Result<TokenAmount> {
    if value <= 0 {
        return Ok(TokenAmount(0));
    }
    Ok(TokenAmount(
        u64::try_from(value).map_err(|_| ArbError::Overflow)?,
    ))
}

/// Convert base-mint raw units into Lamports using the verified rational rate `rate_num / rate_denom`.
///
/// Inverts `base_raw = (lamports * rate_num) / rate_denom`:
/// `lamports = (base_raw * rate_denom) / rate_num`.
/// Checked arithmetic throughout — overflow rejects rather than silently saturating (FIN-06).
pub fn base_raw_to_lamports(
    base_raw: TokenAmount,
    rate_num: u128,
    rate_denom: u128,
) -> Result<Lamports> {
    if rate_num == 0 {
        return Err(ArbError::InvalidInput("zero conversion rate".into()));
    }
    let value = U256::from(base_raw.as_u64());
    if rate_denom == 0 {
        return Err(ArbError::InvalidInput("zero conversion denominator".into()));
    }
    let den = U256::from(rate_denom);
    let num = U256::from(rate_num);
    let converted = value
        .checked_mul(den)
        .ok_or(ArbError::Overflow)?
        .checked_div(num)
        .ok_or(ArbError::Overflow)?;
    let val_u64 = u64::try_from(converted).map_err(|_| ArbError::Overflow)?;
    Ok(Lamports(val_u64))
}

/// Build an authoritative EconomicQuote from component values.
///
/// All token amounts are in the base mint's raw atomic units. SOL-denominated
/// costs are converted using `lamports_to_base_mint_rate / lamports_to_base_rate_denom`.
///
/// Returns `Err(ArbError::Overflow)` on any overflow; never saturates silently.
#[allow(clippy::too_many_arguments)]
pub fn build_economic_quote(
    base_mint: MintId,
    base_decimals: u8,
    borrow_amount_raw: TokenAmount,
    expected_final_amount_raw: TokenAmount,
    minimum_final_amount_raw: TokenAmount,
    flash_loan_fee_raw: TokenAmount,
    dex_fees_raw: TokenAmount,
    transfer_fees_raw: TokenAmount,
    account_creation_cost_raw: TokenAmount,
    other_verified_costs_raw: TokenAmount,
    network_fee_lamports: Lamports,
    priority_fee_lamports: Lamports,
    jito_tip_lamports: Lamports,
    lamports_to_base_mint_rate: u128,
    lamports_to_base_rate_denom: u128,
    min_profit_raw: TokenAmount,
    source_slot: Slot,
    quote_timestamp_ms: u64,
) -> Result<EconomicQuote> {
    if base_decimals > 18 {
        return Err(ArbError::InvalidInput(
            "base mint decimals exceed supported range".into(),
        ));
    }
    if minimum_final_amount_raw.as_u64() > expected_final_amount_raw.as_u64() {
        return Err(ArbError::InvalidInput(
            "minimum output exceeds expected output".into(),
        ));
    }
    let network_raw = lamports_to_base_raw(
        network_fee_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;
    let priority_raw = lamports_to_base_raw(
        priority_fee_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;
    let tip_raw = lamports_to_base_raw(
        jito_tip_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;

    let borrow = U256::from(borrow_amount_raw.as_u64());
    let expected = U256::from(expected_final_amount_raw.as_u64());
    let flash_fee = U256::from(flash_loan_fee_raw.as_u64());
    let dex_fees = U256::from(dex_fees_raw.as_u64());
    let transfer_fees = U256::from(transfer_fees_raw.as_u64());
    let acct_cost = U256::from(account_creation_cost_raw.as_u64());
    let other_costs = U256::from(other_verified_costs_raw.as_u64());
    // Preserve losses as signed values; never turn an unprofitable route into Overflow.
    let gross_signed = signed_difference(expected, borrow)?;
    let min_out = U256::from(minimum_final_amount_raw.as_u64());
    let gross_min_signed = signed_difference(min_out, borrow)?;

    // Total costs, each counted exactly once.
    let total_costs = flash_fee
        .checked_add(dex_fees)
        .and_then(|v| v.checked_add(transfer_fees))
        .and_then(|v| v.checked_add(U256::from(acct_cost)))
        .and_then(|v| v.checked_add(U256::from(other_costs)))
        .and_then(|v| v.checked_add(U256::from(network_raw)))
        .and_then(|v| v.checked_add(U256::from(priority_raw)))
        .and_then(|v| v.checked_add(U256::from(tip_raw)))
        .ok_or(ArbError::Overflow)?;

    let expected_net_signed = if gross_signed >= 0 {
        signed_difference(U256::from(gross_signed as u128), total_costs)?
    } else {
        let loss = U256::from(gross_signed.unsigned_abs());
        let total_loss = loss.checked_add(total_costs).ok_or(ArbError::Overflow)?;
        -i128::try_from(total_loss).map_err(|_| ArbError::Overflow)?
    };
    let minimum_net_signed = if gross_min_signed >= 0 {
        signed_difference(U256::from(gross_min_signed as u128), total_costs)?
    } else {
        let loss = U256::from(gross_min_signed.unsigned_abs());
        let total_loss = loss.checked_add(total_costs).ok_or(ArbError::Overflow)?;
        -i128::try_from(total_loss).map_err(|_| ArbError::Overflow)?
    };
    let net_profit_raw = positive_projection(expected_net_signed)?;
    let gross_profit_raw = positive_projection(gross_signed)?;
    let meets_min_profit = minimum_net_signed
        >= i128::from(min_profit_raw.as_u64());

    Ok(EconomicQuote {
        base_mint,
        base_decimals,
        borrow_amount_raw,
        expected_final_amount_raw,
        minimum_final_amount_raw,
        flash_loan_fee_raw,
        dex_fees_raw,
        transfer_fees_raw,
        account_creation_cost_raw,
        other_verified_costs_raw,
        network_fee_lamports,
        priority_fee_lamports,
        jito_tip_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
        net_profit_raw,
        expected_net_profit_signed_raw: expected_net_signed,
        minimum_net_profit_signed_raw: minimum_net_signed,
        gross_profit_raw,
        gross_profit_signed_raw: gross_signed,
        min_profit_raw,
        meets_min_profit,
        source_slot,
        quote_timestamp_ms,
        calculation_version: ECONOMIC_CALCULATION_VERSION,
    })
}

/// Calculate net profit in base-mint raw units with checked arithmetic.
///
/// This is the authoritative single calculation. All inputs are in base-mint
/// raw units except SOL-denominated costs which are converted via `rate`.
///
/// Conceptually:
///   Net = FinalReceived - Principal - FlashFee - DexFees - TransferFees
///         - AccountCreation - OtherCosts - Network - Priority - JitoTip
#[allow(clippy::too_many_arguments)]
pub fn calculate_net_profit_raw(
    final_received_raw: u64,
    borrow_principal_raw: u64,
    flash_fee_raw: u64,
    dex_fees_raw: u64,
    transfer_fees_raw: u64,
    account_creation_cost_raw: u64,
    other_verified_costs_raw: u64,
    network_fee_lamports: Lamports,
    priority_fee_lamports: Lamports,
    jito_tip_lamports: Lamports,
    lamports_to_base_mint_rate: u128,
    lamports_to_base_rate_denom: u128,
) -> Result<u64> {
    let network_raw = lamports_to_base_raw(
        network_fee_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;
    let priority_raw = lamports_to_base_raw(
        priority_fee_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;
    let tip_raw = lamports_to_base_raw(
        jito_tip_lamports,
        lamports_to_base_mint_rate,
        lamports_to_base_rate_denom,
    )?;

    let final_received = U256::from(final_received_raw);
    let borrow = U256::from(borrow_principal_raw);
    let flash_fee = U256::from(flash_fee_raw);
    let dex_fees = U256::from(dex_fees_raw);
    let transfer_fees = U256::from(transfer_fees_raw);
    let acct_cost = U256::from(account_creation_cost_raw);
    let other_costs = U256::from(other_verified_costs_raw);

    let gross = final_received
        .checked_sub(borrow)
        .ok_or(ArbError::Overflow)?;

    let total_costs = flash_fee
        .checked_add(dex_fees)
        .and_then(|v| v.checked_add(transfer_fees))
        .and_then(|v| v.checked_add(U256::from(acct_cost)))
        .and_then(|v| v.checked_add(U256::from(other_costs)))
        .and_then(|v| v.checked_add(U256::from(network_raw)))
        .and_then(|v| v.checked_add(U256::from(priority_raw)))
        .and_then(|v| v.checked_add(U256::from(tip_raw)))
        .ok_or(ArbError::Overflow)?;

    let net = gross.checked_sub(total_costs).ok_or(ArbError::Overflow)?;
    u64::try_from(net).map_err(|_| ArbError::Overflow)
}

/// Calculate the Kamino flash loan fee in base-mint raw units from the raw
/// Q64.64 on-chain value (`ReserveConfig.fees.flash_loan_fee_sf`).
///
/// Fee = ceil(borrow * sf / 2^64) — the protocol's fixed-point multiplication,
/// with ceiling division so repayment is never understated.
pub fn calculate_flash_fee_raw(borrow_amount: u64, fee_sf: u64) -> Result<u64> {
    if borrow_amount == 0 {
        return Ok(0);
    }
    // A full-scale sf (>= 2^64) would make the fee >= the loan; u64 cannot hold
    // 2^64, so any u64 sf yields a fee strictly below the loan (repayable).
    let num = U256::from(borrow_amount) * U256::from(fee_sf);
    let scale = U256::from(1u128) << 64;
    let (quot, rem) = num.div_rem(scale);
    // Ceiling: if there is a remainder, add 1 so repayment is never understated.
    let fee = if rem.is_zero() {
        quot
    } else {
        quot.checked_add(U256::from(1)).ok_or(ArbError::Overflow)?
    };
    u64::try_from(fee).map_err(|_| ArbError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::MintId;
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    fn wsol_mint() -> MintId {
        MintId(Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap())
    }

    #[test]
    fn net_profit_basic_positive() {
        let q = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(1_000_000_000), // borrow 1 SOL
            TokenAmount(1_005_000_000), // expect 1.005 SOL back
            TokenAmount(1_000_020_000), // conservative output clears the profit floor
            TokenAmount(0), // flash fee 0
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,                   // 1:1 WSOL rate numerator
            1,                   // 1 WSOL rate denominator
            TokenAmount(10_000), // min profit 10_000 raw
            Slot(100),
            1_000_000_000,
        )
        .unwrap();
        assert!(q.is_profitable());
        // net = 1_005_000_000 - 1_000_000_000 - 0 = 5_000_000
        assert_eq!(q.net_profit_raw.as_u64(), 5_000_000);
        assert_eq!(q.gross_profit_raw.as_u64(), 5_000_000);
        assert_eq!(q.minimum_net_profit_signed_raw, 20_000);
    }

    #[test]
    fn flash_fee_uses_q64_64_sf_exactly() {
        // sf = 5% expressed in Q64.64, derived with INTEGER math so the test
        // value is exact (a float 0.05*2^64 rounds up to ...632).
        let sf_5pct: u64 = ((1u128 << 64) / 20) as u64; // 922_337_203_685_477_580
        let fee = calculate_flash_fee_raw(1_000_000_000, sf_5pct).unwrap();
        assert_eq!(fee, 50_000_000, "5% of 1 SOL must be 50M raw units");

        // sf = 0 must be explicit too.
        assert_eq!(calculate_flash_fee_raw(1_000_000_000, 0).unwrap(), 0);

        // The actual main-market WSOL/USDC reserve value (read from mainnet
        // `ReserveConfig.fees.flash_loan_fee_sf`): 11_529_215_046_068 ≈ 6.25e-7.
        // 1 SOL borrowed -> ceil(1e9 * 6.25e-7) = 625 lamports. A bps integer
        // cannot represent this fee; only the Q64.64 path is exact.
        let mainnet_sf: u64 = 11_529_215_046_068;
        let fee = calculate_flash_fee_raw(1_000_000_000, mainnet_sf).unwrap();
        assert_eq!(fee, 625);
    }

    #[test]
    fn net_profit_below_floor() {
        let q = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(1_000_000_000),
            TokenAmount(1_000_001_000), // only 1_000 profit
            TokenAmount(1_000_000_000),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
            TokenAmount(10_000), // min profit 10_000 raw
            Slot(100),
            1_000_000_000,
        )
        .unwrap();
        assert!(!q.is_profitable());
        assert!(!q.meets_min_profit);
    }

    #[test]
    fn no_double_counting_of_costs() {
        // Same cost supplied both as a token fee and as a lamport cost is counted
        // once each only if the caller intentionally includes both; the ledger
        // tracks included/deducted flags. Here we verify each cost is summed once.
        let q = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(10_000_000_000), // borrow 10 SOL
            TokenAmount(10_100_000_000), // 100_000_000 gross
            TokenAmount(10_000_651_000), // min-out covers costs and min-profit floor
            TokenAmount(500_000), // flash fee 5 bps
            TokenAmount(100_000), // dex fee
            TokenAmount(50_000),  // transfer fee
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
            TokenAmount(1_000),
            Slot(100),
            1_000_000_000,
        )
        .unwrap();
        // net = 100_000_000 - 500_000 - 100_000 - 50_000 = 99_350_000
        assert_eq!(q.net_profit_raw.as_u64(), 99_350_000);
        assert_eq!(q.minimum_net_profit_signed_raw, 1_000);
        assert!(q.is_profitable());
    }

    #[test]
    fn test_economic_quote_usdc_rational_rate() {
        // USDC (6 decimals). At SOL price $180:
        // rate_num = 180_000_000 micro-USDC per SOL
        // rate_denom = 1_000_000_000 lamports per SOL
        let usdc_mint = MintId(Pubkey::new_unique());
        let tip_lamports = Lamports(10_000); // 10,000 lamports = 0.00001 SOL = $0.0018 = 1,800 micro-USDC
        let q = build_economic_quote(
            usdc_mint,
            6,
            TokenAmount(100_000_000), // borrow 100 USDC
            TokenAmount(105_000_000), // expect 105 USDC back (gross = 5_000_000)
            TokenAmount(100_018_950), // costs plus the 10,000-raw profit floor
            TokenAmount(6_250), // flash fee 0.00625 USDC
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(5_000), // base network fee 5,000 lamports = 900 micro-USDC
            Lamports(0),
            tip_lamports,        // tip 10,000 lamports = 1,800 micro-USDC
            180_000_000,         // rate numerator (SOL price 180 USD)
            1_000_000_000,       // rate denominator (10^9 lamports)
            TokenAmount(10_000), // min profit 0.01 USDC (10_000 lamports * rate = 1,800 micro-USDC)
            Slot(100),
            1_000_000_000,
        )
        .unwrap();
        assert!(q.is_profitable());
        // gross = 5_000_000 micro-USDC ($5)
        // total costs = 6_250 + 900 + 1,800 = 8,950 micro-USDC ($0.00895)
        // net = 5_000_000 - 8,950 = 4_991_050 micro-USDC
        assert_eq!(q.gross_profit_raw.as_u64(), 5_000_000);
        assert_eq!(q.net_profit_raw.as_u64(), 4_991_050);
        assert_eq!(q.minimum_net_profit_signed_raw, 10_000);
        assert!(q.meets_min_profit);
    }

    #[test]
    fn extreme_loss_is_signed_without_saturation() {
        let result = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(u64::MAX),
            TokenAmount(u64::MAX), // expected == borrow => gross 0
            TokenAmount(u64::MAX),
            TokenAmount(u64::MAX), // fee overflows total costs
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
            TokenAmount(0),
            Slot(100),
            1_000_000_000,
        );
        let quote = result.expect("u64-sized loss is representable as signed PnL");
        assert_eq!(
            quote.expected_net_profit_signed_raw,
            -i128::from(u64::MAX)
        );
        assert!(!quote.is_profitable());
    }

    #[test]
    fn cost_ledger_tracks_double_count() {
        let mut ledger = CostLedger::new();
        let id1 = ledger.add(CostEntry {
            id: CostId(0),
            cost_type: CostType::FlashLoanPremium,
            scope: CostScope::PerLoan,
            amount_raw: 500_000,
            unit: CostUnit::TokenRaw,
            source: "kamino-reserve-flashLoanFeeSf",
            protocol: "kamino",
            observed_slot: Slot(100),
            calculation_method: "borrow * fee_bps / 10_000 (ceil)",
            included_in_quote: true,
            deducted_already: false,
        });
        let id2 = ledger.add(CostEntry {
            id: CostId(0),
            cost_type: CostType::FlashLoanPremium,
            scope: CostScope::PerLoan,
            amount_raw: 500_000,
            unit: CostUnit::TokenRaw,
            source: "kamino-reserve-flashLoanFeeSf",
            protocol: "kamino",
            observed_slot: Slot(100),
            calculation_method: "borrow * fee_bps / 10_000 (ceil)",
            included_in_quote: true,
            deducted_already: true, // already deducted elsewhere
        });
        assert_ne!(id1, id2);
        // Only id1 counts (not deducted); id2 is deducted_already.
        assert_eq!(
            ledger
                .total_included_not_deducted(CostUnit::TokenRaw)
                .unwrap(),
            500_000
        );
        assert_eq!(ledger.entries().len(), 2);
    }

    #[test]
    fn lamports_conversion_wsol_is_identity() {
        assert_eq!(
            lamports_to_base_raw(Lamports(1_000_000_000), 1, 1).unwrap(),
            1_000_000_000
        );
    }

    #[test]
    fn calculate_net_profit_raw_matches_quote() {
        let net = calculate_net_profit_raw(
            1_005_000_000, // final received
            1_000_000_000, // principal
            0,
            0,
            0,
            0,
            0,
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
        )
        .unwrap();
        assert_eq!(net, 5_000_000);
    }

    #[test]
    fn flash_fee_sf_full_scale_ceils_to_the_loan() {
        // sf = u64::MAX ≈ 1 - 2^-64: ceil(loan * sf / 2^64) rounds up to the
        // loan itself — the ceiling never understates repayment.
        let fee = calculate_flash_fee_raw(1_000_000_000, u64::MAX).unwrap();
        assert_eq!(fee, 1_000_000_000);
    }

    #[test]
    fn test_base_raw_to_lamports_identity_and_scaling() {
        // WSOL: 1:1 identity
        let lamports = base_raw_to_lamports(TokenAmount(1_000_000_000), 1, 1).unwrap();
        assert_eq!(lamports, Lamports(1_000_000_000));

        // USDC: e.g. rate 1/5 => base_raw * 5
        let lamports_usdc = base_raw_to_lamports(TokenAmount(1_000_000), 1, 5).unwrap();
        assert_eq!(lamports_usdc, Lamports(5_000_000));

        // Zero rate fails closed
        assert!(base_raw_to_lamports(TokenAmount(1_000_000), 0, 1).is_err());
    }
    #[test]
    fn losing_quote_is_signed_and_rejected_not_overflowed() {
        let q = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(1_000_000),
            TokenAmount(999_000),
            TokenAmount(998_000),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
            TokenAmount(1),
            Slot(1),
            1,
        )
        .unwrap();
        assert_eq!(q.expected_net_profit_signed_raw, -1_000);
        assert_eq!(q.minimum_net_profit_signed_raw, -2_000);
        assert!(!q.is_profitable());
        assert_eq!(q.net_profit_raw.as_u64(), 0);
    }

    #[test]
    fn min_profit_is_already_in_base_mint_units() {
        let q = build_economic_quote(
            wsol_mint(),
            9,
            TokenAmount(1_000_000),
            TokenAmount(1_010_000),
            TokenAmount(1_009_000),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            180,
            1_000_000_000,
            TokenAmount(9_500),
            Slot(1),
            1,
        )
        .unwrap();
        assert_eq!(q.min_profit_raw.as_u64(), 9_500);
        assert_eq!(q.minimum_net_profit_signed_raw, 9_000);
        assert!(!q.is_profitable());
    }

    #[test]
    fn conversion_rejects_zero_denominator_and_rounds_liabilities_up() {
        assert!(lamports_to_base_raw(Lamports(1), 1, 0).is_err());
        assert_eq!(lamports_to_base_raw(Lamports(1), 1, 3).unwrap(), 1);
    }
}
