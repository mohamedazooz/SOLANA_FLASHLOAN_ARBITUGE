# Language Consolidation Plan & Architectural Boundaries

## 1. Architectural Role Assignment & Strategy

This document establishes the binding language governance policy for the repository. Every programming language, framework, and runtime is assigned an explicit, single architectural role.

### 1.1 Language Governance Matrix

| Language | Canonical Responsibility | Permitted Locations | Prohibited Uses |
| :--- | :--- | :--- | :--- |
| **Rust** | **Sole Authority for Production Systems**: Market ingestion, state decoding, route graph, quantitative math, risk gates, V1 tx compilation, Ed25519 signing, Jito landing, persistent storage, Prometheus telemetry, Admin REST/WS API, and Telegram bot. | `crates/*`, `programs/*` | None. |
| **Python** | **Mathematical Oracle, Modeling & Research**: Independent specification-derived mathematical proofs (`audit/math_oracle/oracle.py`), test vector generation, backtesting, and diagnostic RPC probing. | `audit/math_oracle/`, `scripts/*.py` | **NEVER** participate in live trading, quoting, risk validation, or transaction submission. |
| **TypeScript / TSX** | **Operator User Interface (Client-Side Only)**: React SPA monitoring dashboard (`web/src/`). Runs solely in the operator's web browser. | `web/src/` | **NEVER** run as a backend server in production; no participation in the trading path. |
| **JavaScript** | **Differential Test Harnesses & Build Config**: Offline verification scripts comparing Rust math against official protocol SDKs (`scripts/differential-harness/`). | `scripts/differential-harness/`, `web/*.config.js` | **NEVER** run on production execution path. |
| **HTML / CSS** | **Dashboard Presentation Assets**: Static single-page application structure and styles. | `web/index.html`, `web/src/styles.css` | No server-side templating engines. |
| **Shell / Batch** | **Build, CI, Deployment & Automation**: Systemd unit setup, release builds, test verification, secret generation, and environment checks. | `scripts/*.sh`, `deploy/production/*.sh`, root `.sh`/`.bat` | No financial or routing decisions. |
| **SQL** | **Durable Persistence Schemas**: Database schema definitions and migrations. | `crates/arb-store/migrations/*.sql` | No complex business logic in database functions. |

---

## 2. Forensic Evaluation of Proposed Architectural Targets

### 2.1 Rejection of Moving Telegram Bot & Management API to TypeScript
The target proposed in the audit briefing suggested:
> *"TypeScript: Telegram bot, management API, dashboard and monitoring."*

**Brutal Engineering Assessment**: This migration is **STRONGLY REJECTED** on fundamental systems, security, and latency grounds:

1. **Working Rust Implementation Already Exists**:
   * The Telegram bot is fully implemented in idiomatic Rust (`crates/arb-bot/src/telegram.rs`, `telegram_control.rs`).
   * The Management API is fully implemented in idiomatic Rust using Axum 0.8 (`crates/arb-bot/src/web_api.rs`).
   * Both share a single in-memory command authority: `crates/arb-bot/src/control_service.rs` (`ControlService`).
2. **Ground Rule Violation**:
   * Repository Ground Rule #1 explicitly mandates: *"One source of truth per operation... both call ControlService. Do not add parallel implementations; extend the existing one."*
   * Moving Telegram and the admin API to TypeScript would split the control plane into two disparate languages and processes.
3. **Unnecessary Runtime Overhead & IPC Flaw**:
   * Introducing a Node.js daemon for Telegram/API would require an external IPC mechanism (REST, gRPC, or Unix Domain Sockets) to communicate with the Rust trading bot.
   * This adds inter-process serialization overhead, socket latency, process supervision complexity, and dual-process crash recovery failure modes.
4. **Mandate Adherence**:
   * The mission briefing explicitly states: *"Do not migrate working components merely to reduce language count."*
   * Therefore, **Telegram and the Management API must remain in Rust**. TypeScript remains strictly confined to the client-side browser dashboard (`web/`).

---

## 3. Prioritized Consolidation Actions (P0–P6 Matrix)

Rather than cosmetic language migrations, the consolidation plan focuses on eliminating critical security vulnerabilities, enforcing economic invariants, pruning dependency bloat, and streamlining repository architecture.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                       CONSOLIDATION PRIORITY ROADMAP                        │
├────────────────────┬──────────────────────────────────┬─────────────────────┤
│ Priority           │ Action                           │ Target Location     │
├────────────────────┼──────────────────────────────────┼─────────────────────┤
│ P0 (Security)      │ Revoke & Scrub Leaked Secrets    │ scripts/*.py        │
│ P1 (Invariant)     │ Migrate f64 Risk Gates to Int    │ crates/arb-risk     │
│ P2 (Hygiene)       │ Prune Unused npm Bloat           │ web/package.json    │
│ P3 (Security)      │ Remove In-Process pip install    │ scripts/*.py        │
│ P4 (Integrity)     │ Fix Undeclared Peer Dependencies │ differential-harness│
│ P5 (Architecture)  │ Unify Dashboard Asset Serving    │ arb-bot / dashboard │
│ P6 (Maintenance)   │ Clean Root Scripts & CI Workflow │ root & rust-ci.yml  │
└────────────────────┴──────────────────────────────────┴─────────────────────┘
```

### Detailed Action Specifications

#### P0: Immediate Revocation and Scrubbing of Leaked Production Credentials
* **Severity**: **CRITICAL**
* **Target Files**:
  * `scripts/solscan_tool.py` (line 48)
  * `scripts/test_mcp.py` (line 4)
  * `scripts/test_mcp_call.py` (line 4)
  * `scripts/test_pro_endpoints.py` (line 4)
  * `scripts/test_public_tools.py` (line 4)
  * `scripts/decode_jwt.py` (lines 5–6)
  * `scripts/test_keys.py` (entire file)
  * `_backup_pre_unification/test_keys.py` (entire file)
  * `_backup_pre_unification/decode_jwt.py` (lines 5–6)
* **Required Action**:
  1. Immediately revoke all exposed keys on provider portals: Solscan Pro JWT, Helius RPC key (`bb94...[REDACTED]`), Solami key, Shyft keys, Alchemy keys, Birdeye key, Etherscan key, TwelveData key, OpenRouter keys (6 keys), DeepSeek key, Google AI Studio keys, Groq key, Mistral key, Cohere key.
  2. Scrub plain-text tokens from all scripts and rewrite scripts to load credentials strictly from `.env` or process environment variables (`SOLSCAN_API_KEY`, etc.).
  3. Delete `_backup_pre_unification/` entirely.

#### P1: Migrate Floating-Point `f64` Risk Gates to Integer / Fixed-Point Arithmetic
* **Severity**: **HIGH**
* **Target Files**:
  * `crates/arb-risk/src/lib.rs` (lines 240–549)
  * `crates/arb-bot/src/supervisor/opportunity.rs` (lines 795–818)
* **Required Action**:
  * Replace floating-point types (`f64`) in `RiskConfig`, `RiskGate::check_trade_size`, `check_ev_gate`, and `daily_loss_usd` with fixed-point integer representations (microlamports or basis points `u64`/`u128`).
  * Ensure all gating comparisons use integer arithmetic to satisfy the binding repository invariant ("Zero floating-point in financial settlements").

#### P2: Prune Dead and Heavy npm Dependencies from `web/package.json`
* **Severity**: **MEDIUM**
* **Target File**: `web/package.json` (lines 12–13)
* **Required Action**:
  * Remove `@pump-fun/pump-sdk` and `@pump-fun/pump-swap-sdk` from `web/package.json`.
  * Run `npm install` inside `web/` to regenerate `web/package-lock.json`.
  * Verifiably eliminates over 2,500 unused files and ~45 MB of extraneous packages from `node_modules`.

#### P3: Eliminate Unpinned In-Process `pip install` Calls
* **Severity**: **MEDIUM**
* **Target Files**:
  * `scripts/helius_opportunity_probe.py` (lines 71–74)
  * `scripts/read_only_helius_jito_probe.py` (lines 77–80)
* **Required Action**:
  * Remove `subprocess.check_call([sys.executable, "-m", "pip", "install", ...])`.
  * Add a formal `scripts/requirements-dev.txt` with pinned `websockets==12.0` and instruct developers to run `pip install -r scripts/requirements-dev.txt` in a virtual environment.

#### P4: Fix Undeclared Peer Dependencies in Differential Test Harness
* **Severity**: **LOW**
* **Target File**: `scripts/differential-harness/package.json`
* **Required Action**:
  * Declare `@solana/web3.js: ^1.98.0` and `bn.js: ^5.2.1` explicitly in `scripts/differential-harness/package.json`.
  * Ensures reproducible runs across all Node package managers (npm, pnpm, yarn).

#### P5: Unify Dashboard Asset Serving & Retire Monolith Fallback
* **Severity**: **LOW**
* **Target Files**:
  * `dashboard/index.html` (2,949 lines monolith)
  * `crates/arb-bot/src/web_api.rs` (lines 508–515)
* **Required Action**:
  * Remove `include_str!("../../../dashboard/index.html")` fallback from `crates/arb-bot/src/web_api.rs`.
  * Move `dashboard/index.html` to `docs/archive/legacy_ui/`.
  * Ensure the Axum server serves solely the production React bundle from `web/dist/`.

#### P6: Tooling Cleanup, Script Consolidation & CI Verification
* **Severity**: **LOW**
* **Target Files**:
  * Root shell scripts (`run-shadow.sh`, `stop-shadow.sh`, `verification_script.sh`, etc.)
  * `webhook-service/Cargo.toml`
  * `.github/workflows/rust-ci.yml`
* **Required Action**:
  * Consolidate build verification commands into `scripts/verify-linux.sh`.
  * Upgrade `webhook-service` to Axum 0.8 / thiserror 2 or archive it.
  * Add `python audit/math_oracle/oracle.py --verify` as a mandatory step in `.github/workflows/rust-ci.yml`.
