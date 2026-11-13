# Phase 16 — Reproducible Test Commands

**Date:** 2026-11-13
**Host used:** Windows 11, Rust/Cargo 1.99.1, workspace `SOLANA_FLASHLOAN_ARBITUGE`.
All commands are read-only with respect to the chain (no broadcast).

## 1. Static gates

```bash
cargo check --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Expected: all exit 0. `check` took ~5m26s cold; fmt instantly; clippy after a
build ~1–2m.

## 2. Full test suite (offline-safe subset)

The full command is:

```bash
cargo test --workspace --all-targets --locked
```

In a sandbox **without a funded `SOLANA_PRIVATE_KEY`**, two live tests fail
(they attempt a real mainnet `simulateTransaction` with an unfunded fallback
payer → `InvalidAccountForFee`):

```
arb-tx --test mainnet_non_broadcast_simulation
```

Run everything else:

```bash
# Skip the two live-payer tests by selecting the other arb-tx binaries
cargo test -p arb-tx \
  --test adaptive_compiler_tests --test adversarial_tests --test bundle_tests \
  --test challenger_m2_tests --test fuzzing_templates \
  --test intermediate_balance_isolation_tests --test meteora_atomic_builder_tests \
  --test p0_golden_vectors --test prewarmed_template_tests \
  --test three_leg_meme_compiler_tests --test three_pool_flash_arb_tests \
  --test v1_header_differential --test v1_header_unit --test v1_property_fuzz \
  --test v1_size_ladder_tests --test mainnet_alt_integration_tests

cargo test -p arb-types --all-targets
```

If a funded simulation-only key is available (read-only simulate, no
broadcast):

```bash
export SOLANA_PRIVATE_KEY=<base58 simulation key with >0 lamports>
export SOLANA_RPC_PRIMARY=<mainnet RPC URL>
cargo test -p arb-tx --test mainnet_non_broadcast_simulation -- --nocapture
```

## 3. Measured V1 size ladder

```bash
cargo test -p arb-tx --test v1_size_ladder_tests -- --nocapture
```

Writes/updates `reports/phase15_5/V1_SIZE_LADDER.json` (timestamps churn; the
measured byte values are stable).

## 4. Flash-loan atomicity

```bash
cargo test -p arb-tx --test three_pool_flash_arb_tests
cargo test -p arb-tx --test intermediate_balance_isolation_tests
cargo test -p arb-tx primitives::atomic_integrity
cargo test -p arb-engine --test phase17_two_pool_flash_arb_tests
cargo test -p arb-engine --test three_leg_net_economics_tests
cargo test -p arb-sim profitability_gate
```

## 5. V1 transport

```bash
cargo test -p arb-tx --test adaptive_compiler_tests
cargo test -p arb-tx --test v1_property_fuzz
cargo test -p arb-tx --test v1_header_differential
cargo test -p arb-tx --test v1_header_unit
```

## 6. Execution infrastructure

```bash
cargo test -p arb-land
cargo test -p arb-risk
cargo test -p arb-store
```

Covers tip ladder/floor, Kobe leader gating, landing ledger, block-height
recovery, durable dispatch intents, velocity and in-flight caps.

## 7. Live probe (optional, credentials required)

```bash
python scripts/probe_rpc_wss_v1_sim.py
```

Read-only: RPC version, WSS slots, V1 `simulateTransaction`. It does not send a
transaction.

## 8. Evidence artifacts

- `reports/phase15_5/V1_SIZE_LADDER.json` — measured sizes.
- `reports/v1_mainnet_probe/probe_report_2026-11-13.json` — CITED live simulate.
- `reports/phase16/TEST_EVIDENCE_2026-11-13.json` — this audit's aggregate.
