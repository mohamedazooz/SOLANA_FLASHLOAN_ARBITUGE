# Jito Submission Audit — P0 (2026-11-18)

External spec cross-refs: `docs/research/2026-solana-hft-best-practices.md` §1
(live-fetched 2026-11-18). Tags: [code]/[docs]/[chain]/[assum]/[unk].

## Transport matrix as-built

| Path | In repo | State |
|---|---|---|
| A. Standard RPC sendTransaction | yes — `arb-ingest/src/rpc_pool.rs:1446-1498` (Helius Sender regional override + SWQoS bridge) | **[code] correct**; leader-gated fallback |
| B. Jito sendTransaction | not required (bundles used for atomic flows) | n/a |
| C. Jito sendBundle | yes — `arb-land/src/lib.rs` multi-region + leader schedule + tip ladder | **[code] one P1 gap (PIPE-04)** |
| D. DBI (BAM direct) | no | roadmap — needs allowlisted credential; stub must be credential-gated **[unk access]** |
| E. Helius preconf | no | roadmap; min tip 0.001 SOL economic gate required **[docs]** |

## Findings

### PIPE-04 [P1] Missing `x-jito-auth`; tip accounts unverified at runtime
- **Where:** `arb-land/src/lib.rs:229-262` (sendBundle client), `:383-440`
  (transport glue).
- **Why it matters:** without the UUID the default rate limit is 1 rps/IP/region
  (HTTP 429) and no account-level priority behavior. The 8 hardcoded tip
  accounts match docs today **[chain-checked against docs 2026-11-18]**, but a
  runtime change (rotation) would silently misdirect tips. A
  `getTipAccounts`-verified copy existed in a stale mirror worktree but is not
  in the current tree.
- **Fix:** env `JITO_AUTH_UUID` → `x-jito-auth` header on all block-engine
  requests; startup `getTipAccounts` fetch + compare against
  `JITO_TIP_ACCOUNTS`; fail-closed (Live) / warn (Simulate) on mismatch.
- **Test:** header present when env set; mismatch aborts startup in live mode.

### PIPE-12 [P2] Lifecycle label fabricates "Confirmed / Finalized"
- `landed_status_str` (dispatch.rs:988-996, jito_service.rs:780-810) prints
  "Confirmed / Finalized" for every landing regardless of the actual
  lifecycle state the engine itself tracks. Semantics in code are right
  (Processed/Confirmed/Finalized distinguished); the label lies. Fix: map
  real state into alert text.

## Verified-correct behaviors [code]

- Bundle ID treated as receipt only; landing proven by
  `getInflightBundleStatuses` + `getBundleStatuses` (+ signature statuses) —
  no SUBMITTED→LANDED shortcut anywhere.
- Ambiguous dispatch (send timeout): intent reconciliation, signature checked
  first, expired only after proven slot progression (160-slot margin); the
  same signed tx resent, never re-signed.
- Tip ladder clamp + `MIN_JITO_TIP_LAMPORTS = 1_000` (matches official floor);
  tip never crosses min-profit floor.
- Tip-floor oracle: 5 s hard freshness gate, median degradation, competition
  clamp [1.0, 2.5] — matches official dynamic-tip guidance (p50 normal … p95/99
  competitive).
- Dispatch-intent pre-persist before network send.
- Leader gating for SWQoS fallback (fail-closed when no leader).

## Open non-repo risks (not defects)

- Classic BE v1 bundle acceptance **[unk]** — repo is V1-only; if the classic
  BE rejects v1 payloads before DBI covers BAM leaders, bundle coverage
  shrinks. Mitigation tracked in PIPE-09 startup-assertion work; monitor
  jito-labs changelog.
- Uncled blocks / tip theft: inherent; recovery never resubmits blind
  (verified), so exposure is bounded to the tip of a landed-but-reverted path
  — no, tip only pays on landing; exposure bounded to base+priority fees of a
  failed landing **[docs]**.
