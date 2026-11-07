//! Model Context Protocol (MCP) Server for Solana Arbitrage Bot.

pub mod tools;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
