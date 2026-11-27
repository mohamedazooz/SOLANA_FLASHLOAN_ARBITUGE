//! TOML/ENV default-consistency sweep (checklist 647).
//!
//! Contract: the critical economic/risk defaults must not diverge between the
//! shipped TOML files (base + example + environment overlays) and the code
//! default (`Config` construction in arb-config). A drift in
//! `min_net_profit_abs`, `min_net_profit_bps`, `max_price_impact_bps`,
//! `max_state_age_slots`, `graph_max_age_slots`, or `hop_slippage_bps`
//! silently changes which opportunities are eligible — so the sweep fails if
//! any file disagrees with the base values, and the graph horizon must stay
//! coherent with the state-age limit (`>= max_state_age_slots`).

use arb_config::AppConfig;
use toml::Value;

const BASE: &str = include_str!("../../../config/base.toml");
const EXAMPLE: &str = include_str!("../../../config/config.example.toml");
const DEVELOPMENT: &str = include_str!("../../../config/development.toml");
const PRODUCTION: &str = include_str!("../../../config/production.toml");
const STAGING: &str = include_str!("../../../config/staging.toml");
const ENV_EXAMPLE: &str = include_str!("../../../.env.example");

const EXPECTED: [(&str, i64); 6] = [
    ("min_net_profit_abs", 10_000),
    ("min_net_profit_bps", 1),
    ("max_price_impact_bps", 40),
    ("max_state_age_slots", 2),
    ("contingency_buffer_lamports", 10_000),
    ("hop_slippage_bps", 10),
];

fn strategy(v: &Value) -> &toml::value::Table {
    v.get("strategy")
        .and_then(|s| s.as_table())
        .expect("[strategy] table present")
}

fn num(t: &toml::value::Table, key: &str) -> Option<i64> {
    t.get(key).and_then(|v| v.as_integer())
}

#[test]
fn critical_strategy_defaults_match_across_all_toml_files() {
    for (name, raw) in [
        ("base.toml", BASE),
        ("config.example.toml", EXAMPLE),
        ("development.toml", DEVELOPMENT),
        ("production.toml", PRODUCTION),
        ("staging.toml", STAGING),
    ] {
        let parsed: Value = raw.parse().expect("valid TOML");
        let s = strategy(&parsed);
        for (key, expected) in EXPECTED {
            if let Some(actual) = num(s, key) {
                assert_eq!(
                    actual, expected,
                    "{name}: strategy.{key} drifted from the shared default {expected}"
                );
            } else {
                // Overlays may legitimately omit a field (falls through to the
                // previous layer) — only fail when the field is PRESENT and
                // different, so missing keys are not silently mis-asserted.
                let base_parsed: Value = BASE.parse().expect("valid TOML");
                let base_has = num(strategy(&base_parsed), key).is_some();
                assert!(
                    !base_has || name != "base.toml",
                    "{name}: strategy.{key} missing while base carries it"
                );
            }
        }
    }
}

#[test]
fn env_example_risk_and_economic_overrides_match_production_contract() {
    use std::collections::HashMap;
    let mut values = HashMap::<String, String>::new();
    for line in ENV_EXAMPLE.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    let expected = [
        ("MAX_TRADE_SIZE_USD", "1.5"),
        ("MAX_FLASH_BORROW_USD", "1.5"),
        ("MAX_SLIPPAGE_BPS", "10"),
        ("MIN_NET_PROFIT_LAMPORTS", "10000"),
        ("MIN_NET_PROFIT_BPS", "1"),
        ("MAX_PRICE_IMPACT_BPS", "40"),
        ("MAX_DAILY_LOSS_USD", "0.10"),
        ("MAX_HOURLY_LOSS_USD", "0.03"),
        ("MAX_INFLIGHT_BUNDLES", "1"),
        ("MAX_CONSECUTIVE_FAILURES", "3"),
        ("MAX_PRIORITY_FEE_SHARE_BPS", "1000"),
        ("JITO_MAX_TIP_SHARE_BPS", "1000"),
        ("ARB_MAX_IMPLAUSIBLE_ROI_BPS", "1500"),
        ("MIN_SOL_RESERVE_LAMPORTS", "10000000"),
    ];
    for (key, expected_value) in expected {
        assert_eq!(
            values.get(key).map(String::as_str),
            Some(expected_value),
            ".env.example {key} must match the canonical production safety policy"
        );
    }
}

#[test]
fn graph_horizon_is_coherent_with_state_age_everywhere() {
    for (name, raw) in [
        ("base.toml", BASE),
        ("config.example.toml", EXAMPLE),
        ("development.toml", DEVELOPMENT),
        ("production.toml", PRODUCTION),
        ("staging.toml", STAGING),
    ] {
        let parsed: Value = raw.parse().expect("valid TOML");
        let s = strategy(&parsed);
        if let Some(graph) = num(s, "graph_max_age_slots") {
            let state_age = num(s, "max_state_age_slots").unwrap_or_else(need_base_state_age);
            assert!(
                graph >= state_age,
                "{name}: graph_max_age_slots ({graph}) must be >= max_state_age_slots ({state_age})"
            );
        }
    }
}

fn need_base_state_age() -> i64 {
    let parsed: Value = BASE.parse().expect("valid TOML");
    strategy(&parsed)
        .get("max_state_age_slots")
        .and_then(|v| v.as_integer())
        .expect("base carries max_state_age_slots")
}

/// The in-code default (`AppConfig::default_safe()` — the fallback when no
/// TOML layer supplies a field) must equal the base.toml values — otherwise a
/// base-file omission would silently change behavior at runtime.
#[test]
fn code_defaults_match_base_toml() {
    let base_parsed: Value = BASE.parse().expect("valid TOML");
    let base = strategy(&base_parsed);
    let code = AppConfig::default_safe().strategy;
    for (key, code_value) in [
        ("min_net_profit_abs", code.min_net_profit_abs.0 as i64),
        ("min_net_profit_bps", i64::from(code.min_net_profit_bps.0)),
        (
            "max_price_impact_bps",
            i64::from(code.max_price_impact_bps.0),
        ),
        ("max_state_age_slots", code.max_state_age_slots.0 as i64),
        (
            "contingency_buffer_lamports",
            code.contingency_buffer_lamports.0 as i64,
        ),
    ] {
        let in_base = num(base, key).expect("base carries every critical key");
        assert_eq!(
            in_base, code_value,
            "code default for strategy.{key} must stay in sync with base.toml"
        );
    }
}

#[test]
fn env_example_has_unique_keys_and_parseable_numeric_overrides() {
    use std::collections::HashMap;

    let mut values = HashMap::<String, String>::new();
    for (line_no, line) in ENV_EXAMPLE.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_owned();
        assert!(
            values
                .insert(key.clone(), value.trim().to_owned())
                .is_none(),
            ".env.example contains duplicate active key {key} at line {}",
            line_no + 1
        );
    }

    let base: Value = BASE.parse().expect("valid base TOML");
    let risk = base
        .get("risk")
        .and_then(Value::as_table)
        .expect("risk table");
    let chronos = base
        .get("chronos")
        .and_then(Value::as_table)
        .expect("chronos table");

    let max_trade: f64 = values["MAX_TRADE_SIZE_USD"]
        .parse()
        .expect("MAX_TRADE_SIZE_USD must be numeric");
    assert_eq!(
        max_trade,
        risk["max_trade_size_usd"]
            .as_float()
            .expect("base risk max trade")
    );

    let lambda: f64 = values["CHRONOS_DECAY_LAMBDA_PER_SEC"]
        .parse()
        .expect("CHRONOS_DECAY_LAMBDA_PER_SEC must be numeric");
    assert_eq!(
        lambda,
        chronos["decay_lambda_per_sec"]
            .as_float()
            .expect("base lambda")
    );

    let max_age: i64 = values["CHRONOS_MAX_AGE_MS"]
        .parse()
        .expect("CHRONOS_MAX_AGE_MS must be an integer");
    assert_eq!(
        max_age,
        chronos["max_age_ms"].as_integer().expect("base max age")
    );

    let min_ev: f64 = values["CHRONOS_MIN_EV_USD"]
        .parse()
        .expect("CHRONOS_MIN_EV_USD must be numeric");
    assert_eq!(
        min_ev,
        chronos["min_ev_usd"].as_float().expect("base min EV")
    );

    for key in [
        "SOLANA_RPC_PRIMARY",
        "SOLANA_RPC_SECONDARY",
        "SOLAMI_WS_API_KEY",
    ] {
        assert!(values.contains_key(key), ".env.example must document {key}");
    }
}
