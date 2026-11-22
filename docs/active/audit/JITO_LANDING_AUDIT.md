# JITO LANDING AUDIT — Phase 8 (Mission §13)

**Status:** ACTIVE — source-verified 2026-11-22, cross-checked agains the Phase 1
dossier [`docs/active/research/JITO_LANDING_RESEARCH.md`](../research/JITO_LANDING_RESEARCH.md)
(8 tip accounts, min tip 1,000 lamports, acceptance ≠ landing).

---

## 1. Component inventory (evidence)

| Component | File | Status | Key semantics |
|---|---|---|---|
| Bundle submit | `crates/arb-land/src/lib.rs` `send_bundle` (line 458) | ACTIVE | 1..=5 tx guard; base64 + `bundleOnly:true`; multi-engine URL fan-out (global then region-derived engines, deduped, lines 416–431); fan-out returns first `Ok(bundle_id)` / reports all-engine failure with per-engine status (line 520) |
| Multi-region | `send_bundle_multi_region` (line 525) | ACTIVE | Optional auth UUID passthrough |
| Status | `get_bundle_statuses_detailed` (line 565) | ACTIVE | Landing slot + signatures + errors; map to `LandStatus` incl. explicit `Expired` (line 630) |
| Tip accounts | `crates/arb-constants/src/*` `JITO_TIP_ACCOUNTS: [&str; 8]` (line 95) | ACTIVE | `jito_tip_account_for_leader` (hash → index), `..._for_slot`, round-robin; **drift CI test** cross-checks on-chain executor's 8 accounts vs `arb-constants` (workspace test → CI fails on divergence) |
| On-chain validation | `programs/arb-executor/src/validation/jito.rs` `validate_jito_tip_account` | ACTIVE | System-Program-owner + canonical 8; a non-tip account cannot drain proceeds as "tip" |
| Tip pricing | `crates/arb-land/src/bidder.rs` `AdaptiveBidder` | ACTIVE | EV maximization `P(land)·(edge−tip)` over rungs; hurdle = `max(profit_floor, keep_share·edge)`; ceiling `t_max` → **opportunity refused, never repriced** above budget; ladder_growth 1.6 bounded retries; policy `validate()` rejects nonsensical config at construction |
| Tip floor signal | `crates/arb-land/src/tip_oracle.rs` | ACTIVE | REST `tip_floor` + WS `tip_stream`; `DEFAULT_MAX_AGE_MS = 5_000`; **stale snapshot treated as absent, never reused** (module doc + freshness contract) |
| Landing measurement | `crates/arb-land/src/ledger.rs` `LandingLedger` | ACTIVE | Per-price-rung Beta posteriors; **no per-region attribution** (all regions = one auction; per-region posteriors would fabricate attribution — explicit in module doc); schema-versioned (v1) — older schema discarded, not reinterpreted; coverage = Wilson lower bound (z=0.95), 3 lucky landings ≠ certainty; unmeasured rungs fall back to market-implied probability |
| Lifecycle | `crates/arb-land/src/lifecycle.rs` | ACTIVE | 10-state machine `Prepared→Simulated→Signed→Submitted→Processed→Confirmed→Finalized / Failed / Expired / Unknown`; **expiration requires positive on-chain proof** (current block height > `last_valid_block_height`) — polling timeout alone never expires (Invariant 1–2, module header + line 161) |
| SWQoS / RPC fallback | `crates/arb-tx/src/transport/selection.rs` (Phase 5 audited) | ACTIVE, **env-gated OFF** | V1 transports OFF by default; no blind re-dispatch (`arb-store::DispatchIntentRecord` precedes dispatch — `dispatch.rs:734`) |

## 2. Semantic separation (verified)

| Distinction | Where enforced |
|---|---|
| RPC acceptance of `sendBundle` vs bundle **landing** vs tx **confirmation** vs **finalization** | `TransactionLifecycle` states are strictly directional/monotonic; `is_landed()` = {Processed, Confirmed, Finalized}; `landing_status_str` labels only what was proven (lifecycle.rs:66+) |
| Uncled / non-landed | `LandStatus::Expired` set only from engine-reported status (lib.rs:630); non-landed bundles reconcile via `get_bundle_statuses_detailed` — never inferred from silence |
| Retry safety | `Unknown` state forbids resend; recovery reconciles by signature + bundle id before any re-dispatch; `DispatchIntentRecord` makes dispatch intent idempotent (arb-store, wired in `dispatch.rs:734`) |

## 3. Adaptive bidding policy (verified, evidence-gated)

1. Pricing uses **measured** per-rung landing posterior (`LandingLedger.record`), not the
   market prior, whenever observations exist.
2. Coverage is the **Wilson lower bound** — conservative against small samples.
3. Ceiling rule: no opportunity is repriced upward beyond `max_tip_share` of the edge; it
   is refused (`RefuseReason` surfaced via Telegram rejection logs).
4. Higher tips are never assumed to improve net profit — the bidder maximizes expected
   value, not inclusion probability.
5. Tip floor freshness (5 s) + WS push: a stale floor cannot price a bid.

## 4. Honest gaps → `docs/active/OPEN_ISSUES.md`

| Finding | Severity | Gap |
|---|---|---|
| JTO-F-2026-11-22-N1 | P3 | `LandingLedger` is wired but no production soak has produced a statistically meaningful posterior set yet; reports remain at prior fallback until shadow/production data accumulates. |
| JTO-F-2026-11-22-N2 | P3 | Leader-schedule-aware tip placement exists (`jito_tip_account_for_leader`,`leader.rs`) but leader schedule reads are not continuously verified against a second source. |
| JTO-F-2026-11-22-N3 | P3 | `simulate_bundle` available but bundle-level simulation is not mandatorily in the pre-submit ladder for all paths (per-opportunity decision). |

## 5. Failure conduct

- All-engine submit failure → explicit error, no silent fallback to public RPC bundling.
- 1,000-lamport floor enforced as `MIN_JITO_TIP_LAMPORTS` constant.
- Tip validation on-chain prevents misdirection of the tip.
- No unsafe; no panic paths in `arb-land` (grep-audited).

## 6. Tests captured this session

| Suite | Result |
|---|---|
| `cargo test -p arb-land` | **68 passed / 0 failed / 0 ignored** (62 unit + 6 `bundle_rejection_tests`) |
