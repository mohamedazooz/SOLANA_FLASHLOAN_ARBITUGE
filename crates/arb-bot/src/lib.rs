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
/// Mode is controlled exclusively by environment (`ARB_MODE`, plus the
/// explicit `ARB_LIVE` / `ARB_CANARY` arming flags). There is no shadow mode:
/// `Simulate` is the non-broadcast default, and `Live` additionally requires
/// `ARB_LIVE` to be truthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotMode {
    Simulate,
    Live,
    Canary,
}

impl BotMode {
    /// Runtime mode transitions go through a shared `AtomicU8` so the unified
    /// control service can switch paper/live without a restart.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => BotMode::Live,
            2 => BotMode::Canary,
            _ => BotMode::Simulate,
        }
    }
}

impl std::str::FromStr for BotMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "simulate" => Ok(BotMode::Simulate),
            "live" => Ok(BotMode::Live),
            "canary" => Ok(BotMode::Canary),
            other => {
                anyhow::bail!("Unknown bot mode: '{other}'. Allowed modes: simulate, live, canary")
            }
        }
    }
}

impl std::fmt::Display for BotMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BotMode::Simulate => write!(f, "simulate"),
            BotMode::Live => write!(f, "live"),
            BotMode::Canary => write!(f, "canary"),
        }
    }
}

/// LIVE execution is opt-in only when `ARB_LIVE` is explicitly truthy.
///
/// Presence alone must never authorize live trading: `.env.example` ships
/// `ARB_LIVE=0`, and a truthiness bug here would enable signing/broadcasting.
pub fn live_execution_enabled() -> bool {
    is_truthy_flag(std::env::var("ARB_LIVE").ok().as_deref())
}

pub fn is_truthy_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}
