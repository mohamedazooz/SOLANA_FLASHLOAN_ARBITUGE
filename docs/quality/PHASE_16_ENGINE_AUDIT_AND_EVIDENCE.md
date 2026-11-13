# Phase 16 — Engine-by-Engine Audit & Live Evidence (Full Disclosure)

**Date:** 2026-11-13
**Scope:** Every engine in the execution path — pool discovery, DLMM math, flash loan, route algorithm, dynamic sizing, profit calculation, state freshness — audited against **live mainnet** via two independent RPC providers (`api.mainnet-beta.solana.com` solana-core 4.4.0, `solana-rpc.publicnode.com` solana-core 4.4.1, same feature-set 2681577653).
**Mode:** Shadow only (`ARB_LIVE=0`, `DRY_RUN=true`). Zero broadcasts. Zero signatures with funded authority.

This report is written as a full-disclosure senior review: everything that works is shown with evidence, everything that does not work or is not proven is stated plainly.

---

## 1. Pool Discovery Engine — VERIFIED LIVE (publicnode)

**Code:** `crates/arb-ingest/src/discovery.rs`, `arb-engine/src/meteora_pair_discovery.rs`

| Venue | Live result (publicnode) | Method |
|---|---|---|
| Orca | 297 pools above $15k TVL floor | program catalog scan, cap 297, 24 hubs |
| Raydium v3 (CLMM) | 233 pools above TVL floor | program catalog scan |
| Meteora DLMM | 34 candidate pools | program scan + pair discovery |
| Pump.fun | 70 bonding curves paired multi-venue | recent-coins API **403** from sandbox IP → fail-soft to multi-venue pairing |

**What is real:** discovery is dynamic (program-level scans, TVL floor $15k, per-venue caps), not a static pool list. Pool validity is not assumed from address existence — snapshots record owner program, data length (DLMM pool = 904 bytes), and slot.

**Honest limitations:**
- The discovery burst (`getProgramAccounts` fan-out) exceeds **every** public RPC's limits eventually: api.mainnet-beta 429s immediately; publicnode carried the **full discovery to completion with zero HTTP 429** but entered its own cooldown after ~2.5 min of sustained bursts. Continuous operation requires operator RPC (Helius/DRPC/Solami key). This is an infrastructure constraint, not a code defect.
- Pump.fun recent-coins frontend API 403s from datacenter IPs — the pairing fallback works, but front-door surveillance of new curves is degraded outside residential/operator networks.

---

## 2. Meteora DLMM Math — VERIFIED LIVE (quotes from real chain state)

**Code:** `crates/arb-math/src/dlmm.rs`, `crates/arb-dex/src/meteora_dlmm.rs`, `crates/arb-dex/src/dlmm_validation.rs`

Verified components (all integer / Q64.x fixed-point, no floats):

| Component | Function | Evidence |
|---|---|---|
| Price from bin id | `get_price_from_id` (Q64.64 `pow_q64`) | live quotes on 2-pool route |
| Per-bin capacity | `dlmm_active_bin_max_input`, `consume_bin_liquidity` | bins_traversed recorded per hop (0–2 observed live) |
| Base fee | `compute_base_fee` (bin-step-derived) | live: hop1_fee=429/2057, hop2_fee=25658/5620 raw — **from state, not constants** |
| Variable fee (volatility) | `compute_variable_fee` + `update_references_with_decay` | decay references exercised in workspace tests |
| Protocol fee share | `compute_protocol_fee` (bps) | unit-tested |
| Fee side | `dlmm_fee_on_input(collect_fee_mode, swap_for_y)` | live evidence records `fee_on_input=true`, fee in hop-input mint |
| Multi-bin traversal | `quote_dlmm_multi_bin` | 1–12 bins/leg in size ladder; 3-pool routes 1,574–2,465 B |

**Live proof (`reports/phase16/publicnode_2pool_dlmm_evidence.json`):** fresh pool snapshots at slots 468,814,886–87; both swap directions quoted bin-exact; `spread_bps: 0` — the honest answer is that these two pools offered **no** arbitrage (gross −2,818 / −4,515 raw on 1,000,000 in). The system said so instead of faking a win.

**Honest limitations:**
- `spread_bps: 0` on the sampled pairs means the live 2-pool evidence demonstrates correct *rejection*, not profitable *execution*. A profitable route has never been simulated end-to-end on-chain (blocker: funded payer).
- **ALT compression — corrected measurement** (`reports/phase16/ALT_COMPRESSION_MATRIX.json`): with the ALT derived from the SAME construction it compresses (the initial matrix built it from a different construction, so missed random keys fell back to 32-byte static keys at +99 B/bin), the true cost is **+6 B/bin**: the 3-pool route compiles as V0+ALT at 482–536 B for 38–65 accounts (vs 1,574–2,465 B V1) with no 5-bin cap. The 1,232-byte packet limit is unchanged by ALTs and overflow still fails closed. V1 remains the production default (no ALT lifecycle dependency); V0+ALT is the verified fallback with ~696 B of packet headroom at the V1 account ceiling. No silent downgrade in either direction.

---

## 3. Flash Loan (Kamino) — VERIFIED LIVE ON TWO RPC PROVIDERS

**Code:** `crates/arb-tx/src/flash/kamino.rs`, `kamino_flash.rs`; fee math `crates/arb-math/src/economic.rs:495-560`

**The strongest evidence in this repository.** The same V1 envelope (borrow 1,000,000,000 lamports → repay 1,000,010,000) was simulated on two independent providers:

| Provider | solana-core | Result | CU | Logs |
|---|---|---|---|---|
| api.mainnet-beta.solana.com | 4.4.0 | **`err: null`** | 62,584 / 65,165 | 26 lines: `FlashBorrowReserveLiquidity` success (30,999 CU) → `FlashRepayReserveLiquidity` success (25,395 CU) |
| solana-rpc.publicnode.com | 4.4.1 | **`err: null`** | 63,005 | same 26-line pattern (31,418 / 25,397 CU) |

Raw responses committed: `reports/phase16/v1_flash_spine_live.json`, `reports/phase16/v1_publicnode_sim_results.json`.

Fee integrity: live reserve `flash_loan_fee_sf = 11529215046068` (U68F60 ≡ 1 bps) read from chain, byte-matches the compiled constant; discriminators independently recomputed via SHA-256 sighash and match `crates/arb-constants/src/lib.rs:337-339`. The klend fee formula is replicated exactly including the 1-unit minimum-fee floor, the `BorrowTooSmall` guard, and the `flash_loan_fee_sf == u64::MAX` "flash loans disabled" sentinel (fails closed).

Atomicity: repayment ≥ principal + fee is enforced **by the klend program inside the transaction**; shortfall reverts everything including all swaps. Not client-enforced.

**Honest limitations:**
- This is the flash-loan *spine* (borrow + repay, no swaps between). A spine with swaps in between has been built and serialized (3-pool routes, 11 instructions, borrow at index 3 / repay at index 7) and signature-verified, but its **on-chain simulation success** still requires a funded fee payer — operator-gated. NOT VERIFIED: full-route funded sim, broadcast.

---

## 4. Route Algorithm (Bellman-Ford Negative Cycle) — CODE VERIFIED, FAIL-CLOSED OBSERVED LIVE

**Code:** `crates/arb-engine/src/bellman_ford.rs` (900 lines)

- Edge weight = `−ln(rate × fee_complement)`; product of rates around a cycle > 1 ⇔ negative total weight ⇔ arbitrage candidate. Standard transformation (Princeton CS423 reference cited in-module); one extra relaxation pass detects the cycle; cycle extraction yields the exact hop sequence.
- **Full disclosure on floats:** the graph layer uses `f64` weights (`bellman_ford.rs:55,72-78`). This is *candidate discovery only*. It is architecturally impossible for these floats to reach money math: `arb-math` (the leaf crate) contains **zero** floating-point in token accounting — every money path is `u64`/`u128`/`U256` checked. The `f64` in `arb-math/src/latency.rs` is RTT statistics (EWMA of milliseconds), never token amounts.
- Live observation: with stale/no fresh state the strategy layer refuses to even run the graph — `No fresh pools within graph max age; skipping tick (fail closed)` and `Missing fresh SOL/USD reference or invalid USD size cap; skip graph sizing` (soak logs, repeated continuously). No route is ever built from stale prices.

**Honest limitations:**
- The `-ln` transform on `f64` can, at extreme rate magnitudes, produce precision loss in *candidate ranking* (not in sizing or profit). Mitigated by the fact that every candidate is re-priced with exact integer math before it can be signed; a false-positive candidate is wasted work, never lost money. A fixed-point graph variant is a possible future hardening, not a correctness defect.

---

## 5. Dynamic Flash-Loan Sizing — VERIFIED LIVE (optimizer ran on live quotes)

**Code:** `crates/arb-engine/src/meteora_size_optimizer.rs`, `flash_loan_sizing.rs`

- **No fixed borrow amount anywhere in the sizing path.** The optimizer runs a coarse-to-fine geometric scan (live evidence: `coarse_points` 100,000 → 1,337,500 geometric, `candidate_basins: [[100000, 150000]]`, `bounded_early: false`), evaluating each size with real bin-level quotes.
- Constraints baked into evaluation: per-bin liquidity capacity (`dlmm_active_bin_max_input`), CU estimate as a function of bin-array count (`estimate_compute_units(total_bin_arrays)`), network cost converted into quote-mint raw units (`network_cost_in_quote_raw`), slippage floor, and flash-fee scaling.
- `flash_loan_sizing.rs::calculate_optimal_flash_loan` derives the size from depth/fees/costs; `net_profit_lamports` recomputes net in lamports; `calculate_kamino_flash_fee` re-derives the U68F60 fee.
- Live proof: the optimizer's basin output is recorded in the committed evidence JSON — it ran and bounded the search on real state.

**Honest limitation:** the optimizer's per-size evaluations are quote-based (exact math) but the *profitability of the chosen size at execution time* still depends on state freshness between quote and land — handled by the freshness gate and min-output slippage floor, both proven to reject live.

---

## 6. Profit Calculation Engine — VERIFIED (canonical, checked, single source)

**Code:** `crates/arb-math/src/economic.rs` — the single authoritative calculation:

```
Net = FinalReceived − Principal − FlashFee − DexFees − TransferFees
      − AccountCreation − OtherCosts − Network − Priority − JitoTip
```

- `calculate_net_profit_raw` (economic.rs:401-477): **all-U256 checked arithmetic**; every subtraction and addition returns `ArbError::Overflow` on underflow/overflow instead of a plausible number. Lamport-denominated costs (network/priority/tip) are converted to base-mint raw units via an explicit `rate/denom` pair — decimals handled explicitly, no silent scaling.
- `build_economic_quote` produces expected/minimum signed net profit with a `meets_min_profit` flag; `is_profitable()` is the single gate.
- Kamino fee (`kamino_flash_fee_checked`, economic.rs:510): exact klend `FeeCalculation::Exclusive` mirror with ties-away-from-zero rounding, minimum 1-unit fee floor, `BorrowTooSmall`, and disabled-sentinel guards.
- Token-2022 transfer fees: `apply_transfer_fee_ceil` (lib.rs:65-81) uses ceiling rounding (`+9999)/10000`) and a max-fee cap — fee can never exceed the cap or the amount.
- **`saturating_sub` disclosure (audited, per the non-negotiable rule):** instances exist in `clmm.rs:484,488`, `damm_v2.rs:46-49,181`, `lib.rs:80,145`. Each was inspected: in every case the subtrahend is *provably* ≤ the minuend by invariant (fee_rate < FEE_RATE_DENOMINATOR; fee cap ≤ amount; elapsed ≥ activation), so saturation is unreachable — these are quote-math convenience paths, not the settlement engine. The canonical profit engine itself is fully checked with `ok_or(Overflow)`. Verdict: no underflow can hide a loss.
- **Live proof of honesty:** on live routes the engine returned negative/loss assessments (`REALIZED_LOSS`, net −1,120 raw; `BREAK_EVEN`, −282 raw expected) and the gate refused to sign. The fee ledger explicitly records that DLMM fees are in hop-input mints and "NOT comparable to lamport costs" — no unit-mixing.

**Honest limitation:** `net_matches_quote: false` appears in sim evidence where realized output hit the slippage floor rather than the quote — correct behavior (floor binding), but it means the quote→realized reconciliation path has only been exercised in the loss direction. Never proven with a profitable realized route (same funded-payer blocker).

---

## 7. State Freshness & Fail-Closed Chain — VERIFIED LIVE

- Quote provenance recorded per run: `gate_quote_slot`, `gate_lag_slots` (7 observed), `max_allowed_slot_lag`, `gate_is_conservative: true`, `freshness_policy_relaxed: false`.
- A second pass on aged state produced classification `STALE_QUOTE_REJECTION` — the gate killed its own previous quote.
- With both RPC endpoints cooling down, the bot emitted **zero** opportunities, **zero** stale fallbacks, kept WS ingest (`stream_ingest=ok`), durable store (`disk=ok`), and honest health reporting (`rpc=FAIL` surfaced as ERROR).
- Admin surface fail-closed: web API refuses to start without `WEB_ADMIN_PASSWORD`; `/pause`+`/resume` disabled without `SENTINEL_ADMIN_KEY`.

---

## 8. Transport & Infrastructure (publicnode run)

- **RpcPool:** `SOLANA_RPC_1_URL` (publicnode) + `SOLANA_RPC_2_URL` (api.mainnet-beta) active as a 2-endpoint pool ("RpcPool active with 2 endpoint(s)"); rotation + 60 s cooldown on failure observed live; publicnode: **0 HTTP 429** during the whole run.
- **WSS:** `wss://solana-rpc.publicnode.com` connected ("WebSocket [1/1] connected to mainnet"); slot+account stream worker active — the WS ingestion path works on a provider-neutral endpoint, not just Helius.
- **V1 provider compatibility:** V1 simulation accepted by both providers (4.4.0 and 4.4.1) — two independent stacks.
- **Jito:** tip WS live, floor p50=1,000–1,085 / p95=36,964–127,064 lamports observed streaming; leader schedule refreshed (epoch 1085, 1,268 Jito keys).

---

## 9. Complete Findings List (small and large, unvarnished)

**Large (blockers for live):**
1. V1 broadcast end-to-end — NOT VERIFIED (operator authorization + funded key required).
2. Full-route (borrow+swaps+repay) successful on-chain simulation — BLOCKED on funded fee payer (`InvalidAccountForFee` is the correct RPC response to an unfunded payer; not worked around).
3. ≥24 h shadow soak — BLOCKED on operator RPC + geyser/WS feed; public RPCs cannot sustain the discovery burst indefinitely (publicnode: ~2.5 min).
4. V1-over-Jito bundle ingress and validator-set sweep — NOT VERIFIED.

**Medium:**
5. An ALT-compressed variant of the 3-pool DLMM route exists and is verified (482–536 B, bins 1–10/leg); V1 inline accounts suffice at current sizes (≤62 accounts), but a route needing >64 accounts must fall back to V0+ALT (requires the route ALT to be pre-extended one slot ahead — same-slot extension is unusable) or route splitting.
6. Profitable realized route never observed — all live evidence is in the rejection direction (honest, but the profit path is proven by unit tests only).
7. Pump.fun front-door surveillance degraded on datacenter IPs (403 from recent-coins API); pairing fallback works.

**Small / cosmetic:**
8. `generated_at_utc` field in evidence JSONs carries a unix-epoch value in a UTC-labeled field (`"1794548890.028Z"`) — cosmetic schema wart, provenance unaffected (`generated_at_unix` is authoritative).
9. Discovery burst is not adaptive to endpoint rate-limit signals (fixed fan-out); a backpressure-aware discovery scheduler would stretch public endpoints further.
10. Soak logs show tip-floor REST refresh every ~1 s in addition to the WS stream — redundant but harmless; WS alone would suffice.

---

## 10. Verdict per Engine

| Engine | Verdict |
|---|---|
| Pool discovery | **LIVE VERIFIED** (publicnode, full catalog completion) |
| DLMM quote math | **LIVE VERIFIED** (bin-exact quotes from chain state; rejection-honest) |
| Kamino flash loan | **LIVE VERIFIED (simulation)** on 2 providers — borrow+repay `err: null` |
| Route algorithm | **CODE VERIFIED** + fail-closed observed live; candidate-only float layer, integer money math |
| Dynamic sizing | **LIVE VERIFIED** (optimizer ran on live quotes, basin recorded) |
| Profit engine | **CODE VERIFIED** (checked U256, single source) + live honest rejection |
| Freshness/fail-closed | **LIVE VERIFIED** (stale rejection, cooldown behavior, honest health) |
| V1 transport | **MAINNET SIMULATION VERIFIED** (2 providers); broadcast NOT VERIFIED |
| Jito | **INFRA VERIFIED** (WS + leader schedule + tip floor live); bundle landing NOT VERIFIED |

Overall system verdict remains **MAINNET SIMULATION VERIFIED** — upgraded confidence from the second independent provider, unchanged in class.
