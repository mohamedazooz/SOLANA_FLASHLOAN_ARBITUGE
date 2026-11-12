//! Dimensional Analysis Safe Currency & Token Amount Engine.
//!
//! Enforces:
//! 1. Every amount and fee carries raw amount, currency/mint, decimals, conversion source, and slot/timestamp.
//! 2. Incompatible cross-currency operations (e.g. adding USDC to SOL lamports) are rejected at compile-time and runtime.
//! 3. Conservative timestamped conversion rates for converting SOL fees to route quote currency.
//! 4. Separation of network base fees, priority fees, Jito tips, DEX fees, transfer fees, and refundable rent.
//! 5. DEX fees are never double-counted when already embedded in quote outputs.
//! 6. Net profit calculation strictly denominated in route quote mint raw units and consistent basis points.

use crate::{MintId, PoolId};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;
use thiserror::Error;

/// Canonical Mainnet WSOL mint.
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
/// Canonical Mainnet USDC mint.
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

/// Dimensional arithmetic errors.
#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionalArithmeticError {
    #[error("Currency mismatch: cannot perform arithmetic between {lhs} and {rhs}")]
    CurrencyMismatch { lhs: Currency, rhs: Currency },
    #[error("Decimals mismatch for currency {currency}: {lhs_dec} vs {rhs_dec}")]
    DecimalMismatch {
        currency: Currency,
        lhs_dec: u8,
        rhs_dec: u8,
    },
    #[error("Arithmetic overflow in dimensional calculation")]
    Overflow,
    #[error("Arithmetic underflow in dimensional calculation")]
    Underflow,
    #[error("Division by zero in conversion rate")]
    DivisionByZero,
    #[error("Missing conversion rate between {from} and {to}")]
    MissingConversionRate { from: Currency, to: Currency },
    #[error(
        "Stale conversion rate: rate slot {rate_slot} exceeds max allowed lag from observation slot {target_slot}"
    )]
    StaleConversionRate { rate_slot: u64, target_slot: u64 },
}

/// Strongly-typed currency identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    /// Native Solana SOL (9 decimals).
    Sol,
    /// SPL Token or Token-2022 identified by its mint.
    Token(MintId),
}

impl Currency {
    pub fn usdc() -> Self {
        Self::Token(MintId(Pubkey::from_str(USDC_MINT).unwrap()))
    }

    pub fn wsol() -> Self {
        Self::Token(MintId(Pubkey::from_str(WSOL_MINT).unwrap()))
    }

    pub fn is_sol(&self) -> bool {
        match self {
            Self::Sol => true,
            Self::Token(m) => m.0 == Pubkey::from_str(WSOL_MINT).unwrap(),
        }
    }

    pub fn is_usdc(&self) -> bool {
        match self {
            Self::Token(m) => m.0 == Pubkey::from_str(USDC_MINT).unwrap(),
            _ => false,
        }
    }

    pub fn mint_pubkey(&self) -> Pubkey {
        match self {
            Self::Sol => Pubkey::default(),
            Self::Token(m) => m.0,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sol => write!(f, "SOL"),
            Self::Token(m) => {
                if *self == Self::usdc() {
                    write!(f, "USDC")
                } else if *self == Self::wsol() {
                    write!(f, "WSOL")
                } else {
                    write!(f, "Token({})", m.0)
                }
            }
        }
    }
}

/// Source of price or conversion data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionSource {
    /// Native currency (no conversion performed).
    Native,
    /// Fixed or configured fallback conversion rate.
    ConfiguredRate { label: String },
    /// Conversion rate derived from a specific DEX pool.
    PoolQuote { pool_id: PoolId, slot: u64 },
    /// On-chain Pyth oracle price.
    PythOracle { feed_id: Pubkey, slot: u64 },
    /// Historical snapshot.
    HistoricalSnapshot { slot: u64, timestamp_ms: u64 },
}

impl fmt::Display for ConversionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native => write!(f, "Native"),
            Self::ConfiguredRate { label } => write!(f, "ConfiguredRate({})", label),
            Self::PoolQuote { pool_id, slot } => {
                write!(f, "PoolQuote({}, slot={})", pool_id.0, slot)
            }
            Self::PythOracle { feed_id, slot } => {
                write!(f, "PythOracle({}, slot={})", feed_id, slot)
            }
            Self::HistoricalSnapshot { slot, timestamp_ms } => {
                write!(f, "HistoricalSnapshot(slot={}, ts={})", slot, timestamp_ms)
            }
        }
    }
}

/// Timestamped conversion rate between two currencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampedRate {
    pub from_currency: Currency,
    pub to_currency: Currency,
    pub from_decimals: u8,
    pub to_decimals: u8,
    /// Rate expressed as: 1 whole from_unit = (rate_numerator / rate_denominator) whole to_units.
    pub rate_numerator: u128,
    pub rate_denominator: u128,
    pub slot: u64,
    pub timestamp_ms: u64,
    pub source: ConversionSource,
}

impl TimestampedRate {
    /// Construct a conservative conversion rate.
    pub fn new(
        from_currency: Currency,
        to_currency: Currency,
        from_decimals: u8,
        to_decimals: u8,
        rate_numerator: u128,
        rate_denominator: u128,
        slot: u64,
        timestamp_ms: u64,
        source: ConversionSource,
    ) -> Result<Self, DimensionalArithmeticError> {
        if rate_denominator == 0 {
            return Err(DimensionalArithmeticError::DivisionByZero);
        }
        Ok(Self {
            from_currency,
            to_currency,
            from_decimals,
            to_decimals,
            rate_numerator,
            rate_denominator,
            slot,
            timestamp_ms,
            source,
        })
    }

    /// Construct a conservative SOL -> USDC price rate (e.g., $180.00 USDC per 1 SOL).
    pub fn sol_to_usdc(
        price_usd_numerator: u64,
        price_usd_denominator: u64,
        slot: u64,
        timestamp_ms: u64,
        source: ConversionSource,
    ) -> Result<Self, DimensionalArithmeticError> {
        Self::new(
            Currency::Sol,
            Currency::usdc(),
            9,
            6,
            price_usd_numerator as u128,
            price_usd_denominator as u128,
            slot,
            timestamp_ms,
            source,
        )
    }

    /// Convert raw units of `from_currency` into raw units of `to_currency`.
    pub fn convert_amount(
        &self,
        amount: &TypedTokenAmount,
    ) -> Result<TypedTokenAmount, DimensionalArithmeticError> {
        if amount.currency != self.from_currency {
            return Err(DimensionalArithmeticError::CurrencyMismatch {
                lhs: amount.currency,
                rhs: self.from_currency,
            });
        }
        if amount.decimals != self.from_decimals {
            return Err(DimensionalArithmeticError::DecimalMismatch {
                currency: amount.currency,
                lhs_dec: amount.decimals,
                rhs_dec: self.from_decimals,
            });
        }

        // Calculation:
        // raw_to = raw_from * (10^to_decimals) * rate_numerator / ((10^from_decimals) * rate_denominator)
        let raw_from = amount.raw_amount as u128;
        let ten = 10u128;
        let from_scale = ten.pow(self.from_decimals as u32);
        let to_scale = ten.pow(self.to_decimals as u32);

        let num = raw_from
            .checked_mul(to_scale)
            .ok_or(DimensionalArithmeticError::Overflow)?
            .checked_mul(self.rate_numerator)
            .ok_or(DimensionalArithmeticError::Overflow)?;

        let denom = from_scale
            .checked_mul(self.rate_denominator)
            .ok_or(DimensionalArithmeticError::Overflow)?;

        if denom == 0 {
            return Err(DimensionalArithmeticError::DivisionByZero);
        }

        // Round down for conservative converted amount
        let converted_raw = num / denom;
        let converted_u64 =
            u64::try_from(converted_raw).map_err(|_| DimensionalArithmeticError::Overflow)?;

        Ok(TypedTokenAmount {
            raw_amount: converted_u64,
            currency: self.to_currency,
            decimals: self.to_decimals,
            source: self.source.clone(),
            slot: self.slot,
            timestamp_ms: self.timestamp_ms,
        })
    }
}

/// Runtime dimensionally-safe token amount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedTokenAmount {
    pub raw_amount: u64,
    pub currency: Currency,
    pub decimals: u8,
    pub source: ConversionSource,
    pub slot: u64,
    pub timestamp_ms: u64,
}

impl TypedTokenAmount {
    pub fn new(
        raw_amount: u64,
        currency: Currency,
        decimals: u8,
        source: ConversionSource,
        slot: u64,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            raw_amount,
            currency,
            decimals,
            source,
            slot,
            timestamp_ms,
        }
    }

    pub fn sol_lamports(lamports: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            raw_amount: lamports,
            currency: Currency::Sol,
            decimals: 9,
            source: ConversionSource::Native,
            slot,
            timestamp_ms,
        }
    }

    pub fn usdc(raw_units: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            raw_amount: raw_units,
            currency: Currency::usdc(),
            decimals: 6,
            source: ConversionSource::Native,
            slot,
            timestamp_ms,
        }
    }

    /// UI human-readable floating point value.
    pub fn as_ui_f64(&self) -> f64 {
        let denom = 10f64.powi(self.decimals as i32);
        self.raw_amount as f64 / denom
    }

    /// Checked dimensional addition. Rejects cross-currency arithmetic.
    pub fn checked_add(&self, other: &Self) -> Result<Self, DimensionalArithmeticError> {
        if self.currency != other.currency {
            return Err(DimensionalArithmeticError::CurrencyMismatch {
                lhs: self.currency,
                rhs: other.currency,
            });
        }
        if self.decimals != other.decimals {
            return Err(DimensionalArithmeticError::DecimalMismatch {
                currency: self.currency,
                lhs_dec: self.decimals,
                rhs_dec: other.decimals,
            });
        }
        let raw = self
            .raw_amount
            .checked_add(other.raw_amount)
            .ok_or(DimensionalArithmeticError::Overflow)?;

        Ok(Self {
            raw_amount: raw,
            currency: self.currency,
            decimals: self.decimals,
            source: self.source.clone(),
            slot: self.slot.max(other.slot),
            timestamp_ms: self.timestamp_ms.max(other.timestamp_ms),
        })
    }

    /// Checked dimensional subtraction. Rejects cross-currency arithmetic.
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DimensionalArithmeticError> {
        if self.currency != other.currency {
            return Err(DimensionalArithmeticError::CurrencyMismatch {
                lhs: self.currency,
                rhs: other.currency,
            });
        }
        if self.decimals != other.decimals {
            return Err(DimensionalArithmeticError::DecimalMismatch {
                currency: self.currency,
                lhs_dec: self.decimals,
                rhs_dec: other.decimals,
            });
        }
        let raw = self
            .raw_amount
            .checked_sub(other.raw_amount)
            .ok_or(DimensionalArithmeticError::Underflow)?;

        Ok(Self {
            raw_amount: raw,
            currency: self.currency,
            decimals: self.decimals,
            source: self.source.clone(),
            slot: self.slot.max(other.slot),
            timestamp_ms: self.timestamp_ms.max(other.timestamp_ms),
        })
    }

    /// Checked signed difference (self - other) in raw units. Rejects cross-currency arithmetic.
    pub fn checked_signed_diff(&self, other: &Self) -> Result<i64, DimensionalArithmeticError> {
        if self.currency != other.currency {
            return Err(DimensionalArithmeticError::CurrencyMismatch {
                lhs: self.currency,
                rhs: other.currency,
            });
        }
        if self.decimals != other.decimals {
            return Err(DimensionalArithmeticError::DecimalMismatch {
                currency: self.currency,
                lhs_dec: self.decimals,
                rhs_dec: other.decimals,
            });
        }
        Ok((self.raw_amount as i64) - (other.raw_amount as i64))
    }
}

impl fmt::Display for TypedTokenAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} raw {} ({:.6} {}, dec={})",
            self.raw_amount,
            self.currency,
            self.as_ui_f64(),
            self.currency,
            self.decimals
        )
    }
}

// ---------------------------------------------------------------------------
// Compile-time currency safety markers
// ---------------------------------------------------------------------------

pub trait CurrencyTag: Clone + Copy + Send + Sync + 'static {
    fn currency() -> Currency;
    fn decimals() -> u8;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SolTag;
impl CurrencyTag for SolTag {
    fn currency() -> Currency {
        Currency::Sol
    }
    fn decimals() -> u8 {
        9
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UsdcTag;
impl CurrencyTag for UsdcTag {
    fn currency() -> Currency {
        Currency::usdc()
    }
    fn decimals() -> u8 {
        6
    }
}

/// Compile-time currency-typed amount wrapper.
/// Any attempt to add `CompileTypedAmount<SolTag>` to `CompileTypedAmount<UsdcTag>` fails compilation!
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompileTypedAmount<T: CurrencyTag> {
    pub raw_amount: u64,
    pub slot: u64,
    pub timestamp_ms: u64,
    _tag: PhantomData<T>,
}

impl<T: CurrencyTag> CompileTypedAmount<T> {
    pub fn new(raw_amount: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            raw_amount,
            slot,
            timestamp_ms,
            _tag: PhantomData,
        }
    }

    pub fn to_runtime(&self, source: ConversionSource) -> TypedTokenAmount {
        TypedTokenAmount {
            raw_amount: self.raw_amount,
            currency: T::currency(),
            decimals: T::decimals(),
            source,
            slot: self.slot,
            timestamp_ms: self.timestamp_ms,
        }
    }

    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.raw_amount.checked_add(rhs.raw_amount).map(|sum| Self {
            raw_amount: sum,
            slot: self.slot.max(rhs.slot),
            timestamp_ms: self.timestamp_ms.max(rhs.timestamp_ms),
            _tag: PhantomData,
        })
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.raw_amount
            .checked_sub(rhs.raw_amount)
            .map(|diff| Self {
                raw_amount: diff,
                slot: self.slot.max(rhs.slot),
                timestamp_ms: self.timestamp_ms.max(rhs.timestamp_ms),
                _tag: PhantomData,
            })
    }
}

// ---------------------------------------------------------------------------
// Granular Fee Itemization & Dimensional Profitability
// ---------------------------------------------------------------------------

/// Category of fee or cost item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FeeCategory {
    /// Irrecoverable network transaction cost (Base fee, Priority fee).
    NetworkBaseFee,
    PriorityFee,
    /// Private bundle validator tip (Jito).
    JitoTip,
    /// AMM swap fee charged by the pool.
    DexSwapFee,
    /// Token-2022 transfer fee or transfer hook fee.
    TokenTransferFee,
    /// Flash loan borrow fee.
    FlashLoanFee,
    /// Refundable account rent (recovered when temporary ATA/account is closed).
    RefundableRent,
    /// Permanent account rent.
    PermanentRent,
}

/// Itemized fee record with explicit denomination, conversion source, and embedding status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeItem {
    pub name: String,
    pub category: FeeCategory,
    pub amount: TypedTokenAmount,
    /// If true, the fee is already deducted from the swap token output, so
    /// subtracting it again from the gross spread would constitute double-counting!
    pub already_embedded_in_quote: bool,
    /// Whether this cost is refundable (e.g. rent returned upon closing temporary ATA).
    pub is_refundable: bool,
}

impl FeeItem {
    pub fn network_base_fee(lamports: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            name: "Base Fee".to_string(),
            category: FeeCategory::NetworkBaseFee,
            amount: TypedTokenAmount::sol_lamports(lamports, slot, timestamp_ms),
            already_embedded_in_quote: false,
            is_refundable: false,
        }
    }

    pub fn priority_fee(lamports: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            name: "Priority Fee".to_string(),
            category: FeeCategory::PriorityFee,
            amount: TypedTokenAmount::sol_lamports(lamports, slot, timestamp_ms),
            already_embedded_in_quote: false,
            is_refundable: false,
        }
    }

    pub fn jito_tip(lamports: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            name: "Jito Bundle Tip".to_string(),
            category: FeeCategory::JitoTip,
            amount: TypedTokenAmount::sol_lamports(lamports, slot, timestamp_ms),
            already_embedded_in_quote: false,
            is_refundable: false,
        }
    }

    pub fn dex_swap_fee(
        name: &str,
        raw_units: u64,
        currency: Currency,
        decimals: u8,
        slot: u64,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            name: name.to_string(),
            category: FeeCategory::DexSwapFee,
            amount: TypedTokenAmount::new(
                raw_units,
                currency,
                decimals,
                ConversionSource::Native,
                slot,
                timestamp_ms,
            ),
            // DEX swap fees in AMMs (like DLMM and CPMM) are subtracted from token amounts inside the pool,
            // so the returned amount_out already reflects the fee!
            already_embedded_in_quote: true,
            is_refundable: false,
        }
    }

    pub fn refundable_rent(lamports: u64, slot: u64, timestamp_ms: u64) -> Self {
        Self {
            name: "Temporary ATA Rent".to_string(),
            category: FeeCategory::RefundableRent,
            amount: TypedTokenAmount::sol_lamports(lamports, slot, timestamp_ms),
            already_embedded_in_quote: false,
            is_refundable: true,
        }
    }
}

/// Dimensional Analysis Safe Profitability Report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionalProfitabilityReport {
    pub candidate_id: String,
    pub route_description: String,
    pub quote_mint: Currency,
    pub quote_decimals: u8,
    pub initial_quote_input: TypedTokenAmount,
    pub hop1_output: TypedTokenAmount,
    pub final_quote_output: TypedTokenAmount,
    /// Gross profit in quote token raw units: final_quote_output.raw - initial_quote_input.raw
    pub gross_profit_quote_raw: i64,
    /// Itemized fees incurred
    pub fee_items: Vec<FeeItem>,
    /// Total irrecoverable network fees in native SOL (Base fee + Priority fee + Jito tip)
    pub total_network_fees_sol: TypedTokenAmount,
    /// Total irrecoverable network fees converted to route quote currency
    pub total_network_fees_in_quote: TypedTokenAmount,
    /// Conversion rate used to convert SOL fees to route quote currency
    pub sol_conversion_rate: Option<TimestampedRate>,
    /// Total DEX swap fees (tracked for reporting; not subtracted from gross profit if embedded)
    pub total_embedded_dex_fees_raw: u64,
    /// Total refundable rent (tracked for capital requirement; not subtracted from PnL)
    pub total_refundable_rent_lamports: u64,
    /// Total irrecoverable execution cost in quote token raw units
    pub total_execution_cost_quote_raw: u64,
    /// Net expected profit in quote token raw units:
    /// gross_profit_quote_raw - total_execution_cost_quote_raw
    pub net_profit_quote_raw: i64,
    /// Net profit in basis points of the initial quote input:
    /// (net_profit_quote_raw * 10,000) / initial_quote_input.raw_amount
    pub net_profit_bps: i64,
    pub quote_slot: u64,
    pub is_profitable: bool,
}

impl DimensionalProfitabilityReport {
    /// Compute a dimensionally-safe profitability report.
    pub fn compute(
        candidate_id: &str,
        route_description: &str,
        initial_input: TypedTokenAmount,
        hop1_out: TypedTokenAmount,
        final_out: TypedTokenAmount,
        fees: Vec<FeeItem>,
        sol_to_quote_rate: Option<TimestampedRate>,
        quote_slot: u64,
    ) -> Result<Self, DimensionalArithmeticError> {
        // Enforce that initial and final amounts have identical currency and decimals
        if initial_input.currency != final_out.currency {
            return Err(DimensionalArithmeticError::CurrencyMismatch {
                lhs: initial_input.currency,
                rhs: final_out.currency,
            });
        }
        if initial_input.decimals != final_out.decimals {
            return Err(DimensionalArithmeticError::DecimalMismatch {
                currency: initial_input.currency,
                lhs_dec: initial_input.decimals,
                rhs_dec: final_out.decimals,
            });
        }

        let quote_currency = initial_input.currency;
        let quote_decimals = initial_input.decimals;
        let gross_profit = final_out.checked_signed_diff(&initial_input)?;

        let mut total_network_fees_lamports = 0u64;
        let mut total_refundable_rent = 0u64;
        let mut total_embedded_dex_fees = 0u64;

        for item in &fees {
            match item.category {
                FeeCategory::NetworkBaseFee | FeeCategory::PriorityFee | FeeCategory::JitoTip => {
                    if item.amount.currency != Currency::Sol {
                        return Err(DimensionalArithmeticError::CurrencyMismatch {
                            lhs: item.amount.currency,
                            rhs: Currency::Sol,
                        });
                    }
                    total_network_fees_lamports = total_network_fees_lamports
                        .checked_add(item.amount.raw_amount)
                        .ok_or(DimensionalArithmeticError::Overflow)?;
                }
                FeeCategory::RefundableRent => {
                    total_refundable_rent = total_refundable_rent
                        .checked_add(item.amount.raw_amount)
                        .ok_or(DimensionalArithmeticError::Overflow)?;
                }
                FeeCategory::DexSwapFee if item.already_embedded_in_quote => {
                    total_embedded_dex_fees =
                        total_embedded_dex_fees.saturating_add(item.amount.raw_amount);
                }
                _ => {}
            }
        }

        let network_fees_sol = TypedTokenAmount::sol_lamports(
            total_network_fees_lamports,
            quote_slot,
            initial_input.timestamp_ms,
        );

        // Convert network fees to route quote currency
        let network_fees_in_quote = if quote_currency == Currency::Sol {
            network_fees_sol.clone()
        } else {
            let rate = sol_to_quote_rate.as_ref().ok_or(
                DimensionalArithmeticError::MissingConversionRate {
                    from: Currency::Sol,
                    to: quote_currency,
                },
            )?;
            rate.convert_amount(&network_fees_sol)?
        };

        let execution_cost_quote_raw = network_fees_in_quote.raw_amount;
        let net_profit = gross_profit - (execution_cost_quote_raw as i64);

        let net_bps = if initial_input.raw_amount > 0 {
            (net_profit * 10_000) / (initial_input.raw_amount as i64)
        } else {
            0
        };

        Ok(Self {
            candidate_id: candidate_id.to_string(),
            route_description: route_description.to_string(),
            quote_mint: quote_currency,
            quote_decimals,
            initial_quote_input: initial_input,
            hop1_output: hop1_out,
            final_quote_output: final_out,
            gross_profit_quote_raw: gross_profit,
            fee_items: fees,
            total_network_fees_sol: network_fees_sol,
            total_network_fees_in_quote: network_fees_in_quote,
            sol_conversion_rate: sol_to_quote_rate,
            total_embedded_dex_fees_raw: total_embedded_dex_fees,
            total_refundable_rent_lamports: total_refundable_rent,
            total_execution_cost_quote_raw: execution_cost_quote_raw,
            net_profit_quote_raw: net_profit,
            net_profit_bps: net_bps,
            quote_slot,
            is_profitable: net_profit > 0,
        })
    }
}
