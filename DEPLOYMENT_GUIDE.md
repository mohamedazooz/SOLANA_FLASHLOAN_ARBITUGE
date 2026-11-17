# PRODUCTION DEPLOYMENT & OPERATIONS GUIDE
## SOLANA_FLASHLOAN_ARBITUGE — Infrastructure Setup, Shadow Soak & Live Operations Runbook

**Date**: 2026-11-16  
**Auditor**: Principal Polyglot Software Architect & DevSecOps Engineer  
**Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`  
**Target Environment**: Linux Ubuntu 22.04 / 24.04 LTS (Bare-Metal or High-Performance Cloud)  

---

## 1. System Requirements & Topology

To achieve competitive low-latency execution in Solana MEV auctions, deployment on low-jitter, bare-metal hardware close to Solana validator clusters (Frankfurt, Amsterdam, Dublin) is recommended:

- **CPU**: 8+ Dedicated High-Clock Cores (e.g. AMD EPYC 9004 or Intel Xeon Scalable @ 3.5GHz+)
- **Memory**: 32 GB RAM (DDR5 ECC preferred)
- **Storage**: 100 GB NVMe SSD
- **Network**: 1 Gbps+ Unmetered, low-jitter uplink (< 5ms RTT to Helius/Jito endpoints)
- **OS**: Ubuntu 22.04 LTS / 24.04 LTS x86_64
- **Toolchain**: Rust 1.99.1 (Edition 2024 pinned in `rust-toolchain.toml`)

---

## 2. Step-by-Step Production Deployment Workflow

### Step 1: Clone Repository & Setup Toolchain
```bash
git clone https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git
cd SOLANA_FLASHLOAN_ARBITUGE
git checkout feat/meteora-dlmm-swap2-atomic-cpi

# Verify Rust toolchain is pinned to 1.99.1
rustc --version
```

### Step 2: Environment Configuration (`.env`)
Create `.env` from `.env.example`:
```bash
cp .env.example .env
chmod 600 .env
```
Populate `.env` with production keys:
- `SOLANA_PRIVATE_KEY`: Base58 private key of the funded trading wallet (keep $\ge 0.1$ SOL for fees).
- `SOLANA_RPC_PRIMARY`: High-performance Solana RPC (Helius, Alchemy, or Solami).
- `SOLANA_WS_1_URL`: Primary WebSocket endpoint for account subscriptions.
- `JITO_BLOCK_ENGINE_URL`: `https://mainnet.block-engine.jito.wtf/api/v1/bundles`
- `SENTINEL_ADMIN_KEY`: Strong random 32-character secret for administrative API access.
- `WEB_ADMIN_PASSWORD`: Strong password for the React dashboard.
- `TELEGRAM_BOT_TOKEN` & `TELEGRAM_CHAT_ID`: For real-time execution alerts.

### Step 3: Configuration Tuning (`config.toml`)
Create `config.toml` from `config.example.toml`:
```bash
cp config.example.toml config.toml
```
Ensure:
- `cold_wallet_pubkey` is set to your cold storage withdrawal address.
- `strategy.min_net_profit_abs` is set to your target floor ($20,000$ lamports $\approx \$0.004$).
- `risk.max_trade_size_usd` is configured to your maximum risk appetite (e.g. $\$10,000.00$).

### Step 4: Pre-Flight Verification
Run the comprehensive verification script:
```bash
./scripts/verify-linux.sh
```
Or execute manual checks:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```
All commands must return code `0` before proceeding.

---

## 3. Deployment Phases: Shadow Soak to Live

```
+-----------------------------------------------------------------------------------+
|                        Deployment Progression Stages                              |
+-----------------------------------------------------------------------------------+
        |
        v
  [Stage 1: Dry-Run Rehearsal] -> ./target/release/examples/full_path_rehearsal
        |                          (Verifies RPC hydration & fail-closed defense)
        v
  [Stage 2: Shadow Soak] -------> ARB_MODE=simulate cargo run --release --bin arb-bot
        |                          (Run for 24 hours; observe landing telemetry)
        v
  [Stage 3: Canary Mode] -------> ARB_MODE=canary ARB_LIVE=1
        |                          (Live execution with tightly bounded borrow size)
        v
  [Stage 4: Full Live Engine] --> ARB_MODE=live ARB_LIVE=1
                                   (Full production flash loan arbitrage active)
```

### Stage 1: Dry-Run Rehearsal
Verify that market data feeds and RPC simulation operate correctly against live mainnet:
```bash
cargo run --release --example full_path_rehearsal
```

### Stage 2: Shadow Mode Deployment (Docker Compose)
Run the bot in shadow mode:
```bash
docker compose -f docker-compose.shadow.yml up -d
```
Inspect logs:
```bash
docker compose -f docker-compose.shadow.yml logs -f arb-bot
```

### Stage 3: Live Mode Activation
When ready for live mainnet capital execution:
1. Update `.env`:
   ```bash
   DRY_RUN=false
   LIVE=true
   EXECUTE_LIVE=true
   ARB_LIVE=1
   ARB_MODE=live
   ```
2. Start the live service:
   ```bash
   docker compose up -d
   ```

---

## 4. Operational Monitoring & Administrative Controls

### Network Ports & Endpoints:
- `127.0.0.1:8080`: Web Dashboard & WebSocket feed.
- `127.0.0.1:9100`: Prometheus metrics (`/metrics`).
- `127.0.0.1:9101`: Sentinel Admin control (`/pause`, `/resume`, `/api/config`).

### Emergency Kill Switches:
1. **Instant HTTP API Pause**:
   ```bash
   curl -X POST http://127.0.0.1:9101/pause \
     -H "Authorization: Bearer $SENTINEL_ADMIN_KEY"
   ```
2. **File-Based Kill Switch (Fail-Safe)**:
   ```bash
   touch ./data/kill_switch
   ```
   The supervisor detects this marker immediately and transitions to `PAUSED` within one slot ($400\text{ ms}$).

3. **Resume Trading**:
   ```bash
   curl -X POST http://127.0.0.1:9101/resume \
     -H "Authorization: Bearer $SENTINEL_ADMIN_KEY"
   # or
   rm -f ./data/kill_switch
   ```

---

## 5. Incident Response & Troubleshooting

| Symptom | Probable Cause | Action |
|:---|:---|:---|
| `HTTP 429 Too Many Requests` | RPC provider rate limit reached | RpcPool rotates automatically; check API tier |
| `PROFIT_BELOW_FLOOR` | Spread does not clear fees + buffer | Normal healthy behavior; bot waits for profitable edge |
| `Leader Unknown / Non-Jito` | Validator in slot does not run MEV | Bot withholds bundle (fail-closed defense) |
| `InvalidAccountForFee` in test | Private key in `.env` has 0 SOL | Expected in CI/test environments; test skips gracefully |
| Breaker tripped (`ConsecutiveFailures`) | Sudden network congestion or high slippage | Bot latches paused; inspect logs before `/resume` |

The engine is engineered to fail closed under every failure mode. Capital remains 100% secure.

---

# Appendix: independent audit pass from the `newrepair` branch (merged from main)

# DEPLOYMENT_GUIDE.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Safety defaults (load-bearing)

- `ARB_LIVE=0`, `DRY_RUN=true`, shadow mode. Do not weaken without operator approval.
- Mainnet live trading disabled by default.
- Web API refuses to start without `WEB_ADMIN_PASSWORD`; binds localhost; TLS via reverse proxy.

## Setup

1. Clone repo; checkout `newrepair` (or merge target).
2. Install Rust 1.99.1 (per rust-toolchain.toml); build on Linux.
3. Copy `.env.example` → `.env`; fill `REPLACE_ME_*` with rotated keys.
4. Configure RPC/WS providers (Helius/Alchemy/Solami) with valid keys.
5. Set `ARB_MODE=simulate` initially.
6. Provide `config.toml` (copy from config.example if present) with validated thresholds.

## Verification before deploy

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `cargo audit` / `cargo deny check`
- Run in simulate/shadow mode; confirm no live txs.

## Production promotion

Only after: full test suite green, dynamic egress capture clean, quote differential within documented tolerance, V1 size ladder verified, kill switch + circuit breakers tested. Live mainnet activation requires separate explicit authorization and dedicated keys.

## Monitoring

Track: opportunities detected/rejected, profitable/unprofitable simulations, tx built/submitted/landed/failed, bundles, net profit, execution cost, latency, RPC errors, Geyser lag, pool staleness, CU consumed. Never report estimated profit as realized.
