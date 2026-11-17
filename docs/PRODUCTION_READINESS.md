# PRODUCTION READINESS & LAUNCH CHECKLIST

## 1. Production Scope: Meteora DLMM

The initial hardened production scope is restricted to:
- **Lending Venue**: Kamino Lending (`klend`) flash loans.
- **Execution Venue**: Meteora DLMM (`lb_clmm`) 3-leg cycles via `arb-executor` CPI or direct instructions.
- **Base Asset**: Wrapped SOL (WSOL).

---

## 2. Readiness Sign-off Checklist

### Critical Invariants & Architecture
- [x] **Instruction Ordering**: `ComputeBudget` -> `ATA Creation` -> `Kamino FlashBorrow` -> `Swaps (CPI/Direct)` -> `Kamino FlashRepay` -> `Jito Tip`.
- [x] **Borrow Index Alignment**: `FlashRepay` explicitly specifies the exact index of `FlashBorrow` (`borrow_instruction_index = 5`).
- [x] **Kamino Introspection Compatibility**: Borrow and repay remain top-level instructions (never called via nested CPI).
- [x] **Subsidization Immunity**: Intermediate token balances asserted to not exceed initial balances; no inventory drained.
- [x] **Solvency Assertion**: WSOL delta asserted $\ge$ Borrow Amount + Flash Fee + Min Net Profit.
- [x] **Zero Floating-Point**: Entire financial and routing pipeline implemented in integer math (`u64`/`u128`/`U256`).
- [x] **Freshness Bound**: Market states strictly rejected if age $> 1,500$ slots.
- [x] **V0 / V1 Sizing**: ALT compression used for V0 (fits $< 1,232$ bytes) and V1 envelope configured (fits $< 4,096$ bytes).
- [x] **Loaded Account Data Limit**: Configured to 64 MiB (`67,108,864` bytes) for Agave 4.4.1 runtime compatibility.

### Verification & Testing
- [x] `cargo fmt --all -- --check` passes cleanly.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` passes with 0 warnings.
- [x] `cargo test --workspace --all-targets` passes 1,137+ tests with 0 failures.
- [x] Differential testing against Meteora SDK confirms zero quote discrepancy.
- [x] Bankrun simulation verifies 3-leg cycle balance conservation on mainnet forks.

---

## 3. Operational Deployment Steps

1. **Environment Configuration**:
   ```bash
   cp .env.example .env
   # Populate HELIUS_RPC_URL, JITO_BLOCK_ENGINE_URL, KEYPAIR_PATH
   ```
2. **Start in Shadow Mode**:
   ```bash
   ARB_LIVE=0 DRY_RUN=true cargo run -p arb-bot --release
   ```
3. **Monitor Metrics & Jitter**:
   Inspect telemetry at `http://localhost:8080/metrics` or Telegram alerts to ensure quotes match real pool states and simulated bundles succeed.
