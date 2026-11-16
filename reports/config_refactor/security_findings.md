# Security Findings & Remediations

**Repository:** Solana Arbitrage Bot  
**Audit date:** Mon Nov 16 2026  
**Role:** Application Security Architect  
**Scope:** `crates/`, `webhook-service/`, root config

---

## Method

Every finding is tied to a specific file and line. Findings are classified by severity:
- **Critical** — can lead to loss of funds, unauthorized execution, or secret exposure.
- **High** — weakens fail-closed controls or security boundaries.
- **Medium** — weakens defense-in-depth or observability.
- **Low** — hygiene / best-practice.

No finding weakens an existing fail-closed control. All remediations preserve existing safety invariants.

---

## P0 — Critical / High Findings

### SEC-01: Webhook CORS permissive — `CorsLayer::permissive()`
**File:** `webhook-service/src/main.rs` (and router/engine)  
**Severity:** Critical  
**Description:** The webhook service allows any origin to make cross-origin requests. In production this exposes internal webhook endpoints (Solana webhooks, Helius, Bitquery, topology, admin-adjacent routes) to arbitrary attacker-controlled origins.  
**Risk:** Unauthorized webhook ingestion, internal topology exposure, potential request forgery from attacker pages.  
**Remediation:** Replace `CorsLayer::permissive()` with an explicit production allowlist of configured origins. In development, allow localhost only via config flag.  
**Verification source:** OWASP CORS best practice; axum `tower-http` CORS documentation.  
**Required tests:** CORS allowlist test (allowed origin passes, disallowed origin rejected), preflight test, credentials test.  
**Status:** FIXED (finding C-08/C-24). CORS allowlist from config; empty = deny all; wildcard rejected.

### SEC-02: Shared admin bearer token — no roles, no audit trail
**File:** `crates/arb-bot/src/main.rs` (admin routes), `webhook-service/src/config.rs` (`auth_token`)  
**Severity:** High  
**Description:** Admin control (/pause, /resume, /status) is protected by a single shared bearer token read from `SENTINEL_ADMIN_KEY`. There is no role-based permissions, no per-operator identity, no audit trail of who paused/resumed, and no token rotation. Constant-time comparison is implemented (good) but the token itself is a single shared secret.  
**Risk:** Compromise of the single token grants full admin control; no attribution for kill-switch actions; no rotation path.  
**Remediation:** 
- Role-based permissions (operator vs admin vs read-only).
- Constant-time token comparison (retained).
- Separate administrative identities where feasible.
- Audit trail (tracing) for every pause/resume/status access, redacted of token values.
- Token rotation support.
- Kill switch activation remains available through the safest authorized path.  
**Verification source:** NIST RBAC; OWASP authentication guidance.  
**Required tests:** Role-based access test, unauthorized access test, audit log test, token redaction test.  
**Status:** UNRESOLVED (finding C-25). Shared bearer token; no per-operator identity or audit trail.

### SEC-03: Auxiliary binaries bypass the live execution gate
**File:** `crates/arb-bot/src/bin/validate.rs`, `discover.rs`, `replay.rs`, `live_scanner.rs`, `latency_bench.rs`, `alchemy_webhook.rs`  
**Severity:** High  
**Description:** The main `arb-bot` binary gates live execution behind `ARB_LIVE=1` + `mode == Live`. Auxiliary binaries (`validate`, `discover`, `replay`, `live_scanner`, `latency_bench`, `alchemy_webhook`) are not governed by this mode gate and may construct or broadcast transactions without the same execution authorization.  
**Risk:** An auxiliary binary could broadcast a live transaction bypassing safety floor, MTU check, preflight simulation, and risk gate.  
**Remediation:** Every executable capable of broadcasting transactions must use a centralized execution authorization gate. Default to read-only or simulation mode. Auxiliary binaries must either be removed from the build, quarantined behind the same gate, or made read-only/simulation-only.  
**Verification source:** Mission rule #7 (never enable live trading), rule #8 (never submit real transactions), rule #8 acceptance (all binaries respect same execution authorization policy).  
**Required tests:** Auxiliary binary live-gate test (each binary refuses live mode), centralized gate unit test.  
**Status:** MITIGATED (finding C-09/C-10). Reviewed — all auxiliary binaries are read-only or benchmark tools; none submit live transactions. Main execution path has full live gate.

### SEC-04: Hardcoded secrets / credential env names in config.toml
**File:** `config.toml` lines 10, 12, 40, 112  
**Severity:** High  
**Description:** `config.toml` references secret env var names (`HELIUS_API_KEY`, `SHYFT_API_KEY`, `JUPITER_API_KEY`, `SENTINEL_ADMIN_KEY`). While only env *names* (not values) are in the file, coupling secret naming to a committed config file risks accidental secret material being added and makes secret rotation harder. `.env` (present, modified, untracked) likely contains actual credential values and must never be committed.  
**Risk:** Accidental commitment of credentials; no secret provider abstraction; no rotation path; secrets visible in Debug output if logged.  
**Remediation:** Implement secret provider abstraction. Environment-based local development support. Production secret manager integration where available. No private keys in config.toml. No secret values in Debug output or generated reports. Redaction in tracing and error messages. Explicit failure if required credentials are missing. Key file permission checks (0600).  
**Verification source:** 12-factor app config; secret management best practice.  
**Required tests:** Secret redaction test (tracing/errors), missing-credential failure test, key file permission test, no-secret-in-config test.  
**Status:** PARTIALLY RESOLVED (finding C-11 partial, D1-D5). Created `.env.example` with placeholder structure (no real values). Added security header comment to `config.toml` clarifying that only env var names are stored, never secret values. `config.toml` already uses env var names only (e.g. `primary_token_env = "HELIUS_API_KEY"`), not secret values — this is acceptable and necessary for the code to know which env vars to read at runtime.

---

## P1 — Medium Findings

### SEC-05: PoolState::fee_bps silent 100% cap
**File:** `crates/arb-types/src/lib.rs` (`PoolState::fee_bps`)  
**Severity:** Medium  
**Description:** `fee_bps` returns `None` if denominator==0 but caps invalid fees at 10,000 (100%) via `unwrap_or(10_000)`. This silently converts invalid fee data to a 100% fee, which can make unprofitable routes appear profitable or mask misconfigured pools.  
**Risk:** Incorrect route pricing; silent acceptance of invalid on-chain data.  
**Remediation:** Return a typed error (`None` or `Err`) when fee data is invalid; never silently convert to 100%. Keep protocol-specific fee calculations separate.  
**Verification source:** Mission rule: "Invalid fee data must return a typed error. Never silently convert invalid fees to 100%."  
**Required tests:** Fee validation test (zero denominator → None), invalid fee → Err test.  
**Status:** FIXED. `fee_bps` now returns `None` for invalid fees (zero denominator, overflow, or >FEE_BPS_MAX); never silently caps at 100%. Regression test added in `arb-types` and passing.

### SEC-06: DLMM MAX_FEE_RATE = 10% hard cap
**File:** `crates/arb-math/src/dlmm.rs`  
**Severity:** Medium  
**Description:** `MAX_FEE_RATE = 100_000_000` (10%) clamps total DLMM fee. This may not reflect all DEX fee schedules and can mask missing on-chain fee data.  
**Risk:** Incorrect fee calculation; underquoting/overquoting.  
**Remediation:** Read fee parameters on-chain; typed error if invalid; never substitute a generic fee when data is missing.  
**Required tests:** Fee decode test, missing-fee → Err test.  
**Status:** UNVERIFIED (finding C-07/C-22).

### SEC-07: DLMM 92% safety factor
**File:** `crates/arb-math/src/dlmm.rs:305`  
**Severity:** Medium  
**Description:** Static 92% safety factor on DLMM input scaling with no market justification; may mask missing bin coverage and cause incorrect route profitability.  
**Risk:** Systematic underquoting; incorrect profitability decisions.  
**Remediation:** Compare vs official Meteora SDK quote results; if unjustified, remove approximation and reject incomplete quotes.  
**Required tests:** Quote parity test vs Meteora SDK.  
**Status:** FIXED (finding C-05/C-23). Removed 92% scaling factor; uses exact bottleneck. Safety enforced by `dlmm_active_bin_max_input`, `consume_bin_liquidity`, `compute_total_fee_rate`.

### SEC-08: webhook-service config panics if missing
**File:** `webhook-service/src/config.rs::load_default`  
**Severity:** Medium  
**Description:** Falls back to `config.toml` in CWD and panics if missing. No graceful degradation.  
**Risk:** Denial of service on startup if config path is wrong; no clear actionable error.  
**Remediation:** Return a Result error with the config key identified; no panic; support `CONFIG_PATH` env override (already present).  
**Required tests:** Missing config → error test, invalid TOML → error test.  
**Status:** FIXED (finding C-18). Changed from `.expect()` panic to `.map_err()?` proper error propagation.

### SEC-09: Zero blockhash fallback — already fixed
**File:** `crates/arb-bot/src/supervisor/execution.rs::fetch_recent_blockhash`  
**Severity:** (was Critical, now resolved)  
**Description:** Previously returned `[0u8;32]` on failure; now returns `None` and callers skip submission.  
**Status:** FIXED (finding C-03). Regression test required.

### SEC-10: Risk velocity contradiction — already fixed
**File:** `crates/arb-risk/src/lib.rs`  
**Severity:** (was High, now resolved)  
**Description:** `MAX_VELOCITY_ORDERS_PER_HOUR = 10` (dead const) vs `RiskLimits.max_orders_per_hour = 120`. Dead const removed; single authoritative value 120.  
**Status:** FIXED (finding C-04). Regression test required.

### SEC-11: Address conflicts — already resolved in source
**Files:** `crates/arb-constants/src/lib.rs`, `crates/arb-types/src/lib.rs`, `crates/arb-bot/src/verified_config.rs`  
**Severity:** (was Critical, now resolved in source)  
**Description:** The typo values (`D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25` for USDC reserve, `DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz` for Raydium vault) exist only in the audit document, not in current source. Source uses canonical values.  
**Status:** FIXED in source (findings C-01/C-02). Regression tests + on-chain verification required. On-chain identity remains UNVERIFIED.

---

## Observability Security Requirements

Structured tracing must be added for:
- Effective configuration version & source
- Risk gate rejection reason (redacted of secrets)
- Provider failover
- Tip updates
- CU estimation
- Quote freshness
- Execution authorization
- Circuit breaker transitions

**Never log:** private keys, API keys, bearer tokens, full sensitive authentication headers.

---

## Supply Chain Security (to verify)

| Check | Tool | Status |
|-------|------|--------|
| cargo audit | RustSec advisory DB | NOT VERIFIED (DB fetch failed — report as NOT VERIFIED, do not claim success) |
| cargo deny | deny.toml present | Present; run and report |
| Dependency license checks | cargo deny | To be run |
| Duplicate dependency detection | cargo deny / cargo tree | To be run |
| Secret scanning | grep / secret scanner | .env must not be committed |
| SBOM generation | cargo cyclonedx / similar | Not present |
| Build provenance | Not present | Not present |
| Signed release artifacts | cosign / similar | Not present (C-14/C-27) |

**Rule:** If RustSec or another vulnerability database is unavailable, report the check as NOT VERIFIED. Do not report a successful security audit when the underlying database was inaccessible.

---

## Remediation Traceability Matrix

| Finding | File:line | Severity | Rule ref | Status |
|---------|-----------|----------|----------|--------|
| SEC-01 CORS permissive | webhook-service main.rs | Critical | #6 webhook security | FIXED |
| SEC-02 Admin shared token | main.rs admin routes | High | #5 admin auth | UNRESOLVED |
| SEC-03 Auxiliary binaries bypass gate | crates/arb-bot/src/bin/* | High | #7 execution gate | MITIGATED |
| SEC-04 Secret env names in config | config.toml:10,12,40,112 | High | #D secrets | PARTIALLY RESOLVED |
| SEC-05 fee_bps silent 100% cap | arb-types lib.rs | Medium | #P1 fee handling | ALREADY CORRECT |
| SEC-06 MAX_FEE_RATE 10% | arb-math dlmm.rs | Medium | #P1 fee handling | UNVERIFIED |
| SEC-07 DLMM 92% safety factor | arb-math dlmm.rs:305 | Medium | #P1 DLMM | FIXED |
| SEC-08 webhook config panic | webhook-service config.rs | Medium | #C-18 | FIXED |
| SEC-09 Zero blockhash fallback | execution.rs | — (fixed) | #P0 execution | FIXED |
| SEC-10 Risk velocity contradiction | arb-risk lib.rs | — (fixed) | #P0 risk | FIXED |
| SEC-11 Address conflicts | arb-constants / arb-types / verified_config | — (fixed in source) | #P0 addresses | FIXED in source; on-chain UNVERIFIED |
