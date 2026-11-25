# CONFIGURATION SECURITY — Phase 9 (Mission §14)

**Status:** ACTIVE — source-verified 2026-11-22. Cross-referenced with fresh online
research (OWASP secrets handling; NIST 2025 credential-rotation guidance; Solana docs
Keychain signing recommendation — citations in
[`research/SECURITY_BEST_PRACTICES.md`](../research/SECURITY_BEST_PRACTICES.md)).

---

## 1. Configuration selection and precedence (source-verified)

`crates/arb-bot/src/main.rs::parse_args` selects one path in this order:
explicit `--config`, `ARB_CONFIG`, then the canonical default
`config/config.toml`. `crates/arb-bot/src/config.rs::load` parses that single file.
It selects the legacy bot schema (`[bot]` / `[rpc]`) unless the file has top-level
`[mode]` or `[provider]`, in which case it uses the alternate `arb-config::AppConfig`
loader. Both paths load one complete TOML file; neither merges `base.toml`, profile
files, or `execution.toml`/`risk.toml` overlays.

The legacy bot schema applies its explicit environment overrides in
`arb-bot/src/config.rs::load`; the alternate AppConfig schema applies its
allow-listed overrides in `arb-config/src/loader.rs`. CLI `--config` chooses the
file; it does not merge configuration layers.
Default selection is fail-closed and canonical:

1. `config/config.toml` is selected when present.
2. If it is absent, startup fails and instructs the operator to copy
   `config/config.example.toml`; the legacy root `config.toml` is not selected.
3. `ARB_CONFIG` or explicit `--config <PATH>` may select another standalone file.

The tracked root `config.toml` was removed from the repository because it carried
stale, divergent values. The local operator copy is preserved only in the ignored
`config/config.toml`. Startup logs the selected path and appends a JSONL provenance
record containing timestamp, path, SHA-256, and mode (never config values or secrets)
to `data/config_load_audit.jsonl`.

## 2. Standalone configuration files (not merged automatically)

| File | Purpose |
|---|---|
| `config/config.example.toml` | Tracked deployment template; copy to ignored `config/config.toml` |
| `config/config.toml` | Operator-managed canonical local/deployment file; gitignored |
| `config/development.toml` | Standalone development configuration; select explicitly with `--config` |
| `config/staging.toml` | Standalone staging configuration; select explicitly with `--config` |
| `config/testing.toml` | Standalone local testing config; loopback RPC; launch with `ARB_MODE=simulate` and keep `ARB_LIVE` unset; isolated DB/ports |
| `config/production.toml` | Standalone production template; select explicitly with `--config` |
| `config/base.toml`, `execution.toml`, `risk.toml`, `protocols.toml`, `providers.toml`, `webhook.toml` | Reference/configuration source files; not automatically merged by `load()` |

For the legacy bot schema, runtime mode is selected by `ARB_MODE`/`ARB_LIVE`; a
truthy `ARB_LIVE` (or the documented `LIVE_TRADING` alias) is required before live
dispatch. The alternate AppConfig schema additionally has a `mode.live_enabled`
field. Legacy `[bot].mode` / `[bot].live_enabled` keys were removed from templates
because the legacy parser does not use them. Compiled protocol invariants and
mandatory safety checks are enforced independently of the selected TOML file.

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
- `reject_unknown_keys` (line 36) applies to the alternate AppConfig schema only; the legacy bot parser does not yet reject every unknown key.
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
| CFN-F-2026-11-22-N1 | P3 | RESOLVED: root `config.toml` removed from tracking; canonical `config/config.toml` only; startup logs selected path. |
| CFN-F-2026-11-22-N2 | P3 | RESOLVED: standalone `config/testing.toml` added (loopback RPC, `ARB_MODE=simulate` required, isolated DB/ports); selected explicitly with `--config`. |
| CFN-F-2026-11-22-N3 | P3 | PARTIALLY RESOLVED: successful loads append timestamp/path/SHA-256/mode provenance; semantic field-level diffs remain open. |

## 7. Kill switch / breakers status matrix

| Control | Location | Status |
|---|---|---|
| Global kill switch (memory+persistent) | `arb-risk::KillSwitch` | IMPLEMENTED, tested (`kill_switch_blocks`) |
| Fire-time re-check | `dispatch.rs:649–665` | IMPLEMENTED |
| Emergency stop (Telegram/Admin) | `control().is_emergency_stopped()` | IMPLEMENTED |
| RPC breaker | `RpcPool` backoff/token buckets + provider supervisor open-circuit | IMPLEMENTED |
| Jito breaker | all-engine submit failure = hard error (no silent fallback) | IMPLEMENTED |
| Consecutive-failure breaker | `RiskGate` limits + `max_consecutive_failures` validation | IMPLEMENTED |
| Config load provenance | `data/config_load_audit.jsonl` | IMPLEMENTED (append-only timestamp/path/SHA-256/mode; semantic diffs remain open) |
