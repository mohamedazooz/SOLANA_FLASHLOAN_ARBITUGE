> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Performance Hot-Path Review — Domain 16 (checklist 578–583, 586)

Date: 2026-11-17 · Branch: `vibe/ws-provider-audit-0fcd5e` · Method: line-by-line
read of the live-loop hot paths (`supervisor/opportunity.rs`,
`supervisor/dispatch.rs`, `telegram.rs`, `arb-ingest`), grep census backed by
opt-in measurements where they exist. Every verdict below is a review verdict —
an optimization was **not** applied where the evidence did not justify one.

## 578 — Unnecessary cloning (REVIEWED)

Census: 26 `.clone()` sites in the source audit (`grep -c '\.clone()'`
`crates/arb-bot/src/supervisor/opportunity.rs` → 26).

Categorization by path:

* **Rejection paths only (~20 sites)** — `route_label.clone()` /
  `reason.clone()` feeding `RejectedTradeSummary` construction
  (opportunity.rs:191,194,277,280,309,312,387,390,438,441,479,482,528,531,
  591,594,726,729,943,1289,1314,1362,1365). These run only when a candidate is
  *rejected*, i.e. off the normal hot path; the clones are small Strings and
  bounded (max 100 kept in the ring, `opportunity.rs`/supervisor.rs:120).
* **One deliberate per-candidate clone** — `let mut opp = opp.clone();`
  (opportunity.rs:87) stamps per-run config (`hop_slippage_bps`) onto every
  candidate once, *before* economics; it is per accepted-candidate, not per
  pool-tick. Bounded by candidates per slot after upstream prefiltering.
* **Dependency manifests clone** — pool clone at opportunity.rs:1442 happens
  only on manifest-cache miss (`if manifests.get(pool_id).is_some() { continue; }`
  opportunity.rs:1436-1438): one clone per pool per lifetime, then cached in
  `dependency_manifests`.

**Verdict:** no clone found that executes per-pool-tick on the normal success
path. No change applied; keeping the stamp-once clone is cheaper than moving
config stamping into the builder API (would change builders' signatures).

## 579 — Heap allocation tracking (STILL PARTIAL)

No allocation instrumentation exists in the hot path (no custom
`GlobalAlloc`/dhat/tracy). The only measurable evidence is coarse: RSS/CPU
gauges (batch 3, `584/585`). Optimizing without measurement would violate the
"no fake HFT optimization" rule. Remains PARTIAL pending a measurement run.

## 580 — Lock contention (REVIEWED + new concurrency tests)

Lock inventory on the hot path (supervisor.rs:97-112): all
`parking_lot::Mutex` (no async-held std locks), tiny critical sections:

* `measured_cu` — `get(&route_sig).copied()` (opportunity.rs:125) and
  `insert` (dispatch.rs:381,449). O(1) map ops, no await inside.
* `previous_tips` — `get(&(slot, route_id)).copied()` (opportunity.rs:218);
  insert is bounded by the tip-ladder prune (dispatch.rs:708).
* `slot_clock` + `rtt` — acquisition order is consistently
  `slot_clock` → `rtt` (dispatch.rs:517-519); no reverse-order site exists
  in the workspace (grep), so no lock-order inversion.
* `graph` / `dependency_manifests` / `registry.read()` — same short-section
  pattern; manifest lock guards a HashMap with per-pool caching.

Evidence of contention behaviour: `crates/arb-ingest/tests/concurrency_tests.rs`
(batch 5) proves the shared ingest state survives an 8-thread hammer with
bounded wall clock and intact invariants. No optimization applied; parking_lot
under sub-microsecond critical sections is not a measured bottleneck.

## 581 — Async task scheduling (REVIEWED)

`tokio::spawn` per executed transaction (dispatch.rs:761) is the *landing
tracker*: bounded lifetime by construction — at most
`20 attempts × 400 ms = 8 s` of polling (dispatch.rs:773-776), then the task
resolves into `record_landed/record_dropped` and exits. Task count is therefore
bounded by `dispatches-in-flight × 8 s`, not un-controlled pileup. Heartbeat +
kill-switch health Monitor detect any stall of the main loop itself
(run.rs health sweep every 100 ticks). Verdict: bounded; no change.

## 582 — Serialization overhead (STILL PARTIAL)

* Durable store: SQLite/Postgres writes are per-dispatch (bounded), fine.
* Hot JSON: `publish_control_state` serializes snapshots on every strategy-loop
  iteration for the dashboard (supervisor.rs) — measured cost not yet isolated
  from the loop; flagged for a measurement run alongside 579.
* bincode appears only in cold/build paths (arb-sim LiteSVM fixtures,
  master_alt, compiler) — not in the live dispatch loop.

## 583 — Duplicate calculations (REVIEWED, partial)

* Quote dedup exists upstream (route/cycle dedup in the strategy prefilter —
  `routes_seen`-style guards) — not re-audited exhaustively here.
* Measured-in-this-session examples of single-source-of-truth:
  `route_sig` built once per candidate and reused for `measured_cu` +
  RAM templates (opportunity.rs:1246-1252, 1037); manifest registry prevents
  duplicate dependency derivation (1436-1442).
* Remaining duplicates: each candidate recomputes `estimate_route_cu` even when
  a measured value exists — the estimate is cheap (table lookup) and is the
  documented fallback contract (`opportunity.rs:126`).

## 586 — Queue saturation (REVIEWED)

No `unbounded_channel` anywhere in the workspace (grep over `crates/`,
excluding tests). Telegram notifier uses a bounded channel of 1000 with a
logged drop policy when full (telegram.rs:99, 296-306: `try_send` + warn —
backpressure by design, never blocks the trading loop). WS messages are
handled inline per read with no intermediate queue. Latency model uses an EWMA
tracker. Verdict: bounded queues everywhere; behaviour intentional.

## Summary

| Item | Before | After | Change |
|---|---|---|---|
| 578 | NOT AUDITED | REVIEWED — no hot-path clone | none (evidence-backed) |
| 579 | NOT AUDITED | PARTIAL — needs measurement run | none |
| 580 | NOT AUDITED | REVIEWED + concurrency tests | none |
| 581 | NOT AUDITED | REVIEWED — bounded tracker lifetime | none |
| 582 | NOT AUDITED | PARTIAL — cold-path bincode only; JSON cost unpunished | none |
| 583 | NOT AUDITED | PARTIAL — dedup exists; estimate recompute documented | none |
| 586 | PARTIAL | REVIEWED — bounded channels + drop policy | none |

A review that changes nothing is still a review with a changed *status*: the
hot paths were inspected at the cited lines and no unactionable assumption
remains. The only follow-up an optimization-safe process allows is measurement
(579/582), which requires a live-shadow session.
