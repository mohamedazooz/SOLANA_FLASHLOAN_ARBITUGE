# JITO BENCHMARKS — Phase 8 (Mission §13)

**Status:** ACTIVE — 2026-11-22. Binding honesty rule: **no landing-rate, tip-cost or
net-profit number is reported without measured history**. None has yet been produced by a
production soak in this session, so the tables below present the *measurement contract*
(the exact recorded quantities and their sources), not invented values.

---

## 1. Metrics defined and wired (measurement contract)

| Metric | Source | Status |
|---|---|---|
| landing rate (realized) | `LandingLedger` (`landings/attempts`, per price-rung posteriors) | WIRED — awaiting soak data |
| inclusion latency | `observe_landing_latency_ms` (`arb-metrics`; call site `dispatch.rs:1039`) | WIRED |
| tip cost | bidder tip decision (`TipPlan`) + on-chain tip delta | WIRED (plan); realized tip reconcilable from bundle status signatures |
| net profit after tip | `calculate_net_profit_raw` CostLedger (Phase 4, EI-gated) incl. tip rung | WIRED |
| failed landing cost | priority fee + base fee write-off; recorded via `record_outcome` failure rows | WIRED (dispatch outcomes) |
| region performance | **deliberately not attributed** — all regions resolve one auction (`ledger.rs:1–10`) | BY DESIGN |
| leader-specific performance | `leader.rs` + `jito_tip_account_for_leader` | WIRED (placement only) |
| submit RTT | `observe_submit_rtt_ms` (`dispatch.rs:880`) | WIRED |
| adaptive bid EV | `AdaptiveBidder::expected_value` per rung | WIRED |

## 2. Prior / literature values (NOT project measurements)

From Phase 1 research dossier (`docs/active/research/JITO_LANDING_RESEARCH.md`):
min tip 1,000 lamports; tip-floor percentiles published by `bundles.jito.wtf`
(`tip_floor` REST / `tip_stream` WS, freshness 5 s); acceptance ≠ landing; regional
endpoints handled by URL fan-out in `arb-land/src/lib.rs:416–431`. These are inputs to
pricing, never reported as observed performance.

## 3. Benchmark execution plan (what would make numbers real)

1. **Shadow soak ≥ 24 h** (Phase 17) recording: opportunities, simulated results, tip
   plans, dispatch intents, bundle statuses → fills `LandingLedger` observations.
2. A posterior with `observations > 0` per rung switches `coverage()` from market prior
   to Wilson lower bound — the point at which adaptive bidding becomes evidence-driven.
3. Local criterion benchmarks that CAN run offline:
   - `cargo bench -p arb-ingest --bench dual_ingest_bench` (present)
   - `cargo bench -p arb-math --bench cpmm_bench` (present)
   - Bench runs were **not executed in this session**; no local numbers reported.
4. Jito `simulateBundle` availability check per region recorded before treating
   bundle-sim as part of every submit path (JTO-F-2026-11-22-N3).

## 4. Test evidence captured this session (2026-11-22)

| Suite | Result |
|---|---|
| `cargo test -p arb-land` | **68 passed / 0 failed / 0 ignored** |
| `cargo test -p arb-tx` (transport/lifecycle interplay) | 434/0 (Phase 5, re-run this session chain) |

## 5. Findings

- JTO-F-2026-11-22-N1 (P3): ledger posteriors at prior state — no production soak yet.
- JTO-F-2026-11-22-N2 (P3): leader schedule single-source.
- JTO-F-2026-11-22-N3 (P3): bundle-sim not mandatory per-path.
