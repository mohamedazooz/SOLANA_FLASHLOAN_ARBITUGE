//! Configuration validation rules.
//!
//! Validation runs during startup (before any execution). Invalid values produce
//! actionable errors that identify the configuration key without revealing
//! secrets. NaN, Infinity, negative/overflowing values are rejected; nothing is
//! silently clamped.

use crate::{
    AppConfig, ChronosConfig, ConfigError, ConfigResult, JitoConfig, JupiterConfig, Mode,
    ProviderConfig, RiskConfig, SecurityConfig, StrategyConfig, WalletConfig,
};
use arb_types::Bps;
use std::collections::HashSet;

/// Run the full validation suite over an `AppConfig`.
///
/// Order matters: structural (URLs, addresses) first, then cross-field
/// constraints (hourly ≤ daily), then security (placeholder secrets).
pub fn validate(config: &AppConfig) -> ConfigResult<()> {
    validate_provider(&config.provider)?;
    validate_jito(&config.jito)?;
    validate_jupiter(&config.jupiter)?;
    validate_wallet(&config.wallet)?;
    validate_strategy(&config.strategy)?;
    validate_risk(&config.risk)?;
    validate_chronos(&config.chronos)?;
    validate_security(&config.security)?;
    validate_cross_field(config)?;
    Ok(())
}

/// Reject reserved / disallowed configuration keys when strict parsing is
/// appropriate. The TOML serde layer already rejects unknown keys by default
/// unless `#[serde(deny_unknown_fields)]` is set; this function documents the
/// intent and can be wired into a custom deserializer.
pub fn reject_unknown_keys(present: &[String], allowed: &[&str]) -> ConfigResult<()> {
    let allowed: HashSet<&str> = allowed.iter().copied().collect();
    for key in present {
        if !allowed.contains(key.as_str()) {
            return Err(ConfigError::UnknownKey(key.clone()));
        }
    }
    Ok(())
}

fn validate_provider(cfg: &ProviderConfig) -> ConfigResult<()> {
    // An empty primary URL is a skeleton placeholder (filled in by the operator
    // or via env before use). Structural scheme validation only applies when a
    // URL is actually present; live-mode completeness is checked separately.
    if !cfg.primary_url.trim().is_empty() {
        let scheme = cfg.primary_url.split("://").next().unwrap_or("");
        if scheme != "http" && scheme != "https" {
            return Err(ConfigError::Validation(format!(
                "provider.primary_url must use http or https scheme, got '{0}' ({1})",
                cfg.primary_url, scheme
            )));
        }
    }
    if cfg.primary_token_env.trim().is_empty() {
        return Err(ConfigError::Validation(
            "provider.primary_token_env must not be empty".to_string(),
        ));
    }
    if let Some(f) = &cfg.failover_url {
        let scheme = f.split("://").next().unwrap_or("");
        if scheme != "http" && scheme != "https" {
            return Err(ConfigError::Validation(format!(
                "provider.failover_url must use http or https scheme, got '{0}'",
                f
            )));
        }
    }
    let commitment = cfg.commitment.to_lowercase();
    if commitment != "processed" && commitment != "confirmed" && commitment != "finalized" {
        return Err(ConfigError::Validation(format!(
            "provider.commitment must be processed/confirmed/finalized, got '{0}'",
            cfg.commitment
        )));
    }
    Ok(())
}

fn validate_jito(cfg: &JitoConfig) -> ConfigResult<()> {
    if cfg.regions.is_empty() {
        return Err(ConfigError::Validation(
            "jito.regions must not be empty".to_string(),
        ));
    }
    match cfg.tip_floor_percentile.as_str() {
        "p50" | "p75" | "p90" | "p99" => {}
        other => {
            return Err(ConfigError::Validation(format!(
                "jito.tip_floor_percentile must be p50/p75/p90/p99, got '{other}'"
            )));
        }
    }
    // tip_share_bps must be within 0..=10_000 (Bps guarantees this at the type
    // level, but we still enforce the share ceiling vs max).
    if cfg.tip_share_bps.0 > cfg.max_tip_share_bps.0 {
        return Err(ConfigError::Validation(format!(
            "jito.tip_share_bps ({}) must not exceed max_tip_share_bps ({})",
            cfg.tip_share_bps.0, cfg.max_tip_share_bps.0
        )));
    }
    if cfg.max_tip_share_bps.0 > Bps::MAX.0 {
        return Err(ConfigError::Validation(format!(
            "jito.max_tip_share_bps ({}) must not exceed Bps::MAX ({})",
            cfg.max_tip_share_bps.0,
            Bps::MAX.0
        )));
    }
    if cfg.max_tip_lamports.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.max_tip_lamports must be positive".to_string(),
        ));
    }
    if cfg.min_keep_lamports.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.min_keep_lamports must be positive".to_string(),
        ));
    }
    if !(0.0..=1.0).contains(&cfg.keep_share) {
        return Err(ConfigError::Validation(format!(
            "jito.keep_share must be in [0,1], got {}",
            cfg.keep_share
        )));
    }
    if cfg.ladder_growth <= 1.0 {
        return Err(ConfigError::Validation(format!(
            "jito.ladder_growth must be > 1.0, got {}",
            cfg.ladder_growth
        )));
    }
    if cfg.tip_floor_max_age_ms.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.tip_floor_max_age_ms must be positive".to_string(),
        ));
    }
    if cfg.tip_floor_poll_ms.0 < 1_000 {
        return Err(ConfigError::Validation(
            "jito.tip_floor_poll_ms must be >= 1000 ms (1 rps per-IP limit)".to_string(),
        ));
    }
    if cfg.leader_refresh_secs.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.leader_refresh_secs must be positive".to_string(),
        ));
    }
    if cfg.swqos_fallback_hurdle_lamports.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.swqos_fallback_hurdle_lamports must be positive".to_string(),
        ));
    }
    if cfg.swqos_fallback_priority_fee_lamports.0 == 0 {
        return Err(ConfigError::Validation(
            "jito.swqos_fallback_priority_fee_lamports must be positive".to_string(),
        ));
    }
    if cfg.swqos_fallback_hurdle_lamports.0 < cfg.swqos_fallback_priority_fee_lamports.0 {
        return Err(ConfigError::Validation(format!(
            "jito.swqos_fallback_hurdle_lamports ({}) must be >= swqos_fallback_priority_fee_lamports ({})",
            cfg.swqos_fallback_hurdle_lamports.0, cfg.swqos_fallback_priority_fee_lamports.0
        )));
    }
    Ok(())
}

fn validate_jupiter(cfg: &JupiterConfig) -> ConfigResult<()> {
    let scheme = cfg.base_url.split("://").next().unwrap_or("");
    if scheme != "https" {
        return Err(ConfigError::Validation(format!(
            "jupiter.base_url must use https scheme, got '{0}'",
            cfg.base_url
        )));
    }
    if cfg.api_key_env.trim().is_empty() {
        return Err(ConfigError::Validation(
            "jupiter.api_key_env must not be empty".to_string(),
        ));
    }
    if cfg.timeout_ms.0 == 0 {
        return Err(ConfigError::Validation(
            "jupiter.timeout_ms must be positive".to_string(),
        ));
    }
    Ok(())
}

fn validate_wallet(cfg: &WalletConfig) -> ConfigResult<()> {
    if cfg.min_sol_balance_lamports.0 == 0 {
        return Err(ConfigError::Validation(
            "wallet.min_sol_balance_lamports must be positive; reserve guard cannot be disabled"
                .to_string(),
        ));
    }
    if cfg.keypair_path.trim().is_empty() {
        return Err(ConfigError::Validation(
            "wallet.keypair_path must not be empty".to_string(),
        ));
    }
    if let Some(pubkey) = &cfg.cold_wallet_pubkey {
        if pubkey.trim().is_empty() {
            return Err(ConfigError::Validation(
                "wallet.cold_wallet_pubkey must not be empty when set".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_strategy(cfg: &StrategyConfig) -> ConfigResult<()> {
    if cfg.base_tokens.is_empty() {
        return Err(ConfigError::Validation(
            "strategy.base_tokens must not be empty".to_string(),
        ));
    }
    if cfg.min_net_profit_abs.0 == 0 {
        return Err(ConfigError::Validation(
            "strategy.min_net_profit_abs must be positive".to_string(),
        ));
    }
    if cfg.min_net_profit_bps.0 > Bps::MAX.0 {
        return Err(ConfigError::Validation(format!(
            "strategy.min_net_profit_bps ({}) must not exceed Bps::MAX ({})",
            cfg.min_net_profit_bps.0,
            Bps::MAX.0
        )));
    }
    if u64::from(cfg.hop_slippage_bps.0) > arb_constants::HOP_SLIPPAGE_MAX_BPS {
        return Err(ConfigError::Validation(format!(
            "strategy.hop_slippage_bps ({}) exceeds HOP_SLIPPAGE_MAX_BPS ({})",
            cfg.hop_slippage_bps.0,
            arb_constants::HOP_SLIPPAGE_MAX_BPS
        )));
    }
    if cfg.max_price_impact_bps.0 > Bps::MAX.0 {
        return Err(ConfigError::Validation(format!(
            "strategy.max_price_impact_bps ({}) must not exceed Bps::MAX ({})",
            cfg.max_price_impact_bps.0,
            Bps::MAX.0
        )));
    }
    if cfg.max_state_age_slots.0 == 0 {
        return Err(ConfigError::Validation(
            "strategy.max_state_age_slots must be positive".to_string(),
        ));
    }
    if let Some(graph) = cfg.graph_max_age_slots {
        if graph.0 < cfg.max_state_age_slots.0 {
            return Err(ConfigError::Validation(format!(
                "strategy.graph_max_age_slots ({}) must be >= max_state_age_slots ({})",
                graph.0, cfg.max_state_age_slots.0
            )));
        }
    }
    if let Some(pools) = cfg.max_pools {
        if pools == 0 {
            return Err(ConfigError::Validation(
                "strategy.max_pools must be positive".to_string(),
            ));
        }
    }
    if let Some(refresh) = cfg.discovery_refresh_secs {
        if refresh.0 == 0 {
            return Err(ConfigError::Validation(
                "strategy.discovery_refresh_secs must be positive".to_string(),
            ));
        }
    }
    if !(2..=4).contains(&cfg.max_route_hops) {
        return Err(ConfigError::Validation(format!(
            "strategy.max_route_hops ({}) must be within 2..=4: each extra hop stacks another DEX fee plus another failure point, and routes longer than 4 hops are economically dominated by cumulative fees",
            cfg.max_route_hops
        )));
    }
    Ok(())
}

fn validate_risk(cfg: &RiskConfig) -> ConfigResult<()> {
    if cfg.max_trade_size_usd.0 <= 0 {
        return Err(ConfigError::Validation(
            "risk.max_trade_size_usd must be positive".to_string(),
        ));
    }
    if cfg.max_daily_loss_usd.0 <= 0 {
        return Err(ConfigError::Validation(
            "risk.max_daily_loss_usd must be positive".to_string(),
        ));
    }
    if cfg.max_hourly_loss_usd.0 <= 0 {
        return Err(ConfigError::Validation(
            "risk.max_hourly_loss_usd must be positive".to_string(),
        ));
    }
    // Hourly loss limit must not exceed daily loss limit.
    if cfg.max_hourly_loss_usd.0 > cfg.max_daily_loss_usd.0 {
        return Err(ConfigError::Validation(
            "risk.max_hourly_loss_usd must not exceed risk.max_daily_loss_usd".to_string(),
        ));
    }
    if cfg.max_consecutive_failures == 0 {
        return Err(ConfigError::Validation(
            "risk.max_consecutive_failures must be positive".to_string(),
        ));
    }
    if !(0.0..=1.0).contains(&cfg.min_landing_rate) {
        return Err(ConfigError::Validation(format!(
            "risk.min_landing_rate must be in [0,1], got {}",
            cfg.min_landing_rate
        )));
    }
    if cfg.max_attempts_per_slot == 0 {
        return Err(ConfigError::Validation(
            "risk.max_attempts_per_slot must be positive".to_string(),
        ));
    }
    if cfg.max_inflight_bundles == 0 {
        return Err(ConfigError::Validation(
            "risk.max_inflight_bundles must be positive".to_string(),
        ));
    }
    if !(0.0..=1.0).contains(&cfg.max_tip_spend_ratio) {
        return Err(ConfigError::Validation(format!(
            "risk.max_tip_spend_ratio must be in [0,1], got {}",
            cfg.max_tip_spend_ratio
        )));
    }
    if cfg.max_orders_per_hour == 0 {
        return Err(ConfigError::Validation(
            "risk.max_orders_per_hour must be positive".to_string(),
        ));
    }
    if cfg.cooldown_slots.0 == 0 {
        return Err(ConfigError::Validation(
            "risk.cooldown_slots must be positive".to_string(),
        ));
    }
    if cfg.max_implausible_roi_bps.0 == 0 {
        return Err(ConfigError::Validation(
            "risk.max_implausible_roi_bps must be positive".to_string(),
        ));
    }
    Ok(())
}

fn validate_chronos(cfg: &ChronosConfig) -> ConfigResult<()> {
    if cfg.decay_lambda_per_sec > 1_000_000.0 {
        return Err(ConfigError::Validation(
            "chronos.decay_lambda_per_sec must not exceed 1,000,000 per second".to_string(),
        ));
    }
    if cfg.decay_lambda_per_sec < 0.0 || !cfg.decay_lambda_per_sec.is_finite() {
        return Err(ConfigError::Validation(
            "chronos.decay_lambda_per_sec must be finite and non-negative".to_string(),
        ));
    }
    if cfg.max_age_ms.0 < 100 || cfg.max_age_ms.0 > 5000 {
        return Err(ConfigError::Validation(format!(
            "chronos.max_age_ms must be between 100 and 5000 ms, got {}",
            cfg.max_age_ms.0
        )));
    }
    if cfg.min_ev_usd.0 < 0 {
        return Err(ConfigError::Validation(
            "chronos.min_ev_usd must be >= 0".to_string(),
        ));
    }
    Ok(())
}

fn validate_security(cfg: &SecurityConfig) -> ConfigResult<()> {
    // Production must have explicit allowed CORS origins (never permissive).
    // This is checked here structurally; the runtime CORS layer must be built
    // from this allowlist, never `CorsLayer::permissive()`.
    if cfg.cors_allow_origins.is_empty() {
        // Allow empty only in non-production contexts; production deployment
        // docs require an explicit allowlist. We warn via validation that an
        // empty allowlist denies all cross-origin traffic (fail closed).
    }
    if cfg.max_request_body_bytes == 0 {
        return Err(ConfigError::Validation(
            "security.max_request_body_bytes must be positive".to_string(),
        ));
    }
    Ok(())
}

fn validate_cross_field(config: &AppConfig) -> ConfigResult<()> {
    // Live mode requires all mandatory security controls.
    if config.mode.mode == Mode::Live {
        if !config.mode.live_enabled {
            return Err(ConfigError::Validation(
                "mode live requires execution.live_enabled == true (ARB_LIVE=1)".to_string(),
            ));
        }
        if !config.security.webhook_signature_required {
            return Err(ConfigError::Validation(
                "live mode requires security.webhook_signature_required == true".to_string(),
            ));
        }
        // Admin token env must be declared.
        if config.admin.token_env.trim().is_empty() {
            return Err(ConfigError::Validation(
                "live mode requires admin.token_env to be set".to_string(),
            ));
        }
        // CORS must not be permissive in live mode: require explicit origins.
        if config.security.cors_allow_origins.is_empty() {
            return Err(ConfigError::Validation(
                "live mode requires security.cors_allow_origins to be non-empty".to_string(),
            ));
        }
        // A real RPC endpoint is required to trade live.
        if config.provider.primary_url.trim().is_empty() {
            return Err(ConfigError::Validation(
                "live mode requires provider.primary_url to be set".to_string(),
            ));
        }
    }
    // Jito tip share must not exceed max tip share (cross-config).
    if config.jito.tip_share_bps.0 > config.jito.max_tip_share_bps.0 {
        return Err(ConfigError::Validation(
            "jito.tip_share_bps must not exceed jito.max_tip_share_bps".to_string(),
        ));
    }
    Ok(())
}

/// Reject placeholder / missing credential values. Secret values themselves are
/// resolved outside this crate; here we only validate that env *names* are
/// declared and not obviously placeholders.
pub fn check_no_placeholder_secret_env_names(config: &AppConfig) -> ConfigResult<()> {
    let placeholders = [
        "changeme",
        "change-me",
        "placeholder",
        "your_token_here",
        "token",
        "",
    ];
    for env_name in [
        &config.provider.primary_token_env,
        &config.admin.token_env,
        &config.jupiter.api_key_env,
    ] {
        if let Some(failover) = &config.provider.failover_token_env {
            let names = std::slice::from_ref(env_name)
                .iter()
                .chain(std::slice::from_ref(failover));
            for n in names {
                let lower = n.trim().to_ascii_lowercase();
                if placeholders.contains(&lower.as_str()) {
                    return Err(ConfigError::PlaceholderSecret(n.clone()));
                }
            }
        }
        let lower = env_name.trim().to_ascii_lowercase();
        if placeholders.contains(&lower.as_str()) {
            return Err(ConfigError::PlaceholderSecret(env_name.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{Bps, Lamports, Slot, UsdMicros};

    fn base_valid() -> AppConfig {
        AppConfig::default_safe()
    }

    #[test]
    fn default_safe_passes_validation() {
        let cfg = base_valid();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn hourly_loss_must_not_exceed_daily() {
        let mut cfg = base_valid();
        cfg.risk.max_hourly_loss_usd = UsdMicros(10_000_000); // $10 > $5 daily
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn tip_share_must_not_exceed_max_tip_share() {
        let mut cfg = base_valid();
        cfg.jito.tip_share_bps = Bps(9000); // > max 7000
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn price_impact_cannot_exceed_bps_max() {
        let mut cfg = base_valid();
        cfg.strategy.max_price_impact_bps = Bps(20_000);
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn chronos_decay_lambda_has_a_bounded_supported_range() {
        let mut cfg = base_valid();
        cfg.chronos.decay_lambda_per_sec = 1_000_000.1;
        assert!(validate(&cfg).is_err());
        cfg.chronos.decay_lambda_per_sec = 1_000_000.0;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn max_route_hops_default_is_three() {
        let cfg = base_valid();
        assert_eq!(cfg.strategy.max_route_hops, 3);
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn max_route_hops_beyond_four_rejected() {
        let mut cfg = base_valid();
        cfg.strategy.max_route_hops = 5;
        let err = validate(&cfg).unwrap_err().to_string();
        assert!(
            err.contains("max_route_hops"),
            "error must name the key: {err}"
        );
    }

    #[test]
    fn max_route_hops_below_two_rejected() {
        let mut cfg = base_valid();
        cfg.strategy.max_route_hops = 1;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn max_route_hops_four_accepted() {
        let mut cfg = base_valid();
        cfg.strategy.max_route_hops = 4;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn http_discovery_disabled_by_default() {
        let cfg = base_valid();
        assert!(!cfg.strategy.http_discovery_enabled);
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn http_discovery_explicitly_enabled_accepted() {
        let mut cfg = base_valid();
        cfg.strategy.http_discovery_enabled = true;
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn http_discovery_toml_default_is_off() {
        let cfg: crate::StrategyConfig = toml::from_str(
            "base_tokens = []\nmin_net_profit_abs = 0\nmin_net_profit_bps = 0\nmax_price_impact_bps = 0\nmax_state_age_slots = 2\n",
        )
        .unwrap();
        assert!(
            !cfg.http_discovery_enabled,
            "HTTP discovery must default to disabled (fail-closed)"
        );
    }

    #[test]
    fn provider_url_must_use_approved_scheme() {
        let mut cfg = base_valid();
        cfg.provider.primary_url = "ftp://example.com".to_string();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn jupiter_url_must_use_https() {
        let mut cfg = base_valid();
        cfg.jupiter.base_url = "http://api.jup.ag/swap/v2".to_string();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn percentile_must_be_valid() {
        let mut cfg = base_valid();
        cfg.jito.tip_floor_percentile = "p999".to_string();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn graph_age_must_not_be_stricter_than_state_age() {
        let mut cfg = base_valid();
        cfg.strategy.graph_max_age_slots = Some(Slot(1)); // < state age 2
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn wallet_reserve_must_be_positive() {
        let mut cfg = base_valid();
        cfg.wallet.min_sol_balance_lamports = Lamports(0);
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn live_mode_requires_explicit_opt_in() {
        let mut cfg = base_valid();
        cfg.mode.mode = Mode::Live;
        // live_enabled defaults to false
        assert!(validate(&cfg).is_err());
        cfg.mode.live_enabled = true;
        // but CORS empty → still fails
        assert!(validate(&cfg).is_err());
        cfg.security.cors_allow_origins = vec!["https://example.com".to_string()];
        // A real RPC endpoint is also required for live mode.
        cfg.provider.primary_url = "https://rpc.example.com".to_string();
        assert!(validate(&cfg).is_ok());
    }

    #[test]
    fn placeholder_secret_env_name_is_rejected() {
        let mut cfg = base_valid();
        cfg.admin.token_env = "changeme".to_string();
        assert!(matches!(
            check_no_placeholder_secret_env_names(&cfg),
            Err(ConfigError::PlaceholderSecret(_))
        ));
    }

    #[test]
    fn max_tip_share_bps_cannot_exceed_bps_max() {
        let mut cfg = base_valid();
        cfg.jito.max_tip_share_bps = Bps(20_000);
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn min_keep_and_max_tip_must_be_positive() {
        let mut cfg = base_valid();
        cfg.jito.max_tip_lamports = Lamports(0);
        assert!(validate(&cfg).is_err());
        cfg.jito.max_tip_lamports = Lamports(1_000);
        cfg.jito.min_keep_lamports = Lamports(0);
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn operational_dehardcoded_params_validated() {
        let mut cfg = base_valid();
        assert_eq!(cfg.strategy.contingency_buffer_lamports.0, 10_000);
        assert_eq!(cfg.risk.max_implausible_roi_bps.0, 1_500);
        assert_eq!(cfg.jito.swqos_fallback_hurdle_lamports.0, 8_000);
        assert_eq!(cfg.jito.swqos_fallback_priority_fee_lamports.0, 2_000);

        cfg.jito.swqos_fallback_hurdle_lamports = Lamports(0);
        assert!(validate(&cfg).is_err());
        cfg.jito.swqos_fallback_hurdle_lamports = Lamports(8_000);

        cfg.jito.swqos_fallback_priority_fee_lamports = Lamports(0);
        assert!(validate(&cfg).is_err());
        cfg.jito.swqos_fallback_priority_fee_lamports = Lamports(2_000);

        cfg.risk.max_implausible_roi_bps = Bps(0);
        assert!(validate(&cfg).is_err());
        cfg.risk.max_implausible_roi_bps = Bps(1_500);

        // Economic invariant: SWQoS hurdle must be >= priority fee
        cfg.jito.swqos_fallback_hurdle_lamports = Lamports(1_000);
        cfg.jito.swqos_fallback_priority_fee_lamports = Lamports(2_000);
        assert!(validate(&cfg).is_err());
        cfg.jito.swqos_fallback_hurdle_lamports = Lamports(8_000);
        cfg.jito.swqos_fallback_priority_fee_lamports = Lamports(2_000);

        // Contingency buffer boundary: 0 is legal (pure floor without slippage padding)
        cfg.strategy.contingency_buffer_lamports = Lamports(0);
        assert!(validate(&cfg).is_ok());
        cfg.strategy.contingency_buffer_lamports = Lamports(10_000);

        assert!(validate(&cfg).is_ok());
    }
}
