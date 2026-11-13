# 11 — Jito Forensic Audit

Evidence: `audit_work/11_JITO.md`; official docs corroboration in `14_EXTERNAL_RESEARCH_REFERENCES.md` §5.

## 1. Endpoint, auth, bundling

- Regions: `DEFAULT_JITO_REGIONS = ["amsterdam","frankfurt","ny"]` (arb-land/lib.rs:27-29); shipped
  config `["frankfurt","amsterdam","ny"]` (config.toml:23) — same set, different order.
- `JITO_BLOCK_ENGINE_URL` override **honored for simulation/status RPCs only**
  (`primary_endpoint` lib.rs:253-258) — **ignored on the bundle send path**:
  `send_bundle_multi_region` (lib.rs:319-330 → 591-656) reads only the region list; dispatch.rs:419-421
  passes `&self.config.jito.regions`. The documented routing control does not route bundles.
- No authentication anywhere in the transport (`send_bundle`, lib.rs:260-317). Jito permits
  tip-only auth on public endpoints; if the operator relies on the override to reach an
  authenticated proxy, no credential is ever sent.
- `sendBundle` params `[txs, {"encoding":"base64","bundleOnly":true}]` (lib.rs:265-270); max 5 txs;
  concurrent fan-out with short-circuit on first Ok; response requires HTTP success + JSON-RPC
  `result` string. `max_retries = 3` field exists but `#[allow(dead_code)]` — **no retry**.
- Live bundle = **exactly one transaction** (`bundle_txs = vec![tx_bytes.to_vec()]`,
  dispatch.rs:417). Borrow→swaps→repay→tip stay in one atomic tx. Correct for flash-loan semantics —
  never split borrow/repay across independent transactions.
- Dedup across ticks: identical bundles dedup by signature in-engine; different-tip re-sends in the
  same slot are bounded only by `max_inflight_bundles = 32` and the escalation ladder. No
  `(slot, route_id)` idempotency key.

## 2. Tip accounting

- **Who pays:** the bot wallet (payer = signer). Native SOL `SystemProgram::Transfer` (index 2) as the
  **last** instruction, with the `jitodontfront` marker account appended **read-only**
  (arb-tx/lib.rs:243-300; emitted last in every assemble path, :2807/:2977/:3213/:3269/:3319).
- **Tip account selection:** round-robin/leader-mapped from the 8 official accounts
  (arb-constants:57-117; byte-sum leader hash — collision-prone, non-security). Jupiter exotic route
  uses random selection (execution.rs:95, dead code). Two different selection policies coexist.
- **Mint:** SOL. Tips never in WSOL.
- **Inclusion in profit:** yes, sequentially — `net_before_tip` (run.rs:676-679) then
  `calculate_net_profit_raw` with the **actual bid** (run.rs:805-818) and a final `≥ min_net_base_raw`
  floor (:890-908). The Chronos EV gate also receives the bid (:965). All three agree on sign.
- **Deducted once or twice?** Once per transaction (guards `tip > 0`). **But** the requested CU
  price is inflated **uncapped** by `calculate_dynamic_cu_price` (`gross/25_000` capped +500_000 μλ/CU;
  lib.rs:106-109) **on top of** the profit-capped requested price from
  `cap_cu_price_by_profit` (run.rs:656-663) — two elevators, one capped. The purpose-built
  `calculate_fee_deduplicated_cu_price` (bundle-zero-price mode) is **never called** — the bundle
  path pays both the priority fee and the tip, duplicating the auction budget.
- **Charged if bundle doesn't land? No** — tip is inside the single atomic tx; non-inclusion ⇒
  zero lamports move. Code comments confirm (dispatch.rs:640,715).
- **Priority fee vs tip:** accounted separately end-to-end (not double-counted in formulas), but the
  uncapped dynamic-CU addend makes total outflow exceed the priced model.

## 3. Auction and failure analysis

- **Tip floor feed:** live REST (`bundles.jito.wtf/api/v1/bundles/tip_floor`, 1 rps/IP, poll clamped
  ≥1000 ms) + WS push (`tip_stream`); `DEFAULT_MAX_AGE_MS = 5_000`; stale feed ⇒ `current() = None`
  ⇒ **refusal to bid**, never a static default (bidder.rs:195-204). No static fallback in pricing. ✓
- **Requested tip:** chosen as the cheapest rung with positive EV under Wilson-lower-bound ledger
  inclusion probabilities; escalation ladder ≤ `ladder_growth = 1.6`; hurdle `max(1_000_000,
  0.60 × edge)` (shipped) vs code default 0.85; ceiling `min(0.4-edge, 0.95 × edge) = 0.4-edge`.
- **Enforcement gaps:**
  - `MAX_JITO_TIP_SOL` (.env / .env.example) — **never read**.
  - `ENABLE_SOLANA_JITO` — **never read**.
  - `check_tip_lamports` (absolute 2_000_000 cap) — defined, **never called** on the evaluate path
    (arb-risk/lib.rs:560-568 vs :455-548 which calls only `check_tip_share`/`check_tip_spend_ratio`).
  - Only real absolute backstop: `min_net_profit_abs = 1_000_000` lamports and the `tip ≥ net_profit`
    reject (run.rs:1732-1739) plus `max_tip_spend_ratio = 0.98` (tip/(tip+net)) — i.e., on a large
    edge, the bot may bid **up to 98% of tip+net**. Not "never spend more than edge" in any useful sense.
- **Shipped config drift:** `keep_share = 0.60` (vs 0.85), `max_tip_share_bps = 9500` (vs 8500),
  `max_tip_spend_ratio = 0.98` (vs 0.7) — coherent for the $0.20 invariant but far looser than
  library defaults. `TipStrategy::new` silently raises configured share to ≥7500 bps if ever re-wired (arb-land/lib.rs:105).
- **`leader_gate_enabled = false`** → `tip_on_unknown_leader = true` (config.toml:35; supervisor.rs:390) — the bot
  will tip against **unknown leaders**; only known-non-Jito leaders are skipped. Code default is
  fail-closed; shipped config is not.
- **Expired ≠ Failed:** `verify_landing` polls 20×350 ms (~7 s) then one RPC check → `Expired`;
  dispatch records `InflightStatus::Failed` (dispatch.rs:637-639,668-670) — a late landing is booked
  as a negative observation, **corrupting the per-rung EV posteriors that drive all future bids**. No
  delayed re-check exists.
- **Submitted vs landed:** profit counted **only** in the `Landed` arm; RPC `getSignatureStatuses`
  cross-check downgrades Jito "Landed" on-chain failure to Failed; `Sent` is non-terminal.
  Submission ≠ landing is properly distinguished here. ✓
- **Bundle vs transaction:** single-transaction bundles with `bundleOnly: true` — revert protection,
  no uncle-bandit leakage. Split (multi-tx) path exists (`build_jito_tip_transaction`) but unused —
  correctly, since splitting borrow/repay would break flash-loan semantics.
- `simulateBundle` — implemented but **never called**. Fire-time slot wait ≤400 ms; `FAST_PATH_*`
  env shortcuts can skip simulations and waits (dispatch.rs:217-234, FAST_PATH_IMMEDIATE_DISPATCH).
- Pricing plan made against `current_slot` while dispatch targets `current_slot+1` (run.rs:618-619 vs
  :723-724) — inconsistent oracle for the leader gate.

## 4. Why Jito transactions/bundles fail (this system's causal list, evidence-linked)

1. **Oversize transaction** (legacy-only live path; 3-leg routes >1232 B) — dropped pre-dispatch; never reaches Jito.
2. **Guaranteed-revert content** (close pre-borrow WSOL ATA; sf=0 repay; Token-2022 SPL-derivation) — bundle fails, tip unpaid, opportunity lost, and the ledger book it as "Failed" (actually correct here).
3. **Wrong leader** — unknown leader tipped by shipped config; Jito leaders only tip-eligible; non-Jito leader ⇒ bundle cannot land (wasted tip budget scheduling, not wasted lamports).
4. **Expired blockhash** — cached ≤3 s; `lastValidBlockHeight` never tracked; expiry only post-hoc via `Expired` status.
5. **Auction loss** — cheapest-EV-rung entry; no competitive-clearing modeling beyond ledger win-rates.
6. **Tip in ALT (v0 artifacts only)** — latent; live path passes tips inline.
7. **Ledger pollution** — expired-after-7s recorded as Failed ⇒ systematically shrinking computed inclusion probabilities ⇒ ever-lower bids.
8. **`jitodontfront` marker** correctly attached read-only; no failure mode here.
9. **Rate limiting** — 1 rps/IP/region caps the tip-floor poll; poll clamp handles it; bursts of bundles fan out to 3 regions (each 1 rps) — fine.
10. **Unverified simulation bypass** — `FAST_PATH_SKIP_RPC_SIM=true` would ship unsimulated bundles; unset by default.

## 5. Recommended data-driven tip policy (proposal — not implemented)

```
tip* = argmin over rungs r of cost(r) s.t. P_land(r) · E[net | land] − cost(r) > 0,
P_land from Wilson lower bound of the per-rung ledger (existing machinery),
flimap to [MIN_JITO_TIP=1_000, min(t_max, edge·max_share)] with t_max from net_before_tip,
hard cap: tip ≤ min(0.2 × edge, MAX_JITO_TIP_SOL if configured, remainder after min_net floor).
```

Never exceed configured economic limits; refuse when feed stale; escalate ladder ≤1.6×; re-check
late landings after ≥30 s before booking a ledger failure. All four changes are local to
`arb-land` + `dispatch.rs` — none requires touching the instruction builders.
