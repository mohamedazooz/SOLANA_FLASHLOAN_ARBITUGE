> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Production Deployment

**Repository:** Solana Arbitrage Bot  
**Status:** Pre-production (live mode gated off by default)

---

## 1. Default State

The bot ships in **shadow mode** with `live_enabled = false`. Live trading is
impossible without explicit operator action:

```
[bot]
mode = "shadow"
live_enabled = false
```

And `ARB_LIVE` must be a truthy value (`1`/`true`/`yes`/`on`).

## 2. Pre-Flight Checklist (before enabling live)

- [ ] All `config/production.toml` values reviewed and validated.
- [ ] `security.cors_allow_origins` populated with explicit allowlist.
- [ ] `security.webhook_signature_required = true`.
- [ ] `SENTINEL_ADMIN_KEY` set to a strong random value (not placeholder).
- [ ] `HELIUS_API_KEY` / `SHYFT_API_KEY` / `JUPITER_API_KEY` set.
- [ ] Wallet keypair file exists with `0600` permissions.
- [ ] `min_sol_balance_lamports` safety floor set (default 0.05 SOL).
- [ ] `cargo audit` / `cargo deny` run and reported (RustSec NOT VERIFIED if DB unavailable — do not claim clean).
- [ ] All tests pass, including address-conflict and fee fail-closed regressions.
- [ ] Kill switch marker file mechanism tested.
- [ ] Admin /pause and /resume tested with authentication.
- [ ] No secret values in tracing/error output.

## 3. Live Mode Requirements (all enforced at startup)

1. `bot.mode == "live"` AND `bot.live_enabled == true`
2. `ARB_LIVE=1` (truthy)
3. Webhook signature required
4. Non-empty CORS allowlist
5. Real RPC URL set

Any missing control ⇒ startup fails (fail closed).

## 4. Secrets in Production

- Use a production secret manager (not `.env`).
- Secret rotation and revocation procedures documented.
- Least-privilege access for RPC/API keys.
- Key file permission checks enforced.

## 5. Observability in Production

- Structured tracing enabled with redaction of secrets.
- Metrics distinguish static config, dynamic observations, estimated values,
  and verified on-chain values.
- Circuit breaker transitions and risk gate rejections logged (reasons
  redacted).

## 6. What Is NOT Done (open items)

- HSM / secret manager integration (C-11 architectural).
- Per-operator admin identity & audit trail (C-09/C-25).
- Release artifact signing / SBOM / cosign (C-14/C-27).
- `cargo audit` RustSec DB verification (C-13 — report NOT VERIFIED).
- Auxiliary binaries brought under the central execution gate (C-10).
