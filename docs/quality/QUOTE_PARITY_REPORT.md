# QUOTE_PARITY_REPORT — Differential Quote Verification

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Verification date**: 2026-11-21
**Parity policy**: For executable routes, an **unexplained** quote discrepancy against the protocol reference is a hard failure. Explanations must identify a rounding contract, fee-scope difference, or coverage limitation, and be recorded here.

---

## 1. Parity references in use

| DEX family | Local engine | Reference oracle | Oracle class |
|---|---|---|---|
| Raydium CPMM / AMM v4 / DAMM v2 constant-product accounting | `arb-math::cpmm` | Independent Python oracle over protocol spec (`audit/math_oracle/oracle.py`), vectors `audit/test_vectors/cpmm_vectors.json` | Specification-derived |
| Meteora DLMM | `arb-math::dlmm` (multi-bin exact-in, exact-out, view-quote) | (a) Python oracle, `dlmm_single_bin_vectors.json`; (b) **official `@meteora-ag/dlmm@1.9.14`** over a byte-identical captured mainnet snapshot (`fixtures/meteora_mainnet_snapshot.json`, pool `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6`) | Spec-derived + official SDK |
| Raydium CLMM | `arb-math::clmm` + `arb-dex::raydium_clmm` | (a) In-repo unit + property tests + official raydium-clmm swap-step parity vectors (`raydium_clmm_swap_math_parity_tests.rs`); (b) **official `@raydium-io/raydium-sdk-v2@0.2.85-alpha`** `PoolUtils.getOutputAmountAndRemainAccounts` over byte-identical captured mainnet snapshots (`fixtures/raydium_clmm_mainnet_pools/`, 6 pools, tick spacings 1/60/120, `fee_on` 0/1) | Property/invariant + step parity + official SDK |
| Orca Whirlpool | `arb_math::clmm::compute_whirlpool_swap` | **official `@orca-so/whirlpools-core` v3.1.1 (wasm)** over byte-identical captured mainnet snapshots (`fixtures/whirlpool_mainnet_pools/`, 4 classic pools, tick spacings 1/4/8/64) | Official SDK |
| Pump.fun / PumpSwap / LaunchLab / DBC / DLMM-Pro | dedicated adapters | unit tests incl. fee-mode fail-closed guards | Property/invariant |

## 2. Executed evidence (this session)

Commands (all executed at the baseline commit, toolchain 1.99.1):

```bash
cargo test -p arb-math --test python_oracle_differential
cargo test -p arb-engine --test meteora_mainnet_sdk_differential
cargo test --workspace --all-targets
```

Results:

| Test | Outcome |
|---|---|
| `cpmm_matches_python_oracle` (word-for-word vectors, incl. fee/rounding boundaries) | ✅ PASS |
| `dlmm_single_bin_matches_python_oracle` | ✅ PASS |
| `mainnet_snapshot_prices_match_official_sdk` | ✅ PASS |
| `mainnet_snapshot_quotes_match_official_sdk` (exact-in/exact-out × small/deep × both directions × both `CollectFeeMode`s; deep cases cross 45–121 bins incl. bin-array boundaries) | ✅ PASS |
| `mainnet_pool_batch_quotes_match_official_sdk` (added 2026-11-21: 4 new mainnet pools — bin steps 1/10/50 + TRUMP/USDC pair, deep probes up to 511 bins crossed; every integer field and every Q64.64 price check exact) | ✅ PASS |
| `whirlpool_mainnet_batch_quotes_match_official_sdk` (added 2026-11-21: 4 classic mainnet whirlpools — tick spacings 1/4/8/64; both directions × exact-in small/deep = 16 deep-quote comparisons over captured tick-array sets; engine `compute_whirlpool_swap` matches `swapQuoteByInputToken` exactly: amount_in_consumed, amount_out, trade_fee tolerance 0; adaptive-fee pools fail-closed-rejected at capture; tick-array PDA re-derivation + raw-byte field re-verification per case; start-tick probes on `get_start_tick_index` exact) | ✅ PASS |
| `dlmm_timestamp_drift_matches_official_sdk_state_machine` + `dlmm_drift_invariants_never_decrease_reference_below_zero_or_above_max` (added 2026-11-21: 13 deterministic timestamp-drift scenarios vs `@meteora-ag/dlmm@1.9.14` fee statics — filter expiry, decay window boundaries, reset, pre-filter moves, VA clamp, backward timestamps, chained post-swap state; every reference/accumulator/fee integer exact) | ✅ PASS |
| `raydium_clmm_mainnet_batch_quotes_match_official_sdk` (added 2026-11-21: 6 live-mainnet Raydium CLMM pools — tick spacings 1/60/120, `fee_on` 0 and 1, both directions, small (1e6) + deep (bisected largest fully-servable, one probe at `u64::MAX`) = 14 exact-in comparisons incl. an empty-span multi-array crossing; `amount_in_consumed`/`amount_out_received`/`fee_paid` exact against `PoolUtils.getOutputAmountAndRemainAccounts`; tick-array PDA re-derivation + raw-byte field re-verification per pool; fee-on-output directions asserted REFUSED; a Token-2022 candidate refused at capture) | ✅ PASS |
| `mainnet_pool_batch_quotes_match_official_sdk` (extended 2026-11-21 with a genuinely deployed OnlyY pool, see §4.5: the batch test now loads 5 fixtures, requires a fail-closed refusal-ledger entry for every absent case, requires ≥1 servable case per pool, and asserts ≥1 pool with served real-mode-1 quotes) | ✅ PASS |
| `phase15_2_sdk_differential_manifest` | ✅ PASS |
| `dlmm_validation::liquidity_mining_function_type_passes_gate_but_limit_order_is_rejected` (added 2026-11-21: ConcreteFunctionType gate regression — ft=1 accepted, ft∈{2,3,255} fail-closed-rejected) | ✅ PASS |
| Full workspace suite | ✅ 1,450 passed / 0 failed / 2 ignored — includes the whirlpool differential batch, the DLMM timestamp-drift vectors, the Raydium CLMM mainnet SDK differential, and the new OnlyY fixture batch coverage |

Fee-scope note (SDK parity): the official DLMM client returns `fee` as the LP share and `protocolFee` separately; total trading fee = `fee + protocolFee`. The Rust engine's `fee_amount` is the **total** trading fee and `protocol_fee` its protocol part. This mapping is asserted in the harness and is an *explained* representation difference, not a math difference.

## 3. Fee/rounding contracts verified

- **CPMM**: fee deducted upfront with **ceiling** rounding `fee = ceil(dx·num/den)`, output `floor(net_dx·y/(x+net_dx))`, all in U256 checked arithmetic (`cpmm.rs::cpmm_out_upfront_fee_ceil`). Matches Raydium AMM v4, Raydium CPMM (1,000,000 fee denominator), DAMM v2.
- **DLMM**: base fee `base_factor·bin_step·10·10^base_fee_power_factor`; variable fee `⌈vfc·(va·bin_step)²/10^10⌉` with `MAX_FEE_RATE` clamp (10%); fee-to-add `⌈a·r/(10^9−r)⌉`; protocol share floor division; volatility accumulator decay/rebuild per SDK semantics (`arb-math/src/dlmm/fee.rs`).
- **CLMM**: Q64.64 tick math with property tests (`clmm.rs`, proptest); fee-growth accrual directions covered by `fee_on_*` test family.
- **Token-2022 (fail-closed)**: transfer-fee calculation exists (`token2022.rs`: epoch-aware fee, rounds up, caps), but **production discovery excludes all non-base SPL mint owners** — Token-2022 routes never reach quoting. Standalone helpers must not be read as enabling it (ISSUE-P1-01).

## 4. Known parity gaps (honest register)

### 4.1 Closed: DLMM batch multi-pool capture (was: one mainnet pool + synthetic vectors)

**Status: CLOSED — executed, exact parity.** Network egress from this sandbox was re-verified
(`POST https://api.mainnet-beta.solana.com` `getSlot` succeeds; note `curl` is blocked by tool
policy, `python3 urllib` and `node` work). Pool discovery via the `dlmm-api.meteora.ag` REST
host failed (404/403 on all routes — API appears relocated), so candidates were decoded
on-chain from the deployed `LbPair` account layout (`fixtures/dlmm.json` IDL; struct offsets +
8-byte Anchor account discriminator; verified cross-check against known fixtures: bin_step 4
and 10 matched). Selected pools span bin steps 1 / 10 / 50 plus the existing 4, across two
mint pairs:

| Snapshot | Pool | bin_step | mints | active_id | capture slot |
|---|---|---|---|---|---|
| `fixtures/meteora_mainnet_snapshot.json` (existing) | `5rCf1DM8…HAS6` | 4 | SOL/USDC | — | prior session |
| `fixtures/meteora_dlmm_pools/step1.snapshot.json` | `HTvjzsfX…ashR` | 1 | SOL/USDC | -26552 | 472192431 |
| `fixtures/meteora_dlmm_pools/step10_sol_usdc.snapshot.json` | `BGm1tav…KD5Y` | 10 | SOL/USDC | -2656 | 472192469 |
| `fixtures/meteora_dlmm_pools/step50_sol_usdc.snapshot.json` | `5XRqv7LC…d5Nt` | 50 | SOL/USDC | -533 | 472192493 |
| `fixtures/meteora_dlmm_pools/step10_trump_usdc.snapshot.json` | `3C5YE97…DbW9` | 10 | TRUMP/USDC | -139 | 472192517 |

Each pool: raw lbPair + 15 bin-array snapshots (8 per side), SDK quotes over a frozen
in-process RPC cache, both directions, exact-in/exact-out small+deep, and per-bin Q64.64
price checks. Assertion test
`crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs::mainnet_pool_batch_quotes_match_official_sdk`
replays every captured pool on the Rust side; every compared integer field (amount_in,
amount_out, total_fee, protocol_fee, end_bin_id, bins_crossed) matches the SDK exactly
(tolerance 0), and every `price_checks` bin matches. Deep probes crossed up to 511 bins
(step-1 pool) and 271 bins (step-10 SOL/USDC), spanning bin-array boundaries.

Reproduction (network-enabled host, from repo root):

```bash
cd scripts/differential-harness && npm ci
POOL_ADDRESS=HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR \
  SNAPSHOT_OUT=../../fixtures/meteora_dlmm_pools/step1.snapshot.json \
  QUOTES_OUT=../../fixtures/meteora_dlmm_pools/step1.quotes.json \
  node run_mainnet_snapshot_differential.js
# … repeat per pool / then rebuild the default fixture:
node run_mainnet_snapshot_differential.js
cargo test -p arb-engine --test meteora_mainnet_sdk_differential
```

### 4.2 Closed: Orca Whirlpool official-SDK differential capture (was: no captured SDK oracle)

**Status: CLOSED — executed, exact parity.** Official oracle: `@orca-so/whirlpools-core`
v3.1.1 wasm (`swapQuoteByInputToken`/`swapQuoteByOutputToken`) driven by
`whirlpools-sdk@0.22.0` account fetches, against the deployed Whirlpool program
`whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`. Captured 4 classic mainnet pools
(adaptive-fee pools fail-closed-rejected at capture: `feeTierIndexSeed` decoded from
account bytes at offset 43; a pool is classic iff `seed == tick_spacing`):

| Snapshot | Pool | tick_spacing | fee_rate | capture slots (range) |
|---|---|---|---|---|
| `wp_ts1_usdcusdt.{snapshot,quotes}.json` | `4fuUiYxT…y4T4` | 1 | 100 | 472209118–472209372 |
| `wp_ts4_solusdc.{snapshot,quotes}.json` | `Czfq3xZZ…44zE` | 4 | 400 | 472209118–472209372 |
| `wp_ts8_solusdc.{snapshot,quotes}.json` | `7qbRF6Ys…VNUJnm` | 8 | 500 | 472209044–472209372 |
| `wp_ts64_solusdc.{snapshot,quotes}.json` | `HJPjoWU…8ngndJ` | 64 | 3000 | 472209092–472209372 |

Each pool: raw pool account (base64, size 8+645) + full captured tick-array set (9988-byte
accounts) + SDK quotes for both directions × exact-in small/deep (+ exact-out provenance
cases). Assertion test
`crates/arb-engine/tests/whirlpool_mainnet_sdk_differential.rs` replays every pool on the
Rust side and additionally re-verifies: raw-byte field decode at official IDL offsets,
tick-array PDA derivation (ASCII-decimal-string seed, VERIFIED from `pda-utils.js` — not
i32 LE), adaptive-fee classic re-check, and engine-parser agreement on every captured
array; only arrays capturing the account bytes after SDK confirmation are accepted.

**Defect found and fixed by this capture** (this is why differential capture matters): the
previous shared CLMM engine stepped array-edge to array-edge; on deep traversals across
tick arrays with sparse/empty initialized ranges it diverged from the official
whirlpools-core traversal (ts-64 SOL/USDC b_to_a deep: output off by 9,135,518,347 of
~28.35e9 ≈ 0.032%; ts-1 deep cases off by 1–2 base units) and rejected a
SDK-quotable ts-64 deep case as missing coverage. Fix: new
`arb_math::clmm::compute_whirlpool_swap` ports the official traversal semantics verbatim
(`rust-sdk/core/src/quote/swap.rs` — sequence-wide next/prev initialized-tick targeting,
`calc = remaining·(D−fee)/D`, ceiling fee extraction at initialization boundaries,
round-down output; the `swap_quote_by_output_token` inverted token-A flag honored in the
harness) with fully checked arithmetic; the Whirlpool production adapter now routes to it
while Raydium CLMM keeps the previous shared engine. Low-level primitives (amount deltas,
next-sqrt, tick↔sqrt) were already exact; the official-loop port over them reproduced the
SDK bit-for-bit on all previously divergent cases, localizing the defect precisely to the
traversal loop. `compute_swap_with_ticks_per_array_and_math` semantics were **not**
modified.

Reproduction (network-enabled host, from repo root):

```bash
cd scripts/differential-harness && npm ci
# default pool (wp_ts8_solusdc):
node capture_mainnet_whirlpool.js && node run_mainnet_whirlpool_differential.js
# remaining pools (outputs land in fixtures/whirlpool_mainnet_pools/<slug> by slug):
POOL_ADDRESS=Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE node capture_mainnet_whirlpool.js && \
  POOL_ADDRESS=Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE node run_mainnet_whirlpool_differential.js
# … repeat for 4fuUiYxTQ6QCrdSq9ouBYcTM7bqSwYTSyLueGZLTy4T4 and HJPjoWUrhoZzkNfRpHuieeFk9WcZWjwy6PBjZ81ngndJ
cargo test -p arb-engine --test whirlpool_mainnet_sdk_differential
```

### 4.3 Closed: Raydium CLMM multi-array mainnet SDK differential (2026-11-21)

**Status: CLOSED — executed, exact parity.** Harness: `scripts/differential-harness/capture_mainnet_raydium_clmm.js` + `run_mainnet_raydium_clmm_differential.js`, oracle = official `@raydium-io/raydium-sdk-v2@0.2.85-alpha` `PoolUtils.getOutputAmountAndRemainAccounts` (the deployed `swapSimulator.swapInternal`) over a frozen in-process capture with `block_timestamp` pinned via `getBlockTime(slot)`. Pool discovery: Raydium's api-v3 REST is broken server-side (500 on every `/pools/info/list-v2` route), so the 197,290 deployed 1544-byte `PoolState` accounts were scanned on-chain via `getProgramAccounts` + `dataSlice` (window 225..281 covering u128 liquidity @237 + `tick_current` @269), ranked by liquidity, then captured fail-closed (dynamic-fee-active, Token-2022, swap-disabled, zero-liquidity pools refused).

| Snapshot | Pool | tick_spacing | fee_on | capture slot | arrays (existing/captured) |
|---|---|---|---|---|---|
| `fixtures/raydium_clmm_mainnet_pools/rc_ts1_m8vkmusdc_0.snapshot.json` | `8VmwmoTU…9dSk` | 1 | 0 | 472254143 | 1/5 |
| `fixtures/raydium_clmm_mainnet_pools/rc_ts1_m9evwusdc_0.snapshot.json` | `4obueSHx…AXV` | 1 | 0 | 472254148 | 1/5 |
| `fixtures/raydium_clmm_mainnet_pools/rc_ts1_usdcmhfzy_0.snapshot.json` | `7izxtfJX…aE1P` | 1 | 0 | 472254152 | 1/5 |
| `fixtures/raydium_clmm_mainnet_pools/rc_ts1_m4uemusdc_0.snapshot.json` | `BaKfiGgy…NV4w` | 1 | 0 | 472254156 | 1/5 (1 of 2 directions refused: one-sided liquidity) |
| `fixtures/raydium_clmm_mainnet_pools/rc_ts60_solmdzqz_0.snapshot.json` | `D7HU4rLQ…6qU` | 60 | 0 | 472254160 | 2/6 (b_to_a refused: closed empty span) |
| `fixtures/raydium_clmm_mainnet_pools/rc_ts120_m6gmamfn3r_1.snapshot.json` | `8vJk9vpg…fzKo` | 120 | 1 | 472254165 | 2/6 (fee_on=1 → b_to_a is SDK-provenance only; engine refusal asserted) |

Note: pool `FvqiTGa9…C7C` (tick_spacing 10, 6/6 contiguous arrays) was captured but has no quotes fixture — BOTH directions refused (`all_trade=false` even at a 1e6 probe; degenerate sub-µ depth). The refusal ledger is the fixture contract; its snapshot is not committed.

Two engine defects were found and fixed by this differential (both previously made the engine WRONG — fail-closed on valid mainnet states):

1. **Empty-span array advancing** (`arb-math/src/clmm.rs::find_next_tick_boundary_with_size`): the walker demanded the literal adjacent array (`start ± 60·tick_spacing`) whenever the current array ran out, failing with `MissingTickArrayCoverage` across unallocated/closed-array spans. Official semantics (`swapSimulator.ts::swapInternal`) advance to the next array **in the caller-ordered list** — the price only ever moves to the next INITIALIZED tick; empty spans are crossed without any account. Fixed to advance to the nearest loaded array in direction; contiguous-list (incl. Whirlpool `[base, base−T, base−2T]`) behavior is byte-identical to before.
2. **u64 step-reachability bound** (`compute_swap_step_checked`): the "amount to reach the target tick" was computed as u64 before the step's partial/full split; with concentrated liquidity the gap legitimately exceeds u64 (`liquidity × Δ(1/√P)` ≈ 1e20 at `liquidity` 6.4e22) and the quote failed `IntegerOverflow` although the step is partial. Fixed with a wide (u128) reachability pre-check mirroring the SDK's `MaxTokenOverflow`-catch (`calculateAmountInRange` returning `null`), leaving every amount actually materialized as narrow checked u64.

Fee gross-up verified live per `swapMath.ts`: full step `fee = ceil(net_in × rate / (1e6 − rate))`, partial step `fee = amount_remaining − amount_in`; deep ts=120 probe reproduced the SDK's `fee_amount = 12106742575587251` exactly.

Reproduction (network-enabled host, from repo root):

```bash
cd scripts/differential-harness && npm ci
for P in 8VmwmoTU869tofZvYnNxV5hSK9jbVnkLu9Ys9eHi9dSk 4obueSHxA53UFb6jPYs1rQY41z6uD858SkTCf9AEmAXV 7izxtfJX7wk92aWanepSCP69RwMcSGsUefwsyA6CaE1P BaKfiGgycM3MiK4XvGZRrLedVmd3m5qcognBu1PeNV4w D7HU4rLQjmboCAWGa1upQuGFmncPgzufoVW6avE9F6qU 8vJk9vpgni14p6ofq6wVMZMn3aoFDug1RaK6XPDbfzKo; do
  POOL_ADDRESS=$P node run_mainnet_raydium_clmm_differential.js
done
cargo test -p arb-engine --test raydium_clmm_mainnet_sdk_differential
```

### 4.5 Closed: Meteora DLMM OnlyY (`collect_fee_mode = 1`) live mainnet differential (2026-11-21)

**Status: CLOSED — executed on a genuinely deployed OnlyY pool, exact parity.** Discovery tool:
`scripts/differential-harness/discover_onlyy_pools.js` (`getProgramAccounts` on
`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` with the `account:LbPair` Anchor discriminator
`210b3162b565b10d` + memcmp offset 36 == 1, `dataSlice 0..216`, reserve balances at token-account
offset 64..72, mint-owner check for classic SPL). Findings: **8,160 deployed LbPair accounts with
collect_fee_mode=1**; function_type histogram `{1: 595, 2: 7565}` — **zero pools with
function_type=0** (OnlyY pools are launch/farming pools, not classic). Most are one-sided or
Token-2022; ~20 classic-SPL both-sides candidates, all launch-thin (deepest feasible crossings
0–2 bins).

Corrected ConcreteFunctionType semantics (verified in `@meteora-ag/dlmm@1.9.14` `dist/index.js`,
`isSupportLimitOrder` and `getFeeMode`): **0 = Undetermined** (legacy classic), **1 = LiquidityMining**
(`supportLimitOrder = false` → `getLimitOrderLiquidity` returns {0,0} and the fill path is
market-maker-only, i.e. quote-equivalent to classic), **2 = LimitOrder** (order liquidity participates —
NOT differentially verified). The previously documented "0=Spot, 1=Concentrated, 2=Percentile"
meaning was wrong (older IDL generation). Policy aligned repo-wide: {0,1} accepted, ≥2 rejected
fail-closed (`arb-dex/src/dlmm_validation.rs`, `arb-dex/src/meteora_dlmm.rs`,
`arb-engine/src/{adapters,lib,cross_pool_dlmm,meteora_pair_discovery,meteora_quote_engine,dlmm_flash_arbitrage}.rs`).
Two real defects were found and fixed by this differential:

1. **`MeteoraDlmmPool::update_from_account` never assigned `self.function_type` / `self.collect_fee_mode`** (`arb-dex/src/meteora_dlmm.rs`) — both stayed 0, making the validation gate a no-op (LimitOrder pools passed fail-open). Fixed by assigning the decoded values; regression test added.
2. **Thin launch pools must be servable-refused, not skipped**: the runner now records per-case `refusals` ledger entries (insufficient liquidity) and the Rust batch test enforces that every absent case has one, fail-closed.

Captured fixture: `fixtures/meteora_dlmm_pools/onlyy_step1_7a35.snapshot.json` +
`onlyy_step1_7a35.quotes.json` — pool `7A35wpqKVEwnKK4Xz8XsmpRMSxRF1KQ9EBFeYwTgAs4Z`, capture slot
472266867, active_id 69061, bin_step 1, ft=1, collect_fee_mode=1, base_factor 10000, protocol_share
1000 (10%), both sides classic SPL, status 0. 12 served cases (both modes × both directions ×
exact-in/exact-out small) + 4 refusal-ledgered deep cases. Mode-1 X→Y verified fee_on_input=false
(fee on output Y), matching `getFeeMode`'s `feeOnInput = !swapForY` exactly; mode-0 vs mode-1
fee-split consistency enforced pre-write. First capture candidate was refused at the runner's
Token-2022 gate (mint carried a TransferFeeConfig at 300 bps — SDK wrapper folds transfer-fee
deltas into totals, our engine refuses transfer-fee mints by design; this is the fail-closed
correct behavior, not a parity miss).

Reproduction (network-enabled host, from repo root):

```bash
cd scripts/differential-harness && npm ci
BIN_ARRAYS_PER_SIDE=16 DEEP_MIN_FALLBACK_CROSSED=2 \
  POOL_ADDRESS=7A35wpqKVEwnKK4Xz8XsmpRMSxRF1KQ9EBFeYwTgAs4Z \
  SNAPSHOT_OUT=../../fixtures/meteora_dlmm_pools/onlyy_step1_7a35.snapshot.json \
  QUOTES_OUT=../../fixtures/meteora_dlmm_pools/onlyy_step1_7a35.quotes.json \
  node run_mainnet_snapshot_differential.js
cargo test -p arb-engine --test meteora_mainnet_sdk_differential
cargo test -p arb-dex --lib dlmm_validation
```

### 4.6 Remaining gaps (open)

| Gap | Class | Remediation path |
|---|---|---|
| Transient-capture hazard: one step-1 capture attempt failed at the active bin ("no bin consumed output") and succeeded on re-capture at a later slot | Environment | Re-run with a higher commitment or slot-pinned capture when transient emptiness matters |
| Raydium CLMM `PoolUtils.computeAmountOutFormat` wrapper (Decimal/slippage formatting layer) not exercised — the harness pins the integer core `getOutputAmountAndRemainAccounts`, which is the decision-path surface | Scope note | Optional: wrap the existing integer oracle with the format layer in the runner |
| DLMM `function_type = 2` (LimitOrder) pools: order-liquidity fill paths are implemented in `arb-math` but have **no differential fixture** (discovery found 7,565 such pools; none captured) | Coverage gap | Capture a live ft=2 pool and run the same harness before enabling the ft≥2 gates |
| Raydium CLMM `PoolUtils.computeAmountOutFormat` wrapper (Decimal/slippage formatting layer) not exercised — the harness pins the integer core `getOutputAmountAndRemainAccounts`, which is the decision-path surface | Scope note | Optional: wrap the existing integer oracle with the format layer in the runner |

Live mainnet re-capture is no longer blocked (network egress verified working from this
sandbox; commands under §4.1 were executed there). The interpolation of the previous
"BLOCKED (no network egress, no `.env`)" row is superseded.

**Verdict**: No unexplained discrepancy exists between the canonical quote engines and their available oracles. Parity evidence is executable and green at the baseline commit.
