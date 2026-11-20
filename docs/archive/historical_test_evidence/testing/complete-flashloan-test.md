> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Complete Flash Loan Execution Test — Scenario & Status

> Scope: master directive §14 (25-step scenario). This is the single
> acceptance gate for P11. Status below is **honest, per-step**: each step maps
> to concrete test evidence or is explicitly marked NOT PROVEN.
> Baseline commit: `f72afbc`. Environment: Windows dev box (Linux-only build
> path required for final `scripts/verify-linux.sh`).

---

## Safety posture

All execution defaults remain safe: `ARB_LIVE=0`, `DRY_RUN=true`,
`ARB_V1_TRANSPORT_SUPPORTED=false` (operator opt-in only). This test plan does
**not** include a mainnet transaction until the Live Execution Gate (§16 of the
directive) is satisfied with env-documented caps.

## Step-by-step status

| # | Step | Status | Evidence |
|---|---|---|---|
| 1 | Discover real pool | **test** | arb-ingest discovery tests (142 default features) |
| 2 | Discover real token | **test** | discovery + PumpFun fail-closed tests (DATA-06 @ b3a2cca) |
| 3 | Fresh market data | **test** | slot-integrity / no-stale-fallback tests (DATA-01/02 @ e6dfda6) |
| 4 | Real quote | **test** | exact-quote engine tests (arb-math 154 pass) |
| 5 | Price-diff detection | **test** | opportunity tests (arb-engine 54 pass) |
| 6 | Borrow sizing | **test** | economic + sizing tests; raw-bit guard (MATH-04/05 @ 9a334f8) |
| 7 | All fees computed | **test** | `CostLedger` 8-witness single-count test |
| 8 | Net profit | **test** | signed, integer, mint-aware (f72afbc provenance gate) |
| 9 | Route build | **test** | arb-engine route tests |
| 10 | Flash-borrow instructions | **test** | arb-tx flash tests (compile + ordering) |
| 11 | Swap instructions | **test** | per-protocol builder tests (Raydium V4/CLMM, Orca, Meteora DLMM/DAMM, PumpSwap) |
| 12 | Repay instructions | **test** | repay = principal + fee pressed through klend formula |
| 13 | Tx compile (V1/V0) | **test** | compiler tests (0x81 discriminator, 4096-byte ceiling, no CU-instruction in V1) |
| 14 | Account metas valid | **test** | builder-level metas + atomic-integrity structural checks |
| 15 | Instruction ordering | **test** | atomic integrity + ordering validators |
| 16 | CU budget | **test** | sim-fed CU limit ×1.10, no duplicate budget instructions |
| 17 | Priority fee | **test** | total-lamports V1 config path; same-lands feed (e54bb36) |
| 18 | Jito tip | **test (model)** | adaptive bidder refuses floor-consuming bids; ladder caps |
| 19 | Simulation | **test** | dispatch sim gates path |
| 20 | Signing | **test** | latency_bench + builder tests (real keypath, no key logging) |
| 21 | Submission | **test (canary-grade)** | V1 canaries landed+finalized via publicnode (`reports/v1_canary/`) |
| 22 | Landing verification | **test (canary-grade)** | signature+status checks; recovery reconciliation (PIPE-05 @ 173094e) |
| 23 | Tx metadata read | **test (canary-grade)** | finalization metadata parsed in canary reports |
| 24 | Realized PnL from on-chain balances | **NOT PROVEN live** | storage logic signed+provenance-gated (f72afbc) but no live balance-delta event yet |
| 25 | Persistence | **test** | arb-store tests (SQLx), provenance-gated display |

Legend: **test** = executed in this workspace's gate runs;
**test (canary-grade)** = proven for simple-transfer V1 canaries on mainnet,
not yet for a flash-loan arb transaction;
**cluster / unk** = requires the live gate.

## Verdict

Steps 1-20 pass as unit/integration evidence. Steps 21-23 are proven only for
**V1 transport canaries**, not for a full flash-loan arbitrage transaction.
Steps 24-25 logic is proven, but populated **Realized PnL evidence does not
exist** because no live flash-loan execution was run. Per Definition of Done,
the project is **not** to be described as having passed a live flash-loan
execution.

## Runbook to close the gap (P11→P12)

1. Linux box: `bash scripts/verify-linux.sh` (full workspace + live suites with `.env`).
2. Convert canary into flash-loan rehearsal on **devnet/fork first**, keeping
   all gates from directive §16 (env caps: max borrow, notional, priority fee,
   tip, total loss, attempts) — recorded in `.env.example` documentation.
3. Only after user explicitly sets `LIVE_TRADING_ENABLED=true` and funds a
   dedicated test wallet: single bounded mainnet attempt, then compare
   Expected vs Simulated vs Landed vs Realized into the store.

## Evidence summary

- Test counts at baseline: arb-math **154**, arb-ingest **142** (+16 `extra-ingest`),
  arb-bot **72**, arb-engine **54**, arb-sim **31** — all green in this session's gates.
- Canary: `reports/v1_canary/canary_1794590368.json` verdict
  `V1_TRANSPORT_VERIFIED` (version byte 129, landed+finalized).
- Yellow line: `crates/arb-bot/src/supervisor/*` compiles with the gates; the
  yellowstone gRPC module (`arb-ingest`) is **not compiled on Windows** and its
  final verification belongs to `scripts/verify-linux.sh`.
