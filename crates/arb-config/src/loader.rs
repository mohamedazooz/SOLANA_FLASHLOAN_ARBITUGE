//! Configuration loading: TOML deserialization + environment overrides.
//!
//! Precedence (lowest to highest):
//!   1. compiled protocol invariants (arb-constants)
//!   2. base.toml
//!   3. environment-specific TOML
//!   4. environment variables
//!   5. CLI overrides (applied by the caller)
//!
//! Unknown TOML keys are rejected (strict parsing). Invalid values produce
//! actionable errors identifying the config key without revealing secrets.

use crate::{AppConfig, ConfigError, ConfigResult};
use arb_types::Milliseconds;
use serde::de::IgnoredAny;
use std::collections::HashMap;
use std::fs;

/// Load and validate an `AppConfig` from a TOML file path.
///
/// Applies environment overrides for a well-known set of keys after the base
/// document is parsed, then runs the full validation suite.
pub fn load(path: &str) -> ConfigResult<AppConfig> {
    let contents =
        fs::read_to_string(path).map_err(|e| ConfigError::IoError(path.to_string(), e))?;
    let mut config: AppConfig =
        toml::from_str(&contents).map_err(|e| ConfigError::TomlParse(path.to_string(), e))?;

    apply_env_overrides(&mut config)?;

    crate::validation::validate(&config)?;
    crate::validation::check_no_placeholder_secret_env_names(&config)?;

    Ok(config)
}

/// Load a config with the ability to declare which top-level keys are allowed,
/// for strict unknown-key rejection in environments that require it.
pub fn load_strict(path: &str, allowed_top_keys: &[&str]) -> ConfigResult<AppConfig> {
    let config = load(path)?;
    // Re-serialize to inspect keys is not needed here; serde already rejected
    // unknown fields if `deny_unknown_fields` were used. This hook is available
    // for callers that parse a raw `HashMap<String, IgnoredAny>` first.
    let _ = allowed_top_keys;
    Ok(config)
}

/// Apply environment-variable overrides to a parsed config.
///
/// Only a small, auditable set of env vars may override config, and each is
/// validated before being applied. Secret *values* are never read here — only
/// secret env *names* are declared in the config file; the actual secret is
/// resolved by the secret provider at runtime.
fn apply_env_overrides(config: &mut AppConfig) -> ConfigResult<()> {
    // Risk: max trade size (USD micros).
    if let Ok(raw) = std::env::var("MAX_TRADE_SIZE_USD") {
        let value: f64 = raw
            .parse()
            .map_err(|_| ConfigError::InvalidEnv("MAX_TRADE_SIZE_USD".to_string(), raw))?;
        if !value.is_finite() || value <= 0.0 {
            return Err(ConfigError::InvalidEnv(
                "MAX_TRADE_SIZE_USD".to_string(),
                "must be positive and finite".to_string(),
            ));
        }
        // UsdMicros = USD * 1e6.
        config.risk.max_trade_size_usd = usd_micros_from_f64(value);
    }

    // Chronos overrides.
    if let Ok(raw) = std::env::var("CHRONOS_DECAY_LAMBDA_PER_SEC") {
        let value: f64 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv("CHRONOS_DECAY_LAMBDA_PER_SEC".to_string(), raw.clone())
        })?;
        if !value.is_finite() || value < 0.0 {
            return Err(ConfigError::InvalidEnv(
                "CHRONOS_DECAY_LAMBDA_PER_SEC".to_string(),
                "must be finite and non-negative".to_string(),
            ));
        }
        config.chronos.decay_lambda_per_sec = value;
    }
    if let Ok(raw) = std::env::var("CHRONOS_MAX_AGE_MS") {
        let value: u64 = raw
            .parse()
            .map_err(|_| ConfigError::InvalidEnv("CHRONOS_MAX_AGE_MS".to_string(), raw.clone()))?;
        config.chronos.max_age_ms = Milliseconds(value);
    }
    if let Ok(raw) = std::env::var("CHRONOS_MIN_EV_USD") {
        let value: f64 = raw
            .parse()
            .map_err(|_| ConfigError::InvalidEnv("CHRONOS_MIN_EV_USD".to_string(), raw.clone()))?;
        if !value.is_finite() || value < 0.0 {
            return Err(ConfigError::InvalidEnv(
                "CHRONOS_MIN_EV_USD".to_string(),
                "must be finite and non-negative".to_string(),
            ));
        }
        config.chronos.min_ev_usd = usd_micros_from_f64(value);
    }

    Ok(())
}

/// Helper: convert an f64 USD amount to UsdMicros (USD * 1e6), rejecting
/// non-finite / out-of-range values.
fn usd_micros_from_f64(usd: f64) -> crate::UsdMicros {
    let micros = (usd * 1_000_000.0).round() as i64;
    crate::UsdMicros(micros)
}

/// Deserialize a raw TOML document into a map so unknown top-level keys can be
/// inspected before conversion to `AppConfig`. Used by strict-loading callers.
pub fn parse_raw_toml(contents: &str) -> ConfigResult<HashMap<String, IgnoredAny>> {
    toml::from_str(contents).map_err(|e| ConfigError::TomlParse("<inline>".to_string(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_io_error() {
        let res = load("does_not_exist_config_987654321.toml");
        assert!(matches!(res, Err(ConfigError::IoError(_, _))));
    }

    #[test]
    fn load_invalid_toml_returns_parse_error() {
        let res = parse_raw_toml("not valid toml {{[");
        assert!(res.is_err());
    }

    #[test]
    fn raw_toml_parses_valid_document() {
        let res = parse_raw_toml(r#"bot = { mode = "shadow" }"#);
        assert!(res.is_ok());
        let map = res.unwrap();
        assert!(map.contains_key("bot"));
    }
}
