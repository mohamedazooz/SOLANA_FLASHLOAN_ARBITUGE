> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Security

**Repository:** Solana Arbitrage Bot  
**Scope:** Configuration-related security controls  
**Related:** `docs/SECURITY.md`, `reports/config_refactor/security_findings.md`

---

## 1. Secrets Are Never in Config Files

`config/*.toml` and `config/config.example.toml` contain **only secret env var
names**, never secret values:

```toml
[admin]
token_env = "SENTINEL_ADMIN_KEY"   # name only; value from .env / secret manager
```

Real values live in `.env` (local dev) or a production secret manager. `.env`
is never committed; `.env.example` is shipped with placeholder text.

## 2. Placeholder Rejection

At startup, declared secret env names are checked against an obvious-placeholder
list (`""`, `"changeme"`, `"change-me"`, `"placeholder"`, `"your_token_here"`,
`"token"`). A placeholder causes explicit startup failure — it never authorizes
an admin action.

## 3. CORS — Never Permissive

`security.cors_allow_origins` is an explicit allowlist. An empty list denies
all cross-origin traffic (fail closed). `CorsLayer::permissive()` is never
used. Production requires a non-empty, enumerated allowlist.

## 4. Admin Authentication

- Admin routes accept `Authorization: Bearer <token>` or `X-Admin-Token:
  <token>`.
- Comparison is constant-time (no early short-circuit).
- Missing or placeholder token ⇒ admin listener disabled (fail closed).
- Kill switch activation remains available through the safest authorized path.

## 5. Live Mode Gates

Live trading requires **all** of:
1. `bot.mode == "live"` AND `bot.live_enabled == true`
2. `ARB_LIVE` env set to a truthy value
3. `security.webhook_signature_required == true`
4. `security.cors_allow_origins` non-empty (explicit allowlist)
5. `provider.primary_url` non-empty with an approved scheme

Any missing control ⇒ startup validation fails (fail closed).

## 6. No Silent Fallbacks

Invalid values are rejected with actionable errors identifying the config key.
Nothing is silently clamped, guessed, or defaulted to an unsafe value. Missing
blockhash ⇒ error (no zero blockhash). Missing fee ⇒ `Err` (no 100% cap).

## 7. Observability Redaction

Structured tracing logs config version/source, risk gate rejection reasons,
provider failover, tip updates, CU estimation, quote freshness, execution
authorization, and circuit breaker transitions. **Never** log private keys,
API keys, bearer tokens, or full sensitive authentication headers.
