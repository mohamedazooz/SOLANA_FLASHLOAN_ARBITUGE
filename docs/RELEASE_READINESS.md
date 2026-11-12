> **Archive re-validation — 2026-11-08:** The supplied `New.zip` was inspected independently. It has no `.git` directory, root `REPORT.md`, `dashboard/index.html`, or existing `TEST_EVIDENCE.md`. Rust/Cargo are unavailable in the editing environment, so no build/test result is claimed for this exact archive. See [`tasks.md`](../tasks.md) and [`TEST_EVIDENCE.md`](TEST_EVIDENCE.md). The earlier `541cdec` baseline below is historical context only and is not asserted as the identity of this archive. **Live remains NO-GO.**

# Release Readiness

This is the Gate A–F acceptance sheet required by the master prompt. A gate is
`PASS` only with reproducible evidence. Anything not executed is `OPEN` or
`BLOCKED`; code inspection alone is not a pass for a runtime gate.

Baseline: `main` = `541cdec` plus the uncommitted remediation described in
`docs/audit/FINDINGS_REGISTER.md`. No commit, push, archive, or live transaction
is part of this work.

## Gate A — Repository integrity

| Item | Status | Evidence / gap |
|---|---|---|
| Latest GitHub SHA documented | **PASS** | `origin/main` = local `main` = `541cdec…` at start; recorded in `BASELINE.md`. |
| User changes preserved | **PASS** | Pre-existing changes were left intact; remediation is additive. |
| No stale files merged over HEAD without provenance | **PARTIAL** | Old phase docs exist; no provenance audit of every archived file. |
| Every finding classified and linked to evidence/tests | **PARTIAL** | Register updated for this pass; historical `PROMPT` findings remain `NOT REVALIDATED`. |

**Gate A: PARTIAL** — integrity is known, completeness of historical findings is not.

## Gate B — Build and quality

| Item | Status | Evidence / gap |
|---|---|---|
| `cargo fmt --all -- --check` | **PASS** | Exit 0 after `cargo fmt --all`; re-run in Linux container passed. |
| `cargo check --workspace --all-targets` | **PENDING/BLOCKED** | Windows blocked by `yellowstone-grpc-client` Unix socket. Linux Docker result is recorded in `TEST_EVIDENCE.md`; only that result counts. |
| `cargo test --workspace --all-targets` | **PENDING** | Linux run uses `--lib --bins --tests` (bench targets are compiled by check/clippy). Exact counts in `TEST_EVIDENCE.md`. |
| `cargo clippy … -D warnings` | **PENDING** | Linux Docker result in `TEST_EVIDENCE.md`. |
| audit/deny/secret scans | **PARTIAL** | License metadata fixed; `cargo-audit` BLOCKED by RustSec fetch; gitleaks history hits are test-only. |
| Docker build/compose validation | **PENDING** | Compose config was valid before changes; Docker build not yet re-run in this pass. |

**Gate B: BLOCKED** until the Linux check/clippy/test and Docker build results are attached.

## Gate C — Data and quote correctness

| Item | Status | Evidence / gap |
|---|---|---|
| StateIntegrity/fork/gap/reconnect tests | **PARTIAL** | Unit tests for dead slots, recreation, bank IDs, readiness exist; no E2E replay/fault injection. |
| Protocol decoder parity | **OPEN** | No differential fixtures against official SDK/on-chain quotes. |
| No stale/missing/mixed-fork executable quote | **PARTIAL** | Freshness/readiness guards exist; guarded quote is not the only path (`GUARD-01`). |
| Amounts typed, decimals verified | **PASS (source)** | `TokenAmount` + explicit decimals; missing decimals reject. |
| No overflow/default fee fallback in critical paths | **PARTIAL** | Checked math and `Option` fees in reviewed paths; no workspace-wide proof. |
| Split/return hops exactly re-quoted | **OPEN** | Split routing disabled; capability not implemented. |
| One canonical `EconomicQuote` end-to-end | **PARTIAL** | Type exists; not every caller is proven to use it. |

**Gate C: OPEN.**

## Gate D — Atomic execution

| Item | Status | Evidence / gap |
|---|---|---|
| Kamino fee from verified state on all builder paths | **PASS (source)** | `FIN-01`; reserve decoder + ceiling rounding. |
| Borrow→swap→repay semantics proven | **OPEN** | No fork simulation of the complete route (`14.1`/`14.3`). |
| Same guarded quote hash at risk/builder/pre-dispatch | **OPEN** | `GUARD-01`; guarded path has no callers. |
| Simulation Disabled/NotRun/Failed/Timeout rejects | **PARTIAL** | Live: yes. Shadow/canary: simulation optional by design. |
| Min-out/repayment/transaction limits verified | **PARTIAL** | Full floor in single route; no on-chain/fork proof; MTU check exists. |
| Bundle ID never treated as signature/landing proof | **PASS (source)** | Dispatch records acceptance; landing requires status polling. |
| Durable execution state machine + idempotent reconciliation | **OPEN** | Not implemented/tested (`14.5`). |

**Gate D: OPEN.**

## Gate E — Risk, security and operations

| Item | Status | Evidence / gap |
|---|---|---|
| Credentials revoked/rotated; no secrets in source/history/artifacts | **BLOCKED** | Rotation is an operator action; not performed/verified here. |
| Persistent KillSwitch, operator auth, budgets tested | **PARTIAL** | Kill switch persists (test); admin auth implemented; budgets have unit checks; no drill. |
| Dashboard truthful, complete cost/PnL separation | **OPEN** | No dashboard exists; Prometheus + Telegram only. |
| Alerts/metrics/audit journal/restart recovery tested | **PARTIAL** | Metrics/alerts exist; restart recovery test covers kill switch only; no audit journal. |
| Shadow soak + fault-injection report | **OPEN** | Not run. |
| Live disabled by default; no live tx without authorization | **PASS** | Default shadow; `ARB_LIVE` truthy + mode Live; no tx sent. |

**Gate E: OPEN.**

## Gate F — Evidence and release

| Item | Status | Evidence / gap |
|---|---|---|
| `FINDINGS_REGISTER.md` complete | **PARTIAL** | Updated with this pass; historical list not fully revalidated. |
| `PHASE_STATUS.md` 12.1–15 with evidence | **PARTIAL** | Updated honestly; many rows remain OPEN/PARTIAL. |
| `ARCHITECTURE.md` matches runtime | **PASS (source)** | Written from runtime wiring; guarded path gap documented. |
| `PROTOCOL_COMPATIBILITY.md` per protocol/version | **PASS (source)** | Matrix + caveats. |
| `OPERATIONS_RUNBOOK.md` | **PASS** | Covers host/restart/kill switch/provider outage. |
| `SECURITY.md` + rotation procedure | **PASS** | Rotation procedure included. |
| `.env.example` placeholders only | **PASS** | All values are `REPLACE_ME_*`/safe defaults; admin auth note added. |
| Dashboard connected, not mock | **FAIL** | No dashboard in the repository. |
| Release notes + reproducible logs + commit SHA | **OPEN** | No commit created; logs in `TEST_EVIDENCE.md`; no final archive. |
| Final ZIP without `.env`/keys/build artifacts | **OPEN** | Not produced. |

**Gate F: OPEN.**

## Overall verdict

**Not Phase 15 complete. Not production-ready. Live must stay disabled.**

The highest-value remaining work, in order:

1. Wire `quote_route_guarded` + `revalidate_before_dispatch` through the single
   supervised path and assert the same quote hash at risk/build/pre-dispatch
   (`GUARD-01`, `12.14`/`14.2`).
2. Implement a durable execution state machine with idempotent reconciliation and
   ambiguous-timeout tests (`14.5`).
3. Add differential protocol fixtures and a fork simulation of the exact route
   (`12.13`, `13.2`, `14.1`/`14.4`).
4. Build a real dashboard and a 24–72h shadow soak with fault injection
   (`15.1`–`15.3`).
5. Rotate credentials, run a full-history secret scan and `cargo audit`, and
   attach the evidence.
6. Only then consider a controlled-live run with explicit operator approval.
