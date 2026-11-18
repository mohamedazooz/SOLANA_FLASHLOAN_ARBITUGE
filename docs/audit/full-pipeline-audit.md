# Full Pipeline Audit — P0 (2026-11-18)

> Master index of P0 audit findings. Classification tags: **[code]** verified in code,
> **[docs]** verified by official documentation, **[sim]** verified by simulation,
> **[chain]** verified on-chain, **[assum]** assumption, **[unk]** unverified.
> Severity: P0 (money loss / false accounting), P1 (correctness), P2 (robustness).

## Method

Five parallel passes over the repository at `main` = `bad3042`:
three read-only code audits (math/economics, end-to-end pipeline, data/ingest) and two
live research passes (Jito / BAM, Solana protocol + Kamino). Evidence includes exact
file:line locations read first-hand.

## Findings

### Accounting (correctness of money)
| ID | Sev | Summary | Location | Status |
|---|---|---|---|---|
| MATH-01 | P0 | Realized PnL stored via `u64::try_from`; negative deltas dropped, then overwritten with an estimate fallback | `arb-bot/src/supervisor/dispatch.rs:869-881, :929-932`; `arb-store` `postgres_store.rs:190; lib.rs:205,331` | fixed @ f72afbc |
| MATH-02 | P1 | Metrics consume the **pre-dispatch estimate**, not the on-chain actual (`add_net_profit` at dispatch time) | `dispatch.rs:947` | fixed @ f72afbc |
| PIPE-01 | P1 | `pnl_daily` aggregates without `status='landed'` filter; simulated/canary rows contaminate daily PnL | `arb-store/src/lib.rs:305-334`; `postgres_store.rs:260-290` | fixed @ f72afbc |
| PIPE-03 | P1 | Dashboard "net" figures derive from MATH-02 estimate feed | dispatch/store path | fixed @ f72afbc |
| MATH-03 | P1 | Pump-path flash-fee approximated `sf >> 32` instead of canonical U68F60 `fee_from_sf` | `strategy.rs:454`, `pump_boost_arb.rs`, `pump_fun_raydium.rs` | fixed @ 7f4693c |
| MATH-04 | P2 | `ExecutionCosts` unit ambiguity (lamports vs base token) — latent while WSOL-only | `arb-math/src/economic.rs` + `economic_integration.rs` | fixed @ 9a334f8 |
| MATH-05 | P2 | `CostLedger` defined, never consumed by production path | `arb-math/src/economic.rs` | fixed @ 9a334f8 |
| PIPE-13 | P2 | Strategy ranking mixes different-mint net profits (no common quote) | supervisor strategy sort | fixed @ 3582d67 |
| PIPE-06 | P2 | Failed-tx loss booked as flat 5,000-lamport constant (ignores priority fee + tip actually spent) | `dispatch.rs:1045` | fixed @ fcfb93a |

### Ingest / data integrity
| ID | Sev | Summary | Location | Status |
|---|---|---|---|---|
| DATA-01 | P1 | `get_multiple_accounts_with_slot` fallback mixes per-chunk slots and silently drops failed chunks | `arb-ingest/src/rpc_pool.rs:1112-1201` | fixed @ 9c66cbd |
| DATA-03 | P2 | Meteora CPMM `fee_numerator` cast to `u16` → 2,500 ppm reads as 2,500 bps (×100 fee on discovery only) | `meteora_pair_discovery.rs:240-241, :389-390` | fixed @ 912e22d |
| DATA-02 | P2 | DexScreener unlabeled raydium pools default to AMMv4 decoder | dexscreener fetcher | open |
| DATA-05 | P2 | DAMM v2 / DBC / DLMM-Pro discoverable but unpriceable (no decode branch) — safe only because never routed; requires explicit exclusion | arb-dex | open |
| DATA-04 | P2 | Full tick-array/bin-array clone per quote (perf) | `adapters.rs:673, 687-705, 752-754` | fixed @ a12296d |
| DATA-06 | P2 | Unofficial pump.fun endpoints; mock-tolerant fallbacks | pump facade | open |
| DATA-07 | P2 | Phoenix unsupported (no variant) — documented limitation | arb-dex | open |
| DATA-08 | P2 | Yellowstone gRPC account stream does not feed pool registration (identity placeholder) | arb-ingest gRPC path | open |
| DATA-09 | P2 | `slot_integrity` mutex acquired 3× per update | arb-ingest | fixed @ 04f0112 |
| DATA-10 | P2 | `PacedAccountFetcher::from_env` per-call construction → TLS handshake churn | arb-ingest | fixed @ da2fa07 |
| DATA-11 | P2 | Priority-fee estimate fabricates 10,000 lamports/CU on parse failure instead of surfacing | `rpc_pool.rs` get_priority_fee_estimate | fixed @ a7ae487 |

### Pipeline / execution
| ID | Sev | Summary | Location | Status |
|---|---|---|---|---|
| PIPE-04 | P1 | Jito `sendBundle` sends **no** `x-jito-auth` header; tip accounts hardcoded, runtime `getTipAccounts` verification absent | `arb-land/src/lib.rs:229-262, :383-440` | fixed @ b94eac0 |
| PIPE-09 | P2 | AGENTS.md says "V0/V1 compiler" while code is V1-only (sim + live); Simulate/Live flag asymmetry unasserted at startup | dispatch + docs | closed — see Closure notes |
| PIPE-05 | P2 | `send_failed` dispatch intents never reconciled | supervisor/recovery.rs | fixed @ 173094e |
| PIPE-07 | P2 | `KILL_SWITCH_DISABLED=1` full bypass; shipped `.env:238` has `BYPASS_RISK_GATE=1` | risk gate | fixed @ 2b798aa |
| PIPE-08 | P2 | Shipped config weakens gates: `max_state_age_slots` 2→300, `max_tip_share_bps` 9500, `max_inflight_bundles` 32, `max_daily_loss_usd` 500 | config.toml | fixed @ 409b387 |
| PIPE-10 | P2 | Duplicated CU math between `flash::compiler` and adaptive compiler | arb-tx | fixed @ eb8c03b |
| PIPE-11 | P2 | Slot-linear `attempt_id` (INSERT OR REPLACE) + route-id collision window; no cross-tick opportunity dedup | arb-store | fixed @ 14231ec |
| PIPE-12 | P2 | `landed_status_str` hardcodes "Confirmed / Finalized" regardless of actual lifecycle | `dispatch.rs:988-996`, `jito_service.rs:780-810` | fixed @ fcfb93a |
| PIPE-14 | P2 | Serialized blocking RPC probes on the hot path | supervisor | closed — see Closure notes |

## Closure notes (2026-11-18)

### PIPE-09 — closed (research correction + verified consistent)

**SIMD-0296/0385 Transaction V1 IS ACTIVATED on mainnet**: epoch 1035,
2026-09-15 ~01:00 UTC, Agave 4.2.2, feature gate
`txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL`. Sources verified same day:
solana.com/upgrades/larger-transaction-sizes, solana-foundation/solana-dev-skill,
Helius blog, crypto.news. The original finding text was written under the stale
assumption that V1 was still review-stage; that assumption is wrong. Code verified
consistent with activation: `compile_adaptive` (arb-tx/transport/adaptive.rs) never
silently downgrades V1 to V0/Legacy, and `ARB_V1_TRANSPORT_SUPPORTED` stays
fail-closed default-false (run.rs) pending the on-chain canary
(`crates/arb-tx/examples/v1_canary_broadcast.rs`, evidence protocol
`reports/v1_canary/`) because chain activation does **not** prove endpoint support
(Jito/RPC must also accept 4096-byte V1 envelopes). **[docs][code]** The docs-side
residual (AGENTS.md "V0/V1") is tracked under the standing docs-sync rule; AGENTS
already states the V1 invariants correctly.

### PIPE-14 — closed (verified: not a bug)

Every RPC access on the strategy/dispatch path was read first-hand:

1. `kamino_available_flash_liquidity` (supervisor/run.rs:61) reads the local
   snapshot cache — no RPC.
2. `fetch_payer_token_balances` (supervisor/run.rs:127, called from
   run.rs:324 in `capture_three_leg_dlmm`) fires **only after** the exact-quote
   3-leg pipeline returns a strictly positive gross chain (run.rs:277-279) — a
   once-per-candidate, fail-closed inventory-provenance check (PR24 §2.2), not a
   per-tick probe. Unfetchable balances reject the chain (fail closed).
3. `rpc_pool.get_balance(&payer)` at dispatch.rs:215 runs only in
   BotMode::Live **and** only when `MIN_SOL_RESERVE_LAMPORTS > 0`; the default
   fast path (0) skips it entirely.
4. `rpc_pool.get_slot()` (run.rs:1082) fires only every 5th tick and only when
   the WS slot stream is stale — engineered stream-failure fallback, never a
   steady-state probe.
5. The startup `get_multiple_accounts` seeding (run.rs:752) runs once at start.
6. No `block_in_place`, `thread::sleep`, or std-blocking call was found in the
   arb-bot hot path (`pip wait`-style grep first-hand).

The only sequential pair (Live-mode payer balance + preflight simulation) exists
in an operator-opted conservative mode where the extra RTT is the accepted cost.
No change made. **[code]**

### Side-note — `is_fresh(1500)` unit verification: not a mismatch

`supervisor/execution.rs` freshness ceiling for verified economic inputs
(Kamino flash-loan fee SF, mint decimals, WSOL rate, tip ceiling) is **slots**,
not milliseconds: it is the named constant
`arb_constants::KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS = 1_500`
(arb-constants/src/lib.rs:544), which is the same ceiling that admits the
Kamino RPC-snapshot fallback store into the same function
(execution.rs:266-270). Economic *parameters* are slow-changing config-like
accounts and are deliberately separated from pool-state quote freshness
(`strategy.max_state_age_slots`, live-capped at 2 by
`validate_live_freshness`). The magic literal `1500` was replaced with the
named constant (single source of truth); behavior is unchanged.
**[code]**

## Verified correct (explicitly, not by omission)

- Canonical Kamino U68F60 flash-fee chain: `fee_bits = max(borrow×sf, 2^60)`,
  ties-away rounding, `BorrowTooSmall` raw-bit guard, `sf == u64::MAX` disabled
  sentinel, single fee source re-exported from `arb-math`. **[code][chain]**
- Instruction ordering: budget(≡config in V1) → idempotent ATAs → flashBorrow →
  legs → flashRepay(principal) → close ATAs → tip last. V1 adaptive compiler strips
  budget ixs to `Message.config` with correct `borrow_instruction_index` remap.
  **[code]**
- `atomic_integrity::verify_atomic_integrity` pre-sign gate. **[code]**
- No slippage/price-impact/fee double counting found anywhere on the executable
  path; tip deducted exactly once; slippage is a min-out floor only. **[code]**
- Recovery: ambiguous send → signature-status reconciliation only, same signed tx
  resent (never re-signed), 160-slot blockhash margin. **[code]**
- Dispatch-intent pre-persist before network send. **[code]**
- Supply-vault-liquidity borrow bounding, fail-closed when unverifiable. **[code]**
- All program IDs correct against live checks, including Raydium CAMM
  `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` (briefs claiming otherwise were
  wrong; repo right). **[chain]**
- Fail-closed freshness architecture end-to-end (slot_integrity, quote_readiness,
  provider supervisor, atomic batch publication). **[code]**
- Jito tip-floor oracle: 5 s freshness hard gate, degradation, competition clamp;
  tip can never consume the min-profit floor. **[code]**

## Fix plan (executed as PRs, in order)

1. **P1 accounting PR** — MATH-01/02 + PIPE-01/02/03: signed realized PnL persisted,
   provenance-gated aggregation (`status='landed'` + live mode only), metrics consume
   actual. Regression tests for each.
2. **P1 Jito PR** — PIPE-04: auth header from env + startup tip-account verification.
3. **P1 config/kill-switch PR** — PIPE-07/08: validated hard-caps, deny-by-default
   bypass flags.
4. **P1 math PR** — MATH-03: kill `sf >> 32` approximation on all pump paths.
5. **P1 ingest PR** — DATA-01 mixed-slot fallback.
6. **P2 batch PRs** — PIPE-05/06/09/10/12, DATA-03/04/10/11, PIPE-11/13/14,
   DATA-02/05/06.
