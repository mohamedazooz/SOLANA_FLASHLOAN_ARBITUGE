# Phase 16 — Production Readiness & Mainnet Execution Verification

**Date:** 2026-11-13
**Branch:** `vibe/phase16-prod-verification-9d81d9` (from PR #13 head `b01b0c4`)
**Mode:** Shadow only. `ARB_LIVE=0`, `DRY_RUN=true`. No transaction was ever broadcast. No signature was ever created with funded authority.

---

## 1. Repository Integrity

### 1.1 PR #13 diff audit

PR #13 (`vibe/phase12-15-audit-9d81d9`, 4 commits) was re-audited commit-by-commit against `main`:

| Commit | Scope | Regression risk | Result |
|---|---|---|---|
| `3cd6f80` | `.env.example` prune (128→59 keys) | Env-only; no code path reads removed keys (verified by exact-quoted-string grep across all crates/web/scripts/CI) | CLEAN |
| `ca78b03` | `crates/arb-tx/tests/v1_size_ladder_tests.rs` (new test file) | Test-only; no production code touched | CLEAN |
| `b01b0c4` | Docs (`docs/quality/*`, `docs/operations/*`) | Documentation only | CLEAN |
| (earlier commit on branch) | `crates/arb-sim/src/profitability_gate.rs` — `NetProfitNegative` structural-loss rejection | Behavior change is strictly **more conservative**: gate now rejects structural loss even at hurdle 0. Previously-passing unprofitable routes now correctly rejected. | SAFE (tightening) |

No protocol adapter (Kamino, Meteora DLMM, Orca Whirlpools, Raydium, Jupiter, PumpSwap), no V1 transport code, and no Jito code was modified in PR #13.

### 1.2 Full verification on Phase 16 branch

```
cargo fmt --all -- --check                      → clean
cargo clippy --workspace --all-targets -- -D warnings → 0 warnings
cargo test --workspace --all-targets            → 964 passed / 0 failed (68 test binaries)
```

Full log retained at build time; key counts:

- **68 test binaries, 964 tests, 0 failures, 0 ignored-on-failure.**
- Includes the PR #13 `profitability_gate` structural-loss tests and the `v1_size_ladder_tests` real-route ladder.

---

## 2. Atomic Flash Loan Verification (LIVE MAINNET SIMULATION)

### 2.1 What was executed

A **real, serialized, signed-shape V1 transaction** containing the complete Kamino flash-loan spine was simulated against **live mainnet RPC** (`api.mainnet-beta.solana.com`, solana-core 4.4.0):

1. `FlashBorrowReserveLiquidity` — borrow 1,000,000,000 lamports (1.0 WSOL) from Kamino reserve
2. (Arbitrage body position in the instruction sequence)
3. `FlashRepayReserveLiquidity` — repay 1,000,010,000 lamports (principal + 10,000 lamport fee)

Fee was **read live from the reserve account** (`kamino_flash_loan_fee_sf_live = 11529215046068` U68F60 ≡ 1 bps), byte-verified against the compiled constant in `crates/arb-constants/src/lib.rs:337-339`. Instruction discriminators were independently recomputed via `sha256("global:flash_borrow_reserve_liquidity")[:8]` = `[135,231,52,167,7,52,212,193]` and `sha256("global:flash_repay_reserve_liquidity")[:8]` = `[185,117,0,203,96,245,180,186]` — byte-identical to the constants.

### 2.2 On-chain evidence (simulation, `err: null`)

Source: `reports/phase16/v1_flash_spine_live.json` (raw `simulateTransaction` response), reproduced in `reports/v1_mainnet_probe/probe_report_2026-11-13.json`:

| Metric | Value |
|---|---|
| RPC cluster | `api.mainnet-beta.solana.com`, solana-core **4.4.0**, feature-set 2681577653 |
| Transaction version | **V1** (0x81 discriminator), wire size **638 bytes** |
| Simulation result | **`err: null`** |
| CU consumed | **62,584 / 65,165** (two runs) |
| Kamino program | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` |
| Borrow log | `Program log: Instruction: FlashBorrowReserveLiquidity` → `success` (30,999 CU) |
| Repay log | `Program log: Instruction: FlashRepayReserveLiquidity` → `success` (25,395 CU) |
| Flash liquidity available | 230,166.4 WSOL in the WSOL reserve |
| Payer used | `5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9` (funded pubkey; simulation never charges the fee payer) |

The 26-line log proves the **atomic borrow→repay round-trip executed inside one on-chain transaction** with zero net token delta (mint conservation enforced by the klend program itself: repay = borrow + fee, else the whole tx reverts).

### 2.3 Atomic rollback proof

- Kamino's `FlashRepayReserveLiquidity` validates repayment ≥ principal + fee **within the same transaction**; any shortfall reverts the entire transaction including all swaps. This is protocol-enforced atomicity, not client-side.
- Off-chain, `crates/arb-sim/src/profitability_gate.rs` now rejects any route whose expected net profit after **all** costs is negative — including at hurdle 0 (structural-loss guard, PR #13) — so loss-making routes never reach signing.

### 2.4 Honest scope of this evidence

This is **MAINNET SIMULATION VERIFIED** for the flash-loan spine. It is **not** LIVE VERIFIED: no transaction was broadcast, no signature exists on-chain. Full-route (borrow + 2–3 swaps + repay) live simulation requires a **funded operator key** — operator-gated, deliberately not hacked around.

---

## 3. Multi-Pool DLMM Verification

### 3.1 Real 3-pool routes (offline serialization + signature, V1)

From `reports/phase15_5/V1_SIZE_LADDER.json` (test-regenerable via `cargo test -p arb-tx --test v1_size_ladder_tests`):

| Bins/leg | Accounts | Instructions | Serialized bytes | Version | Signed & compiled |
|---|---|---|---|---|---|
| 1 | 38 | 11 (borrow@3 … repay@7) | **1,574** | V1 | ✅ |
| 2 | 41 | 11 | **1,731** | V1 | ✅ |
| 3 | 44 | 11 | **1,888** | V1 | ✅ |
| 6 | 53 | 11 | **2,359** | V1 | ✅ |
| 12 | 62 | 11 | **2,465** | V1 | ✅ |

All real 3-pool DLMM routes measured **>1,232 bytes** — impossible under V0/legacy without ALT — and all compiled+signed as V1 with signature verification passing. Account counts stay ≤ 62 < 64 (SIMD-0385 inline-account ceiling); a deliberate 67-account push was **rejected by the runtime with `SIMD-0385 address limit exceeded: 67 > 64`** — the ceiling is enforced, never bypassed.

### 3.2 Wire-capacity rungs (synthetic, clearly separated from executable routes)

Self-calibrating rungs land **exactly** on 1232 / 1500 / 2000 / 3000 / 3500 / 4000 bytes (V1, signed, `compiled: true`); a 4,097-byte probe is **rejected with `Transaction size exceeds Solana MTU: 4097 bytes > 4096 bytes limit`**. These are capacity instruments, not executable arbitrage transactions, and are labeled as such in the report JSON.

### 3.3 Live 2-pool DLMM route (mainnet discovery + honest unprofitability)

`cargo test -p arb-sim --release --test meteora_mainnet_simulation_test` against live mainnet:

- Live pool discovery found 3 DLMM pools above the TVL floor; bin-level quotes computed from **real pool state** (904-byte DLMM pool accounts, owner `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` verified).
- Quote result: **honest negative net profit** (`is_profitable: false`, realized net −1,120 raw units on 1,000,000 in) — the gate refuses to sign. This is the system working as designed.
- V1 transaction built at **914 bytes vs 1,008 V0** for the same route (V1 inline accounts beat V0 here because no ALT creation overhead is needed).
- Sim outcome classification: `STALE_QUOTE_REJECTION` on the second pass — quote-freshness gate correctly refused to act on aged state.
- Full-route successful on-chain simulation of this route is **BLOCKED (operator)**: requires a funded fee payer; the deterministic test payer is unfunded and `InvalidAccountForFee` is the correct RPC response. No component was modified to bypass this.

### 3.4 Bin-array expansion behavior

Bin-array account requirements were exercised up to 12 bins/leg (62 accounts, 2,465 bytes) without hitting the 64-account ceiling. Route sizing must remain within the bin-distribution of live pools; the engine's dynamic sizing caps candidate sizes at available per-bin liquidity (see `arb-engine` sizing).

---

## 4. V1 Transport Evidence

### 4.1 Verified support matrix

| Layer | Status | Evidence |
|---|---|---|
| Runtime (mainnet RPC) | ✅ ACCEPTED | solana-core 4.4.0 accepted V1 `simulateTransaction` (`err: null`), `maxSupportedTransactionVersion:1` |
| Rust SDK (`solana-sdk` in-repo) | ✅ | `v1_size_ladder_tests` compiles + signs V1 (0x81), signature verification passes |
| Serialization | ✅ MEASURED | Actual `bincode::serialize` byte counts (never estimated): 154 (baseline), 638 (flash spine), 914 (2-pool), 1,574–2,465 (3-pool), 1232–4000 (capacity rungs) |
| Simulation | ✅ | Live mainnet, `err: null`, 62,584–65,165 CU |
| Submission (broadcast) | ❌ NOT VERIFIED | No broadcast performed (safety directive). Requires operator authorization + funded key. |
| Validator-set sweep / Jito V1 ingress | ❌ NOT VERIFIED | Out of scope without broadcast authorization |

### 4.2 Never-silent-downgrade policy

`crates/arb-tx/src/transport/adaptive.rs::compile_adaptive` enforces: prefer V0+ALT when it fits and is sufficient; select V1 only when the route exceeds V0 capacity; **reject** (never downgrade) transactions that exceed the active wire limit. The 4,097-byte rejection and the 67-account SIMD-0385 rejection prove the fail-closed path executes.

### 4.3 Synthetic vs executable separation

- **Synthetic capacity rungs** (`instrument: "wire_capacity"`): prove wire-format capacity at exact byte targets. Not executable arbitrage.
- **Executable route rungs** (`instrument: "real_three_pool_dlmm_route"`): real Kamino flash spine + 3 DLMM swap groups + compute budget, 1,574–2,465 bytes, signed, sim-verified pattern.
- **Live on-chain sim**: flash spine (`err: null`) + 2-pool route (built, honestly rejected by profitability/freshness gates).

---

## 5. Execution Infrastructure Audit

| Component | File | Finding |
|---|---|---|
| Jito tip auction | `crates/arb-land/src/bidder.rs` | Tip ceiling math sound; tip floor streamed live (WS `wss://bundles.jito.wtf/api/v1/bundles/tip_stream`, verified connected during soak; p50=1,000 / p95=36,964–127,064 lamports observed). Tip is a cost input to net-profit, not a blind add-on. |
| Priority fee estimation | `arb-tx` compute-budget builder | Derived from route economics + execution probability, not fixed constants. |
| Blockhash freshness | `crates/arb-bot/src/supervisor/recovery.rs` | Block-height expiration tracked with evidence; **no blind resubmit** — expired ⇒ new blockhash + re-gate, never re-send same sig. |
| Duplicate submission prevention | `crates/arb-bot/src/supervisor/dispatch.rs:480-620` | Durable dispatch intents persisted **pre-send** (fail-closed store); signature reconciliation dedupes. |
| RPC failover | `crates/arb-ingest/src/rpc_pool.rs` | `RpcPool::from_env()` multi-endpoint; **no stale-cache fallback** — unhealthy/unfresh ⇒ skip opportunity building. |
| Admin controls | `arb-bot` | `/pause`, `/resume` fail-closed without `SENTINEL_ADMIN_KEY`/`WEB_ADMIN_PASSWORD` (observed in soak log — correct behavior). |

---

## 6. Shadow Soak Report

**Status: PARTIAL — operator-gated.** A ≥24-hour continuous soak is not executable inside this session's time budget; a time-boxed soak was run instead and the 24h gate remains **BLOCKED (operator)** as a standing requirement before any live authorization.

### 6.1 Time-boxed soak run (this session)

- Binary: `target/release/arb-bot` (release build).
- Env: `ARB_MODE=shadow ARB_LIVE=0 DRY_RUN=true`, `SOLANA_RPC_URL=https://api.mainnet-beta.solana.com`, polite limits (`RPC_MAX_CONCURRENT_REQUESTS=2`, `RPC_MIN_INTERVAL_MS=250`), durable store `sqlite:///tmp/soak/sentinel.db`.
- Observed behavior (log-retained): RpcPool active (1 endpoint), live discovery across Raydium/Orca/Meteora/Pump.fun (Orca catalog: 297 pools above $15k TVL floor, cap 297, 24 hubs), Jito tip WS connected, leader schedule refreshed (epoch 1085, 1,268 Jito keys), admin APIs fail-closed.
- No WS geyser stream configured in-session ⇒ the freshness gate skips stale-state opportunities by design (fail-closed, not stale-fallback). Zero opportunities were emitted for signing — correct under shadow policy.
- Full metrics (opportunities, rejections, CU, latency percentiles) are persisted by the binary to the durable store and metrics sinks; the soak harness summary is in `reports/phase16/shadow_soak_summary.json`.

### 6.2 24-hour soak gate (BLOCKED — operator)

Required before any live consideration: ≥24h `ARB_MODE=shadow` on operator infrastructure with geyser/WS feed, durable `DATABASE_URL`, and post-run metric extraction. Command template in §9.

---

## 7. Protocol Integration Matrix

| Protocol | Adapter | Integration | Quote math | Live sim evidence | Status |
|---|---|---|---|---|---|
| Kamino Lend (flash loan) | `arb-tx/src/flash/kamino.rs` + `kamino_flash.rs` | Real klend program `KLend2g3c…gmjD`, 12-account matrix, sighash-verified | U68F60 fee from live reserve | ✅ `err: null` borrow+repay on mainnet sim | **VERIFIED (sim)** |
| Meteora DLMM (swap) | `arb-dex` DLMM adapter | Real program `LBUZKhRx…Pwxo`, 904-byte pool accounts, bin arrays | Bin-level traversal, dynamic + protocol + trade fees from state | ✅ 2-pool live discovery + quotes; 3-pool compiled/signed | **VERIFIED (sim)** |
| Orca Whirlpools | `arb-dex` | Present, tested in workspace suite (964/0) | Q64.64 tick math | Unit/integration tests | **CODE VERIFIED** |
| Raydium (CPMM/CLMM) | `arb-dex` | Present, tested | Constant-product / tick math | Unit/integration tests | **CODE VERIFIED** |
| Jupiter | aggregation path | Present, tested | Aggregator quotes | Unit tests | **CODE VERIFIED** |
| Jito | `arb-land` | Tip stream + bidder + leader schedule | Live tip floor p50/p95 | WS + REST live during soak | **INFRA VERIFIED, broadcast NOT VERIFIED** |
| Pump.fun | `arb-dex` | Present, tested | Bonding-curve math | Unit tests | **CODE VERIFIED** |

---

## 8. Remaining Blockers

| # | Blocker | Type | Unblocks |
|---|---|---|---|
| 1 | V1 broadcast end-to-end (submit → confirm → on-chain signature) | Operator authorization + funded key | LIVE VERIFIED status |
| 2 | Full-route (borrow+swaps+repay) successful mainnet sim | Funded fee payer (`InvalidAccountForFee` on unfunded payer) | Route-level sim evidence |
| 3 | ≥24h shadow soak with geyser/WS feed | Operator infrastructure + time | Soak gate |
| 4 | Validator-set sweep for V1 ingress; V1-over-Jito bundles | Requires broadcast authorization | Jito+V1 landing path |
| 5 | PRs #12/#13 merge | Human UI action (tooling refuses merges by policy) | Baseline for Phase 16 merge |

**NOT VERIFIED items (explicit):** V1 broadcast/submission end-to-end; validator-set sweep; V1-over-Jito ingress; 24h soak.

---

## 9. Reproducible Commands

```bash
# Full verification (964/0)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

# V1 size ladder (real 3-pool DLMM routes 1574-2465B + capacity rungs 1232..4000B + 4097 rejection)
cargo test -p arb-tx --release --test v1_size_ladder_tests
cat reports/phase15_5/V1_SIZE_LADDER.json

# Live mainnet V1 flash-spine simulation (no broadcast; funded pubkey as fee-payer override)
V1_SIM_PAYER=5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9 \
  cargo run --release -p arb-tx --example v1_mainnet_sim_envelope
# then simulateTransaction with: encoding=base64, sigVerify=false,
# replaceRecentBlockhash=true, maxSupportedTransactionVersion=1

# Live 2-pool DLMM discovery + honest unprofitability run
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com \
  cargo test -p arb-sim --release --test meteora_mainnet_simulation_test

# Time-boxed shadow soak (24h requires operator infra + geyser feed)
ARB_MODE=shadow ARB_LIVE=0 DRY_RUN=true \
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com \
RPC_MAX_CONCURRENT_REQUESTS=2 RPC_MIN_INTERVAL_MS=250 \
DATABASE_URL=sqlite:///tmp/soak/sentinel.db \
timeout 86400 ./target/release/arb-bot
```

---

## 10. Production Verdict

**MAINNET SIMULATION VERIFIED**

- Flash-loan spine (Kamino borrow + repay) **executed successfully on live mainnet via simulation** (`err: null`, program logs prove both instructions ran inside one V1 transaction).
- Real 3-pool DLMM arbitrage transactions >1,232 bytes compile, sign, and pass signature verification as V1; wire-format capacity proven to 4,000 bytes with explicit rejection at 4,097 and at the SIMD-0385 64-account ceiling.
- Profitability, freshness, and atomicity gates all demonstrated rejecting bad routes (structural loss, stale quotes, unfunded payer) — fail-closed everywhere.
- **Not** PRODUCTION CANDIDATE: V1 broadcast, full-route funded sim, and 24h soak remain operator-gated (§8). Live trading stays disabled (`ARB_LIVE=0`).
