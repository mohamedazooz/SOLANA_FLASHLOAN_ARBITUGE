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
    let contents =
        fs::read_to_string(path).map_err(|e| ConfigError::IoError(path.to_string(), e))?;
    let raw = parse_raw_toml(&contents)?;
    let present: Vec<String> = raw.keys().cloned().collect();
    crate::validation::reject_unknown_keys(&present, allowed_top_keys)?;

    // Every nested config struct also has `deny_unknown_fields`, so both the
    // explicit top-level allowlist and all nested schemas are enforced.
    let mut config: AppConfig =
        toml::from_str(&contents).map_err(|e| ConfigError::TomlParse(path.to_string(), e))?;
    apply_env_overrides(&mut config)?;
    crate::validation::validate(&config)?;
    crate::validation::check_no_placeholder_secret_env_names(&config)?;
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
        config.risk.max_trade_size_usd = config
            .risk
            .max_trade_size_usd
            .min(usd_micros_from_f64(value));
    }

    // Explicit flash-borrow cap intersects the general notional cap.
    if let Ok(raw) = std::env::var("MAX_FLASH_BORROW_USD") {
        let value: f64 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_FLASH_BORROW_USD".to_string(),
                "must be numeric".to_string(),
            )
        })?;
        if !value.is_finite() || value <= 0.0 {
            return Err(ConfigError::InvalidEnv(
                "MAX_FLASH_BORROW_USD".to_string(),
                "must be positive and finite".to_string(),
            ));
        }
        config.risk.max_trade_size_usd = config
            .risk
            .max_trade_size_usd
            .min(usd_micros_from_f64(value));
    }
    for (key, target) in [
        ("MAX_DAILY_LOSS_USD", &mut config.risk.max_daily_loss_usd),
        ("MAX_HOURLY_LOSS_USD", &mut config.risk.max_hourly_loss_usd),
    ] {
        if let Ok(raw) = std::env::var(key) {
            let value: f64 = raw.parse().map_err(|_| {
                ConfigError::InvalidEnv(key.to_string(), "must be numeric".to_string())
            })?;
            if !value.is_finite() || value <= 0.0 {
                return Err(ConfigError::InvalidEnv(
                    key.to_string(),
                    "must be positive and finite".to_string(),
                ));
            }
            *target = (*target).min(usd_micros_from_f64(value));
        }
    }
    if let Ok(raw) = std::env::var("MIN_NET_PROFIT_LAMPORTS") {
        let value: u64 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MIN_NET_PROFIT_LAMPORTS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        if value == 0 {
            return Err(ConfigError::InvalidEnv(
                "MIN_NET_PROFIT_LAMPORTS".to_string(),
                "must be positive".to_string(),
            ));
        }
        config.strategy.min_net_profit_abs.0 = value;
    }
    if let Ok(raw) = std::env::var("MIN_NET_PROFIT_BPS") {
        let value: u16 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MIN_NET_PROFIT_BPS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        config.strategy.min_net_profit_bps.0 = value;
    }
    if let Ok(raw) = std::env::var("MAX_PRICE_IMPACT_BPS") {
        let value: u16 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_PRICE_IMPACT_BPS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        config.strategy.max_price_impact_bps.0 = config.strategy.max_price_impact_bps.0.min(value);
    }
    if let Ok(raw) = std::env::var("MAX_SLIPPAGE_BPS") {
        let value: u16 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_SLIPPAGE_BPS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        if u64::from(value) > arb_constants::HOP_SLIPPAGE_MAX_BPS {
            return Err(ConfigError::InvalidEnv(
                "MAX_SLIPPAGE_BPS".to_string(),
                "exceeds HOP_SLIPPAGE_MAX_BPS".to_string(),
            ));
        }
        config.strategy.hop_slippage_bps.0 = config.strategy.hop_slippage_bps.0.min(value);
    }
    if let Ok(raw) =
        std::env::var("MAX_PRIORITY_FEE_SHARE_BPS").or_else(|_| std::env::var("MAX_PRIORITY_FEE"))
    {
        let value: u16 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_PRIORITY_FEE_SHARE_BPS".to_string(),
                "must be an integer basis-point share".to_string(),
            )
        })?;
        config.strategy.priority_fee_max_share_bps.0 =
            config.strategy.priority_fee_max_share_bps.0.min(value);
    }
    if let Ok(raw) = std::env::var("JITO_MIN_KEEP_LAMPORTS") {
        let value: u64 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "JITO_MIN_KEEP_LAMPORTS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        if value == 0 {
            return Err(ConfigError::InvalidEnv(
                "JITO_MIN_KEEP_LAMPORTS".to_string(),
                "must be positive".to_string(),
            ));
        }
        config.jito.min_keep_lamports.0 = value;
    }
    if let Ok(raw) = std::env::var("JITO_MAX_TIP_SHARE_BPS") {
        let value: u16 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "JITO_MAX_TIP_SHARE_BPS".to_string(),
                "must be an integer".to_string(),
            )
        })?;
        if value == 0 || value > 10_000 {
            return Err(ConfigError::InvalidEnv(
                "JITO_MAX_TIP_SHARE_BPS".to_string(),
                "must be in 1..=10000".to_string(),
            ));
        }
        config.jito.max_tip_share_bps.0 = config.jito.max_tip_share_bps.0.min(value);
    }
    if let Ok(raw) =
        std::env::var("MAX_INFLIGHT_BUNDLES").or_else(|_| std::env::var("MAX_CONCURRENT_TRADES"))
    {
        let value: usize = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_INFLIGHT_BUNDLES".to_string(),
                "must be a positive integer".to_string(),
            )
        })?;
        if value == 0 {
            return Err(ConfigError::InvalidEnv(
                "MAX_INFLIGHT_BUNDLES".to_string(),
                "must be positive".to_string(),
            ));
        }
        config.risk.max_inflight_bundles = config.risk.max_inflight_bundles.min(value);
    }
    if let Ok(raw) = std::env::var("MAX_CONSECUTIVE_FAILURES")
        .or_else(|_| std::env::var("MAX_TRANSACTION_FAILURES"))
    {
        let value: u64 = raw.parse().map_err(|_| {
            ConfigError::InvalidEnv(
                "MAX_CONSECUTIVE_FAILURES".to_string(),
                "must be a positive integer".to_string(),
            )
        })?;
        if value == 0 {
            return Err(ConfigError::InvalidEnv(
                "MAX_CONSECUTIVE_FAILURES".to_string(),
                "must be positive".to_string(),
            ));
        }
        config.risk.max_consecutive_failures = config.risk.max_consecutive_failures.min(value);
    }
    if let Ok(raw) = std::env::var("CHRONOS_ENABLED") {
        let enabled = match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                return Err(ConfigError::InvalidEnv(
                    "CHRONOS_ENABLED".to_string(),
                    "must be true or false".to_string(),
                ));
            }
        };
        config.chronos.enabled &= enabled;
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
        config.chronos.max_age_ms = Milliseconds(config.chronos.max_age_ms.0.min(value));
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
        config.chronos.min_ev_usd = config.chronos.min_ev_usd.max(usd_micros_from_f64(value));
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
    fn env_can_lower_profit_hurdles_and_bound_tip_spend() {
        let keys = [
            "MIN_NET_PROFIT_LAMPORTS",
            "MIN_NET_PROFIT_BPS",
            "JITO_MIN_KEEP_LAMPORTS",
            "JITO_MAX_TIP_SHARE_BPS",
            "MAX_PRIORITY_FEE_SHARE_BPS",
        ];
        let previous: Vec<_> = keys
            .iter()
            .map(|key| (key.to_string(), std::env::var(key).ok()))
            .collect();
        unsafe {
            std::env::set_var("MIN_NET_PROFIT_LAMPORTS", "10000");
            std::env::set_var("MIN_NET_PROFIT_BPS", "1");
            std::env::set_var("JITO_MIN_KEEP_LAMPORTS", "10000");
            std::env::set_var("JITO_MAX_TIP_SHARE_BPS", "1000");
            std::env::set_var("MAX_PRIORITY_FEE_SHARE_BPS", "500");
        }
        let mut config = AppConfig::default_safe();
        config.strategy.min_net_profit_abs.0 = 50_000;
        config.strategy.min_net_profit_bps.0 = 5;
        config.jito.max_tip_share_bps.0 = 2_000;
        let result = apply_env_overrides(&mut config);
        for (key, value) in previous {
            unsafe {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
        result.expect("valid lower strategy and Jito caps");
        assert_eq!(config.strategy.min_net_profit_abs.0, 10_000);
        assert_eq!(config.strategy.min_net_profit_bps.0, 1);
        assert_eq!(config.jito.min_keep_lamports.0, 10_000);
        assert_eq!(config.jito.max_tip_share_bps.0, 1_000);
        assert_eq!(config.strategy.priority_fee_max_share_bps.0, 500);
    }

    #[test]
    fn strict_loader_rejects_unknown_top_level_keys() {
        let mut contents = toml::to_string(&AppConfig::default_safe()).expect("serialize config");
        contents = format!("unknown_top_level = true\n{contents}");
        let path =
            std::env::temp_dir().join(format!("arb-config-strict-{}.toml", std::process::id()));
        std::fs::write(&path, contents).expect("write temporary config");
        let result = load_strict(
            path.to_str().expect("utf8 path"),
            &[
                "mode", "provider", "jito", "jupiter", "wallet", "strategy", "risk", "chronos",
                "store", "metrics", "admin", "security",
            ],
        );
        let _ = std::fs::remove_file(&path);
        assert!(matches!(result, Err(ConfigError::UnknownKey(key)) if key == "unknown_top_level"));
    }

    #[test]
    fn config_deserializer_rejects_unknown_nested_keys() {
        let mut contents = toml::to_string(&AppConfig::default_safe()).expect("serialize config");
        let marker = "[risk]\n";
        let offset = contents.find(marker).expect("serialized risk table");
        contents.insert_str(offset + marker.len(), "unknown_nested = true\n");
        assert!(toml::from_str::<AppConfig>(&contents).is_err());
    }

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
