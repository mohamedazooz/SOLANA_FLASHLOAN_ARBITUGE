# 04 — Pool Discovery and Monitoring Audit

Full evidence: `audit_work/04_DISCOVERY_FRESHNESS.md`. Mainnet reads were used
(fetch-based probes elsewhere in the repo); this audit performed **no** live transactions and no
account writes.

## 1. Discovery paths

| Source | Code | Role |
|---|---|---|
| Geyser stream (preferred) | `yellowstone.rs:79`, handlers `lib.rs:743/850` | streaming account updates; honest slot stamping (`lib.rs:703-733`) |
| Helius WS | `helius_ws.rs:381`; update `slot.max(latest_slot).max(current_slot())` :588-592 | streaming; **fail-open stamping** |
| HTTP 750 ms worker | `registry.rs:596 fetch_live_pool_states`; stamp :604-834, applied :1114-1126 | batch polling; **stamps every pool with max(caller watermark, every fetched account's context slot)** |
| Seeds/watch set | discovery.rs (DexScreener/GMGN/pumpfun) | seeding only, not hot-path quotes |
| RPC poller fallback | rpc_pool.rs:43/62 | batch getMultipleAccounts, 100 at a time |

Pool identity: `PoolState { pool_id, mints, reserves, decimals, execution_accounts, … }` —
`decode_pool_info` (:1217) and per-protocol decoders build execution accounts (:988-1113:
RaydiumCpmm/Clmm/Orca/MeteoraDlmm/RaydiumV4). **`decode_pool_info` is dead outside tests** —
mint verification at registration is non-functional; identity is effectively assumed, not checked.

## 2. Scenario matrix

| Scenario | How handled | Verdict |
|---|---|---|
| A. Two pools same DEX | Engine treats each `PoolState` as a node with edges; DFS finds 2-cycles (`find_base_cycles`) | OK |
| B. Three pools same DEX | Same, with 3-hop DFS (`dfs_multi_hop`, lib.rs:1003) | OK |
| C. Two DEXs | Cross-pool via graph; DLMM-specific path `find_cross_pool_dlmm_cycles` | OK (but see math defects) |
| D. ≥3 DEXs | Graph edges protocol-agnostic | OK in principle |
| E. Multiple pools sharing mints | No dedup of equivalent pool identities beyond `pool_id`; same mint-pair can appear N times — legitimate for price discovery | OK |
| F. Different token ordering | `PoolKind`-specific; DLMM reserves normalized by mint comparison (`compute_dlmm_reserves` registry.rs:1986) **but** all quote/clamp/strategy direction derivation uses `pool_base == from_mint` passed as `swap_for_y` — catalog/decode ordering mismatch ⇒ well-formed but **wrong quotes/capacity clamps/fee placement** for pools whose catalog base ≠ decoded token_x | **FAIL** |
| G. Different decimals | Decimals decoded from mint accounts; `cycle_profit_at` zeroes both legs (lib.rs:209-210) | **FAIL (partial)** |
| H. Token-2022 mints | Excluded at discovery by policy (good) — but priced fee-free where admitted; ATAs SPL-derived (broken) | **FAIL** |
| I. Incomplete/stale/changing state | Missing account → pool skipped (fail-closed) ✓; stale → see §3 | Mixed |

## 3. Freshness — per-pool slot, snapshot slot, slot semantics

- **Fail-open stamping (both streaming and HTTP paths).** Helius WS: stamped
  `slot.max(latest_slot).max(current_slot())` (helius_ws.rs:588-592); HTTP worker:
  `max(caller watermark, every fetched account's context slot)` (registry.rs:604-834).
  Downstream age checks (run.rs:455/497, execution.rs:292-311) can therefore see **age 0 for
  arbitrarily old data**. Yellowstone stamps honestly but feeds the same cache.
- **Graph admission is a single coarse filter**: `graph_max_age_slots = 300` (~2 minutes) at
  run.rs:488-500 vs `max_state_age_slots = 60` and the risk default of 2. Pools ~5× staler than
  risk policy allow enter the graph; the missing-hop case maps to `u64::MAX` and is rejected
  (fail-closed) — but only against the inflated stamped slot.
- **No coherent multi-pool snapshot policy is wired.** `slot_integrity.rs` + `quote_readiness.rs`
  + `rpc_reconcile.rs` implement a genuinely fail-closed coherent-read gate (dependency sets, one
  provider, one generation, bank identity, dead-slot rejection) — **none of it is called on the
  live hot path.** Only `validate_execution_readiness` (dispatch.rs:369) runs pre-send, and it is
  the weaker check. The full gate lives behind the unused `guarded.rs`.
- **Conflicting snapshots.** Cross-provider (AccountSnapshotStore): WS always beats RPC bootstrap.
  Quote cache (MarketStateCache): last writer with highest stamped `(slot, write_version)` wins —
  Yellowstone's real account write_version is compared against a shared global counter
  (registry.rs:1939-1943) — **incomparable domains ordered as comparable.**
- **Incoherent route case (documented):** pool A fresh (slot 100), pool B stale (stamped 100 but
  fetched at 90) in a 2-hop route: engine sees both as age 0, quotes confidently, and only the
  pre-send lite-SVM/RPC simulation (if reached) would catch divergence — and for
  compilation-doomed routes it never reaches it (dispatch.rs:170).
- Missing/zero reserves ⇒ quote returns 0/None (fail-closed). Zero-liquidity bins hard-error in
  the quote after the parser drops them (dlmm.rs:835-838) — false negatives, fail-closed.

## 4. Pool identity and ordering validation

| Check | Status |
|---|---|
| Program-id dispatch (`program_id_to_protocol`) | Missing MeteoraDammV2 and LaunchLab (both decode to None) — dead adapters (§ cross-protocol) |
| X/Y ordering from account layout | Yes for decoders (byte offsets); **not tied into quote direction for DLMM** (see scenario F) |
| Mint decimals from mint account | Yes, except `cycle_profit_at` |
| Pool type/kind from account owner | Yes |
| Oracle account for DLMM = `[b"oracle", lb_pair]` PDA | **NO — `oracle = pool_id` is written into `PoolExecutionAccounts` (registry.rs:1054)** |
| Reserve/vault PDA derivations | Yes |
| Writability of critical accounts | Not checked vs ALT/v0 layout (no ALT use anyway) |

## 5. Synchronization, conflicts, missing accounts

- Missing account → pool skipped (compile proceeds without it) ✓ fail-closed.
- Conflicting providers → WS wins at provenance; at the quote layer, last-writer-wins on
  uncomparable version counters (§3).
- No periodic re-validation of pool structural invariants post-registration (fee, status,
  collect_fee_mode decoded once; status change ⇒ stale quote, silent).
- Purge of dead slots exists (slot_integrity purge) but is not on the hot path.

## 6. Verdicts per scenario

A–E: **PASS** (mechanically). F: **FAIL** (direction/reserve mismatch for misordered DLMM pools).
G: **FAIL** (decimals zeroed in cycle profit). H: **FAIL** (2022 unsupported end-to-end).
I: **FAIL** (stale treated as fresh — fail-open stamping; no coherent snapshot gate).

## 7. Required evidence gaps
- No run of the full ingest pipeline against current mainnet with coherence enforced (blocked by
  live-RPC-only test + audit constraints).
- No proof that WS/HTTP stamping inflation cannot be triggered by a hostile or lagging provider.
