# Jito Integration Audit - Sentinel Solana Arbitrage Bot
Date: 2026-11-11 - Read-only audit, no code changes, no build run.
Scope: `arb-bot/{jito_service.rs, supervisor/run.rs, supervisor/dispatch.rs, supervisor/execution.rs, supervisor.rs}`,
  `arb-land/{lib.rs, bidder.rs, leader.rs, ledger.rs, tip_oracle.rs}`, `arb-tx/{lib.rs, adaptive.rs,
  pump_fun_raydium.rs, master_alt.rs, compiler.rs}`, `arb-constants/src/lib.rs`, `arb-risk/src/lib.rs`, `config.toml`,
  `config/config.example.toml`, `.env.example`, `.env` (key presence only).

---

## 0. Executive summary

The Jito path is a genuinely sophisticated, mostly fail-closed design: live REST+WS tip-floor oracle, leader/kobe
  landability gate, Wilson-lower-bound per-rung landing ledger, EV-optimal rung selection, ladder-escalation cap,
  atomic single-transaction flash-loan bundle with the tip at the end, and dual-track landing verification before
  profit is counted. However, the shipped configuration silently flips several of those safety invariants to fail-open
  or overly permissive values, the `.env` "live flags" are not wired to any code gate, the configured
  `JITO_BLOCK_ENGINE_URL` override is honoured for status polls but **ignored on the actual bundle send path**, and
  the "master ALT" contains all 8 tip accounts (should not be the case), even though the live send path currently uses
  legacy v0-less transactions with no lookup tables.

---

## 1. Block Engine endpoints, auth, bundle construction, tip account selection

### 1.1 Endpoints and regional selection
- `crates/arb-land/src/lib.rs:27-29` - `DEFAULT_JITO_REGIONS = ["amsterdam","frankfurt","ny"]`.
- `JitoLander::endpoint()` - `arb-land/src/lib.rs:215-230`. Empty/`"mainnet"` ->
  `https://mainnet.block-engine.jito.wtf/api/v1/bundles`; full `http(s)://` URLs accepted (appended with
  `/api/v1/bundles` if absent); otherwise `https://<region>.mainnet.block-engine.jito.wtf/api/v1/bundles`. Region
  matching is case-insensitive and trims whitespace (lib.rs:827-860 tests).
- `endpoints()` - lib.rs:234-249: `JITO_BLOCK_ENGINE_URL` env override first (if it passes fail-closed validation),
  then region-derived engines deduplicated, global mainnet as last resort.
- `primary_endpoint()` - lib.rs:253-258: used for `simulateBundle`, `getBundleStatuses`, `getInflightBundleStatuses`,
  `getTipAccounts` - override applies there, but **not** to the fire-path send (see 1.3).
- Config `regions = ["frankfurt","amsterdam","ny"]` (config.toml:23) overrides the code default ordering (amsterdam,
  frankfurt, ny) - same set, different order (order irrelevant; all regions are raced concurrently).
- `.env.example:93` documents `JITO_BLOCK_ENGINE_URL=https://mainnet.block-engine.jito.wtf/api/v1/bundles`. `.env`
  contains `JITO_BLOCK_ENGINE_URL`, `MAX_JITO_TIP_SOL`, `ENABLE_SOLANA_JITO` (presence noted, values not printed).
- `resolved_override_url()` - lib.rs:205-213: override accepted only if non-empty, not containing `REPLACE_ME`, and
  starting with `https://`; otherwise silently ignored. Good fail-closed construction.

### 1.2 Auth
No authentication anywhere in the Jito transport. `send_bundle` / `send_bundle_multi_region` (lib.rs:260-317, 591-656)
  post bare JSON-RPC with no `Authorization`/`x-jito-*` header; grep for `auth|x-jito|8f2e` in `arb-land/src` returns
  nothing. The block engine is paid purely via the tip; this codebase requires no API key. If the operator relies on
  `JITO_BLOCK_ENGINE_URL` pointing at an authenticated/rate-limited proxy, that proxy receives no credential from this
  code.

### 1.3 Bundle construction and send path
- `send_bundle` caps bundles at 1..=5 txs (lib.rs:260-263), encodes base64 (lib.rs:264, 683-686), `sendBundle` params
  `[txs, {"encoding":"base64","bundleOnly":true}]` (lib.rs:265-270).
- Fan-out is **concurrent** over all resolved endpoints via `join_all`; first `Ok(bundle_id)` short-circuits but every
  endpoint was already hit (lib.rs:274-317). Response parsing requires HTTP success **and** a JSON-RPC `result`
  string; anything else is an error string per endpoint (lib.rs:283-302). Final failure aggregates all errors
  (lib.rs:313-316). `bundleOnly: true` is set - the bundle is not forwarded onto gossip - correct for MEV privacy.
- **`send_bundle_multi_region` ignores the env override** (lib.rs:319-330 -> free fn `send_bundle_multi_region`,
  lib.rs:591-656, which reads only `regions`). `dispatch.rs:419-421` calls
  `self.jito_lander.send_bundle_multi_region(&bundle_txs, &self.config.jito.regions)` - so on the live fire path the
  configured `JITO_BLOCK_ENGINE_URL` (and the whole `endpoints()`/`override_url` machinery) is bypassed; only the
  region-derived engines receive the bundle. The override is honoured for simulation and status RPCs only. Finding:
  **config foot-gun - the documented Jito override does not affect the path that spends the tip.**
- The dispatch layer builds exactly **one** transaction per bundle: `let bundle_txs = vec![tx_bytes.to_vec()]`
  (dispatch.rs:417). Multi-tx bundles are supported by the transport but never used.

### 1.4 Tip accounts used in code (8 official Mainnet accounts, duplicated verbatim)
`arb-constants/src/lib.rs:57-66` and duplicated verbatim in `arb-tx/src/pump_fun_raydium.rs:22-31` (byte-identical;
  verified by diff):
```
ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49
DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh
DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL
ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt
Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY
96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5
3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT
hFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe
```
Selection functions: `jito_tip_account_at / _round_robin / _for_leader / _for_slot` (arb-constants:84-117). The
  leader->account mapping hashes the leader identity by **summing bytes** (`acc.wrapping_add(b as usize)`,
  arb-constants:102-108) - a weak, collision-prone hash; deterministic but not uniform. Acceptable for cache-bust
  avoidance, not a security control. `jito_tip_account_for_slot` falls back to `slot % 8` when the leader is unknown
  (arb-constants:112-117).
- Live native/flash route: `tip_account_for_slot(target_slot)` with leader resolution (run.rs:619,
  jito_service.rs:194-197).
- Jupiter exotic route (currently disabled): `arb_land::get_random_tip_account()` round-robin - **inconsistent** with
  the native path's deterministic selection (execution.rs:95).
- Split route: same `tip_account` threaded through `assemble_split_flash_arbitrage` (run.rs:1186).
- Tests reference `JITO_TIP_ACCOUNTS[0..8]`; `bundle_tests.rs:517` asserts `len() == 8`.

### 1.5 Tip account writability / ALT
- Tip instruction accounts: `AccountMeta::new(payer, true)` and `AccountMeta::new(tip_account, false)`
  (arb-tx/lib.rs:255-258). `AccountMeta::new` sets `is_writable: true` unconditionally (arb-tx/lib.rs:188-195). So the
  **tip account is writable** - correct for a transfer destination - and the payer is signer+writable. Tip account is
  **not** a signer, correct.
- The tip transfer carries the Jito `jitodontfront` marker as a **read-only** account appended to the tip instruction
  (arb-tx/lib.rs:274-300; `JITODONTFRONT` marker at arb-constants:132-140). The code comments correctly note the
  marker must be read-only and must ride on a real instruction, not stand alone (arb-tx/lib.rs:263-273).
- **ALT finding:** `master_alt.rs:537-548` inserts **all 8 Jito tip accounts into the "master ALT"** as
  `AddressCategory::JitoTipAccount`, owned by the system program; `compiler.rs:1339-1344` asserts `jito_in_alt == 8`;
  the master-ALT builder maps all 8 at launch time. Tip accounts in a lookup table violate the requirement that tip
  accounts be referenced directly by the bundle's tip transfer. However: the live dispatch path builds **legacy
  v0-less** transactions via `build_signed_transaction` -> `compile_message` with **no lookup tables**
  (run.rs:1258-1259 comment "do NOT use the fabricated ALT ... Always build a legacy (v0-less) signed transaction";
  arb-tx/lib.rs:3472-3485). So on the current hot path tip accounts are passed inline, not via ALT. The ALT
  contamination is latent (only v0 compile paths reference it) but the invariant is already broken in the artifacts.


---

## 2. Tip accounting

### 2.1 Who pays / which instruction / which mint
- Payer: the bot keypair (`config.toml [wallet] keypair_path = "solana_keypair.json"`). The cold wallet pubkey
  (`cold_wallet_pubkey`) is never used. The tip is a **native SOL** `SystemProgram::Transfer` (program `1111...11`,
  instruction index 2, 8-byte lamport amount LE) from `payer` to the tip account (arb-tx/lib.rs:244-261). No SPL mint
  is involved; WSOL is used only for the loan leg.
- Tip instruction is emitted **last**, after the flash repay and ATA close ("6. Jito Tip at the end",
  arb-tx/lib.rs:3213-3220; same pattern in `assemble_arbitrage` lib.rs:2977-2984, `assemble_jupiter_flash_arbitrage`
  lib.rs:3269-3276, `assemble_split_flash_arbitrage` lib.rs:3319-3325, `assemble_atomic_bundle` lib.rs:2807-2814). The
  entire tx is atomic: borrow -> swaps -> repay -> tip.
- Solvency floor in `derive_swap_legs` explicitly adds the tip on top of borrow+fee+min-profit before computing
  min-out (arb-tx/lib.rs:3019-3022 comment "Enforce full on-chain solvent floor: borrow + flash_fee + jito_tip +
  net_retained_profit").

### 2.2 Tip deducted once or twice
Once per transaction, and only if `tip_amount.as_u64() > 0` (guards at lib.rs:2808, 2978, 3214, 3270, 3320). The tip
  is **not** added to the base or priority fee anywhere else. However:
- The transaction *also* pays a **priority fee** via `build_compute_unit_price(dynamic_cu_price)`, where
  `dynamic_cu_price = calculate_dynamic_cu_price(gross_profit, cu_price)` elevates the requested price by
  `gross_profit/25_000` capped at +500_000 ?lamports (arb-tx/lib.rs:106-109, 2893-2898, 3076-3081) - i.e. the
  requested `cu_price` is a *floor*, and the builder silently inflates it. `cap_cu_price_by_profit` bounds the
  *requested* price by `PRIORITY_FEE_MAX_SHARE_BPS` of gross (run.rs:656-661), but the builder's un-capped
  `dynamic_cu_price` addend (lib.rs:107-109) applies **after** that cap and is not re-bounded. Two independent fee
  elevators, only one of which is profit-capped - finding.
- `calculate_fee_deduplicated_cu_price` exists (lib.rs:114-124, "bundle-only mode returns 0 to eliminate priority fee
  duplication and concentrate 100% of priority budget into dynamic JitoTip") but is **never called** - the bundle path
  pays both the priority fee and the tip, duplicating the auction budget.
- run.rs keeps `priority_fee_lamports` (run.rs:663) and `tip` as separate line items in the cost basis (comment at
  run.rs:621-623), and both appear in the Chronos EV gate input (run.rs:964-965) and in the `calculate_net_profit_raw`
  cost subtraction (run.rs:805-809). Accounting is separate, not double-counted - but ?2.2 makes the total outflow
  higher than the priced model.

### 2.3 Is the tip charged if the bundle doesn't land?
No - correct by construction. The tip is an instruction **inside** the single atomic transaction; if the bundle is not
  included, the transaction never executes and zero lamports move. If the transaction lands but any instruction fails
  (e.g. slippage, repay check), the whole tx reverts - again no tip. Code comments reflect this: dispatch.rs:640
  ("Unlanded/expired Jito bundles pay 0 SOL; do not count phantom tip loss or trip breaker") and dispatch.rs:715
  ("Unaccepted bundle / transport error does not spend tip or base tx fee (0 SOL loss)").
Caveat: if the bundle lands, the tip **is** paid even though the arbitrage may have extracted nothing beyond min-out -
  inherent to Jito and correctly modelled (tip is decided on expected value before submission).

### 2.4 Separate accounting of priority fee vs tip
Yes: priority fee is `cu_price x cu_limit / 1e6` lamports (run.rs:663) priced by `adaptive_cu_price` (p75 of recent
  landed CU prices +25%, `TipStrategy::adaptive_cu_price`, arb-land/lib.rs:157-170; `recent_landed_cu` starts empty ->
  defaults to 100_000 ?lamports/CU, arb-land/lib.rs:161-163); tip is the bidder's decision. Both are deducted in
  `net_before_tip` (run.rs:676-679) and in the Chronos gate - but the tip is also a direct input to the flash-repay
  min-out floor (?2.1).

---

## 3. Bundle semantics and flash-loan repayment

- Bundles are **single-transaction** in practice (dispatch.rs:417). Transport allows up to 5 (lib.rs:261-263,
  592-594).
- The flash-loan **borrow -> swaps -> repay-with-index** sequence is inside that one transaction (lib.rs:3120-3121
  borrow, swaps 3141-3184, repay 3200-3206 with `build_kamino_flash_repay_with_index(.., borrow_idx as u8)` binding
  the repay to the borrow's instruction index). Atomicity preserves the Kamino repayment constraint: if repay fails,
  borrow and swaps revert too - no standalone borrow can leak.
- Pump.fun-first routes unwrap WSOL before hop 1 and re-wrap before repay (lib.rs:3164-3167, 3188-3198); Pump.fun-last
  routes re-create the WSOL ATA + self-transfer + sync-native before repay - still one transaction.
- Split route (N-way DLMM split + mandatory return hop, run.rs:1114-1200) is a **single**
  `assemble_split_flash_arbitrage` transaction (arb-tx/lib.rs:3282-3325): borrow -> all legs -> return hop -> repay ->
  tip. The return hop is enforced in code (run.rs:1165-1183 comment: "Omitting it would leave the flash loan holding
  the intermediate token and fail the repay").
- Jupiter route is also single-tx (`assemble_jupiter_flash_arbitrage`) - but Jupiter execution is hard-disabled
  (`route_via_jupiter = false`, run.rs:1110), so `resolve_jupiter_swap_route` (execution.rs:59-104) is dead code in
  production, including its use of a **random** tip account (execution.rs:95) which would otherwise break tip-account
  determinism.
- `build_jito_tip_transaction` / `build_marked_jito_tip_transaction` (lib.rs:3449-3468) would enable a 2-tx "swap +
  tip" bundle with the tip split out of the atomic tx - exists, unused.
- `assemble_flash_arbitrage` rejects non-closing routes and non-WSOL start mints (lib.rs:3123-3134) - the flash loan
  cannot route atomically unless borrow mint == final repay mint.


---

## 4. Submission flow: simulation -> response -> confirmation

### 4.1 Simulation before dispatch (Live mode)
1. LiteSVM in-memory simulation - sub-ms, rejects on error (dispatch.rs:200-215).
2. RPC `simulateTransaction` preflight - skipped if `FAST_PATH_SKIP_RPC_SIM=true` or
  `ENABLE_PREFLIGHT_SIMULATION=false`; default **on** for Live mode (`preflight_env.unwrap_or(self.mode ==
  BotMode::Live)`, dispatch.rs:217-234). On simulation failure: record + alert + **skip dispatch**
  (dispatch.rs:245-271). On RPC call error: fail-over to `RpcPool::simulate_transaction`; if that errors too, **fail
  closed** (dispatch.rs:328-334).
3. **Jito `simulateBundle` is never called.** `JitoLander::simulate_bundle` (arb-land/lib.rs:332-357) has no call
  sites (grep verified).
4. Fire-time slot-window wait: for non-SWQoS routes whose target slot (`current+1`) is Jito-landable,
  `slot_clock.schedule(target_slot, rtt, 5)` and sleep if wait <= 400 ms (`SLOT_MS`) (dispatch.rs:339-364).
  `FAST_PATH_IMMEDIATE_DISPATCH=true` bypasses the wait.
5. Final stream-readiness guard (`validate_execution_readiness`) re-checked immediately before submission
  (dispatch.rs:366-397).
6. Target slot for the tip account is `current+1` (run.rs:618) but the pricing plan is made against **`current_slot`**
  (run.rs:723-724) - the leader gate prices against the *current* slot's leader while dispatch targets the next slot;
  `dispatch.rs:348-351` does use `target_slot` for the fire-time wait. Inconsistent horizon between the two `bids_for`
  calls.

### 4.2 Response parsing / bundle id handling
- HTTP 2xx + JSON-RPC `result` string required; `bundle_id` kept as opaque string (lib.rs:282-301). Any non-success is
  an aggregated error; `send_bundle` does not retry by itself (`max_retries = 3` field exists but is
  `#[allow(dead_code)]`, lib.rs:180-181, 196).
- Duplicate submission prevention: fan-out sends the **identical** bundle to all regions; Jito dedups by bundle
  id/signature, so identical bundles cannot double-land. Across ticks, the same opportunity could be re-submitted with
  a *different tip* (different signature -> different bundle id) in the same slot - no dedup key on `(slot, route_id)`
  at the send site; only `previous_tips` (escalation cap, run.rs:457-459) and `risk_gate.max_inflight_bundles = 32`
  (config.toml:103; arb-risk check arb-risk/lib.rs:627-638) bound this. `record_order` / `record_bundle_sent` called
  per submission (dispatch.rs:452-454).
- No explicit idempotency, no auction-timeout parameter. Blockhash staleness: cached <= 3 s (execution.rs:106-140),
  zero hash explicitly rejected (run.rs:1279-1301). Validity window (~60-90 s) is not tracked via
  `lastValidBlockHeight` - expiry is detected only post-hoc via status polls returning `Expired`.

### 4.3 Landing confirmation
- `verify_landing` (jito_service.rs:425-520) - the only confirmation path actually used (dispatch.rs:522-532):
  - 20 attempts x 350 ms (~7 s), each iteration: (a) `getBundleStatuses` on the **primary** endpoint (override-aware)
  parsing `confirmation_status` (`processed->Sent`, `confirmed|finalized->Landed`, `failed->Failed`, `dropped->Dropped`,
  `expired->Expired`; missing/result-less body -> `Sent`, arb-land/lib.rs:413-449); (b) cross-check via Solana RPC
  `getSignatureStatuses` - `Confirmed|Finalized->Landed`, `Failed->Failed`; on RPC `Failed`, returns `Failed` even if
  Jito said `Landed` (jito_service.rs:450-468) - revert detection.
  - If Jito reports `Dropped|Expired`, RPC is consulted before accepting the verdict; a landed-on-chain signature
  upgrades it to `Landed` (jito_service.rs:469-482).
  - After the loop, one last RPC check, else `Expired` (jito_service.rs:510-519).
  - **Ledger pollution risk:** dispatch.rs:637-639 and 665-670 call `js.record(rung, InflightStatus::Failed)` for
  **both** `Dropped|Expired` and `Failed`. `Expired` after the 7 s window can mean "Jito cache evicted / status lag",
  not "bundle failed"; recording it as a negative observation biases the per-rung landing posterior downward and
  progressively suppresses bidding. There is no re-check after a longer delay for bundles that land late.
- `getInflightBundleStatuses`-based `poll_until_terminal` (jito_service.rs:396-416) and `poll_bundle_until_terminal` /
  `check_status` (arb-land/lib.rs:539-587) are **defined but never called** by the dispatch path; the
  5-minute-lookback inflight endpoint and its `Invalid` semantics (lib.rs:457-464, 693-716) are unused in production.
- Landing statistics feed `risk_gate.record_bundle_sent/landed` (dispatch.rs:454, 594, 641, 671) and the
  `LandingLedger` (dispatch.rs:584-591, 637-639, 668-670) which persists to `data/adaptive_ledger.json`
  (config.toml:36) with a u32 schema gate (ledger.rs:24, 145-154).
- Telemetry: `alert_execution_sent` fires on acceptance (dispatch.rs:448-450); `inc_fills` / `add_net_profit` /
  `alert_trade_landed` fire **only** on `Landed` (dispatch.rs:536-632). `record_outcome(record.id, "sent", None)`
  records submission (dispatch.rs:460). PnL/fill counting is gated on landing - verified, not assumed.

### 4.4 Timeouts / retries / RTT
- HTTP clients: dispatch client 5 s timeout, tcp_nodelay, pool 8/host (arb-land/lib.rs:48-58); `JitoService` 30 s
  (jito_service.rs:135-138); tip oracle 3 s (tip_oracle.rs:59-62); tip WS reconnect 2 s (tip_oracle.rs:193-197).
- `send_bundle` has no retry; lander `max_retries = 3` is dead (lib.rs:181, 196). Verify polls: 20 x 350 ms ? 7 s for
  Jito, 20 x 400 ms ? 8 s for SWQoS (dispatch.rs:505-531). A bundle landing after the window is booked `Expired`
  (jito_service.rs:519) - see ?4.3.
- Note the tip-floor poll interval is clamped to >= 1 000 ms (public endpoint 1 rps/IP) - jito_service.rs:525.


---

## 5. Failure analysis - tip floor vs requested tip, hardcodes, "never spend more than edge"

### 5.1 Enforced bounds (all must agree to fire)
1. Bidder hurdle: `max(profit_floor, keep_share x edge)` (bidder.rs:110-113, 125-140). Shipped: `keep_share = 0.60`,
  `min_keep_lamports = 1_000_000` (config.toml:28,31) -> hurdle = max(1_000_000, 0.6 x edge). Note the shipped
  `keep_share=0.60` is far more permissive than the code default `0.85` (arb-constants:77).
2. Bidder ceiling (`t_max`): `min(edge ? hurdle, max_tip_share x edge)` (bidder.rs:143-154). Shipped
  `max_tip_share_bps = 9500` -> `max_tip_share = 0.95` (supervisor.rs:370) vs code default `0.85`. Ceiling =
  min(0.4-edge, 0.95-edge) = **0.4-edge** for hurdle-dominated cases.
3. Rung grid: only market rungs <= ceiling are candidates; winner is EV-optimal under the Wilson-lower-bound inclusion
  probability from the ledger (bidder.rs:260-301, ledger.rs:90-102).
4. Escalation ladder: a retry may raise the previous tip by <= `ladder_growth` (1.6, config.toml:32;
  bidder.rs:217-230).
5. Edge surplus check: `t_max = net_before_tip ? min_net_floor ? 10_000` contingency; if `t_max <
  MIN_JITO_TIP_LAMPORTS (1_000)` the opportunity is skipped (run.rs:684-707); bidder result is hard-capped at `t_max`
  (run.rs:735).
6. Live risk gate: `check_tip_share` (`tip_share_bps <= max_tip_share_bps` = **9500**, arb-risk/lib.rs:551-559),
  `check_tip_lamports` (`tip <= max_tip_lamports = 2_000_000` - **defined but never called on the evaluate path**,
  arb-risk/lib.rs:560-568 vs 455-548), `check_tip_spend_ratio` (`tip/(tip+net) <= max_tip_spend_ratio = 0.98`,
  arb-risk/lib.rs:640-659; risk config `[risk] max_tip_spend_ratio = 0.98`, config.toml:104), inflight bundle cap 32,
  `min_landing_rate = 0` (config.toml:99).
7. Hard invariant in the opportunity validator: `tip >= net_profit` ? reject ("negative ROI after tip")
  (run.rs:1732-1739); flash repayment `expected_out >= size_in + flash_fee` enforced (run.rs:1709-1730).
8. Chronos EV gate receives `jito_tip_lamports` explicitly (run.rs:965) and can veto on expected-value decay.

So "never spend more than the edge" **is** enforced structurally at several independent layers. The weakest points are
  the *configured* limits, not the code: `max_tip_share_bps = 9500`, `max_tip_spend_ratio = 0.98`, `keep_share = 0.60`
  (shipped) allow paying almost the whole edge; the $0.20 net-floor logic (`min_net_profit_abs = 1_000_000`,
  `min_keep_lamports = 1_000_000`, config.toml:28,83) is the only hard "keep something" backstop. Importantly, the
  absolute lamport cap that would backstop large edges (`check_tip_lamports`) is dead code - the 0.002-SOL
  `max_tip_lamports` is **not enforced anywhere**.

### 5.2 Tip floor vs requested tip
Requested tip is always a live rung of the landed-tip distribution (p25..p99 from `tip_floor` REST / `tip_stream` WS),
  never a hardcoded constant on the live path. The protocol minimum `MIN_JITO_TIP_LAMPORTS = 1_000` (bidder.rs:23) is
  the bid floor. `DEFAULT_JITO_TIP_FLOOR_LAMPORTS = 10_000` (arb-constants:71) is used only as `TipStrategy`'s floor -
  and `TipStrategy::pick_dynamic_tip`/`pick_tip` have **no call sites** in the pricing path anymore (only
  `adaptive_cu_price` is used, run.rs:655 vs supervisor.rs:168-173). Dead-but-armed: `TipStrategy::new` silently
  raises the configured max share to >= 7500 bps (`share_bps.max(7500)`, arb-land/lib.rs:105) - if anyone re-enables
  it, a 5500 bps config becomes a 7500 bps cap. Shipped `[jito] tip_share_bps = 5500` (config.toml:25) therefore
  prices nothing.

### 5.3 Auction competition
Competitive pressure enters via the market rungs themselves. The code does not model competing bundles or validator
  tip policy; it assumes inclusion probability ? the rung's historical win rate (ledger) and refuses when EV <= 0.
  There is no explicit "min tip to be competitive in this slot" beyond the published distribution; the auction is
  entered at the cheapest rung with positive EV, not at a competitive-clearing price by design.

### 5.4 MAX_JITO_TIP_SOL handling
**Not read anywhere in the crates.** `grep` across `crates/` finds `JITO_BLOCK_ENGINE_URL` only. The advertised
  absolute cap in `.env.example:94` (`MAX_JITO_TIP_SOL=0.005`) and in `.env` changes nothing. The effective caps are:
  bidder ceiling (edge-relative), risk-gate `max_tip_share_bps = 9500` (share of gross), `max_tip_spend_ratio = 0.98`
  (share of tip+net). `max_tip_lamports = 2_000_000` is passed to `RiskLimits` (supervisor.rs:131) and consumed by
  `check_tip_lamports`, which is never invoked (?5.1.6). **Finding: the documented SOL-denominated safety cap is
  fictional; worst-case tip on a large edge is bounded only by the bidder ceiling and the 98% spend-ratio rule.**

---

## 6. Submitted vs landed - does the code inflate?

- Submission path records outcome `"sent"` (no profit) immediately (dispatch.rs:460); `inc_attempts` at dispatch time
  (run.rs:1101), but `inc_fills` / `add_net_profit` **only** inside the `Landed` arm of the async tracker
  (dispatch.rs:595-597). `add_simulated_net_profit` is used for canary/shadow (dispatch.rs:87, 106), separate from
  realized metrics.
- Telegram alerts: `alert_opportunity_with_telemetry` (profit shown as **projected**, not realized) fires on send
  (dispatch.rs:435-447) and on canary/simulate; `alert_execution_sent` fires on acceptance (dispatch.rs:448-450);
  `alert_trade_landed` fires only on `Landed` with measured slot and timings (dispatch.rs:620-632). The "Execution
  finalized/landed ... Net Profit: +... SOL" string is printed only in the `Landed` arm (dispatch.rs:564-578). Alert texts
  accurately label "accepted" vs "landed".
- `store.record_outcome(rec_id, "landed", Some(net_profit_val))` attaches profit **only** to `"landed"`
  (dispatch.rs:579-583); `"dropped"`/`"failed"`/`"sent"` carry no profit.
- `risk_gate.on_success()` only on `Landed`; `on_failure` only on `Failed` (revert), not on `Dropped` - dropped
  bundles don't trip the circuit breaker (intended, dispatch.rs:640).
- `LandStatus::Sent` is non-terminal; if the 20-poll loop never reaches a verdict, the result is `Expired`
  (jito_service.rs:519) -> recorded as a ledger failure. **A bundle that landed after ~7 s is booked as a failure.** It
  will not be counted as profit (correct), but the learning signal is corrupted (?4.3). No code path reports `Sent` as
  landed - verified; no code path counts profit before confirmation - verified.

---

## 7. Tip oracle - live or static, staleness

- `TipOracle` (tip_oracle.rs): REST poll of `https://bundles.jito.wtf/api/v1/bundles/tip_floor` (`TIP_FLOOR_URL`,
  tip_oracle.rs:26) at a configurable interval (shipped 1000 ms, config.toml:34; clamped >= 1 000 ms for the 1 rps/IP
  limit, jito_service.rs:525) **plus** a WebSocket push reader from `wss://bundles.jito.wtf/api/v1/bundles/tip_stream`
  (`TIP_WEBSOCKET_URL`, tip_oracle.rs:28; `run_ws` tip_oracle.rs:154-211), launched via `run()`
  (tip_oracle.rs:207-211) and driven from `JitoService::run` (jito_service.rs:522-536). Both feeds write the same
  cache; either can seed it.
- Parsing accepts the documented one-element SOL-float array or a bare object; missing rungs degrade to the
  **median**, never to zero (tip_oracle.rs:245-265). `sol_to_lamports` rounds and saturates (tip_oracle.rs:268-279).
- Freshness: `DEFAULT_MAX_AGE_MS = 5_000` (tip_oracle.rs:32) - a snapshot older than `max_age_ms` (shipped 5000,
  config.toml:33) returns `None` from `current()` (tip_oracle.rs:120-128), and a missing market is a **refusal**,
  never a default price (bidder.rs:195-204 "a missing snapshot is a refusal, not a discount"; refusal path
  jito_service.rs:713-733).
- **No static fallback anywhere in the pricing path.** The only static tip constants are
  `DEFAULT_JITO_TIP_FLOOR_LAMPORTS = 10_000` (vestigial `TipStrategy`, never called for pricing) and
  `MIN_JITO_TIP_LAMPORTS = 1_000` (block-engine floor). If the REST/WS feeds die, the bot **stops bidding** - the
  opposite of a silent static-floor bug. Correct design.
- WS failure: logs and retries in 2 s; REST keeps the cache independently (tip_oracle.rs:148-153, 192-198). Individual
  REST failures leave the last snapshot in place, which simply ages out (tip_oracle.rs:213-231).
- Stale-comment bug: run.rs:418-420 says "Jito cost uses the configured conservative tip ceiling until a slot-aware
  tip-floor feed is integrated" - outdated; the feed exists and is wired.
- Leader gate refresh: every 60 s (`leader_refresh_secs = 60`, config.toml:37) via `getEpochInfo` +
  `getLeaderSchedule` (current-epoch no-arg fallback when providers return null, jito_service.rs:330-350) joined
  against the kobe registry (`https://kobe.mainnet.jito.network/api/v1/validators`, jito_service.rs:33) filtered to
  `running_jito == true`, indexed by identity **and** vote account (leader.rs:160-215). `tip_on_unknown_leader` is
  configurable (?8). Unknown/stale schedule ? `Landability::Unknown` ? refusal when `tip_on_unknown_leader = false`.


---

## 8. Do ENABLE_SOLANA_JITO / live flags gate the send path? What is the default?

- **`ENABLE_SOLANA_JITO` is not read by any code.** It appears only in `.env.example:95` (`=true`) and `.env`
  (presence noted). There is no `std::env::var("ENABLE_SOLANA_JITO")` anywhere in `crates/`. Same for
  `MAX_JITO_TIP_SOL` (?5.4). Live participation is governed entirely by `config.toml [bot] mode = "live"`
  (config.toml:5) and `BotMode::Live` (dispatch.rs:126) - plus the Jito leader gate (`leader_gate_enabled`,
  config.toml:35 -> `tip_on_unknown_leader: !leader_gate_enabled`, supervisor.rs:386-390).
- **Default polarity is inverted in the shipped config.** Code default `leader_gate_enabled = true` ? fail-closed
  (unknown leader ? refuse, known non-Jito ? skip) (main.rs:218-220; jito_service.rs:104-108 `tip_on_unknown_leader:
  false`). Shipped `config.toml` sets `leader_gate_enabled = false` ? `tip_on_unknown_leader = true` ? **bundles (and
  tips) are sent against slots whose leader is unknown**; only known-non-Jito leaders are skipped. The comment on
  config.toml:35 describes this accurately, but it is the opposite of the code's fail-closed default, and
  `config/config.example.toml:34` ships `true`.
- `config.toml [bot] canary = false` and `mode = "live"` - the shipped configuration is armed for real bundles.
- Other env gates on the live path (all string-parsed booleans, fail-closed default off): `FAST_PATH_SKIP_RPC_SIM` /
  `ENABLE_PREFLIGHT_SIMULATION` (dispatch.rs:217-234), `FAST_PATH_IMMEDIATE_DISPATCH` (dispatch.rs:339-349),
  `BYPASS_CHRONOS_GATE` (run.rs:947-954), `BYPASS_RISK_GATE` (run.rs:1033-1038), `MIN_SOL_RESERVE_LAMPORTS`
  (dispatch.rs:129). None of them disable Jito.
- **Bottom line:** an operator who believes `ENABLE_SOLANA_JITO=false` or `MAX_JITO_TIP_SOL` will disarm tip spending
  will be surprised - the bot will still build bundles with tips up to the bidder's ceiling and dispatch them, because
  `mode="live"` and the leader gate is configured fail-open for unknown leaders. The only real arm/disarm switches are
  `mode` and the (mis-set) leader gate.

---

## 9. Findings, ranked

1. **`JITO_BLOCK_ENGINE_URL` override is ignored on the bundle send path** - `dispatch.rs:419-421` ->
  `JitoLander::send_bundle_multi_region` (arb-land/lib.rs:319-330, 591-656) uses only region-derived endpoints; the
  validated env override applies solely to simulation/status RPCs (lib.rs:253-258, 343-344, 487). The documented
  control for routing bundles does not route bundles.
2. **`ENABLE_SOLANA_JITO` and `MAX_JITO_TIP_SOL` are dead knobs** - no code references them; the effective absolute
  tip cap (`check_tip_lamports`, arb-risk/lib.rs:560-568) is defined but **never called** on the evaluate path
  (arb-risk/lib.rs:455-548 calls only `check_tip_share` and `check_tip_spend_ratio`). Worst-case tip is bounded only
  by the bidder ceiling (~0.4-edge) and `max_tip_spend_ratio = 0.98`.
3. **Shipped config disables the fail-closed leader gate** - `leader_gate_enabled = false` (config.toml:35) inverts to
  `tip_on_unknown_leader = true` (supervisor.rs:390), flipping the code's fail-closed default; the example config
  ships `true`.
4. **Expired-after-7s bundles are booked as ledger failures** - `verify_landing` times out to `Expired`
  (jito_service.rs:519) and dispatch records `InflightStatus::Failed` (dispatch.rs:637-639, 668-670), corrupting the
  per-rung landing posteriors that drive bid EV; no delayed re-check exists.
5. **Priority fee duplicated with the tip** - `calculate_fee_deduplicated_cu_price` (lib.rs:114-124, designed to zero
  the CU price in bundle-only mode) is never used; `assemble_*` always injects `build_compute_unit_price` inflated by
  an uncapped `gross_profit/25_000` addend (lib.rs:106-109, 2893-2898, 3076-3081) on top of a profit-capped requested
  price (run.rs:656-663).
6. **Master ALT contains all 8 Jito tip accounts** (master_alt.rs:537-548; compiler.rs:1339-1344 asserts 8-in-ALT).
  Live path is legacy v0-less and safe today (run.rs:1258-1259; arb-tx/lib.rs:3472-3485), but the artifact violates
  the "tip accounts must not be ALT-resolved" invariant; the tip ix relies on inline keys.
7. **Config/code default drift toward aggression** - `keep_share 0.60` vs 0.85, `max_tip_share_bps 9500` vs 8500,
  `max_tip_spend_ratio 0.98` vs 0.7 - coherent with the $0.20 invariant but far looser than library defaults;
  `TipStrategy::new` silently raises `max_share_bps` to >= 7500 (arb-land/lib.rs:105) should it ever be re-wired.
8. **Robustness nits** - leader->tip-account "hash" is a byte-sum (arb-constants:102-108, collision-prone);
  Jupiter/random tip selection inconsistent with leader-mapped native path (execution.rs:95, dead code but armed);
  pricing plan made against `current_slot` while dispatch targets `current_slot+1` (run.rs:723-724 vs 348, 618-619);
  `previous_tips` keyed on `current_slot` with 2-slot retention (run.rs:412-414, 457-459, 710-714);
  `record_bundle_sent` uses `current_slot` not the target slot (dispatch.rs:454); stale "no slot-aware tip-floor feed"
  comment (run.rs:418-420); bundle `simulateBundle` implemented but never called (arb-land/lib.rs:332-357);
  `getInflightBundleStatuses` machinery unused (jito_service.rs:396-416, arb-land/lib.rs:465-537); `.env.example`
  `MAX_JITO_TIP_SOL=0.005` contradicts shipped `max_tip_lamports=2_000_000`; the leader gate prices the slot being
  priced, not the slot dispatched.
9. **Positive confirmations** - tip paid once, only on landing, native SOL, SystemProgram transfer, correct
  writability, atomic with borrow/swaps/repay, `bundleOnly=true`, `jitodontfront` marker read-only on the tip ix,
  live+WS tip floor with stale-refusal (no static floor in pricing), EV/Wilson rung pricing, ladder cap, profit
  counted only on `Landed` (dispatch.rs:595-597), RPC cross-check of Jito "Landed" (jito_service.rs:450-468),
  single-tx bundle (never splits borrow/repay), tip >= net-profit invariant enforced (run.rs:1732-1739), fail-closed
  preflight simulation, unknown-status->`Sent` mapping avoids false terminal verdicts (arb-land/lib.rs:813-819).
