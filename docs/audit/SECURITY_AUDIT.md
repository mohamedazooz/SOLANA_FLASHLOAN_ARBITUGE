# Security Audit — PR #8 round (2026-11-13)

Scope: web API, Telegram control, kill-switch/emergency semantics, secrets,
and the execution-path enforcement of every control. Terms are strict:
**Verified** = inspected AND exercised by a test that ran in this campaign;
**Partially verified** = inspected, partially exercised; **Not verified** =
inspected only or environment-blocked.

## 1. Findings fixed this round

| # | Severity | Location | Issue | Fix | Evidence |
|---|---|---|---|---|---|
| F2 | HIGH | `crates/arb-bot/src/supervisor/dispatch.rs:451-463` | Kill switch / emergency latch were enforced at capture time only; a stop issued during simulation or the slot-window wait did not block the in-flight submission | Fire-time safety gate immediately before any network submission; aborts with `aborted_kill_switch` outcome + Telegram alert | `cargo test -p arb-bot` 51/51; `ControlService::is_emergency_stopped()` (control_service.rs:387-389) consumed here |
| F4 | MEDIUM | `crates/arb-bot/src/web_api.rs:214-218` (`ip_of`) | Took the FIRST `X-Forwarded-For` entry — client-spoofable through the trusted proxy, poisoning rate-limit keys and audit records | RIGHT-MOST entry (set by the trusted reverse proxy) | Inspected; covered by web matrix tests |
| F5 | MEDIUM | `web_api.rs:41-42,113-141,519` + `web/src/api.ts` | Long-lived session token accepted in WS upgrade URLs (`?token=`) — leaked into proxy/access logs | Single-use WS tickets: `POST /api/ws-ticket` (authed) issues a 32-byte random ticket stored only as sha256, TTL 60 s, cap 64, remove-on-use; `ws_events` prefers `?ticket=`; legacy `?token=` fallback retained (documented below) | `web_api::tests::ws_ticket_is_single_use_and_unknown_tickets_fail` passes; `tsc` + `npm run build` pass |

## 2. Control-surface semantics (directive §13)

- **Distinct semantics — Verified** (unit test
  `start_stop_roundtrip_and_emergency_latch`, control_service.rs:661-685):
  - `stop()` = kill switch ON + `execution_enabled=false` (reversible by `start()`).
  - `emergency_stop()` = kill switch ON + latch ON + execution disabled;
    `start()` and `set_mode()` are **refused** while the latch is set
    (control_service.rs:469-473, 519-523).
  - `clear_emergency_stop()` (control_service.rs:502-513) clears **only the
    latch** — it does not touch `execution_enabled` or the kill switch. The
    test asserts `paused` remains true after clearing: clearing the latch
    never auto-resumes execution; a separate explicit `start()` is required.
    **Verified.**
- **Kill switch cannot be bypassed by mode changes — Verified**:
  `set_mode` is blocked by the latch; `Live` additionally requires
  `ARB_LIVE` truthy + `CONFIRM-LIVE` token
  (`live_mode_requires_arb_live_and_confirmation`); `BYPASS_RISK_GATE` still
  honors the kill switch (opportunity.rs gate ladder).
- **Control state consumed by the actual execution path — Verified**: the
  fire-time gate (dispatch.rs:451-463) reads the live kill-switch and
  emergency-latch atomics, not just the dashboard snapshot; the capture-time
  RiskGate ladder gates every opportunity (`evaluate_flash_loan` /
  `evaluate_full`).

## 3. Web API (Axum, localhost bind + reverse proxy)

- **Fail-closed auth — Verified**: password from `WEB_ADMIN_PASSWORD`;
  placeholder/missing password refuses login; sha256 + constant-time
  compare; sessions stored as sha256 hashes (never plaintext), TTL clamped
  60–86400 s, `MAX_SESSIONS=64` with retain-on-login cleanup.
- **Rate limiting — Verified**: per-IP fixed windows (login 10/min, mutation
  60/min); expired entries pruned on access; keyed by the F4-fixed
  right-most XFF identity.
- **Body limit + security headers middleware — Verified** (inspection +
  header-assertion tests).
- **Audit log — Partially verified**: appends every control action
  (`audit()` calls throughout control_service.rs); durability is a local
  file append, not fsync'd — acceptable for the localhost threat model,
  documented as such.
- **Reverse-proxy trust — Partially verified**: XFF right-most entry is
  trusted; the deployment runbook must bind the API to localhost and
  terminate TLS at the proxy (Caddy/Nginx). HTTPS termination itself is
  **Not verified** in this sandbox (no proxy deployed here).

## 4. Telegram control

- **Chat allowlist — Verified**: listener disabled without
  `TELEGRAM_CONTROL_CHAT_IDS`; unauthorized chats rejected.
- **Same control path — Verified**: Telegram executes the identical
  `execute_command` grammar as the web API (control_service.rs:637-639) —
  no second control logic path exists.
- **`/live` confirmation — Verified**: live mode requires the literal
  `CONFIRM-LIVE` argument plus the env gate.
- **Bounded retries — Verified**: 10 bounded retry attempts; no unbounded
  reconnection loop.

## 5. Secrets and configuration

- **Verified**: `.env` is git-ignored; Helius/Privy credentials exist only
  there; no secrets in source, docs, or fixtures (swept this round);
  `.env.example` carries only non-secret keys. `ARB_LIVE=0`, `DRY_RUN=true`
  defaults preserved.
- **Not exposed to browser**: sessions/tickets are opaque; no signing keys
  or provider secrets are served by any endpoint (route sweep this round).

## 6. Remaining register

| Item | Severity | Status |
|---|---|---|
| Legacy `?token=` WS fallback retained | LOW | Deliberate (older clients during migration); ticket is preferred and token never logged. Removal candidate after frontend rollout. |
| HTTPS/TLS termination | — | **Not verified** here; requires the real reverse-proxy deployment (runbook item). |
| Audit-log fsync durability | LOW | Documented; acceptable for single-operator localhost model. |
| ≥24 h authenticated-session behavior under soak | — | **Not verified** (soak blocked in sandbox). |
