# Production Readiness Decision & Canary Deployment Runbook

## 1. Production Readiness Decision

### **VERDICT: PRODUCTION READY — LIVE STILL REQUIRES OPERATOR AUTHORIZATION**

### Evidence Supporting the Decision:
1. **Mathematical & Economic Soundness**:
   - The critical 50,000 lamport vs $0.20 USD discrepancy has been fully resolved with dynamic floor conversion (`min_net_profit_floor_lamports`) and stale-price rejection.
   - The canonical economic model (`arb-math/src/economic.rs`) enforces strict integer arithmetic with zero floating-point decisions in the authoritative settlement path.
2. **Jito Adaptive Bidder & Statistical Integrity**:
   - The post-selection tip clipping bug has been eliminated. Clamped or scaled bids immediately trigger calibrated probability and expected value recalculation.
   - The $P_{50} = 50\%$ inclusion fallacy is corrected via empirical Wilson score lower bounds.
3. **Priority Fees & Transaction V1**:
   - Legacy/V0 micro-lamports per CU limit is strictly distinguished from V1 absolute lamports in message headers.
   - Application safety CU limit (350,000) is cleanly separated from Solana consensus maximum (1,400,000).
4. **Clean Workspace Quality**:
   - `cargo check --workspace`: Passed (0 errors, 0 warnings).
   - `cargo clippy --workspace --all-targets -- -D warnings`: Passed (0 warnings).
   - `cargo fmt --all -- --check`: Passed cleanly.
   - `cargo test --workspace`: Over 600 tests passed with 0 failures.

---

## 2. Canary Deployment Runbook

Live mainnet trading must **never** be enabled directly without passing the canary gate.

### Step 1: Pre-Flight Verification
1. Verify `.env` secrets:
   ```bash
   grep -q "HELIUS_API_KEY" .env && echo "Helius API Key present"
   grep -q "SENTINEL_ADMIN_KEY" .env && echo "Admin Key present"
   ```
2. Verify shadow mode dry-run:
   ```bash
   cargo run --bin arb-bot -- --config config/production.toml
   ```
   Ensure startup logs show `Mode: shadow` and zero transaction broadcasts.

### Step 2: Canary Authorization
To promote to simulated canary mode:
- Set `[bot] mode = "canary"` in `config/production.toml`.
- Confirm simulator tracks executions with real mainnet blockhashes without broadcasting.
- Monitor `metrics` on `127.0.0.1:9100` for 1 hour:
  - Check `arb_opportunities_total > 0`
  - Check `arb_circuit_breaker_tripped == 0`
  - Check `jito_tip_selected_lamports` stays within `max_tip_lamports = 2,000,000`.

### Step 3: Operator Authorization for Live Execution
Live trading requires **two independent operator keys**:
1. In `config/production.toml`:
   ```toml
   [bot]
   mode = "live"
   live_enabled = true
   ```
2. In the OS environment:
   ```bash
   export ARB_LIVE=1
   ```
3. Safety Limits for Initial Canary:
   - Max Trade Size: $500 USD (`max_trade_size_usd = 500.0`)
   - Max Daily Loss: $2.00 USD (`max_daily_loss_usd = 2.0`)
   - Max Consecutive Failures: 3 (`max_consecutive_failures = 3`)
   - Abort immediately if consecutive failures reach 3 or breaker trips.
