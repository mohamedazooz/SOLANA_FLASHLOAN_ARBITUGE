# PR #7 Control Plane Audit (2026-11-13)

Scope: the unified control plane delivered by PR #7 (`feat/unified-pipeline-webapp`,
head `d3490dd`) and hardened on PR #8. Terms are strict: **Verified** =
inspected in code AND exercised by a test that ran in this campaign;
**Partially verified** = inspected, partially exercised; **Not verified** =
inspected only or environment-blocked.

## A. One source of truth — VERIFIED

- Telegram and the Web API both execute the identical
  `ControlService::execute_command` grammar
  (`crates/arb-bot/src/control_service.rs:620-660`): the same
  `start/stop/shadow/paper/live/emergency-stop/clear-emergency-stop/config`
  handlers, the same audit trail, the same event bus. No second control
  logic path exists (grep sweep: no `set_mode`/`emergency_stop` call sites
  outside `control_service.rs`).
- Canonical runtime state lives in `ControlService` atomics
  (`execution_enabled`, `emergency_stopped`, mode byte) — the dashboard
  reads `StatusSnapshot` built from those same atomics
  (control_service.rs:285-300), not from a parallel cache.

## B. Runtime safety — VERIFIED

| Requirement | Evidence |
|---|---|
| Emergency stop immediately effective | `emergency_stop()` sets kill switch + latch + disables execution (control_service.rs:490-500); the dispatch **fire-time gate** (dispatch.rs:451-463) re-checks both before any network send — a stop issued during simulation wins |
| Stop prevents new dispatches | kill switch checked in the RiskGate ladder (`opportunity.rs`) and at fire time |
| In-flight reconciliation is safe | `recovery.rs` never resubmits; resolution is evidence-based (see PR8_EXECUTION_RECOVERY_AUDIT.md) |
| Config validated before activation | `ConfigOverrides` validated in `update_config`; invalid values rejected with `ControlError` |
| Mode changes auditable | every control action appends to the audit log (`audit()` now fsync'd) and emits a `control` event |
| Restart never restores unsafe live state | mode is **not persisted**; process restarts in shadow/paper default; `set_mode(Live)` requires `ARB_LIVE=1` + `CONFIRM-LIVE` per process lifetime |

## C. Authentication and security — VERIFIED (with documented residuals)

- Fail-closed password (`WEB_ADMIN_PASSWORD` missing/placeholder → API
  refuses to start; web_api.rs:85-92). sha256 + constant-time compare.
- Sessions: 32-byte random tokens stored as sha256 hashes, TTL clamped
  60–86400 s, MAX_SESSIONS=64 — replay of an expired token fails; raw
  token never stored server-side.
- Rate limits: login 10/min/IP, mutations 60/min/IP, keyed by the
  **right-most** X-Forwarded-For entry (spoof-resistant; PR #8 fix F4).
- WebSocket: single-use 60 s tickets (`POST /api/ws-ticket`) — the
  long-lived token never appears in a URL (PR #8 fix F5; the PR #7
  `?token=` fallback was **removed** in this integration round).
- Body limit + security headers middleware; audit log fsync'd.
- Secrets: `.env` git-ignored; no secret in API responses or the frontend
  bundle (sweep this round: bundle contains only `arb_session_token`
  sessionStorage handling).
- Residuals: HTTPS termination is a deployment-runbook item (not
  verifiable in sandbox); audit log is single-file (single-operator model).

## D. Dashboard reflects backend state — VERIFIED (unit level)

- Every displayed value maps to a `StatusSnapshot` field built from live
  atomics/registries: mode, paused, emergency, slot, pools_tracked,
  net_pnl_sol, route_coverage (supervisor.rs `route_coverage()` —
  computed from the registry read lock, no stale cache), executions ring,
  logs ring, opportunities ring.
- Unavailable data renders as unavailable: `live_sol_price()` returns
  `None` when no verified price exists and the PnL path skips rather than
  guesses; provider/route panels poll `/api/status` + `/api/routes` fresh
  every 5 s. No hardcoded operational metric found in `web/src/`
  (sweep this round).
- **Partially verified**: full browser-level rendering was exercised via
  `tsc` + production build + e2e HTTP/WS tests, not a visual manual pass
  (no browser in sandbox).

## E. Frontend quality — VERIFIED (static + build)

- TypeScript strict compile clean (`tsc --noEmit`); production build
  reproducible (`npm run build`, lockfile install).
- WS reconnect: exponential backoff + jitter (PR #8), re-authenticates by
  fetching a **fresh single-use ticket** per connection; auth expiry
  handled by 401 → re-login flow in `api.ts`.
- No duplicated backend state: the frontend holds only the session token +
  last snapshot; all authoritative state re-fetched from the API.

## Findings register (PR #7 scope)

| # | Severity | Location | Issue | Status |
|---|---|---|---|---|
| C1 | HIGH | web_api.rs (PR #7 `?token=` WS auth) | long-lived token in URLs | **FIXED on PR #8** (single-use tickets; fallback removed this round) |
| C2 | HIGH | control plane → dispatch | kill switch not checked at fire time | **FIXED on PR #8** (fire-time gate) |
| C3 | MEDIUM | web_api.rs `ip_of` | spoofable first XFF entry | **FIXED on PR #8** (right-most) |
| C4 | LOW | control_service.rs `audit()` | no fsync | **FIXED this round** (sync_data) |
| C5 | INFO | deployment | TLS termination | Runbook item (SHADOW_MODE_RUNBOOK.md) |
