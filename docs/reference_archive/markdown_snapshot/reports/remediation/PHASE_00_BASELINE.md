# Phase Zero Baseline — Forensic Snapshot & Architectural Investigation

## 1. Forensic Snapshot

- **Date / Timestamp**: 2026-11-16T08:20:00Z
- **Active Branch**: `fix/production-economic-engine-hardening`
- **Tracked Upstream**: `origin/main`
- **Base Commit SHA**: `fd5e337` (`Merge pull request #41 from mohamedazooz/vibe/dead-code-cleanup-b3cc62`)
- **Remote Origin**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git`
- **Working Tree State**: Clean. Zero uncommitted tracked modifications.
- **Ignored / Stashed Artifacts**:
  - `stash@{0}`: `On feat/enhanced-telegram-and-landing-telemetry: wip-uncommitted-before-fix-branch` (preserved intact).
  - Runtime untracked/ignored: `.env`, `.kilo/`, `data/`, `logs/`, `target/`.
- **Rust Toolchain**:
  - `rustc 1.99.1 (58cf8f9c3 2026-10-13)`
  - `cargo 1.99.1 (5f94df478 2026-08-27)`
  - Target: `x86_64-pc-windows-msvc` / Linux cross-compatible (Yellowstone gRPC dependency aware).

### Baseline Validation Results
1. `cargo check --workspace`:
   - Status: **PASSED (0 errors, 0 warnings)**
2. `cargo clippy --workspace --all-targets -- -D warnings`:
   - Status: **PASSED (0 warnings)**
3. `cargo fmt --all -- --check`:
   - Status: **PASSED (0 formatting diffs)**
4. `cargo test --workspace --lib`:
   - Status: **PASSED (603 lib unit tests passed across all 14 crates, 0 failed)**

---

## 2. Full Architectural Investigation

### 2.1 The Economic and Execution Lifecycle
The complete opportunity lifecycle traces through 16 distinct stages:

```
[1. Market Ingestion] (Yellowstone gRPC / Geyser WebSocket / Account Subscribe)
        ↓
[2. Pool State Validation] (Reserve freshness, slot lag <= max_state_age_slots, mint decimal decode)
        ↓
[3. Opportunity Generation] (Cross-DEX cycle discovery: Raydium V4/CPMM/CLMM, Meteora DLMM/DAMM, Orca Whirlpool, Pump.fun)
        ↓
[4. Route Simulation & Sizing] (Exact AMM math: constant product, concentrated liquidity, bin arrays)
        ↓
[5. Flash Loan Cost Basis] (Kamino Reserve U68F60 sf calculation: borrow * sf / 2^60, rounding ties away)
        ↓
[6. Base Transaction Fee] (5,000 lamports standard signature fee)
        ↓
[7. Compute Budget Sizing] (Protocol-aware per-hop CU + ephemeral ATA overhead + 15% safety buffer)
        ↓
[8. Priority Fee Calculation] (V0: micro-lamports/CU * limit; V1: absolute total in transaction message header)
        ↓
[9. Jito Tip Estimation] (Adaptive Bidder: hurdle, ceiling t_max, published percentile rungs, empirical Wilson bounds)
        ↓
[10. Profitability Gate] (Strict invariant: Realized Output - Borrowed - Flash Fee - Base Fee - Priority Fee - Tip >= Min Profit)
        ↓
[11. Risk Gate & Sizing Limits] (Max trade size USD, consecutive loss limits, cooldown slots, circuit breaker)
        ↓
[12. Transaction Compilation] (V0 with ALT lookup table chunking vs V1 0x81 discriminator, 4,096-byte wire)
        ↓
[13. Bundle / Transport Submission] (Jito Block Engine multi-region bundle submission vs SWQoS direct RPC fallback)
        ↓
[14. Landing Verification] (Inflight bundle status polling, signature status lookup, slot progression check)
        ↓
[15. Settlement & Reconciliation] (Actual balance delta verification, fee deduction reconciliation)
        ↓
[16. Retry & Recovery Loop] (Idempotent retry with 1.6x ladder cap, ambiguity recovery, expiry verification)
```

---

## 3. Discovered Architectural Inconsistencies & Production Blockers

1. **Critical USD/Lamport Conversion Defect**:
   - `crates/arb-bot/src/supervisor/opportunity.rs` claims `P_net >= $0.20 USD` but references `min_net_profit_abs = 50_000` lamports.
   - At SOL = $200, 50,000 lamports is exactly **$0.01 USD**, not $0.20 USD. A true $0.20 floor requires 1,000,000 lamports.
   - `min_net_profit_floor_lamports` was implemented in `crates/arb-bot/src/config.rs` but is bypassed in `from_arb_config`.
2. **Post-Selection Tip Clipping**:
   - In `crates/arb-land/src/bidder.rs`: when `winner.tip` is capped by `ladder_growth`, `winner.inclusion_p` is NOT recomputed. It keeps the high-rung probability while bidding a lower tip.
   - In `crates/arb-bot/src/supervisor/opportunity.rs`: `tip.as_u64().min(t_max)` clamps the chosen tip downwards without re-verifying that the clipped tip still yields positive EV and acceptable landing probability.
3. **Statistical Rung Probability Fallacy**:
   - `Percentile::probability` hardcodes `P50 = 0.50` through `P99 = 0.99`. Historical tip percentiles represent the distribution of landed tips across all Solana MEV, not the conditional probability of landing a specific arbitrage transaction.
4. **Configuration & Default Drift**:
   - `DEFAULT_KEEP_SHARE` is 0.85 in `arb-constants`, but 0.60 in `base.toml`.
   - `DEFAULT_MAX_TIP_SHARE_BPS` is 8,500 in `arb-constants`, but 7,000 in `base.toml`.
   - `DEFAULT_MIN_KEEP_LAMPORTS` is 50,000 in `arb-constants`, but 10,000 in `base.toml`.
5. **Hardcoded Application Limits**:
   - `MAX_COMPUTE_UNIT_LIMIT = 350_000` is labeled as an application policy, but can be conflated with Solana runtime limits (1,400,000 CU).
   - Hardcoded `gross >= 50_000` checks in `supervisor/opportunity.rs` bypass configured thresholds.
