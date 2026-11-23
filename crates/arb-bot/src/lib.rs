//! Shared library components for `arb-bot` and its diagnostic binaries.

pub mod config;
pub mod control_service;
pub mod daily_report;
pub mod errors;
pub mod health_monitor;
pub mod heartbeat;
pub mod jito_service;
pub mod supervisor;
pub mod telegram;
pub mod telegram_control;
pub mod verified_config;
pub mod web_api;

/// Supported operational modes for the bot.
///
/// Mode is controlled by environment (`ARB_MODE`, plus explicit arming flags).
/// - Observe: Listens to public WebSocket streams, tracks pools, decodes states.
/// - Analyze: Discovers opportunities, analyzes depth, pricing, liquidity, and fees without simulating or dispatching.
/// - Simulate: Builds atomic transactions and executes preflight simulation against RPC.
/// - PaperTrade: Simulates transactions and records hypothetical outcomes in the durable store.
/// - Live: Full execution on chain. Additionally requires `ARB_LIVE=1` or `LIVE_TRADING=1`.
/// - Canary: Non-broadcast verification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotMode {
    Observe = 0,
    Live = 1,
    Canary = 2,
    Analyze = 3,
    PaperTrade = 4,
    Simulate = 5,
}

impl BotMode {
    /// Runtime mode transitions go through a shared `AtomicU8` so the unified
    /// control service can switch modes without a restart.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => BotMode::Observe,
            1 => BotMode::Live,
            2 => BotMode::Canary,
            3 => BotMode::Analyze,
            4 => BotMode::PaperTrade,
            _ => BotMode::Simulate,
        }
    }
}

impl std::str::FromStr for BotMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "observe" => Ok(BotMode::Observe),
            "shadow" => Ok(BotMode::Simulate),
            "analyze" => Ok(BotMode::Analyze),
            "simulate" => Ok(BotMode::Simulate),
            "paper" | "paper_trade" | "papertrade" => Ok(BotMode::PaperTrade),
            "live" => Ok(BotMode::Live),
            "canary" => Ok(BotMode::Canary),
            other => {
                anyhow::bail!(
                    "Unknown bot mode: '{other}'. Allowed modes: shadow, observe, analyze, simulate, paper_trade, live, canary"
                )
            }
        }
    }
}

impl std::fmt::Display for BotMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BotMode::Observe => write!(f, "observe"),
            BotMode::Analyze => write!(f, "analyze"),
            BotMode::Simulate => write!(f, "simulate"),
            BotMode::PaperTrade => write!(f, "paper_trade"),
            BotMode::Live => write!(f, "live"),
            BotMode::Canary => write!(f, "canary"),
        }
    }
}

/// LIVE execution is opt-in only when `ARB_LIVE` or `LIVE_TRADING` is explicitly truthy.
pub fn live_execution_enabled() -> bool {
    // A shadow-only build must never arm live dispatch, even through a runtime
    // control command or an accidentally inherited environment variable.
    #[cfg(feature = "shadow-only")]
    {
        false
    }
    #[cfg(not(feature = "shadow-only"))]
    {
        is_truthy_flag(std::env::var("ARB_LIVE").ok().as_deref())
            || is_truthy_flag(std::env::var("LIVE_TRADING").ok().as_deref())
            || is_truthy_flag(std::env::var("LIVE").ok().as_deref())
            || is_truthy_flag(std::env::var("EXECUTE_LIVE").ok().as_deref())
    }
}

pub fn is_truthy_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}

#[cfg(test)]
mod bot_mode_tests {
    use super::BotMode;

    #[test]
    fn shadow_mode_alias_selects_non_broadcast_simulation() {
        assert_eq!("shadow".parse::<BotMode>().unwrap(), BotMode::Simulate);
        assert_eq!(" SHADOW ".parse::<BotMode>().unwrap(), BotMode::Simulate);
    }

    #[cfg(feature = "shadow-only")]
    #[test]
    fn shadow_only_build_cannot_arm_live_execution() {
        assert!(!super::live_execution_enabled());
    }
}
