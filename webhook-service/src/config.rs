//! Strongly-typed runtime configuration parsed strictly from `config.toml` via `toml` crate.
//!
//! Controls server parameters, deduplication thresholds, expression filters,
//! supported Solana AMMs, and redundant pool routing structures.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

/// Root configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub dedup: DedupConfig,
    pub filter: FilterConfig,
    pub amms: AmmsConfig,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
}

/// HTTP Server and runtime settings.
///
/// `Debug` is implemented manually: `auth_token` is redacted so config
/// debugging/tracing can never leak secret material.
#[derive(Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub auth_token: String,
    pub channel_buffer_size: usize,
    pub worker_threads: usize,
    /// Explicit CORS allowlist (origins). An empty list disables CORS headers
    /// entirely; `CorsLayer::permissive()` is never constructed.
    #[serde(default)]
    pub cors_allowed_origins: Vec<String>,
    /// Maximum accepted request body size in bytes (per OWASP webhook
    /// guidance). Requests above this are rejected before parsing.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    /// Environment variable NAME holding the auth token. When set, the env
    /// value overrides `auth_token` at load time and MUST exist (fail-closed);
    /// committed config files never contain production secrets.
    #[serde(default)]
    pub auth_token_env: String,
    /// Per-client requests allowed per 60-second window. `0` disables rate
    /// limiting. Applied to every route before authentication.
    #[serde(default = "default_rate_limit_requests_per_minute")]
    pub rate_limit_requests_per_minute: u32,
    /// Maximum accepted event age in seconds (replay protection): events
    /// older than this, far-future timestamps, or missing timestamps are
    /// rejected. `0` disables the guard.
    #[serde(default)]
    pub max_event_age_secs: u64,
}

fn default_max_body_bytes() -> usize {
    1_048_576
}

fn default_rate_limit_requests_per_minute() -> u32 {
    600
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth_token", &"***")
            .field("auth_token_env", &self.auth_token_env)
            .field("channel_buffer_size", &self.channel_buffer_size)
            .field("worker_threads", &self.worker_threads)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("max_body_bytes", &self.max_body_bytes)
            .field(
                "rate_limit_requests_per_minute",
                &self.rate_limit_requests_per_minute,
            )
            .field("max_event_age_secs", &self.max_event_age_secs)
            .finish()
    }
}

/// Strict Deduplication parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    pub local_cache_ttl_secs: u64,
    pub redis_url: String,
    pub redis_ttl_ms: u64,
    pub prune_interval_secs: u64,
}

/// Expression Webhook and log evaluation filter settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    pub drop_failed_transactions: bool,
    pub min_lamport_transfer: u64,
    pub only_monitored_amms: bool,
}

/// AMM Program IDs loaded dynamically from configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmmsConfig {
    pub raydium_v4: String,
    pub raydium_cpmm: String,
    pub raydium_clmm: String,
    pub orca_whirlpool: String,
    pub meteora_dlmm: String,
    pub meteora_dynamic: String,
    pub jupiter_v6: String,
    pub pump_fun: String,
}

impl AmmsConfig {
    /// Builds a fast reverse-lookup map from Solana Program ID to AMM label.
    pub fn build_program_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert(self.raydium_v4.clone(), "raydium_v4".to_string());
        map.insert(self.raydium_cpmm.clone(), "raydium_cpmm".to_string());
        map.insert(self.raydium_clmm.clone(), "raydium_clmm".to_string());
        map.insert(self.orca_whirlpool.clone(), "orca_whirlpool".to_string());
        map.insert(self.meteora_dlmm.clone(), "meteora_dlmm".to_string());
        map.insert(self.meteora_dynamic.clone(), "meteora_dynamic".to_string());
        map.insert(self.jupiter_v6.clone(), "jupiter_v6".to_string());
        map.insert(self.pump_fun.clone(), "pump_fun".to_string());
        map
    }
}

/// Target Token routing metadata containing redundant pools across multiple AMMs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig {
    pub mint: String,
    pub symbol: String,
    pub redundant_pools: Vec<PoolConfig>,
}

/// Redundant Pool specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    pub pool_id: String,
    pub amm: String,
    pub quote_mint: String,
    pub accounts: Vec<String>,
}

impl AppConfig {
    /// Loads configuration from default paths or `CONFIG_PATH` environment variable.
    pub fn load_default() -> Result<Self> {
        let path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".to_string());
        Self::load_from_file(path)
    }

    /// Reads and parses the TOML file, then resolves secret material from the
    /// environment (fail-closed: a configured env-var name that is missing or
    /// empty aborts startup with an actionable error naming only the variable).
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)
            .with_context(|| format!("Failed to read config file at: {}", path_ref.display()))?;

        let mut config: Self = toml::from_str(&contents).with_context(|| {
            format!(
                "Failed to parse TOML configuration from: {}",
                path_ref.display()
            )
        })?;

        if !config.server.auth_token_env.is_empty() {
            let resolved = crate::secrets::resolve_secret_from_env(&config.server.auth_token_env)
                .map_err(|e| anyhow::anyhow!("server.auth_token_env: {}", e))?;
            config.server.auth_token = resolved.expose().to_string();
        }

        config.validate()?;
        Ok(config)
    }

    /// Validates core invariant conditions.
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            anyhow::bail!("Server port must be non-zero");
        }
        if self.server.channel_buffer_size == 0 {
            anyhow::bail!("channel_buffer_size must be greater than zero");
        }
        if self.dedup.redis_ttl_ms == 0 {
            anyhow::bail!("redis_ttl_ms must be greater than zero");
        }
        if self.dedup.local_cache_ttl_secs == 0 {
            anyhow::bail!("local_cache_ttl_secs must be greater than zero");
        }
        if self.server.rate_limit_requests_per_minute > 0
            && self.server.rate_limit_requests_per_minute < 10
        {
            anyhow::bail!(
                "server.rate_limit_requests_per_minute must be 0 (disabled) or at least 10"
            );
        }
        Ok(())
    }
}
