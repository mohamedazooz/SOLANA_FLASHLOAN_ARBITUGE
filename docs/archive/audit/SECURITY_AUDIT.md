# Security Audit

Status: source and configuration review at baseline `541cdec` plus the
remediation in the current working tree. This is not a penetration test and not
a runtime security certification. No secret value is reproduced in this
document.

## 1. Scope and threat model

Assets in priority order:

1. The operational Solana keypair (signing authority for the hot wallet).
2. Operator/admin credentials and provider API keys.
3. Funds held by the hot wallet (fees/tips only; no proprietary trading capital
   is assumed by the strategy).
4. Repository integrity and release artifacts.
5. Market-data integrity (a false quote can cause a signed loss).

Threats considered:

- Credential leakage through git history, `.env`, image layers, logs, or
  Telegram messages.
- Unauthenticated operator endpoints (`/pause`, `/resume`).
- Accidentally enabling live execution (env presence vs. truthiness).
- Kill switch that does not survive a restart.
- A compromised dependency or unmaintained crate.
- Prompt/instruction injection from market data or webhooks into an agent loop
  (no LLM is in the signing path in this repository).

Out of scope: host compromise, Solana validator/Jito compromise, physical
access, and side-channel attacks on the signing process.

## 2. Findings and remediation

| ID | Severity | Status | Summary | Remediation |
|---|---|---|---|---|
| ADMIN-01 | High | **FIXED (source)** | `/pause` and `/resume` were unauthenticated; `AdminConfig.token_env` was parsed but unused. | Both routes now require `Authorization: Bearer <token>` or `X-Admin-Token`; constant-time compare; missing/placeholder token disables the whole admin listener (fail closed); rejections are logged. |
| KILL-01 | High | **FIXED (source)** | `KillSwitch` was in-memory only; `/pause` was lost on restart. | `KillSwitch::with_file` persists a marker, starts blocked if the marker exists, removes it on `/resume`, and logs persistence failures. Supervisor uses `KILL_SWITCH_FILE` or `data/kill_switch`. Test: `kill_switch_marker_survives_restart`. |
| LIVE-03 | High | **FIXED (source)** | `ARB_LIVE` was checked with `is_ok()`, so `ARB_LIVE=0` authorized live. | New `is_truthy_flag` / `live_execution_enabled`; only `1/true/TRUE/yes/on` authorize. Used in startup and dispatch. Test: `arb_live_requires_an_explicit_truthy_value`. |
| CANARY-01 | Medium | **FIXED (source)** | Canary incremented fills and realized PnL without sending. | Canary now increments `simulated_net_profit_total` only and records a `canary` outcome with a `Canary-Simulated-` label. |
| SEC-01 | Medium | **NOT_REPRODUCED** | gitleaks history hits were a test-only URL fixture. | No credential rotation indicated by that scan. Full-history/release-artifact scan still required. |
| DEP-01 | Medium | **FIXED (metadata)** | All workspace crates omitted a license expression, failing `cargo deny` licenses. | Added `license.workspace = true` to all 14 crates and `MIT OR Apache-2.0` to `webhook-service`; workspace license is `MIT OR Apache-2.0`. Dependency advisories remain unverified (see §5). |
| BUILD-03 | High | **FIXED (source)** | Dockerfile copied a non-existent `config/` directory; compose mounted `./config`. | Added `config/config.example.toml`, updated the image to copy it as a safe default, and documented the read-only production mount. Docker build validation is tracked in `TEST_EVIDENCE.md`. |

## 3. Fail-closed controls (verified by inspection)

- **Live gate**: `--mode live` requires `ARB_LIVE` truthy; dispatch repeats the
  check. A falsy/absent value cannot reach signing/broadcast.
- **Simulation**: in Live, RPC simulation failure/timeout/`success=false` returns
  before submission. LiteSVM absence is explicit and does not fake success.
- **Risk gate**: kill switch, breaker, velocity, losses, notional, state age,
  impact, tip caps, min profit, inflight bundles. Any failure rejects the route.
- **Admin**: missing/placeholder token disables the listener; a wrong token gets
  `401`; the listener binds to the configured loopback address.
- **Secrets**: `.env` and keypairs are gitignored and excluded from the Docker
  build context; the image runs as non-root `arb`.
- **Logging**: secret values are not printed by the reviewed code paths; admin
  auth failures log a rejection, not the presented token.

## 4. Residual risks

1. **Credentials have not been rotated.** The repository cannot prove that keys
   ever exposed outside git are revoked. Operator must rotate the Solana keypair,
   provider keys, Telegram token, and admin token before any live run.
2. **No secrets manager/HSM.** The keypair is a file/env input. File permissions
   and host security are the only controls.
3. **Admin token is a shared bearer secret**, not per-operator identity. There is
   no audit trail of *who* paused/resumed beyond a log line.
4. **Telegram** messages can contain route/transaction data; the bot token is a
   high-value secret. Chat membership is an access-control boundary.
5. **Dependency advisories are unverified** because the RustSec DB could not be
   fetched in this environment. `cargo-deny` advisories/bans/sources passed on
   the last successful run; that is not a vulnerability-free claim.
6. **Auxiliary binaries** (`live-scanner`, `validate`, `discover`, `replay`) are
   not governed by the main mode gate and need a separate review.
7. **No release artifact signature** (cosign/SBOM) is produced.

## 5. Dependency and supply-chain status

| Check | Status | Evidence |
|---|---|---|
| `cargo deny check` | Licenses fixed in metadata; full rerun recorded in `TEST_EVIDENCE.md`. | `deny.toml` allows MIT/Apache-2.0/BSD/ISC/etc., denies unknown registries/git and yanked crates. |
| `cargo audit` | **BLOCKED** | RustSec DB fetch failed repeatedly; no cached DB. Vulnerability status unknown. |
| `cargo fmt/check/clippy/test` | See `TEST_EVIDENCE.md`. | Linux Docker verification is the reference environment. |
| Secret scan | Test-only false positives in history; no live credential found in scanned paths. | gitleaks results in `BASELINE.md`. |
| Docker context | `.dockerignore` excludes `.env`, keypairs, `target/`, `data/`, `docs/`, `.git/`. | Inspected. |

## 6. Required actions before any live authorization

1. Rotate every credential that has ever been in a shell history, chat, CI log,
   or git object; revoke the old ones.
2. Move the keypair to a dedicated signer/HSM or a host-only file with least
   privilege; never mount it into a container that also has network egress to
   untrusted services.
3. Replace the shared admin token with per-operator credentials and an audit
   log; keep the listener on loopback behind an authenticated tunnel.
4. Run a full-history secret scan (`gitleaks git --log-opts=--all`) and archive
   the result with the release.
5. Run `cargo audit`/`cargo deny` with a working advisory DB and attach the
   output.
6. Complete the fault-injection and shadow-soak acceptance in
   `RELEASE_READINESS.md`.

See [`SECURITY.md`](../SECURITY.md) for the rotation procedure.
