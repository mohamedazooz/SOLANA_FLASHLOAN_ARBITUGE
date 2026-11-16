# Documentation Index

Organized per the Diátaxis principle: each document has one job. If a document
disagrees with the code, the code wins — and the doc should be fixed in the
same PR.

## Start here

- [`PROJECT_STATE.md`](PROJECT_STATE.md) — current state snapshot, verdict, and
  what remains. Read this first.

## Architecture (explanation + reference)

- [`architecture/ARCHITECTURE.md`](architecture/ARCHITECTURE.md) — system
  components and how they connect.
- [`architecture/CODE_TREE.md`](architecture/CODE_TREE.md) — annotated file
  tree: every crate, module, and what it owns.
- [`architecture/BUSINESS_LOGIC.md`](architecture/BUSINESS_LOGIC.md) — business
  analysis: strategy, revenue logic, decision pipeline, risk policy.
- [`architecture/EXECUTION_LIFECYCLE.md`](architecture/EXECUTION_LIFECYCLE.md) —
  opportunity → quote → risk → build → compile → simulate → sign → submit →
  confirm → reconcile.
- [`architecture/PROTOCOL_COMPATIBILITY.md`](architecture/PROTOCOL_COMPATIBILITY.md) —
  per-protocol support matrix, fail-closed behavior.
- [`architecture/WEB_APPLICATION.md`](architecture/WEB_APPLICATION.md) — React
  app, Axum API, control service, security model.

## Operations (how-to)

- [`operations/DEPLOYMENT_RUNBOOK.md`](operations/DEPLOYMENT_RUNBOOK.md) —
  VPS deployment, Docker, HTTPS reverse proxy, environment setup.
- [`operations/OPERATIONS_RUNBOOK.md`](operations/OPERATIONS_RUNBOOK.md) —
  day-2 operation, pause/resume, incident response.
- [`operations/MASTER_LIVE_EXECUTION_ROADMAP.md`](operations/MASTER_LIVE_EXECUTION_ROADMAP.md) —
  gated path from shadow to live (operator decision required).
- [`operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md`](operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md) —
  three-pool V1 live activation plan.

## Economics (domain explanation)

- [`economics/ECONOMICS_AND_SIZING.md`](economics/ECONOMICS_AND_SIZING.md) —
  cost model, borrow sizing, net-profit enforcement.
- [`economics/THREE_POOL_FLASH_NET_ECONOMICS_2026-11-12.md`](economics/THREE_POOL_FLASH_NET_ECONOMICS_2026-11-12.md) —
  three-pool route economics worked example.
- [`economics/P0_FEE_ACCOUNTING_EVIDENCE.md`](economics/P0_FEE_ACCOUNTING_EVIDENCE.md) —
  fee accounting evidence.

## Reference

- [`reference/SDK_PARITY.md`](reference/SDK_PARITY.md) — differential parity
  with official SDKs (Meteora, Kamino, Solana).
- [`reference/V1_LIVE_EXECUTION_PATH_2026-11-12.md`](reference/V1_LIVE_EXECUTION_PATH_2026-11-12.md) —
  Solana Transaction V1 wire format and transport.
- [`reference/METEORA_TWO_POOL_V1_2026-11-12.md`](reference/METEORA_TWO_POOL_V1_2026-11-12.md) —
  two-pool V1 builder details.
- [`reference/V1_ONLY_RELEASE_POLICY.md`](reference/V1_ONLY_RELEASE_POLICY.md) —
  V1-only release profile policy.
- [`adr/`](adr/) — architecture decision records.

## Quality (evidence)

- [`quality/TEST_EVIDENCE.md`](quality/TEST_EVIDENCE.md) — test campaign
  commands and results.
- [`quality/SECURITY_AUDIT_2026-11-12.md`](quality/SECURITY_AUDIT_2026-11-12.md) —
  security audit register.
- [`quality/SECURITY.md`](quality/SECURITY.md) — secret inventory and rotation.
- [`quality/PRODUCTION_READINESS_2026-11-13.md`](quality/PRODUCTION_READINESS_2026-11-13.md) —
  readiness gates and verdict.
- [`quality/REPO_DISCOVERY_2026-11-13.md`](quality/REPO_DISCOVERY_2026-11-13.md) —
  repository discovery + remediation register.

## Archive (frozen history — provenance only)

[`archive/`](archive/) contains superseded phase reports, older audits, and
research notes. They document how the system got here. Do not cite them as
current behavior; do not update them.
