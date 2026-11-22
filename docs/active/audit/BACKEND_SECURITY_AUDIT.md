# BACKEND SECURITY AUDIT — Phase 10 (Mission §15)

**Status:** ACTIVE — source-verified 2026-11-22. Backend surface audited:
`crates/arb-bot/src/web_api.rs` (1,365 lines), `crates/arb-bot/src/control_service.rs`,
Telegram control, and env/secret handling.

---

## 1. API surface (verified — `web_api.rs:777–806`)

| Endpoint | Method | Auth | Notes |
|---|---|---|---|
| `/api/login` | POST | password (rate-limited) | const-time compare; issues 32-byte random token |
| `/api/logout` | POST | token | revokes session |
| `/api/status`, `/api/telemetry`, `/api/ticker`, `/api/opportunities`, `/api/executions`, `/api/rejections`, `/api/logs`, `/api/pnl`, `/api/providers`, `/api/routes` | GET | token | read-only projections |
| `/api/config` | GET POST | token + mutation rate limit | threshold changes (ledgered as ConfigOverrides) |
| `/api/control/{action}` | POST | token + mutation rate limit | actions table below |
| `/api/ws-ticket` → `/api/ws` | POST → GET(WS) | token → single-use ticket | long-lived token never in URL (line 39) |
| `/health` | GET | none | liveness only |

## 2. Control actions (`control_service.rs:855–876`)

`status, start, resume, stop, pause, paper/simulate, live, opportunities, pnl,
providers, config, emergency-stop, clear-emergency-stop, kill…`

- **HIGH-RISK GATE**: `live` requires `ARB_LIVE=1` (env) AND explicit `confirm="CONFIRM-LIVE"`
  body token (lines 755–775); test `live_mode_requires_arb_live_and_confirmation`
  (`control_service.rs:973`) enforces fail-closed.
- Emergency stop set/clear is audit-logged via the safety-action log path (line 381).

## 3. Auth internals (verified)

| Property | Implementation | Evidence |
|---|---|---|
| Password source | `WEB_ADMIN_PASSWORD` env only; placeholder values (`changeme`, `placeholder`, …) rejected at startup | web_api.rs:79–90 |
| Storage | sha256 of password; tokens stored as sha256 hashes — raw token never persisted | web_api.rs:52, 101, 180 |
| Comparison | constant-time byte compare for password (zipped diff, lines 158–164); token compare hash-based | — |
| Session TTL | `WEB_SESSION_TTL_SECS` bounded; expiry checked on `authorize` (line 191) | — |
| CSRF | Bearer header only, no cookies → CSRF surface removed by design (module doc, line 8) | — |
| Rate limits | per-IP fixed window: login `LOGIN_RATE_LIMIT_PER_MIN`, mutations `MUTATION_RATE_LIMIT_PER_MIN` (lines 139–206) | — |
| WS | single-use tickets (`ws_ticket_is_single_use_and_unknown_tickets_fail`, line 1242) + ticket hash TTL map (lines 111–137) | — |
| Body limit | 64 KiB global (`DefaultBodyLimit`, line 805) | — |
| Headers | OWASP-secure baseline middleware on every response (lines 808+) | — |

## 4. Secret handling (verified)

- Secrets are env-resolved by NAME (`config/base.toml`); TOML carries no secret values.
- Admin HTTP + web API bind loopback by default (`config/*:listen = 127.0.0.1:*`,
  `WEB_BIND` default `127.0.0.1:8080` in main.rs:546).
- `check_no_placeholder_secret_env_names` rejects placeholder env names at config load
  (validation.rs:414).
- No private keys or signing secrets reachable from any API handler (module doc, line 13).

## 5. Findings

| Finding | Severity | Summary |
|---|---|---|
| BCK-F-2026-11-22-N1 | P3 | Session tokens are in-memory only (state loss on restart = re-login — acceptable for single-operator, noted deliberately) |
| BCK-F-2026-11-22-N2 | P3 | Per-IP limiting trusts `X-Forwarded-For` parsing conventions; behind an untrusted proxy this is spoofable — bind loopback / front with a trusted proxy only (already default) |
| BCK-F-2026-11-22-N3 | P3 | Password hashing is plain sha256 (no salt/stretch); acceptable at loopback exposure and single-operator, but should move to Argon2id before any non-loopback exposure |

No authentication bypass found. No `unwrap` on request paths audited. Telegram control
shares the same `execute_command` gate (emergency actions logged; `live` requires the
same CONFIRM-LIVE token).
