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
| `phase15_2_sdk_differential_manifest` | ✅ PASS |
| Full workspace suite | ✅ 1,398 passed / 0 failed / 2 ignored (109 suites) — same run as QUOTE/economic/V1 suites below |

Fee-scope note (SDK parity): the official DLMM client returns `fee` as the LP share and `protocolFee` separately; total trading fee = `fee + protocolFee`. The Rust engine's `fee_amount` is the **total** trading fee and `protocol_fee` its protocol part. This mapping is asserted in the harness and is an *explained* representation difference, not a math difference.

## 3. Fee/rounding contracts verified

- **CPMM**: fee deducted upfront with **ceiling** rounding `fee = ceil(dx·num/den)`, output `floor(net_dx·y/(x+net_dx))`, all in U256 checked arithmetic (`cpmm.rs::cpmm_out_upfront_fee_ceil`). Matches Raydium AMM v4, Raydium CPMM (1,000,000 fee denominator), DAMM v2.
- **DLMM**: base fee `base_factor·bin_step·10·10^base_fee_power_factor`; variable fee `⌈vfc·(va·bin_step)²/10^10⌉` with `MAX_FEE_RATE` clamp (10%); fee-to-add `⌈a·r/(10^9−r)⌉`; protocol share floor division; volatility accumulator decay/rebuild per SDK semantics (`arb-math/src/dlmm/fee.rs`).
- **CLMM**: Q64.64 tick math with property tests (`clmm.rs`, proptest); fee-growth accrual directions covered by `fee_on_*` test family.
- **Token-2022 (fail-closed)**: transfer-fee calculation exists (`token2022.rs`: epoch-aware fee, rounds up, caps), but **production discovery excludes all non-base SPL mint owners** — Token-2022 routes never reach quoting. Standalone helpers must not be read as enabling it (ISSUE-P1-01).

## 4. Known parity gaps (honest register)

| Gap | Class | Remediation path |
|---|---|---|
| CLMM/Whirlpool have no captured official-SDK differential fixture | Coverage gap (not a known defect) | Capture mainnet snapshot + SDK quotes for Raydium CLMM and Orca Whirlpools via a harness analogous to `scripts/differential-harness/` |
| DLMM differential covers one mainnet pool + synthetic oracle vectors | Coverage gap | Batch-capture several pools across bin steps / fee modes |
| Variable-fee volatility state depends on `last_update_timestamp` handling; re-quotes on live snapshots exercise decay paths only in unit tests | Coverage gap | Capture timestamped snapshots; add timestamp-drift vectors |
| Live mainnet re-capture/re-run **BLOCKED in this sandbox** (no network egress, no `.env`) | Environment | On a network-enabled host: `set -a; . ./.env; set +a && node scripts/differential-harness/capture_mainnet_snapshot.js && node scripts/differential-harness/run_mainnet_snapshot_differential.js && cargo test -p arb-engine --test meteora_mainnet_sdk_differential` |

**Verdict**: No unexplained discrepancy exists between the canonical quote engines and their available oracles. Parity evidence is executable and green at the baseline commit.
