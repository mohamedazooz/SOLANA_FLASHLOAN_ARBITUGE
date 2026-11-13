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
