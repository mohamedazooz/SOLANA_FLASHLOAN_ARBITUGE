# Audit 04 — Pool Discovery, Monitoring & Route Freshness (Solana arb bot)

Read-only audit. No code was changed; `cargo` was not run. All citations are `file:line` against `D:\work_new`.
Scope: `arb-ingest` (discovery, registry, slot_integrity, rpc_pool, rpc_reconcile, quote_readiness),
`arb-engine` (graph build, base/cross-pool-DLMM/multi-hop/Bellman-Ford cycle discovery),
`arb-bot/supervisor/run.rs` (+`execution.rs`, `dispatch.rs`). Findings marked **[HIGH]/[MED]/[LOW]**.

---

## Executive summary

Discovery is catalog-driven (Orca/Raydium/DexScreener + pump.fun API), mints/decimals are largely
verified on-chain at *registration* time, and a genuinely fail-closed quote-readiness gate exists in
`quote_readiness.rs`/`slot_integrity.rs`/`rpc_reconcile.rs`/`dependency_manifest.rs`. **That gate is not
wired into the live hot path.** The live path stamps pool state with a global "max observed slot"
(not the account's own context slot) in two places, widens the graph freshness window (300 slots) far
past the quote freshness policy (60 slots), and accepts two fee sources verbatim from catalogs.
The one place directionality is genuinely mint-verified (DLMM reserves via `compute_dlmm_reserves`)
is *not* the place directionality is *used* (`quote_pool`, `edge_max_input_capacity`, cross-pool DLMM
strategy all derive `a_to_b` from the registered mint order), so an ordering mismatch produces
well-formed but wrong quotes.

---

## 1. Pool discovery

### 1.1 Sources (all catalog/API, never `getProgramAccounts`)

- Orca catalog `https://api.orca.so/v1/whirlpool/list` — `discovery.rs:43`, reader `discover_orca` `discovery.rs:581`.
- Raydium v3 catalog `api-v3.raydium.io/pools/info/list` — `discovery.rs:45`, reader `discover_raydium` `discovery.rs:452`; filters by `programId` → `RaydiumV4`/`Cpmm`/`Clmm` (`discovery.rs:466-477`).
- DexScreener token-pairs per hub mint (SOL/USDC/USDT + ~25 hard-coded "hub" memecoins/majors) —
  `discovery.rs:27-136, 47` for hubs, `discover_meteora_meteora_hub` `discovery.rs:663`; filtered to
  `dexId=="meteora"` with `labels` containing `"DLMM"` so DAMM pools are never registered as LbPairs (`discovery.rs:671-680`).
- DexScreener search fallback — `discover_meteora_search` `discovery.rs:732`.
- Pump.fun: dynamic BankkRoll/pump.fun API top-curves (`discovery.rs:222-231`) + cross-venue pairing
  that derives bonding-curve PDAs for any `…pump`-suffixed mint already found on AMMs (`discovery.rs:236-273`).
- Pre-seeded "verified mainnet" set for the boot path (Raydium SOL/USDC, Orca SOL/USDC x4 fee tiers,
  Meteora DLMM SOL/USDC x3, Raydium CPMM SOL/USDC) — `registry.rs:447-572` (`mainnet_default`).
- Registration cap/quotas: per-venue top-N by TVL with `max_per_venue=33`, `max_pools=200`,
  `select_watch_set_per_venue` `discovery.rs:884-940`; dual-30 selector `select_dual_30_watch_set` `discovery.rs:962-986`.
  Comment at `discovery.rs:877-883` documents why (cross-venue coverage over raw TVL).

### 1.2 Identity validation at discovery

- Discriminator/owner **not** checked for catalog identity. Pool address is taken from the catalog
  and trusted; for Raydium the pool account is re-fetched and vault pubkeys decoded, pools whose
  vaults fail to decode are dropped ("fail closed", `discovery.rs:535-575`). Orca/Meteora pools are
  registered from the catalog with **no on-chain read of the pool account at registration**.
- `decode_pool_info` (`registry.rs:1217-1391`) *can* read mints/vaults/fees/decimals from the account
  at fixed offsets and rejects all-default keys. **It is dead code in the live path** — it is only
  referenced by tests (`test_decode_pool_info_*`, `registry.rs:2164-2279`) and exported
  (`arb-ingest/src/lib.rs:35`). Nothing at register-time compares the catalog-declared
  base/quote mints to the on-chain mints.
- On-chain mints **are** consulted for decimals (`discovery.rs:810-875`, `resolve_mint_decimals`;
  also `registry.rs:1394-1439`): Token-program-owner-only (`owner != TOKEN_PROGRAM_ID` → skip,
  drops Token-2022 entirely, `discovery.rs:839-843`), base-layout mint decode, `decimals` at offset 44,
  mint/freeze authority option tags must be 0 for non-hub mints (discovery "token safety gate",
  `discovery.rs:853-870`; same logic in registry `registry.rs:1419-1435`). Hubs (WSOL/USDC/USDT) are exempt.
  Anything with unknown/zero-after-normalize decimals is dropped from the watch set
  (`discovery.rs:330-357`) — fail-closed.
- Raydium entries with catalog `decimals` are normalized to `u8` with no on-chain cross-check at
  register time, but Raydium CPMM/CLMM accounts embed decimals which `decode_pool_info` reads
  (`registry.rs:1260-1276, 1339-1354`) — again, unused.

### 1.3 X/Y ordering and swap direction — **the central design weakness** [HIGH]

Registered ordering is *catalog-declared* (Orca `tokenA/tokenB`, Raydium `mintA/mintB`, DexScreener
`baseToken/quoteToken`, `discovery.rs:611-628, 707-728, 601-610`):

- Raydium V4: vaults read at bytes 336/368, mints at 400/432 — `decode_pool_info` `registry.rs:1219-1247`
  (never called at runtime).
- Raydium CPMM: vaults 72/104, mints 168/200, decimals 331/332 — `registry.rs:1240-1278`.
- Raydium CLMM: mints 73/105, vaults 137/169, decimals 233/234 — `registry.rs:1336-1364`.
- Orca Whirlpool: mint A 101/133, mint B 181/213; vaults 133/165, 213/245 — `registry.rs:1284-1308`.
- Meteora DLMM: mint X 88/120, mint Y 120/152, reserves X 152/184, Y 184/216 — `registry.rs:1313-1334`.

- There is **no runtime check** that the registered `base_mint`/`quote_mint` equal the mints inside the
  pool account. The one function that *knows* on-chain ordering, `compute_dlmm_reserves`
  (`registry.rs:1986-2035`), compares pool `token_x` (offset 88) against the registered mints and
  normalizes reserves into registered ordering — proving the authors knew the catalog ordering is not
  guaranteed to match on-chain X/Y. If neither mint matches, it returns `Some((0,0))` and the pool is
  skipped (`registry.rs:1014-1021, 1956-1969`) — fail-closed there.
- **But the quote engine does not do this comparison.** `quote_pool` (`arb-engine/src/lib.rs:99,144`),
  `edge_max_input_capacity` (`lib.rs:522,538`), `cycle` sizing, and the cross-pool DLMM strategy
  (`cross_pool_dlmm.rs:354-360` via `quote_pool`) all compute `a_to_b = (edge.pool_base == edge.from_mint)`.
  `a_to_b` is passed into `arb_math::dlmm::quote_dlmm_view(…, swap_for_y = a_to_b)` where **`swap_for_y
  == true` means X→Y** (`arb-math/src/dlmm.rs:101-102, 643-644, 904-917`). For a DLMM pool whose
  on-chain X is the *registered quote* mint (exactly the DexScreener base/quote-vs-TokenX/TokenY
  inversion that `compute_dlmm_reserves` guards against), every quote, capacity clamp and fee placement
  (`collect_fee_mode`, `dlmm_fee_on_input`) is evaluated in the **opposite direction** to the reserves
  it was given. Result: smooth-looking, well-formed, wrong outputs — not an error. **[HIGH]**
- CLMM path has the same structural assumption (`a_to_b = pool_base == from_mint`, `lib.rs:82`), though
  Orca's catalog ordering appears to match the on-chain layout (both are "tokenA/tokenB"); it is not
  verified, so a mis-ordered catalog row is silent (r_in/r_out would be correct because
  `compute_clmm_virtual_reserves` reads X=L/√P vs Y=L·√P at `registry.rs:1944-1957`, but then
  `a_to_b` and the active tick/traversal direction would disagree with reserve_in/out). **[MED]**
- Direction for swaps is otherwise purely implicit: both directions of every edge are pushed in
  `build_from_pools` (see §2); there is no signed/normalized "anchor" token per venue.

### 1.4 Discovery freshness & staleness of the watch list

- Discovery is refreshed on a timer (`refresh_secs`, min 30s) — `run.rs:66, 255-282`. Discovery
  *itself* does not expire registered pools: `register_discovered` only adds (idempotent dedup by
  `pool_id`, `discovery.rs:989-1004`, `registry.rs:575-580`); a pool that disappears from catalogs
  stays registered forever (until process restart). Stale/illiquid-but-registered pools keep consuming
  a fetch slot per tick. **[LOW]**
- Discovery TVL/rank data is never trusted for pricing — reserves, fees and bins are re-read on-chain
  every fetch (registry comment `registry.rs:1148`; fee re-read `registry.rs:903-915`). *Except* for
  the two fee fail-open paths in §3.4.

---

## 2. Graph edges, cycles, dedup

- Edge construction: `MarketGraph::build_from_pools` (`arb-engine/src/lib.rs:419-486`). Every pool
  with `is_active && reserve_in>0 && reserve_out>0 && !has_risky_token && is_fee_valid()` becomes **two
  directed edges** (base→quote with `(reserve_in, reserve_out)`, and quote→base with
  `(reserve_out, reserve_in)`, `lib.rs:427-470`). Inactive/zero/risky/invalid-fee pools are dropped at
  the edge — fail-closed. Fee is recomputed as `(fee_numerator*10000)/fee_denominator`
  (`lib.rs:426`).
- Reverse edges are explicit (not derived), so both quotes propagate identically.
- `MarketEdge` carries `pool_id, protocol, kind, from/to_mint, pool_base/pool_quote, reserve_in/out,
  fee_bps, clmm, dlmm, pump_fun, transfer_fee_bps, token_programs, execution_accounts, active_bin_id`
  (`lib.rs` ~lines 47-66, grep `MarketEdge`).
- Cycle discovery, three tiers (all in `run.rs:511-606`):
  1. `find_base_cycles` — anchored DFS enumeration, 2-hop (`base→i→base`, distinct pools,
     `lib.rs:589-595`) and 3-hop (`base→i→j→base`, distinct pools, `lib.rs:610-626`);
     size via closed-form `optimal_two_pool`/`optimal_three_pool` with CLMM/DLMM active-bin capacity
     clamps (`lib.rs:593-607, 633-654`); fail-closed capacity at `edge_max_input_capacity`
     (`lib.rs:488-563`).
  2. `find_cross_pool_dlmm_cycles` — same-pair two DLMM pools, both directions, unordered pair dedup
     via `seen_unordered` (`cross_pool_dlmm.rs:455-484`), structured rejections
     (`CrossPoolRejectionReason` incl. `StalePoolState`, `cross_pool_dlmm.rs:75,91`), economics-gated.
  3. Fallback chain when no DFS opps: `BellmanFordEngine::find_negative_cycles`
     (`bellman_ford.rs:231-…`, super-source formulation, |V|−1 relaxations + final-pass cycle trace,
     dedups extracted cycles via `seen_cycles`, nodes capped at 5000 `bellman_ford.rs:239-243`) then
     `find_multi_hop_cycles` (DFS up to 5 hops with visited-pool set `lib.rs:969-1068`).
- Route dedup: cross-pool DLMM opps are retained only if their ordered `(first_pool, last_pool)` pair
  is new vs DFS results (`run.rs:563-575`). No dedup exists between 2-hop and 3-hop DFS results, and
  Bellman-Ford only runs when both are empty (`run.rs:586-601`), so no cross-engine duplicates reach
  the risk gate in practice. Multi-hop has no global dedup vs DFS either. **[LOW]**
- Multi-hop pruning uses f64 and a 2.0x theoretical remaining-rate heuristic
  (`lib.rs:1044-1056`); can prune feasible routes but cannot invent infeasible ones (fail-closed-ish).
- **No freshness is checked inside the engine.** `find_base_cycles`, `find_cross_pool_dlmm_cycles`,
  `find_negative_cycles`, `find_multi_hop_cycles`, `evaluate_arbitrage_cycle` and
  `make_economic_opportunity` (`lib.rs:345-417, 1070-1200`) never look at `PoolState.slot` /
  `last_update_slot`. Freshness is entirely the caller's job — and the caller only filters
  `p.slot.0 > 0 && current_slot - p.slot.0 <= max_graph_age_slots` before `build_from_pools`
  (`run.rs:494-500`); DFS-created opportunities are stamped with `current_slot` regardless of pool
  age (`lib.rs:684,941`; `cross_pool_dlmm.rs:543`; `bellman_ford.rs:526`). **[MED]** A pool admitted
  under the 300-slot graph window can yield a "current-slot" opportunity which then passes any
  downstream slot checks that trust `opp.slot`.
- Dedup of *tick structures* (not routes): CLMM tick arrays are only accepted if
  `facade.start_tick_index == expected_start` and the array covers the current tick (WS path
  `helius_ws.rs:634-641`; fetch path `registry.rs:758-771` + `clmm_tick_array_covers_tick`
  `registry.rs:1959-1971`). Empty/incomplete arrays → pool skipped (fetch) or update refused (WS) —
  fail-closed. DLMM: decoded bins must contain the active bin else pool skipped
  (`cross_pool_dlmm.rs:349-352`); WS DLMM updates refused until bins are hydrated
  (`helius_ws.rs:694-699`) — fail-closed.

---

## 3. Freshness: per-pool slot vs snapshot slot vs RPC context slot vs WS slot

### 3.1 The slot sources

| Source | API | Slot semantics |
|---|---|---|
| WS accountNotification (Helius) | `context.slot` per notification | per-message chain slot (`helius_ws.rs:779-785, 796-797`) |
| WS slotNotification (Helius) | `result.slot` | marks `record_slot_update(slot, parent, SLOT_PROCESSED)` + `mark_helius_connected` — no dead-fork semantics distinguished from Geyser (`helius_ws.rs:760-774`) |
| Yellowstone Geyser gRPC | `slot_update.slot/parent/status/bank_id` | full dead-fork model, `SlotIntegrity` (`slot_integrity.rs:22-166`), wired in `yellowstone.rs:195-219` |
| HTTP `getAccountInfo` (paced fetch) | `result.context.slot` | per-account RPC context slot, `processed` commitment (`registry.rs:154-160, 250-283`) |
| HTTP `getMultipleAccounts` (Kamino fallback / discovery) | `result.context.slot` | per-call context slot (`rpc_reconcile.rs:126-132`; `rpc_pool.rs:1100-1201`) |
| `getSlot` | `processed` (`rpc_pool.rs:1065-1070`) | latest processed chain slot |

### 3.2 [HIGH] Fail-open freshness stamping in **both** ingest paths

Pool state carried by `MarketStateCache` is what the graph and (partly) the risk gate trust for age.
Instead of the account's own context slot, the state slot is the **max over everything fetched (plus
the caller's current watermark)**:

- HTTP 750ms worker: `max_observed_slot` starts at `current_slot.0` and is raised by *every* account
  (pools, vaults, derived arrays) — `registry.rs:604,627,667,738`. Every `PoolState` is then stamped
  `Slot(max_observed_slot)` with `last_update_slot` likewise (`registry.rs:1114,1126`). A pool whose
  own RPC read landed at slot N−k is stamped with the watermark of the freshest *other* account, or
  with `current_slot` itself even if all reads were behind it. Age checks downstream (`run.rs:455`,
  `run.rs:497`, `max_state_age_slots`) then see age 0 for arbitrarily old data. Commitment is
  `processed`, i.e. pre-confirmation reads presented as current. **[HIGH]**
- Helius WS account updates: `effective_slot_u64 = slot.max(latest_slot).max(yellowstone.current_slot())`
  (`helius_ws.rs:588-592`; applied at `helius_ws.rs:625-630, 671-676, 706-711`). Same inflation: a late
  or replayed notification is re-stamped to "now"; fork-invalid WS slots (never marked dead on this
  path — Helius slotNotifications call `record_slot_update` but account updates only skip
  `slot_is_dead`, not dead *banks*, and never check the pool account's own bank) pass. **[HIGH]**
- Contrast: Yellowstone/Geyser gRPC path stamps `update.slot` honestly
  (`arb-ingest/src/lib.rs:703-733`, `apply_raw_account`), and rejects dead slots
  (`lib.rs:634-645, 547-572`). That is the correct behavior — it just doesn't protect the
  HTTP/Helius-fed states that share the same `MarketStateCache`.

### 3.3 Multi-pool route with one stale pool — **fails closed at the route level, but only against the stamped slot**

`max_state_age_slots(hops, pools, current_slot)` (`arb-bot/src/supervisor/execution.rs:292-311`)
computes the *maximum* age over hop pools; a hop pool missing from the snapshot yields `u64::MAX`
("never silently fresh", doc comment `execution.rs:286-291`). It is enforced fail-closed by the risk
gate (`arb-risk/src/lib.rs:578-586`) and re-checked in the opportunity loop
(`run.rs:1005-1010`). The per-route check is therefore sound — **but** it operates on the inflated
`last_update_slot` from §3.2, so "stale by N" can read 0. A stale Raydium leg stamped fresh + a fresh
Orca leg is admitted. `opp.slot = current_slot` likewise launders route age
(`arb-engine/src/lib.rs:382,684`).

### 3.4 Age thresholds in play

- `strategy.max_state_age_slots` = **60** slots ("state age window (~24s)", `config.toml:86`);
  `graph_max_age_slots` = **300** (`config.toml:87`,
  default = `max_state_age_slots` if unset, `.max(1)`, `run.rs:488-493`). Graph admission is ~5×
  looser than quote/risk freshness — a known, documented widening but still a 150s+ staleness
  admission window. **[MED]**
- Kamino verified-economic-inputs freshness: "primary" store fresh ≤ **1000** slots, final
  `is_fresh(1500)` bail (`execution.rs:170-180, 276-281`) — much looser than risk-gate 60. **[MED]**
- Risk gate `limits.max_state_age_slots` default 2 (`arb-risk/src/lib.rs:220,239`); production config
  overrides via strategy (60).
- `chronos` scoring decays age penalties but does not reject (`arb-math/src/chronos.rs:165-185, 400-476, 517`).
- Quote-readiness contexts are caller-parameterized `max_age_slots` (no global default); execute-time
  guard re-checks `stream_connected/generation/opps slot age` (`quote_readiness.rs:122-143`,
  used at `dispatch.rs:369-392`) — sane semantics (`ObservedSlotBehindQuote`, `QuoteTooOld`), but
  again only checks the *opportunity* slot vs stream watermark, not per-pool coherence.
- `validate_execution_readiness` is the only readiness call on the live path; the full dependency-set
  gate (`build_quote_dependency_set`) is used only by the opt-in guarded path.

### 3.5 Fail-open list (unknown/stale treated as fresh) — consolidated

1. `effective_slot = max over accounts AND caller watermark`, stamped onto every pool —
   `registry.rs:604-834, 1114-1126`.
2. WS max-stamping — `helius_ws.rs:588-592, 625-630, 671-676, 706-711, 815-825`.
3. Graph admits up to 300-slot-old pools while risk policy is 60 — `config.toml:86-87`,
   `run.rs:488-500`.
4. Opportunities stamped with `current_slot` regardless of pool age — `arb-engine/src/lib.rs:382,684,941`,
   `bellman_ford.rs:526`, `cross_pool_dlmm.rs:543`, `lib.rs:382`.
5. Raydium CPMM on-chain fee decode failure → catalog `fee_bps` substituted (`.or(Some(p.fee_bps))`,
   `registry.rs:908-909`); header claim "registry re-reads the real fee and skips when it cannot"
   (`discovery.rs:64-68`) is false for CPMM.
6. Raydium CLMM `Some(p.fee_bps.max(1))` — fee taken from catalog unconditionally, fabricated floor of
   1 bps (`registry.rs:910-911`).
7. `PoolState` hardcodes `is_active: true`, `has_risky_token: false`, `transfer_fee_bps: 0`,
   `token_programs: Default::default()` for all HTTP-fetched pools (`registry.rs:1120-1133`); nothing
   verifies Token-2022 transfer fees on the fetch path (Token-2022 mints are excluded at *discovery*
   time, so a pre-seeded or mainnet_default pool would bypass the gate if it pointed at a t-22 mint).
8. `parse_account_info_response` accepts any `context.slot` including 0 on missing context
   (`.unwrap_or(0)`, `registry.rs:253-256`); slot 0 states are filtered only later by
   `p.slot.0 > 0` in graph admission (`run.rs:497`) — a 0-slot state could still enter the account
   store / pump paths.
9. Helius WS slot stream marks `record_slot_update(slot, parent, SLOT_PROCESSED)` — a dead fork is only
   excluded for pools fetched *after* the dead-slot purge; WS account updates never check dead banks
   (`is_bank_dead`) at decode time (`helius_ws.rs:815-825` compares only `slot_is_dead`).
10. `validate_dependency_group` allows cross-slot bank mismatch when slot skew ≤ `max_slot_skew`
    (only same-slot bank conflicts are rejected) (`provider_supervisor.rs:164-196`).
11. `fetch_live_pool_states` fetches accounts with **`processed` commitment and no `minContextSlot`**
    (`rpc_pool.rs:1126,1157,1184`; per-account paced fetch `registry.rs:154-160`) — no monotonicity
    guarantee vs WS watermark.
12. `apply_pool_update` orders pool states solely by `(slot, write_version)` in `MarketStateCache`
    (`lib.rs:547-577`): HTTP worker's slot floor (§3.2) lets an HTTP-polled state overwrite a strictly
    fresher Yellowstone/Helius update at an equal-or-lower slot with a numerically larger write version
    counter; conversely Yellowstone's *real* account write_version (lib.rs `apply_raw_account` uses
    `update.write_version`) is compared against the shared monotonic counter `next_pool_write_version()`
    (`registry.rs:1939-1943`) — two incomparable domains ordered as if comparable. **[MED]**
13. Pump curve `fee_bps: 100` fabricated at discovery ("PumpFun fee is fixed", `discovery.rs:282`).
14. Kamino fallback freshness accepted at ≤1000/1500 slots vs 60-slot risk policy —
    `execution.rs:170-180,276-281`.

Fail-closed (for contrast): graph admits nothing when `fresh_pools.is_empty()` (`run.rs:503-507`);
risk gate rejects `state_age > limit` (`arb-risk/src/lib.rs:578-586`); CLMM/DLMM quote returns 0 with
missing arrays/bins (`lib.rs:89-199`); fetch-path pools skipped on missing vaults/reserves/zero depth/
undecodable fees except §5/§6 (`registry.rs:856-876, 915-930, 988-992`).

---

## 4. Incoherent snapshot policy

An explicit policy **exists but is not enforced on the live quoting path**:

- `SnapshotReconciler` (`slot_integrity.rs:188-269`): a bootstrap generation is ready only when *every
  required key* returned by **one** `getMultipleAccounts` response at **one** context slot ≥ target
  slot; duplicates/wrong generation/old context rejected (`slot_integrity.rs:218-260`). Used only by
  `rpc_reconcile.rs` bootstrap (`fetch_and_publish`, `rpc_reconcile.rs:90-190`) — the right tool, not
  in the hot loop, and every snapshot it publishes is `is_startup: true` (`rpc_reconcile.rs:172-181`),
  which the quote gate rejects by design (`quote_readiness.rs:76-77`).
- `build_quote_dependency_set` (`quote_readiness.rs:41-111`): complete dependency set, one provider,
  one generation, bank identity required, dead slot/bank rejected, future slots rejected,
  `target_slot - slot ≤ max_age_slots`, and `validate_dependency_group` enforces
  provider/bank/skew coherence (`provider_supervisor.rs:153-196`, same-provider + same-or-skew-free
  banks + `max_slot_skew`). This is a real incoherent-snapshot policy.
- `AccountSnapshotStore` arbitration (`lib.rs:124-219`, `apply`/`apply_batch`): bootstrap can never
  overwrite live; live always supersedes bootstrap; provider switch rejected; same-slot bank conflict
  rejected; atomic all-or-nothing batch publish; cross-provider writes rejected outright (the code
  itself notes provider slot/write-version values are not globally comparable — coherent with our §3.5/12).
- **[MED] Not wired:** the live quote path does not call `build_quote_dependency_set` or
  `quote_dependencies_for_route` (only `dispatch.rs:369` calls the weaker
  `validate_execution_readiness`, comparing only stream generation and opp slot age). The full policy
  lives behind `guarded.rs` ("opt-in safe entry point … callers must migrate every live path to it",
  `guarded.rs:2-4, 38-118`), i.e. currently unused in production flow.
- `pump_state.rs` shows the intended inline shape for the PumpSwap path: pool/vault snapshots must
  share the *exact same slot*, mints must not be newer than the pool, fee/config must be within
  `max_state_age_slots` (`pump_state.rs:353-500`); contaminated snapshots are rejected and counted
  (`refresh_pump_swap_states`, `pump_state.rs:525-640`). This incoherence policy does not generalize to
  AMM pools.
- Coherence gap even where the gate runs: bank identity compared only *within the same slot*; opposite
  banks at adjacent slots pass with `max_slot_skew ≥ 1` (`provider_supervisor.rs:181-191`).

---

## 5. Reserve sync & Token-2022 & decimals

- **Vault-balance pools** (Raydium V4/CPMM): both vault token accounts fetched via paced RPC, balance
  decoded at SPL offset 64..72 (`registry.rs:2037-2062`); Token-2022 base layout shares this offset,
  but `decode_spl_token_amount_bytes` does not inspect `owner` or the token-account mint, and
  **transfer-fee / permanent-delegate / transfer-hook semantics are never deducted**; zero balance on
  either leg → pool skipped, fail-closed (`registry.rs:848-869`). `mint` of the vault is not checked
  against the registered mint at all — a vault pubkey collision/poisoned registration silently wires
  the wrong token. **[MED]** Discovery does screen Token-2022 *mints* out for Raydium (programId check,
  `decode_raydium_clmm/cpmm` vault decoders are fixed offsets, registry.rs:1175-1215), and Orca's
  catalog explicit `token2022` flag skip is present (`discovery.rs:591-597`).
- **CLMM pools** (Orca/Raydium): reserves are *computed*, not read: `x=L/√P`, `y=L·√P`
  (`compute_clmm_virtual_reserves`, `registry.rs:1944-1957`) — X/Y attribution is to *on-chain*
  token X/token B mint pools; see §1.3 for the registration-order inversion risk.
- **DLMM pools**: reserves computed from active bin and price-from-id, summed over bins when active
  bin empty (`compute_dlmm_reserves`, `registry.rs:1986-2035`). Mint cross-check as described in §1.3.
  Missing bins → pool silently dropped from the tick (availability loss, no error surfaced).
  Note `(Some((0,0)))` is also returned when pool-mint comparison fails; both become the same Skip.
- **Decimals**: discovery-time chain resolution with safety gates (`discovery.rs:810-875`,
  `registry.rs:1394-1439`); `dec <= 18` accepted always (no cross-program check beyond t-22); zero →
  pool dropped. `decode_pool_info` would supply decimals for CPMM/CLMM but is unused.
- WS path: `apply_raw_account` sets `decimals_* = 0` and relies on the (untrue) claim "Pools with
  decimals == 0 are rejected by the economic engine (fail closed)" (`lib.rs:666-671`) — **nothing in
  the engine reads `pool.decimals_*`**; `build_from_pools`, `bellman_ford`, cross-pool, sizing all
  treat decimals as base_decimals from config (`arb-engine/src/lib.rs:686-703, 1219;` grep shows only
  `config.base_decimals`). So a WS-only pool would be priced with **config** decimals regardless of
  the real mint. (In practice HTTP fetch precedes WS updates, so decimals are eventually overwritten —
  by a higher-`write_version` write whose ordering vs the WS write is governed by §3.5/12.) **[MED]**
- **Wrong decimals**: they'd silently corrupt USD conversions (trade sizes, P&L) — engine math itself
  is raw-base-unit consistent. No runtime assertion ties pool decimals to mint account decimals after
  registration; a catalog reporting wrong mint decimals for a mint already cached in `known` from a
  *different* venue is never re-checked (`discovery.rs:330-357` trusts cached `known` set).

---

## 6. Conflicting snapshots (RPC vs WS) — which wins

Two independent arbitration layers, one per cache:

- **AccountSnapshotStore** (provenance layer): WS/live **always beats** RPC bootstrap regardless of
  slot; bootstrap never overwrites live even at numerically higher slot; cross-provider writes are
  rejected pending explicit generation reset; same-slot different-bank rejected (`lib.rs:124-219`).
  For `is_startup` HTTP bootstrap (`rpc_reconcile.rs:172-181`) this is a hard win for stream data.
  **→ WS wins at the provenance layer.**
- **MarketStateCache** (quote layer): arrival order of `(slot, write_version)` decides
  (`lib.rs:547-577`). Because HTTP stamps slots with the caller's watermark (§3.2) and bumps a shared
  monotonic counter (`next_pool_write_version()`, `registry.rs:1939-1943`), an HTTP 750ms write with
  `slot = watermark` outranks any Yellowstone/Helius update at an earlier slot, and outranks
  Yellowstone writes at the same slot whose real account write_version happens to be numerically
  smaller than the shared counter. Helius-WS writes use the same counter, so Helius vs HTTP is
  last-writer-wins at equal slot; Yellowstone-gRPC vs HTTP is *effectively HTTP-wins* at equal slot
  and always wins at higher stamped slot. Dead slots do evict pools and reject updates
  (`lib.rs:547-556, 597-633`) — the one ordering rule that survives provider mixing.
  **→ At the quote layer, whoever fetched last with the highest stamp wins, not whoever read
  fresher chain state.** The `dual_ingest.rs` shadow comparator treats ≤5-slot skew as "fresh_enough"
  (`dual_ingest.rs:161-171`) and only *reports* mismatches, never rejects.
- Cross-layer disagreement is possible (store says snapshot fresh, cache holds HTTP-stamped state) —
  nothing reconciles them; the pool-level cache is what the graph consumes.

---

## 7. Missing account behavior

- Missing **pool account** (RPC null/decode error): pool omitted from `Vec<PoolState>` for this tick
  (`registry.rs:606-623` — fetch errors are `debug!`-logged and `filter_map`'d out; `fetch_live_pool_states`
  line 596, error handling 612-616). Graph then sees the pool as absent → excluded from edges this
  tick (freshness-unknown impossible since absent). Not an error surfaced to ops. **[LOW]**
- Missing **vault** (Raydium): treated as `0` balance → pool skipped with "missing live reserves"
  (`registry.rs:854-869`) — fail-closed.
- Missing **derived account** (tick/bin array): pool's arrays empty → CLMM quote returns 0 / DLMM
  reserve decode returns None → pool skipped (`registry.rs:915-930, 1007-1017`; CLMM arrays explicitly
  `.unwrap_or_default()`, `registry.rs:929-933`; DLMM bins `.unwrap_or_default()` then rejected at
  `compute_dlmm_reserves` bins-empty check, `registry.rs:1989-1991`).
- Missing **fee**: undecodable fee → pool skipped, *except* CPMM catalog fallback and CLMM catalog
  pass-through (§3.5/5-6).
- Missing **snapshot in route dependency check**: `MissingAccount` → hard error → quote refused;
  `is_startup` → `StartupSnapshot` error (`quote_readiness.rs:60-77`). Execution guard requires a
  non-zero observed watermark (`quote_readiness.rs:130-131`).
- RPC-wide failure: 750ms worker logs and produces zero pools → `fresh_pools` empty → tick skipped
  (fail-closed), and if WS is also down, `current_slot` freezes; slot-age gates reject everything.
  BUT: stale `MarketStateCache` entries are *retained* (not proactively purged except on dead slots),
  so an empty tick is distinguishable from a fresh tick only by the admission filters, and WS-stamped
  "fresh" quotes could still be constructed from books that no longer update if admission gates are
  mis-set (they currently pass via §3.2). **[MED]**
- Default `ReserveIn = ReserveOut = 0` is used as a sentinel for PumpSwap/PumpFun/CLMM/DLMM failures
  (`registry.rs:979-1021`) and filtered by `reserve_in==0||reserve_out==0` (`registry.rs:988-992`) —
  never reaches the graph. Note an *actual* on-chain 0-balance vault is indistinguishable from a
  fetch miss for Raydium (same skip) — acceptable fail-closed semantics.

---

## 8. Places where unknown/stale is treated as fresh (fail-open) — checklist

Beyond §3.5 (already all fail-open), the audit adds:

- Opportunities are handed to the risk layer carrying `slot: current_slot` regardless of what slot the
  underlying pools/tick-data belong to (`arb-engine/src/lib.rs:382,406,684,941`;
  `bellman_ford.rs:526`; `cross_pool_dlmm.rs:543`). Downstream `opp.slot`-based checks therefore cannot
  detect staleness of any hop.
- `emit_market_snapshot`/ingest path resets the cached SOL price each cycle to avoid stale price reuse
  (`run.rs:446-449`) — that good hygiene is not mirrored for pool reserves.
- Quote freshness gate's `AheadSlot`/`FutureSlot` checks use `context.target_slot = observed_slot`
  (`lib.rs:817-831` ignores the caller's `target_slot` and substitutes the live watermark) — weaker than
  the API signature suggests.
- Helius WS account updates are applied to the snapshot store with `bank_id: None`, permanently
  failing `MissingBankIdentity` for those keys — so the strict path *cannot* be satisfied by the path
  the bot actually runs; anyone wiring `quote_dependencies` onto Helius-fed state gets a hard, silent
  fail-closed that masquerades as a stream problem.
- `BellmanFordEngine::find_negative_cycles` runs only when DFS+cross-pool produced nothing, on
  `fresh_pools` — fine — but it reads `edge.weight` built from raw float reserves; small pools below
  the graph admission produce phantom cycles only bounded by pool freshness admission, i.e. §3.2 again.
- `verify freshness before submission` in `dispatch.rs:369-392` re-checks stream connectivity and opp
  slot age but **not** that the Jito tip account clock or blockhash is fresh (blockhash cache is
  `Instant`-based) — out of scope but noted.

---

## 9. Notable positive controls (for balance)

- Token-2022 excluded at discovery for all four venues (Orca flag, Raydium programId check, mint
  owner check at decimal resolution, `discovery.rs:328-357, 618-628, 839-843`).
- Fee denominator-0 pools can never be priced (`arb-types` `is_fee_valid`, `arb-types/src/lib.rs:940-944`).
- CLMM/DLMM quote functions return 0 rather than falling back to CPMM (`arb-engine/src/lib.rs:104-107,
  160-163`); WS tick/bin updates refused until covering arrays exist (`helius_ws.rs:634-641, 694-699`).
- Dead-fork invalidation physically purges pools/accounts/dedupe entries (`lib.rs:597-633`); Kamino and
  pump paths carry slot-coherent joins (`pump_state.rs:353-500`).
- `SnapshotReconciler` + atomic `apply_batch` give a true all-or-nothing bootstrap publication mechanism.

---

## 10. Prioritized remediation targets (informational only)

1. Stamp pool state with its **own** RPC context slot (fetch returns it) and propagate per-leg slots
   into routing freshness; keep watermark only for watermark purposes. (§3.2)
2. Normalize DLMM/CLMM quote direction against **on-chain** X/Y (mint compare once at decode, cache a
   `base_is_token_x` flag on the edge). (§1.3)
3. Call `quote_dependencies`/`GuardedRouteQuote` (or at least `build_quote_dependency_set`) inside the
   per-opportunity loop; until then the entire quote-readiness subsystem is decorative. (§4)
4. Kill the catalog-fee fallbacks (CPMM `.or(Some(p.fee_bps))`, CLMM `Some(p.fee_bps.max(1))`) — skip on
   decode failure like every other venue. (§3.5/5-6)
5. Align `graph_max_age_slots` with `max_state_age_slots` (60) or weight risk by actual per-hop age
   with bounded graph window. (§3.4)
6. Track provider provenance in `MarketStateCache` and refuse cross-provider overwrite at equal slot
   (store already knows how; apply it to the pool cache). (§6)
7. Reject/retire pools that leave the catalog or repeatedly fail fetch (watch-set expiry). (§1.4)
8. Wire `decode_pool_info` (or equivalent) into `register_discovered` to verify catalog mints/vaults
   against the account — turning §1.2 from dead code into the registration gate it was written to be.
