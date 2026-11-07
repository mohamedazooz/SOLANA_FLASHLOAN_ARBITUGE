//! Strongly-typed Solami Data & Event Models for High-Frequency Solana Webhooks.
//!
//! Maps Solami stream & webhook event types:
//! - `swap` (DEX swaps across Raydium, Meteora, PumpSwap, etc.)
//! - `liquidity` (LP additions/removals)
//! - `token_create` / `launches` (New token mints)
//! - `pool_create` (New AMM pools)
//! - `transfer` (SPL token / native SOL transfers)
//! - `graduating` / `graduated` (Meme bonding curve graduations)
//! - `surge` / `radar` / `trending` (Volume & price velocity triggers)

use crate::models::{AccountData, SolanaTransactionPayload, TokenTransfer};
use serde::{Deserialize, Deserializer, Serialize};

/// Top-level Solami Webhook request envelope supporting single, batch, or wrapped events.
#[derive(Debug, Clone, Serialize)]
pub enum SolamiWebhookPayload {
    Batch(Vec<SolamiEvent>),
    Single(SolamiEvent),
}

impl<'de> Deserialize<'de> for SolamiWebhookPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.is_array() {
            let list = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(SolamiWebhookPayload::Batch(list))
        } else if let Some(events) = value.get("events").and_then(|v| v.as_array()) {
            let list = serde_json::from_value(serde_json::Value::Array(events.clone()))
                .map_err(serde::de::Error::custom)?;
            Ok(SolamiWebhookPayload::Batch(list))
        } else if let Some(data) = value.get("data").and_then(|v| v.as_array()) {
            let list = serde_json::from_value(serde_json::Value::Array(data.clone()))
                .map_err(serde::de::Error::custom)?;
            Ok(SolamiWebhookPayload::Batch(list))
        } else {
            let item = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
            Ok(SolamiWebhookPayload::Single(item))
        }
    }
}

impl SolamiWebhookPayload {
    /// Normalizes incoming payload into a flat list of Solami events.
    pub fn into_events(self) -> Vec<SolamiEvent> {
        match self {
            Self::Batch(list) => list,
            Self::Single(ev) => vec![ev],
        }
    }
}

/// Individual Solami Event representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiEvent {
    /// Event type: swap, liquidity, token_create, pool_create, graduation, surge, etc.
    #[serde(default, rename = "type")]
    pub event_type: String,

    /// Unique transaction signature (or generated synthetic hash).
    #[serde(default)]
    pub signature: Option<String>,

    /// Solana slot number.
    #[serde(default)]
    pub slot: Option<u64>,

    /// Unix timestamp in seconds or milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,

    /// Associated DEX (e.g. pumpswap, raydium_clmm, meteora_dlmm, raydium_cpmm, orca).
    #[serde(default)]
    pub dex: Option<String>,

    /// Target pool public key address.
    #[serde(default)]
    pub pool: Option<String>,

    /// Primary token mint address.
    #[serde(default)]
    pub address: Option<String>,

    /// Alternative token mint field.
    #[serde(default)]
    pub mint: Option<String>,

    /// Wallet / trader address executing the event.
    #[serde(default)]
    pub trader: Option<String>,

    /// Side for swaps: "buy" or "sell".
    #[serde(default)]
    pub side: Option<String>,

    /// Volume or trade size in USD.
    #[serde(default)]
    pub amount_usd: Option<f64>,

    /// Volume in USD alternative field.
    #[serde(default)]
    pub volume_usd: Option<f64>,

    /// Raw base token amount.
    #[serde(default)]
    pub base_amount: Option<u64>,

    /// Raw quote token amount.
    #[serde(default)]
    pub quote_amount: Option<u64>,

    /// Base token mint.
    #[serde(default)]
    pub base_mint: Option<String>,

    /// Quote token mint.
    #[serde(default)]
    pub quote_mint: Option<String>,

    /// True if event is from historical backfill.
    #[serde(default)]
    pub backfill: Option<bool>,

    /// Associated account pubkeys involved in this event.
    #[serde(default)]
    pub accounts: Option<Vec<String>>,
}

impl SolamiEvent {
    /// Returns the unique signature or generates a deterministic fallback.
    pub fn get_or_generate_signature(&self) -> String {
        if let Some(ref sig) = self.signature {
            if !sig.is_empty() {
                return sig.clone();
            }
        }
        // Fallback: deterministic event key from type, pool, trader, and slot/timestamp
        format!(
            "solami_{}_{}_{}_{}",
            self.event_type,
            self.pool.as_deref().unwrap_or("unknown_pool"),
            self.slot.unwrap_or(0),
            self.timestamp.unwrap_or(0)
        )
    }

    /// Converts the Solami event into the unified `SolanaTransactionPayload`
    /// for downstream filtering, deduplication, and AMM routing.
    pub fn to_transaction_payload(&self) -> SolanaTransactionPayload {
        let signature = self.get_or_generate_signature();
        let slot = self.slot.unwrap_or(0);
        let timestamp = self.timestamp.unwrap_or(0);
        let trader = self.trader.clone().unwrap_or_default();
        let source = self.dex.clone().unwrap_or_else(|| "SOLAMI".to_string()).to_uppercase();

        let primary_mint = self.address.clone().or_else(|| self.mint.clone()).unwrap_or_default();
        let mut account_data = Vec::new();

        // Include pool address in account data for AMM routing
        if let Some(ref pool_addr) = self.pool {
            account_data.push(AccountData {
                account: pool_addr.clone(),
                native_balance_change: 0,
                token_balance_changes: Vec::new(),
            });
        }

        // Include extra accounts if present
        if let Some(ref extra_accs) = self.accounts {
            for acc in extra_accs {
                account_data.push(AccountData {
                    account: acc.clone(),
                    native_balance_change: 0,
                    token_balance_changes: Vec::new(),
                });
            }
        }

        // If base/quote mints exist, create token transfers
        let mut token_transfers = Vec::new();
        if !primary_mint.is_empty() {
            token_transfers.push(TokenTransfer {
                from_user_account: Some(trader.clone()),
                to_user_account: self.pool.clone(),
                from_token_account: None,
                to_token_account: None,
                token_amount: self.base_amount.map(|v| v as u128).unwrap_or(0),
                raw_token_amount: self.base_amount.unwrap_or(0),
                mint: primary_mint,
            });
        }

        SolanaTransactionPayload {
            signature,
            slot,
            timestamp,
            fee: 5000,
            fee_payer: trader,
            transaction_type: self.event_type.to_uppercase(),
            source,
            description: Some(format!(
                "Solami {} on {} (USD: {})",
                self.event_type,
                self.dex.as_deref().unwrap_or("unknown"),
                self.amount_usd.or(self.volume_usd).unwrap_or(0.0)
            )),
            account_data,
            instructions: Vec::new(),
            inner_instructions: Vec::new(),
            native_transfers: Vec::new(),
            token_transfers,
            raw: None,
            events: None,
            transaction_error: None,
        }
    }
}
