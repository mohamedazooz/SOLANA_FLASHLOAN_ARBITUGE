# Production Readiness — Gate Checklist

> Scope: master directive §22 (Definition of Done) and §16 (Live Execution
> Gate). One row per requirement, with classification and evidence.
> Audited 2026-11-19. Baseline: `f72afbc`. PR: #76.

---

## Verdict

**NOT production-ready for live trading.** The system is audit-complete
(22/22 pipeline rows closed) and correctness-verified on all executable math
and persistence paths, but a **full live mainnet flash-loan execution has never
been run**, and final Linux verification (`scripts/verify-linux.sh`) is
pending. All safety defaults remain fail-closed.

## Definition of Done checklist

| # | Requirement | Status | Evidence |
|---|---|---|---|
| 1 | Full capture-cycle audited | done | `full-pipeline-audit.md` 22/22 closed |
| 2 | Current official-spec research | done | `docs/research/2026-solana-hft-best-practices.md` (live fetches, dated) |
| 3 | Current Jito conditions verified | done | tip floors fetched 2026-11-18; auth+tip-account runtime verify (PIPE-04 @ b94eac0) |
| 4 | Financial math corrected | done | MATH rows closed; `CostLedger` single-count witnesses @ 9a334f8 |
| 5 | Double counting removed | done + **test** | fee/price-impact/slippage single-count regressions green |
| 6 | Profitability-driven borrow sizing | done + **test** | economic sizing + raw-bit guard (MATH-04/05) |
| 7 | Correct slippage protection | done + **test** | `slippage-audit.md` + min-out regressions |
| 8 | Route/hop optimization | done | arb-engine comparison by expected-net-value |
| 9 | Per-stage latency measured | **partial** | instrumentation exists in code; no committed p50-p99 report — see `latency-audit.md` LAT-1..3 |
| 10 | Correct transactions | done + **test** | compiler tests: V1 0x81 / 4096 B / no ALT / no CU-instr; V0 fallback explicit |
| 11 | Repay validated | done + **test** | klend raw-bit guard + atomic integrity |
| 12 | Submission tested | partial | **canary-grade only** (simple transfers); no flash-loan submission live |
| 13 | Independent landing verification | done for canaries | `reports/v1_canary/` signature+status; not yet for arb tx |
| 14 | Realized PnL from on-chain evidence | **logic done, no live event** | f72afbc signed provenance-gated store; dashboard hides PnL without evidence |
| 15 | Tests pass | done (Windows gate set) | arb-math 154, arb-ingest 142(+16), arb-bot 72, arb-engine 54, arb-sim 31 |
| 16 | Docs + dashboard updated | done | this audit series + web dashboard fields separated Expected/Realized |

## Live Execution Gate (directive §16) status

| Gate | Status |
|---|---|
| User explicit authorization | **not yet given for live run** |
| `LIVE_TRADING_ENABLED=true` | false by default (fail-closed) |
| Dedicated test wallet + risk budget + caps (borrow/notional/priority/tip/loss/attempts) | env-var surface exists; **not configured for a live run** |
| Fresh pool state, successful sim, correct repay, positive conservative net | enforced per-attempt by supervisor |
| Valid signature, no duplicate submission | enforced (PIPE-05 recovery @ 173094e) |

## Known limitations to declare before any live run

1. Linux-only build path: yellowstone gRPC module never compiled on Windows —
   `scripts/verify-linux.sh` must be green first.
2. LAT-1..3 observability gaps (§10 report absent).
3. COMP-1/2 competition monitoring absent (implicit bidding model only).
4. No multi-day soak test has been run.

## Priorities to reach ready

1. Linux full verification + yellowstone compile check.
2. `reports/latency/` committed benchmark (closes LAT-2).
3. P11 rehearsal on devnet/fork, then request explicit live-gate authorization.
