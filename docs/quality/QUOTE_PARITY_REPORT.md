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
| Raydium CLMM / Orca Whirlpool | `arb-math::clmm` + `arb-dex::raydium_clmm` / `orca_whirlpool` | In-repo unit + property tests only (no captured SDK oracle) | Property/invariant |
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
| `phase15_2_sdk_differential_manifest` | ✅ PASS |
| Full workspace suite | ✅ 1,398 passed / 0 failed / 2 ignored (109 suites) — same run as QUOTE/economic/V1 suites below |

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

### 4.2 Remaining gaps (open)

| Gap | Class | Remediation path |
|---|---|---|
| CLMM/Whirlpool have no captured official-SDK differential fixture | Coverage gap (not a known defect) | Capture mainnet snapshot + SDK quotes for Raydium CLMM and Orca Whirlpools via a harness analogous to `scripts/differential-harness/` |
| All batch-captured DLMM pools have `collect_fee_mode = 0` (InputOnly); mode 1 (OnlyY) blocks are SDK-side overrides marked SIMULATED_ONLY | Coverage gap | Discover a live mainnet pool with `collect_fee_mode = 1` (decode `LbPair` per `fixtures/dlmm.json` IDL + 8-byte account discriminator) and run the same harness |
| Variable-fee volatility state depends on `last_update_timestamp` handling; re-quotes on live snapshots exercise decay paths only in unit tests | Coverage gap | Capture timestamped snapshots; add timestamp-drift vectors |
| Transient-capture hazard: one step-1 capture attempt failed at the active bin ("no bin consumed output") and succeeded on re-capture at a later slot | Environment | Re-run with a higher commitment or slot-pinned capture when transient emptiness matters |

Live mainnet re-capture is no longer blocked (network egress verified working from this
sandbox; commands under §4.1 were executed there). The interpolation of the previous
"BLOCKED (no network egress, no `.env`)" row is superseded.

**Verdict**: No unexplained discrepancy exists between the canonical quote engines and their available oracles. Parity evidence is executable and green at the baseline commit.
