# CONFIGURATION SECURITY — Phase 9 (Mission §14)

**Status:** ACTIVE — source-verified 2026-11-22. Cross-referenced with fresh online
research (OWASP secrets handling; NIST 2025 credential-rotation guidance; Solana docs
Keychain signing recommendation — citations in
[`research/SECURITY_BEST_PRACTICES.md`](../research/SECURITY_BEST_PRACTICES.md)).

---

## 1. Precedence chain (verified)

Documented in `crates/arb-config/src/loader.rs:1–8` and `config/base.toml:6–8`:

```
compiled protocol invariants → config/base.toml → env-specific TOML
(development | staging | production) → environment variables (whitelisted set,
loader.rs:48–70) → CLI args
```

**Resolved ambiguity this phase**: two legacy single-file entry points still exist —
`config.toml` (root, auto-mapped from `.env`, values divergent from the layered
defaults, e.g. `min_keep_lamports = 20000` vs layered risk defaults) and
`config/config.example.toml`. `arb-bot/src/main.rs:42–50` picks `config.toml` FIRST when
both exist. This is registered as **CFN-F-2026-11-22-N1 (P3)**: legacy root files are
duplicates that can silently select stale values; the appended layers (`config/base.toml`
+ profile) are the canonical scheme. Remediation requires an operator decision
(delete/redirect legacy files) — not silently done now.

## 2. Explicit profiles (verified on disk)

| Profile | File | Notes |
|---|---|---|
| base | `config/base.toml` | `mode = "shadow"` default; `live_enabled = false`; **secrets are ENV-VAR NAMES, never values** (header, lines 1–8) |
| development | `config/development.toml` | present |
| staging | `config/staging.toml` | admin listen pinned `127.0.0.1:9100/9101` |
| production | `config/production.toml` | listen pinned `127.0.0.1:9100/9101` (lines 82–85) — **loopback-only by default** |
| sim/execution/risk/protocols/providers/webhook | `config/*.toml` | layered semantics files |

Live mode requires the **double gate** `ARB_LIVE=1` AND `live_enabled = true`
(`config/base.toml` header). Default = shadow. Production config can never override
compiled protocol invariants or mandatory safety limits.

## 3. Strict validation (fail-closed)

`crates/arb-config/src/validation.rs` (645 lines) — runs at startup BEFORE any execution;
nothing silently clamped (module doc lines 1–6):

- URL scheme checks (http/https only, provider + failover), commitment enum.
- Jito: `tip_floor_percentile ∈ {p50,p75,p90,p99}`, `tip_share_bps ≤ max_tip_share_bps ≤
  Bps::MAX`, `max_tip_lamports > 0`, `min_keep_lamports > 0`.
- Risk: `max_trade_size_usd > 0`, `max_daily_loss_usd > 0`,
  `max_hourly_loss_usd ≤ max_daily_loss_usd` (cross-field, line 283),
  `max_consecutive_failures > 0`.
- Security: `check_no_placeholder_secret_env_names` (line 414) rejects placeholder
  secret env-var names; secret VALUES never live in TOML — only names resolved via env
  at runtime.
- `reject_unknown_keys` (line 36) for strict key sets.
- Admin/metrics listen defaults are `127.0.0.1` (validation.rs:491–496) and profile
  files pin loopback — Web API default bind is `{WEB_BIND:-127.0.0.1:8080}`
  (`arb-bot/src/main.rs:546`).

## 4. Enforcement points for risk limits (verified)

- `crates/arb-risk/src/lib.rs` `RiskGate` (line 454) + `KillSwitch` (line 115):
  engage = memory block FIRST, then persist; persistence failure is recorded as an
  error state (`persistence_error`, lines 157–170) — the gate never silently degrades to
  memory-only without a surfaced error.
- Fire-time kill-switch re-check immediately before send:
  `dispatch.rs:649–665` (aborts submission, records outcome, notifies Telegram).
- Pre-gate in opportunity evaluation: `opportunity.rs:816–817`.
- Durable dispatch intent must persist BEFORE send or the transaction is refused:
  `dispatch.rs:747` (retry-safe, R12).
- BABYSITTER: Contingency & rollback doctrine quotes are live-gated the same way (CI
  test `contingency_rollback_doctrine.rs` exists in `arb-risk`).

## 5. Secrets vs fresh research (2025–2026 citations)

| Best practice (source) | Repo state | Gap |
|---|---|---|
| Env vars visible to all processes; prefer secret manager in prod, inject only needed vars (OWASP, siebly.io 2026) | Secrets = env NAMES in TOML, resolved via provider abstraction ("env for local dev, secret manager in production", base.toml:4–5) | Provider abstraction exists; documented hook, actual prod secret-manager integration = operator deployment choice (KEY_MANAGEMENT doc) |
| Event-driven rotation > arbitrary periodic rotation (NIST, fxnx.com, Jul 2025) | Admin tokens compared via `constant_time_eq` (`main.rs:370–372`); rotation is a documented operator procedure | No automated key-rotation tooling — documented manual runbook only |
| Production signing via Keychain/KMS backends (solana.com/docs/tools/security) | Local keypair file loading (`ARB_KEYPAIR_PATH`) | Production KMS backend not implemented — acceptable for shadow; listed in KEY_MANAGEMENT |
| Malware targets `.env`/wallet keystore on VPS (tradoxvps.com, Jan 2026) | `.env` gitignored; gitleaks in CI (verified by Secret scanning check on PR #130) | VPS hardening runbook covers key custody (see operations docs) |

## 6. Findings

| Finding | Severity | Summary |
|---|---|---|
| CFN-F-2026-11-22-N1 | P3 | Legacy `config.toml` (root) is auto-selected FIRST over layered `config/` when both exist and carries divergent tip/risk values; needs operator decision to delete or redirect. |
| CFN-F-2026-11-22-N2 | P3 | Profile set lacks a dedicated `testing` profile (dev/staging/prod present, base default shadow). |

## 7. Kill switch / breakers status matrix

| Control | Location | Status |
|---|---|---|
| Global kill switch (memory+persistent) | `arb-risk::KillSwitch` | IMPLEMENTED, tested (`kill_switch_blocks`) |
| Fire-time re-check | `dispatch.rs:649–665` | IMPLEMENTED |
| Emergency stop (Telegram/Admin) | `control().is_emergency_stopped()` | IMPLEMENTED |
| RPC breaker | `RpcPool` backoff/token buckets + provider supervisor open-circuit | IMPLEMENTED |
| Jito breaker | all-engine submit failure = hard error (no silent fallback) | IMPLEMENTED |
| Consecutive-failure breaker | `RiskGate` limits + `max_consecutive_failures` validation | IMPLEMENTED |
| Config change audit log | documented operator procedure | GAP — see findings in RISK_CONTROL_POLICY (P3 log hook) |
