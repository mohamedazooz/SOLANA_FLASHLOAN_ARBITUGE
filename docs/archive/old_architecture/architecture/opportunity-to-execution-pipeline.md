> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Opportunity-to-Execution Pipeline — Architecture Reference

> Diátaxis: **reference + explanation**. Source of truth for every stage of the
> capture cycle: input, output, source of truth (SoT), calculations, dependencies,
> rejection reasons, failure modes, and how to test each stage.
> Audited 2026-11-18 (P0). Findings referenced as MATH-xx / PIPE-xx / DATA-xx per
> `docs/audit/full-pipeline-audit.md`.

```
Market Data → Pool Discovery → Token Identification → Pool State Validation
→ Price Normalization → Candidate Generation → Route Construction
→ Exact Swap Quoting → Borrow Sizing → Cost Calculation → Profitability Decision
→ Slippage Validation → Transaction Construction → Simulation → Submission
→ Landing Verification → Realized PnL → Persistence
```

---

## Stage 1 — Market Data (ingestion)

| | |
|---|---|
| **Input** | Yellowstone gRPC account streams (pool + oracle accounts), RPC `getMultipleAccounts`, DexScreener/GECKO metadata REST providers |
| **Output** | Versioned `PoolSnapshot { slot, state }` published atomically into `arb-ingest` registry |
| **SoT** | On-chain account bytes at a named slot. **Never** cached REST data for prices on the executable path |
| **Mechanism** | gRPC decode (zero-copy LE parsing) → `slot_integrity` barrier → batched snapshot publication |
| **Rejections** | Account owner mismatch; decode length mismatch; slot regression; provider unhealthy (fail-closed — no fallback to stale data) |
| **Failure modes** | DATA-01: `get_multiple_accounts_with_slot` mixes chunk slots on fallback and drops chunks silently → mixed-slot snapshots enter validation upstream. DATA-09: triple mutex acquisition per update (latency). DATA-08: gRPC account stream does not feed pool registration (dead ingestion capacity) |
| **Tests** | Unit: decoder golden bytes per protocol. Regression: mixed-slot batch rejected (DATA-01 fix). Perf: snapshot publish p99 |

## Stage 2 — Pool Discovery

| | |
|---|---|
| **Input** | RPC `getProgramAccounts` (Raydium V4/CPMM/CLMM, Orca Whirlpools, Meteora DLMM/DAMM v2, PumpSwap AMM, Kamino reserves) + periodic DexScreener refresh |
| **Output** | Pool registry entries keyed by `(program, pool_address)`, deduped |
| **SoT** | On-chain program accounts; program IDs from `arb-constants` (all verified live 2026-11-18, including CAMM `CAMMCzo5…`) |
| **Mechanism** | Per-protocol discovery modules (`arb-dex/src/*/discovery*`); dedup by address; authority/PDA re-derivation for PumpSwap |
| **Rejections** | Uninitialized pool; wrong owner program; duplicate address |
| **Failure modes** | DATA-02: DexScreener "raydium"-labeled pools default to AMMv4 decoder → wrong quote math. DATA-03: Meteora CPMM `fee_numerator` mis-cast u16 → fee ×100 error. DATA-05: DAMM v2 / DBC / DLMM-Pro discovered but no decode branch → priced by neighbor only if bridged (must be excluded from executable routes until decoders exist) |
| **Tests** | Integration: discovery against live RPC skips gracefully when rate-limited (0 pools = env, not bug). Unit: fee field parse for each protocol (DATA-03 regression) |

## Stage 3 — Token Identification

| | |
|---|---|
| **Input** | Mints from pool vaults + (optional) platform metadata (creator fees for PumpBOOST) |
| **Output** | `TokenInfo { mint, decimals, token_program, transfer_fee_extension? }` |
| **SoT** | Mint account decode (SPL Token / Token-2022) |
| **Rejections** | Mint authority present + unverified token (risk gate); freeze authority active; Token-2022 transfer-fee mint on unpriced path (must carry `TransferFeeConfig` deduction in math or reject) |
| **Failure modes** | Mismatched decimals between modules (audit: none found — decimals carried in snapshots end-to-end, VERIFIED-CORRECT) |
| **Tests** | Fuzz: extreme decimals (0, 9, 14) through quote chain; Token-2022 transfer-fee mint arithmetic |

## Stage 4 — Pool State Validation

| | |
|---|---|
| **Input** | Latest snapshot per pool |
| **Output** | Quote-ready flag per pool |
| **Mechanism** | `quote_readiness` gate: status/liquidity/threshold checks per protocol (klend status@4856, Raydium V4 status/need_take_pnl, CLMM active tick validity, DLMM active-bin sanity) |
| **Rejections** | Stale slot beyond `max_state_age_slots`; status bits disabled; zero/absurd liquidity |
| **Failure modes** | PIPE-08: shipped config loosens `max_state_age_slots` to 300 — violates freshness posture in Live; must be capped |
| **Tests** | Regression: stale snapshot → opportunity skipped, never executed |

## Stage 5 — Price Normalization

| | |
|---|---|
| **Input** | Raw reserves / sqrt-price / active bin |
| **Output** | Canonical integer spot price per direction (Q or fixed-point per protocol) |
| **SoT** | Protocol invariant math in `arb-dex` adapters (the only quoting code path) |
| **Mechanism** | Spot from invariant (CPMM), Q64.64 sqrtP (CLMM), bin price with bin-step (DLMM) |
| **Failure modes** | Spot price must **never** be used as executable quote downstream (see Stage 8) — audit confirmed quotes are computed, spot only used for pre-filtering |
| **Tests** | Golden vectors per protocol vs known-good outputs |

## Stage 6 — Candidate Generation

| | |
|---|---|
| **Input** | Quote-ready pools, per-mint adjacency |
| **Output** | Pre-filtered multi-hop cycles per base mint |
| **Mechanism** | Hop-limited cycle enumeration bounded by CU/account/byte budgets; rejects routes whose account count or size cannot fit a V1 transaction |
| **Rejections** | Cycle reuses same pool consecutively (guaranteed loss by fees); hops exceed compile budget; mints mismatch at a seam |
| **Tests** | Unit: cycle closure + mint-seam checks; route-hash determinism |

## Stage 7 — Route Construction

| | |
|---|---|
| **Input** | Candidates |
| **Output** | Ranked routes (initially by gross spread; final ranking after Stage 10) |
| **Failure modes** | PIPE-13: strategy-level sort mixes different-mint net profits for comparison — cross-mint sorting must normalize to a common quote (WSOL) before comparing. ROADMAP item, P4 phase |
| **Tests** | Property: same-mint comparison only |

## Stage 8 — Exact Swap Quoting

| | |
|---|---|
| **Input** | Route + snapshot state + nominal input |
| **Output** | `RouteQuote { per-hop out, min_out per hop, cu estimate }` — executable math, not spot |
| **SoT** | `arb-dex` adapter quote functions, integer math matching program rounding (checked_add/mul/div) |
| **Mechanism** | Sequential hop composition: hop N+1 consumes hop N's actual output. Price impact is **inherent** in each quote (output net of trade against the curve) — never re-deducted (audited: no double counting found) |
| **Rejections** | Any hop slips below its own curve bounds; intermediate mint unsupported for ATA creation |
| **Failure modes** | DATA-04: CLMM/DLMM per-quote tick/bin clone churn (perf, not correctness); MATH-03: Pump paths approximate the Kamino flash fee `sf >> 32` instead of canonical U68F60 (underprices fee ~0.1bps→~167bps gap at boundary — must route through `arb-math` empirical) |
| **Tests** | Golden test: single-hop vs multi-hop composition consistency; regression MATH-03: pump cycle uses `fee_from_sf(sf)` exactly |

## Stage 9 — Borrow Sizing

| | |
|---|---|
| **Input** | Route quote function + Kamino reserve live liquidity |
| **Output** | Optimal borrow `B*` (closed-form for CPMM cycles; bounded coarse-to-fine search for piecewise CLMM/DLMM), bounded by `min(supply_vault_liquidity, config_ceiling)` |
| **SoT** | `arb-engine` optimizer + live supply-vault decode (fail-closed: unverifiable liquidity ⇒ skip) |
| **Mechanism** | Expected-Net-Value maximization under all costs; pieces tested at bin/tick boundaries, NOT naive binary search over non-concave region |
| **Failure modes** | Supply vault bypass = **never** (verified). Config ceiling alone = **never** (verified — invariant holds) |
| **Tests** | Property: B* ≤ vault liquidity always; scalar: B* maximizes EV in golden synthetic cycle; boundary: piece breakdown at tick transitions |

## Stage 10 — Cost Calculation (Cost Ledger)

| | |
|---|---|
| **Input** | B*, route, CU estimate, current tip floor |
| **Output** | `CostLedger`: flash fee (canonical `arb-math`, single source — kamino.rs re-export), swap fees (**inside** quotes — never re-added, audited), priority fee (adaptive CU pricing, once), Jito tip (clamped, once), base fee 5,000, transfer fees if Token-2022, rent for ATA closes (reclaimed, net ~0) |
| **SoT** | `arb-math/src/economic.rs` |
| **Failure modes** | MATH-04: `ExecutionCosts` unit ambiguity (lamports vs base-token) — latent, WSOL-only today; enforce unit types when any non-WSOL quote path lands. MATH-05: `CostLedger` struct defined but unused in production path (dead abstraction or missing integration — decide: consume it in opportunity.rs, or delete) |
| **Tests** | Golden: no double-added swap fee; tip clamp `net_after_tip ≥ MIN_NET_PROFIT`; priority fee deduped exactly once |

## Stage 11 — Profitability Decision (hard gates)

| | |
|---|---|
| **Input** | B*, Costs, MinOuts |
| **Output** | Go / No-Go + sized Opportunity |
| **Mechanism** | Signed integer PnL in `build_economic_quote` (i128, VERIFIED-CORRECT); gates: `expected_net > MIN_NET_PROFIT` **after** tip; slippage bounds per hop |
| **Failure modes** | MATH-01 (P0): dispatch stores realized PnL via `u64::try_from` → **losses silently dropped or worst-case flipped to estimates**; PIPE-01: `pnl_daily` aggregates without `status='landed'` filter; PIPE-03: metrics consume estimate, producing false-positive dashboards. These are P1 correctness fixes in the accounting layer — decision logic itself is sound |
| **Tests** | Regression: negative realized round-trips through store as negative (MATH-01 fix); aggregation excludes simulated/canary (PIPE-01 fix) |

## Stage 12 — Slippage Validation

| | |
|---|---|
| **Input** | Per-hop min_out from Stage 8, fresh state slots |
| **Mechanism** | min_out = curve output × (1 − per-hop tolerance), applied **once per hop as a floor**, tolerance dynamic (volatility/pool-type). Never deducted from expected profit (no double count — audited) |
| **Rejections** | Route fails freshness gate; tolerance exceeds protocol maximum |
| **Tests** | Fuzz: malformed pool with zero liquidity → min_out rejects; property: min_out ≤ quoted out |

## Stage 13 — Transaction Construction

| | |
|---|---|
| **Input** | Opportunity (borrow, legs, min_outs, tip) |
| **Output** | V1 transaction: budget config in `Message.config` (no ComputeBudget ixs), idempotent ATA creation, flashBorrow → legs → flashRepay(principal) → close ATAs → tip **last** (`tip_ix_count > 0`) |
| **SoT** | `arb-tx` compiler + `adaptive.rs`; `atomic_integrity::verify_atomic_integrity` pre-sign gate |
| **Rejections** | Atomicity violation; `borrow_instruction_index` remap correct (adaptive.rs:436-452); byte ceiling 4,096; ≤64 accounts |
| **Failure modes** | PIPE-10: duplicated CU math between compiler and flash modules — consolidate. V1-only compliance verified |
| **Tests** | Unit: assembler golden layouts; Property: verifier rejects budget-ix leftovers in V1 |

## Stage 14 — Simulation

| | |
|---|---|
| **Mechanism** | Local sim (mock or forked) first; Live requires RPC-sim pass with CU readback feeding margin (×1.10) into `Message.config` |
| **Rejections** | Any sim error, CU over-run, byte over-run |
| **Failure modes** | Simulation ≠ profitability: enforce that dashboards never show a sim pass as realized profit (PIPE-03 fix covers) |
| **Tests** | Integration: sim-fail path aborts before sign; CU margin recomputed on dev-readback |

## Stage 15 — Submission

| | |
|---|---|
| **Mechanism** | Jito sendBundle to region set routed by leader schedule (`go_to_next_leader`, tip ladder clamp); SWQoS/SPQoS fallback with leader gate; **pre-persist dispatch intent** (supervisor/recovery.rs) |
| **Failure modes** | PIPE-04: no `x-jito-auth` header → production rate-limits unmitigated; fix to attach header from env. Ambiguous dispatch ⇒ reconciliation only (never blind resubmit — verified correct) |
| **Tests** | Regression: 429 path is an error event, never a silent success; intent persisted before send |

## Stage 16 — Landing Verification

| | |
|---|---|
| **Mechanism** | `getInflightBundleStatuses` → `getBundleStatuses` lifecycle (Processed/Confirmed/Finalized); recovery waits past 160-slot blockhash margin; signature status checked before "expired" |
| **Failure modes** | PIPE-12: `landed_status_str` labels all as "Confirmed / Finalized" regardless of actual lifecycle — fix to reflect real status. Bundle ID receipt ≠ landing (semantics already honored in code) |
| **Tests** | Mock jito: each lifecycle transition maps to correct state name |

## Stage 17 — Realized PnL (from on-chain evidence only)

| | |
|---|---|
| **Input** | Landed signature → balance deltas on-attested accounts |
| **Mechanism** | Actual mode reads on-chain balances; estimate mode explicitly labeled; **Signed** i64 accounting (MATH-01 fix phase) |
| **Rejections** | No on-chain proof ⇒ Realized stays null (dashboards never backfill estimates) |
| **Failure modes** | MATH-01 (the P0 accounting defect), MATH-02 (estimate feeds metrics pre-dispatch) |
| **Tests** | Golden: landed winning tx → realized signed delta; landed losing tx (tip > spread) → negative realized stored, not estimate |

## Stage 18 — Persistence

| | |
|---|---|
| **Mechanism** | `arb-store` attempts ledger + intents reconciliation. Metrics dashboards: Expected vs Realized in separate fields |
| **Failure modes** | PIPE-11: slot-linear `attempt_id` (INSERT OR REPLACE) + route-id collision window; PIPE-05: `send_failed` intents never reconciled — P1 phase items |

---

## Cross-stage invariants (must never break)

1. One discovery pipeline, one economic calculator (`arb-math`), one execution entry (`Supervisor`). Freshness fail-closed everywhere.
2. Cost each paid **exactly once**; quote-derived costs never re-added; slippage only ever a min-out floor.
3. Snapshot slot consistency per opportunity (`snapshot_slot` recorded; stale ring-fenced).
4. Atomicity: singe-tx borrow→legs→repay enforced by verifier.
5. No fabricated data anywhere: 0 pools / sim-only / failed probes are explicit, never silently bridged by stale or fabricated values.
