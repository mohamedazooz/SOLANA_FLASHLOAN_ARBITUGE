# Configuration Architecture & Defaults Drift Remediation

## 1. Single Authoritative Configuration Architecture

The configuration subsystem is governed by `crates/arb-config/src/lib.rs` and loaded deterministically via `loader::load(path)`.
Strict precedence order is enforced:
1. **Protected Protocol & Compile-Time Safety Invariants** (defined in `crates/arb-constants` and `crates/arb-math`). Never overrideable by runtime files or operator commands.
2. **Base Configuration (`config/base.toml`)**: Standard deployment-independent operational parameters.
3. **Environment-Specific Configuration (`config/production.toml` / `config/development.toml`)**: Environment overrides (RPC endpoints, commitment levels, metrics ports).
4. **Environment Variables**: Overrides for deployment secrets (`HELIUS_API_KEY`, `SENTINEL_ADMIN_KEY`, `SOL_USD_PRICE`, `MIN_NET_PROFIT_USD`).
5. **CLI / Authorized Operator Overrides**: Explicit session parameters.

---

## 2. Forensic Investigation of Defaults Drift

| Parameter | Previous Location 1 | Previous Location 2 | Identified Root Cause | Remediated Canonical Truth |
|---|---|---|---|---|
| Minimum Net Profit | `30_000` (`crates/arb-bot/src/config.rs:313`) | `50_000` (`arb-constants` & `base.toml`) | Earlier DLMM test code used 30k; global default was 50k | Unified to `DEFAULT_MIN_PROFIT_LAMPORTS = 50_000` |
| Maximum Tip Share | `8,500` BPS (`arb-constants:77`) | `7,000` BPS (`config/base.toml:35`) | Constants allowed 85% tip; production TOML capped at 70% | Harmonized to `7,000` BPS (70% max tip share) |
| Keep Share | `0.85` (`arb-constants:79`) | `0.60` (`config/base.toml:38`) | Contradictory keep share vs tip share | Harmonized to `0.60` keep share |
| Min Keep Lamports | `50_000` (`arb-constants:83`) | `10_000` (`config/base.toml:36`) | High global floor broke calm pool opportunities | Harmonized to `10_000` base with contention elevation |
| Base Transaction Fee | `5,000` lamports | `5,000` lamports | Uniform across workspace | Retained as Solana consensus standard |
| Contingency Buffer | `10_000` lamports (`opportunity.rs`) | `10_000` lamports | Hardcoded magic number | Parameterized into strategy config |
| SWQoS Hurdle & Fee | `8,000` & `2,000` (`jito_service.rs`) | Unlinked raw integers | Hardcoded inline literals | Promoted to named constants `SWQOS_MIN_EDGE_HURDLE_LAMPORTS` & `SWQOS_DEFAULT_PRIORITY_FEE_LAMPORTS` |

---

## 3. Dynamic Configuration & Safety Invariants
1. **Fail-Fast Startup Validation**:
   - `crates/arb-config/src/validation.rs` validates all numeric ranges, URL schemes, and non-empty key names.
   - Any negative, infinite, NaN, or out-of-range value halts the process immediately before WebSocket connections or trading loops initialize.
2. **Immutable Security Controls**:
   - Live trading requires both `[bot] mode = "live"`, `live_enabled = true`, and the process environment `ARB_LIVE=1`.
   - The Telegram `/config` endpoint rejects any attempt to mutate core risk limits (`max_trade_size_usd`, `max_daily_loss_usd`, `max_consecutive_failures`) beyond safety boundaries.
