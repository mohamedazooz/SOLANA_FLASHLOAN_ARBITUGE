# KNOWN_ISSUES (consolidated)

**Rule:** every issue here has an owning register entry in `docs/active/MASTER_TASK_REGISTER.md` with full evidence; this page is the operator-facing roll-up. Details live in `docs/active/OPEN_ISSUES.md` and `docs/active/status/INACTIVE_FEATURES.md`.

## Open

| ID | Severity | Component | Summary | Blocking? |
|---|---|---|---|---|
| TXC-N1 | P2 | arb-tx / transports | V1-transport toggles default OFF; requires on-cluster dispatch evidence before enabling | operator (on-cluster probe) |
| EXEC-F-N1 | P2 | programs/arb-executor | Vanity `ArbExec1…` program ID is a placeholder; program not deployed | only for INACTIVE CPI path |
| CFN-N1/N2/N3 | P3 | config | legacy root-config precedence, no `testing` profile, no config-audit log | config-init workstream |
| HFT-N2/N3 | P3 | ingestion | dual-ingest stream consistency unproven at runtime; provider disagreement heuristics | TXC-N1 adjacent |
| BCK/FRT P3 x5 | P3 | web | sha256 login upgrade before non-loopback exposure; misc frontend polish | before any public exposure |
| DIAG-N1 residual | P3 | arb-engine | DLMM limit-order differential fixtures with non-zero LO layers missing | test-fixture work |
| TM12 residual | P3 | arb-store | `record_outcome_detail` not yet threaded through every reconciliation call site | next engine PR |
| PG-CI | P3 | CI | PostgresStore evolution test needs a PG service in CI (no PG sandbox) | infra |

## Resolved (this engagement — see register for evidence)

- Migration-order legacy-DB bug (`idx_outcomes_pnl` before `provenance`) — fixed + regression test (PR #146).
- Quote-gate mis-rejection of first-class DLMM LimitOrder pools — source-verified FunctionType semantics, gate corrected; live re-run 0 QUOTE_FAILED (PR #145).
- Per-mint Kamino reserve wiring (USDC reserve-mismatch noise eliminated) — PR #145.
- Deploy SHA-race, missing health/rollback, mutable action tags — PR #147.
- Double-counted network costs in route net computation; single-unit doctrine (WSOL 9dp / base-mint raw) enforced — phases 13/14 merged work.

## Standing market condition (not an issue, recorded)

All measurable 2-pool WSOL opportunities are net-negative on mainnet as of 2026-11-23 — the engine fail-closed verdict is correct behavior. Re-evaluate when volatile events open real spreads.
