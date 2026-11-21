# SHADOW_EXECUTION_REPORT — Shadow Mode Status

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Report date**: 2026-11-21

---

## 1. Safety posture (unchanged, load-bearing)

- `ARB_LIVE=0`
- `DRY_RUN=true`
- Shadow-mode defaults untouched in this mission; no path in this session's
  changes enables broadcasting, fund movement, or live trading.

## 2. What this session could verify (in sandbox)

| Evidence class | Status |
|---|---|
| At-commit build/format/lint | ✅ `cargo fmt --check`, `cargo check`, `cargo clippy -D warnings` all PASS |
| At-commit test suite | ✅ 1,398 passed / 0 failed / 2 ignored (109 suites) |
| Quote parity (offline oracles) | ✅ see [QUOTE_PARITY_REPORT.md](QUOTE_PARITY_REPORT.md) |
| Per-server validation gates (fail-closed rejections, stale-state rejection, insufficient-liquidity rejection, unprofitable rejection) | ✅ matrix tests green |
| Shadow-mode runtime with live providers | ❌ **BLOCKED** — sandbox has no network egress and no `.env` |
| Non-broadcast `simulateTransaction` against mainnet | ❌ **BLOCKED** — same cause |

## 3. BLOCKED items with exact reproducible commands

Run on a network-capable host with a funded-operator `.env` (Helius/RPCFast
keys, etc.):

```bash
# 1. Restore the operator environment (never commit .env):
set -a; . ./.env; set +a

# 2. Live-snapshot capture and official-SDK quote differential:
node scripts/differential-harness/capture_mainnet_snapshot.js
node scripts/differential-harness/run_mainnet_snapshot_differential.js
cargo test -p arb-engine --test meteora_mainnet_sdk_differential

# 3. Full workspace gate on the networked host:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

# 4. 24 hour Shadow soak (existing orchestration):
./run-shadow.sh
# ... observe not later than 24h later, then:
cat reports/daily/*latest*.json
```

The Docker path (`docker-compose.shadow.yml`) is the isolation-compatible
alternative.

## 4. Shadow soak gate criteria (unchanged, restated)

The soak counts as evidence only when, under `ARB_LIVE=0`:
1. Providers hold multi-hour subscriptions with measured reconnects.
2. Opportunities are discovered, sized, simulated, and **rejected or shadow-
   recorded** with reason codes (never dispatched).
3. Local quote vs simulation-output deltas are recorded per candidate.
4. No transport ambiguity: all simulated records carry slot + context.
5. Realized PnL remains exactly zero (nothing broadcast) and the estimated
   buckets are distinguishable in `reports/` + metrics.

Nothing in this report may be read as such evidence: **no soak has run at this
commit**. Historical shadow runs (see `docs/active/CURRENT_STATUS.md`) predate
the current commit and are recorded as capabilities, not as current proof.

## 5. Missing provider coverage / unavailable historical evidence

- Yellowstone gRPC path: dependency present, feature-gated, client integration
  pending (Phase 4 in `docs/active/MASTER_ROADMAP.md`).
- Phase 5 WS-vs-gRPC comparative latency benchmark: not run.
- Historical `0x1773` diagnostics: preserved under `reports/phase17_cross_dex/`
  with their caveats; the same-snapshot replay remains open (ISSUE-P0-04).
