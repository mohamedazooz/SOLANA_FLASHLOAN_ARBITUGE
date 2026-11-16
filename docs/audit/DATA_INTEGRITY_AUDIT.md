<<<<<<< HEAD
# Data Integrity Audit

Status: source review of ingestion, state promotion, readiness, and freshness
guards. Replay/fault-injection acceptance is **not** established.

## 1. Sources

| Source | Role | Notes |
|---|---|---|
| Yellowstone gRPC (Solami/Helius) | Primary account + slot stream | `arb-ingest/src/yellowstone.rs`, `solami_stream.rs`. |
| RPC pool | Bootstrap, reconciliation, transaction send | `rpc_pool.rs`, `rpc_reconcile.rs`. |
| Provider supervisor | Health/failover of stream providers | `provider_supervisor.rs`; generic WS failover is not fully proven. |
| Discovery providers | Pool catalog (DexScreener, GMGN, BitQuery, Pump.fun) | Discovery only; a catalog entry is not quoteable until state is ingested. |
| Pump dev stream | Pump.fun creation/graduation events | Feed for Pump state; execution remains disabled. |

## 2. Readiness model

`AccountSnapshot` carries `pubkey`, `owner`, `slot`, `write_version`,
`provider_id`, `bank_id`, and `generation`.

`quote_dependencies_for_route(..., quote_slot, max_age_slots, max_slot_skew)`
returns `QuoteReadinessError` when any dependency is:

- missing,
- owned by the wrong program,
- older than `max_age_slots`,
- ahead of/below the expected slot beyond `max_slot_skew`,
- from a dead bank or a mixed generation,
- from a slot whose bank was invalidated.

A route cannot be quoted if any required dependency is unready. There is no
"best effort" mode.

## 3. Slot and fork integrity

`arb-ingest/src/slot_integrity.rs` maintains:

- processed/confirmed/finalized/first-shred/completed/created-bank/dead status,
- parent links and known descendants,
- dead-slot invalidation (a dead slot invalidates known descendants; a recreated
  bank reopens that slot but not old descendants),
- bank-id tracking (`observe_with_bank`, `invalidate_bank`, `is_bank_dead`),
- `SnapshotReconciler` generation checks: a bootstrap batch from the wrong
  generation is rejected; `ready()` requires every key from one context slot.

Unit tests cover dead-slot descendants, recreation, and bank-id changes. These
are unit tests, not a live fork replay.

## 4. Freshness policy

| Policy | Config | Behavior |
|---|---|---|
| Quote freshness | `strategy.max_state_age_slots` (default 2) | A pool older than this is not executable; `max_state_age_slots()` fails closed on an unresolvable pool. |
| Discovery graph horizon | `strategy.graph_max_age_slots` (`None` → quote freshness) | Explicit policy; the historical arbitrary `*4 .max(8)` multiplier was removed (`GRAPH-01`). |
| Slot skew | readiness API parameter | Accounts outside the allowed skew are rejected. |
| Simulation readiness | `validate_execution_readiness` | Re-checks stream connection, generation, current slot, and quote slot immediately before send. |

The graph is built from `fresh_pools` only; a stale pool is never used as a
fallback (`L-05`).

## 5. Data promotion and decoding

- Pool state is decoded from account bytes with protocol-specific length and
  discriminator checks. A wrong owner/length/discriminator fails decode.
- PumpSwap's `can_edit_creator_fee` is populated (`DEX-01` fixed).
- Decimals come from mint metadata; a missing decimal is not defaulted in
  critical paths.
- Token-2022 risky extensions are rejected; transfer fees are applied at the
  active epoch.
- Discovery catalog entries are not automatically executable; the registry must
  ingest their state first.

## 6. Gaps and risks

1. **No snapshot-to-reconciliation E2E replay.** Unit tests cover slot logic, but
   a recorded stream replayed through ingest → registry → quote → dispatch is not
   attached (12.15–12.16).
2. **Provider parity.** The same account from two providers is not
   differential-tested for byte equality/write-version ordering.
3. **Mixed-fork executable quote.** The guarded quote path proves evidence
   equality, but it is not the only quote path (`GUARD-01`). A route could be
   built from a non-guarded path with weaker evidence.
4. **Reconnect behavior.** Reconnect counters exist; a forced-disconnect fault
   injection with an in-flight quote is not tested.
5. **Discovery freshness.** A catalog refresh can add a pool; the acceptance that
   no catalog-only pool is quoteable is enforced by construction, but not
   runtime-tested.
6. **Pump state.** Pump.fun/PumpSwap quote state is ingested, but their execution
   adapters are disabled; parity between quote state and executable state is not
   proven.
=======
# PHASE 6 — Data Pipeline & State Integrity Audit

## Ingestion surfaces (all verified in code)

| Surface | Implementation | Write path |
|---|---|---|
| WebSocket (JSON-RPC) | helius_ws.rs, subscription budget `WS_MAX_ACCOUNT_SUBS` (default 48, ≤1024) | 3 fail-closed arms (756/807/842) |
| HTTP polling | registry.fetch_live_pool_states @750ms | run.rs:385/451 |
| Discovery refresh | 60–90s, Orca catalog + Raydium v3 + DexScreener (DLMM only) | registry.register |
| Yellowstone gRPC | IngestClient plumbing present | provider exposure NOT VERIFIED in current env |

## State integrity rules (verified)

- **Write-version ordering**: `next_pool_write_version()` monotonic; `apply_pool_update`
  rejects older writes (the `let _ =` sites are intentional older-write rejections).
- **Slot freshness**: `state.slot`, `last_update_slot`, `last_update_ms` updated on every
  accepted write; quote freshness consumed downstream by gates.
- **Fail-closed on incomplete hydration**:
  - WS DLMM arm: drops update until active BinArray present (helius_ws.rs:794 region).
  - WS Orca arm: drops update until tick arrays cover `tick_current` (helius_ws.rs:779).
  - `compute_dlmm_reserves`: None on missing active bin, zero depth, or price-band
    divergence > `DLMM_RESERVE_PRICE_TOLERANCE` (4x) — rewritten fail-closed in `228c330`.
- **No stale-cache fallback**: no code path re-serves stale reserves after provider
  failure; provider failure → pool dropped from graph → no opportunities built.
  Verified by exhaustive cache-writer enumeration (ARCHITECTURE_AUDIT.md).

## Provider health & rotation

- RpcPool: primary + fallbacks from `SOLANA_RPC_PRIMARY`, `SOLANA_RPC_FALLBACK_1/2`,
  `ALCHEMY_RPC_URL`; 429 → 60s cooldown + rotation (soak16: 3 rotations, zero impact).
- Batch-100 `getMultipleAccounts` (official limit) — replaced ~430 sequential
  fetches/cycle; 429s: 39,519 (soak4) → 83 (soak10) → 6–9 (soak15/16).
- WS reconnect: present; seed_initial_accounts batched with 3-attempt retry.

## Duplicate / out-of-order updates

- WS: write-version guard drops stale rewrites.
- HTTP: full-state refresh overwrites atomically per pool.

## Findings

| # | Finding | Severity | Status |
|---|---|---|---|
| D-1 | `pairs` degrades on some rediscovery cycles (163→91) | P3 | OPEN (decimals cache added; needs long soak) |
| D-2 | Yellowstone gRPC path unexercised in current env | P3 | DOCUMENTED |
| D-3 | `SLOT_DEAD` / processed-fork invalidation: handled via slot+write-version guards; no dedicated SLOT_DEAD state machine | P3 | DOCUMENTED (works for current single-ws topology) |
>>>>>>> origin/main
