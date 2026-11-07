//! High-performance, loss-free data models for Solana Webhooks (Helius Enhanced & Raw).
//!
//! Numeric Precision Invariant: All transaction values, amounts, slots, lamports,
//! and nonces strictly use `u64` or `u128` to prevent precision loss.
//! Floating-point types are strictly forbidden for blockchain numerical data.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Custom deserializer module allowing deserialization of `u64` from either
/// a JSON integer or a JSON string without precision loss.
pub mod string_or_u64 {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = u64;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 64-bit unsigned integer or a string representing one")
            }

            fn visit_u64<E>(self, value: u64) -> Result<u64, E>
            where
                E: serde::de::Error,
            {
                Ok(value)
            }

            fn visit_u32<E>(self, value: u32) -> Result<u64, E>
            where
                E: serde::de::Error,
            {
                Ok(value as u64)
            }

            fn visit_i64<E>(self, value: i64) -> Result<u64, E>
            where
                E: serde::de::Error,
            {
                if value >= 0 {
                    Ok(value as u64)
                } else {
                    Err(E::custom(format!("negative value cannot be u64: {value}")))
                }
            }

            fn visit_i32<E>(self, value: i32) -> Result<u64, E>
            where
                E: serde::de::Error,
            {
                if value >= 0 {
                    Ok(value as u64)
                } else {
                    Err(E::custom(format!("negative value cannot be u64: {value}")))
                }
            }

            fn visit_str<E>(self, value: &str) -> Result<u64, E>
            where
                E: serde::de::Error,
            {
                value
                    .parse::<u64>()
                    .map_err(|e| E::custom(format!("failed to parse u64 from string '{value}': {e}")))
            }
        }

        deserializer.deserialize_any(Visitor)
    }

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(*value)
    }
}

/// Custom deserializer module allowing deserialization of `Option<u64>` from either
/// null, a JSON integer, or a JSON string.
pub mod option_string_or_u64 {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NumericOrString {
            Num(u64),
            Str(String),
            Null,
        }

        match Option::<NumericOrString>::deserialize(deserializer)? {
            Some(NumericOrString::Num(n)) => Ok(Some(n)),
            Some(NumericOrString::Str(s)) => {
                if s.trim().is_empty() {
                    Ok(None)
                } else {
                    s.parse::<u64>().map(Some).map_err(serde::de::Error::custom)
                }
            }
            Some(NumericOrString::Null) | None => Ok(None),
        }
    }

    pub fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(v) => serializer.serialize_some(v),
            None => serializer.serialize_none(),
        }
    }
}

/// Custom deserializer module for `u128` from JSON number or string.
pub mod string_or_u128 {
    use super::*;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a 128-bit unsigned integer or a string representing one")
            }

            fn visit_u64<E>(self, value: u64) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                Ok(value as u128)
            }

            fn visit_u128<E>(self, value: u128) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                Ok(value)
            }

            fn visit_i64<E>(self, value: i64) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                if value >= 0 {
                    Ok(value as u128)
                } else {
                    Err(E::custom(format!("negative value cannot be u128: {value}")))
                }
            }

            fn visit_i128<E>(self, value: i128) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                if value >= 0 {
                    Ok(value as u128)
                } else {
                    Err(E::custom(format!("negative value cannot be u128: {value}")))
                }
            }

            fn visit_str<E>(self, value: &str) -> Result<u128, E>
            where
                E: serde::de::Error,
            {
                value
                    .parse::<u128>()
                    .map_err(|e| E::custom(format!("failed to parse u128 from string '{value}': {e}")))
            }
        }

        deserializer.deserialize_any(Visitor)
    }

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }
}

/// Incoming webhook request payload wrapper.
/// Providers like Helius or custom bots may send a single transaction or an array batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WebhookPayload {
    Batch(Vec<SolanaTransactionPayload>),
    Single(SolanaTransactionPayload),
}

impl WebhookPayload {
    /// Extracts all transactions into a vector for unified processing.
    pub fn into_transactions(self) -> Vec<SolanaTransactionPayload> {
        match self {
            Self::Batch(list) => list,
            Self::Single(item) => vec![item],
        }
    }
}

/// Top-level transaction payload received from Helius or enhanced webhook providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTransactionPayload {
    /// Transaction unique signature (base58 hash).
    pub signature: String,

    /// Block slot at which the transaction was processed.
    #[serde(default, with = "string_or_u64")]
    pub slot: u64,

    /// Unix timestamp in seconds.
    #[serde(default)]
    pub timestamp: i64,

    /// Fee paid in lamports.
    #[serde(default, with = "string_or_u64")]
    pub fee: u64,

    /// Fee payer account public key.
    #[serde(default)]
    pub fee_payer: String,

    /// Transaction classification type (e.g. SWAP, TRANSFER, UNKNOWN).
    #[serde(default, rename = "type")]
    pub transaction_type: String,

    /// Transaction source (e.g. RAYDIUM, ORCA, METEORA, JUPITER, PUMP_FUN).
    #[serde(default)]
    pub source: String,

    /// Human-readable description (if parsed by Helius).
    pub description: Option<String>,

    /// Transaction error object if failed. None if successful.
    pub transaction_error: Option<serde_json::Value>,

    /// List of token transfers executed during the transaction.
    #[serde(default)]
    pub token_transfers: Vec<TokenTransfer>,

    /// List of native SOL transfers executed during the transaction.
    #[serde(default)]
    pub native_transfers: Vec<NativeTransfer>,

    /// Account data entries modified during transaction.
    #[serde(default)]
    pub account_data: Vec<AccountData>,

    /// Outer instruction set.
    #[serde(default)]
    pub instructions: Vec<Instruction>,

    /// Inner instructions grouped by instruction index.
    #[serde(default)]
    pub inner_instructions: Vec<InnerInstructionGroup>,

    /// Raw transaction data if included by provider.
    pub raw: Option<RawTransactionData>,

    /// Additional event metadata (e.g., DEX swap details).
    pub events: Option<EnhancedEvents>,
}

/// Enhanced Token Transfer representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    pub from_token_account: Option<String>,
    pub to_token_account: Option<String>,
    #[serde(default, with = "string_or_u128")]
    pub token_amount: u128,
    #[serde(default, with = "string_or_u64")]
    pub raw_token_amount: u64,
    #[serde(default)]
    pub mint: String,
}

/// Enhanced Native SOL Transfer representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTransfer {
    pub from_user_account: Option<String>,
    pub to_user_account: Option<String>,
    #[serde(default, with = "string_or_u64")]
    pub amount: u64,
}

/// Account data entry modified during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountData {
    pub account: String,
    #[serde(default, with = "string_or_u64")]
    pub native_balance_change: u64,
    #[serde(default)]
    pub token_balance_changes: Vec<TokenBalanceChange>,
}

/// Token balance delta for an account.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalanceChange {
    pub user_account: String,
    pub token_account: String,
    pub mint: String,
    #[serde(default, with = "string_or_u128")]
    pub raw_token_amount: u128,
}

/// Outer Solana Instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instruction {
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub program_id: String,
    #[serde(default)]
    pub inner_instructions: Vec<CompiledInnerInstruction>,
}

/// Grouping of inner instructions corresponding to an outer instruction index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InnerInstructionGroup {
    #[serde(default, with = "string_or_u64")]
    pub index: u64,
    #[serde(default)]
    pub instructions: Vec<CompiledInnerInstruction>,
}

/// Compiled or parsed inner instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledInnerInstruction {
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub program_id: String,
}

/// Raw transaction data structure representing on-chain compiled message and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTransactionData {
    pub signatures: Vec<String>,
    pub message: RawMessage,
    pub meta: Option<RawMeta>,
}

/// Raw Solana Message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMessage {
    pub account_keys: Vec<String>,
    pub header: RawHeader,
    pub recent_blockhash: String,
    pub instructions: Vec<RawInstruction>,
}

/// Raw Solana Header.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHeader {
    pub num_required_signatures: u8,
    pub num_readonly_signed_accounts: u8,
    pub num_readonly_unsigned_accounts: u8,
}

/// Raw Solana Compiled Instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawInstruction {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: String,
}

/// Raw Transaction Metadata including execution logs and balance changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMeta {
    pub err: Option<serde_json::Value>,
    #[serde(default, with = "string_or_u64")]
    pub fee: u64,
    #[serde(default)]
    pub pre_balances: Vec<u64>,
    #[serde(default)]
    pub post_balances: Vec<u64>,
    pub log_messages: Option<Vec<String>>,
}

/// Enhanced event metadata provided by Helius/DEX parsers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedEvents {
    pub swap: Option<SwapEvent>,
}

/// Parsed Swap Event representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapEvent {
    pub native_input: Option<NativeTransfer>,
    pub native_output: Option<NativeTransfer>,
    pub token_inputs: Option<Vec<TokenTransfer>>,
    pub token_outputs: Option<Vec<TokenTransfer>>,
    pub token_fees: Option<Vec<TokenTransfer>>,
    pub native_fees: Option<Vec<NativeTransfer>>,
    pub inner_swaps: Option<Vec<serde_json::Value>>,
}

/// Resolved pool event emitted to worker pipeline after routing analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPoolEvent {
    pub signature: String,
    pub slot: u64,
    pub timestamp: i64,
    pub pool_id: String,
    pub amm_type: String,
    pub quote_mint: String,
    pub modified_accounts: Vec<String>,
    pub token_transfers: Vec<TokenTransfer>,
}

