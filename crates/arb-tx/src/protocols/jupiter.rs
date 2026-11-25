//! Jupiter Swap API v2 quote client.
//!
//! Read-only quotes use `GET /order` without a `taker`, which returns
//! `transaction: null`. Venue-isolated validation uses `GET /build` with an
//! explicit taker and `dexes`; this builds instructions in the response but
//! never signs or submits them. No method in this client broadcasts a swap.

use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use std::time::Duration;

pub struct JupiterClient {
    pub api_keys: Vec<String>,
    pub current_key_idx: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    client: Client,
    endpoint: String,
}

impl Default for JupiterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl JupiterClient {
    /// Base URL for the current Jupiter Swap API v2.
    pub const DEFAULT_ENDPOINT: &'static str = "https://api.jup.ag/swap/v2";

    /// Create a client using the canonical variable, with the old quote URL
    /// retained as a migration alias. The value must be an HTTPS base URL.
    pub fn new() -> Self {
        let endpoint = std::env::var("JUPITER_BASE_URL")
            .or_else(|_| std::env::var("JUPITER_QUOTE_API_URL"))
            .unwrap_or_else(|_| Self::DEFAULT_ENDPOINT.to_string());
        Self::with_endpoint(endpoint)
    }

    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        Self::with_endpoint_and_timeout(endpoint, Duration::from_millis(3_000))
    }

    pub fn with_endpoint_and_timeout(endpoint: impl Into<String>, timeout: Duration) -> Self {
        let mut api_keys = Vec::new();
        for name in ["JUPITER_API_KEY", "JUP_API_KEY"] {
            if let Ok(key) = std::env::var(name) {
                let key = key.trim();
                if !key.is_empty() && !api_keys.iter().any(|existing| existing == key) {
                    api_keys.push(key.to_string());
                }
            }
        }
        Self::with_keys_and_timeout(endpoint, api_keys, timeout)
    }

    pub fn with_keys_and_timeout(
        endpoint: impl Into<String>,
        mut api_keys: Vec<String>,
        timeout: Duration,
    ) -> Self {
        if api_keys.is_empty() {
            for name in ["JUPITER_API_KEY", "JUP_API_KEY"] {
                if let Ok(key) = std::env::var(name) {
                    let key = key.trim();
                    if !key.is_empty() && !api_keys.iter().any(|existing| existing == key) {
                        api_keys.push(key.to_string());
                    }
                }
            }
        }
        api_keys.retain(|key| !key.trim().is_empty());
        api_keys.dedup();
        if api_keys.is_empty() {
            tracing::warn!("Jupiter API key not configured; quote calls will fail closed");
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(timeout)
            .build()
            .unwrap_or_default();

        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        Self {
            api_keys,
            current_key_idx: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            client,
            endpoint,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Get a quote from the v2 Meta-Aggregator. `taker` is deliberately omitted,
    /// so Jupiter returns a quote only and does not assemble a transaction.
    pub async fn get_quote(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
    ) -> Result<serde_json::Value> {
        validate_quote_args(input_mint, output_mint, amount)?;
        let params = vec![
            ("inputMint", input_mint.to_string()),
            ("outputMint", output_mint.to_string()),
            ("amount", amount.to_string()),
            ("slippageBps", slippage_bps.to_string()),
        ];
        let response = self.request_json("order", &params).await?;
        if !response
            .get("transaction")
            .is_none_or(serde_json::Value::is_null)
        {
            bail!("Jupiter read-only order unexpectedly returned a transaction");
        }
        validate_quote_response(&response, amount)?;
        Ok(response)
    }

    /// Compare against one Metis venue using the v2 Router `/build` endpoint.
    /// `/build` requires a taker address; the default method takes it from the
    /// public operator address env var. The returned instructions are never
    /// signed or submitted by this client.
    pub async fn get_quote_dexes(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
        dexes: Option<&[&str]>,
    ) -> Result<serde_json::Value> {
        let taker = std::env::var("OPERATOR_PUBLIC_KEY")
            .or_else(|_| std::env::var("SOLANA_PUBLIC_KEY"))
            .context("OPERATOR_PUBLIC_KEY is required for venue-specific Jupiter /build quotes")?;
        self.get_quote_dexes_for_taker(input_mint, output_mint, amount, slippage_bps, dexes, &taker)
            .await
    }

    pub async fn get_quote_dexes_for_taker(
        &self,
        input_mint: &str,
        output_mint: &str,
        amount: u64,
        slippage_bps: u16,
        dexes: Option<&[&str]>,
        taker: &str,
    ) -> Result<serde_json::Value> {
        validate_quote_args(input_mint, output_mint, amount)?;
        if taker.trim().is_empty() {
            bail!("taker address must not be empty");
        }
        let mut params = vec![
            ("inputMint", input_mint.to_string()),
            ("outputMint", output_mint.to_string()),
            ("amount", amount.to_string()),
            ("taker", taker.trim().to_string()),
            ("slippageBps", slippage_bps.to_string()),
        ];
        if let Some(dexes) = dexes.filter(|list| !list.is_empty()) {
            if dexes.iter().any(|name| name.trim().is_empty()) {
                bail!("DEX labels must not contain empty values");
            }
            params.push(("dexes", dexes.join(",")));
        }
        let response = self.request_json("build", &params).await?;
        validate_quote_response(&response, amount)?;
        Ok(response)
    }

    async fn request_json(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<serde_json::Value> {
        if self.api_keys.is_empty() {
            bail!("Jupiter API key not configured");
        }
        if !self.endpoint.starts_with("https://") {
            bail!("Jupiter endpoint must use HTTPS");
        }
        let url = format!("{}/{path}", self.endpoint);
        let total_keys = self.api_keys.len();
        let attempts = (total_keys * 2).max(2);

        for attempt in 0..attempts {
            let key_idx = (self
                .current_key_idx
                .load(std::sync::atomic::Ordering::Relaxed)
                + attempt)
                % total_keys;
            let response = match self
                .client
                .get(&url)
                .query(params)
                .header("x-api-key", &self.api_keys[key_idx])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    self.current_key_idx.store(
                        (key_idx + 1) % total_keys,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    if attempt + 1 < attempts {
                        let delay_ms = 300_u64.saturating_mul((attempt as u64 + 1).min(4));
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        tracing::warn!(
                            "Jupiter v2 /{path} transport failure; rotating quote key and retrying"
                        );
                        continue;
                    }
                    return Err(error)
                        .with_context(|| format!("Jupiter v2 /{path} request failed"));
                }
            };
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
                || status.is_server_error()
            {
                self.current_key_idx.store(
                    (key_idx + 1) % total_keys,
                    std::sync::atomic::Ordering::Relaxed,
                );
                if attempt + 1 < attempts {
                    let delay_ms = 300_u64.saturating_mul((attempt as u64 + 1).min(4));
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    continue;
                }
            }
            if !status.is_success() {
                bail!("Jupiter v2 /{path} returned HTTP {status}");
            }
            let value: serde_json::Value = response
                .json()
                .await
                .with_context(|| format!("Jupiter v2 /{path} returned invalid JSON"))?;
            if let Some(message) = value.get("error").and_then(serde_json::Value::as_str) {
                bail!("Jupiter v2 /{path} error: {message}");
            }
            if let Some(message) = value.get("message").and_then(serde_json::Value::as_str)
                && (message.contains("Too many requests") || message.contains("rate limit"))
            {
                if attempt + 1 < attempts {
                    self.current_key_idx.store(
                        (key_idx + 1) % total_keys,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    continue;
                }
                bail!("Jupiter v2 /{path} rate limit exhausted");
            }
            return Ok(value);
        }
        bail!("Jupiter v2 /{path} exhausted all configured API-key attempts")
    }
}

fn validate_quote_args(input_mint: &str, output_mint: &str, amount: u64) -> Result<()> {
    if input_mint.trim().is_empty() || output_mint.trim().is_empty() {
        bail!("Jupiter input/output mint must not be empty");
    }
    if input_mint == output_mint {
        bail!("Jupiter input and output mints must differ");
    }
    if amount == 0 {
        bail!("Jupiter quote amount must be positive");
    }
    Ok(())
}

fn validate_quote_response(value: &serde_json::Value, expected_input: u64) -> Result<()> {
    let input = value
        .get("inAmount")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok())
        .context("Jupiter quote is missing a valid inAmount")?;
    let output = value
        .get("outAmount")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok())
        .context("Jupiter quote is missing a valid outAmount")?;
    if input != expected_input || output == 0 {
        bail!("Jupiter quote amount mismatch or zero output");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_uses_v2_base_url_and_normalizes_slashes() {
        let client = JupiterClient::with_keys_and_timeout(
            "https://api.jup.ag/swap/v2///",
            vec!["test-key".to_string()],
            Duration::from_millis(500),
        );
        assert_eq!(client.endpoint(), "https://api.jup.ag/swap/v2");
    }

    #[test]
    fn quote_arguments_reject_zero_same_mint_and_empty_mint() {
        assert!(validate_quote_args("", "out", 1).is_err());
        assert!(validate_quote_args("same", "same", 1).is_err());
        assert!(validate_quote_args("in", "out", 0).is_err());
        assert!(validate_quote_args("in", "out", 1).is_ok());
    }

    #[test]
    fn quote_response_must_match_input_and_have_nonzero_output() {
        let valid = serde_json::json!({"inAmount":"100","outAmount":"99"});
        assert!(validate_quote_response(&valid, 100).is_ok());
        assert!(validate_quote_response(&valid, 101).is_err());
        let zero = serde_json::json!({"inAmount":"100","outAmount":"0"});
        assert!(validate_quote_response(&zero, 100).is_err());
    }

    #[test]
    fn request_payload_names_v2_quote_routes() {
        // These tests are intentionally pure: they prevent reverting to the
        // removed /quote route or accidentally exposing a taker on /order.
        assert_eq!(
            JupiterClient::DEFAULT_ENDPOINT,
            "https://api.jup.ag/swap/v2"
        );
        let client = JupiterClient::with_keys_and_timeout(
            JupiterClient::DEFAULT_ENDPOINT,
            vec!["test-key".to_string()],
            Duration::from_millis(500),
        );
        assert!(client.endpoint().ends_with("/swap/v2"));
    }
}
