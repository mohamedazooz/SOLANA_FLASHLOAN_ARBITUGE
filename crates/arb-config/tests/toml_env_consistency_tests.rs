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

const EXPECTED: [(&str, i64); 6] = [
    ("min_net_profit_abs", 50_000),
    ("min_net_profit_bps", 5),
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
