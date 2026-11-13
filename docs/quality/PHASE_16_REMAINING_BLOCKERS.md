# Phase 16 — Remaining Blockers

**Date:** 2026-11-13
**Rule:** a blocker closes only with independently verifiable evidence, never by
declaration. Ordered by criticality for a live launch decision.

## Critical (must close before any production claim)

### B-1 · 24-hour shadow soak — ❌ OPEN
- **Why:** no ≥24 h continuous run exists.
- **Needed:** a VPS shadow run per `PHASE_16_SHADOW_SOAK_REPORT.md`, with
  uptime proof and machine-readable metrics for the full window.
- **Owner:** operator with Helius credentials + persistent host.

### B-2 · Authorized mainnet broadcast + on-chain confirmation — ❌ OPEN
- **Why:** live is disabled; no signed transaction was sent; no signature or
  slot exists.
- **Needed:** explicit written authorization, `ARB_LIVE=1` +
  `SOLANA_TX_V1_DIRECT_ENABLED=1`, a funded signer, and a confirmed signature
  with `getSignatureStatuses`/`getTransaction` evidence. Until then, **never
  claim execution**.
- **Owner:** human owner (per `LIVE_READINESS_CHECKLIST.md`).

### B-3 · Live non-broadcast simulation needs a funded payer — ❌ OPEN
- **Why:** `mainnet_non_broadcast_simulation.rs` fails with
  `InvalidAccountForFee` because no `SOLANA_PRIVATE_KEY` is available and the
  test falls back to an unfunded seed. This is the only test failure in the
  suite (2 tests).
- **Needed:** either skip with an explicit "no funded payer" reason, or inject
  a dedicated, minimal-funded simulation-only key. Do **not** weaken assertions.
- **Impact:** the Raydium/Orca live-simulate acceptance evidence is currently
  absent.

### B-4 · Soak metrics are not persisted — ❌ OPEN
- **Why:** the store schema lacks gross/net breakdown, CU, fees, rejection
  reasons, and latency (see `PHASE_16_SHADOW_SOAK_REPORT.md` §4).
- **Needed:** schema migration + writer + export, then re-run the soak.

### B-5 · Real (non-synthetic) end-to-end route evidence — ❌ OPEN
- **Why:** size-ladder "real routes" use `Pubkey::new_unique()` bin arrays; the
  live probe only simulates a V1 envelope, not each venue's swap.
- **Needed:** fork/litesvm or mainnet-simulate a real Kamino→DLMM route with
  real bin arrays and confirm program logs + CU.

## High (needed for a defensible production decision)

### B-6 · Per-venue acceptance matrix — ⚠️ PARTIAL
- Meteora DLMM has structural + size evidence; Raydium V4/CPMM/CLMM, Orca,
  Pump.fun, PumpSwap, Meteora DAMM v2, Raydium LaunchLab do **not** have
  differential + fork acceptance evidence.
- Jupiter is a quote adapter only and is not wired into execution (ADR-0008).

### B-7 · Documentation drift — ⚠️ PARTIAL (this PR)
- `PROTOCOL_COMPATIBILITY.md` (2026-11-08) and ADR-0007 still describe
  Pump.fun/PumpSwap as disabled; source shows them enabled. Reconciled in
  `PHASE_16_PROTOCOL_MATRIX.md`; the original docs should be marked superseded
  or updated by their owners.

### B-8 · `verify_atomic_integrity` over-claimed checks — ✅ DOC FIXED
- Doc comment corrected in this PR to state that account-meta/token-program
  checks are advisory and that size checking is a legacy lower bound only.
  Functional gap remains: the function cannot validate V1 size or mint chains
  (delegated to builders).

## Low / hygiene

- `reports/phase15_2/*.json` and `V1_SIZE_LADDER.json` are rewritten by test
  runs (timestamps), producing diff churn; consider gating artifact writes
  behind an env flag.
- README/AGENTS still claim a Linux-only build; the workspace builds on
  Windows (verified).

## Gate summary

| Live-readiness gate | State |
|---|---|
| 1. Code & tests | ✅ (offline) / ⚠️ 2 live-payer tests |
| 2. Shadow soak ≥24 h | ❌ |
| 3. Live broadcast evidence | ❌ |
| 4. Per-venue acceptance | ⚠️ |
| 5. Metrics persistence | ❌ |
| 6. Signed human authorization | ❌ |

**Overall: NOT PRODUCTION READY.**
