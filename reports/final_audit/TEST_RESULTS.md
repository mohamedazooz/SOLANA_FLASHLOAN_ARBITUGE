# Test Results (PHASE L) — executed this audit

Command: `cargo test --workspace --all-targets` → **994 passed, 0 failed** (2 ignored by design).
`cargo fmt --all -- --check` → clean. `cargo clippy --workspace --all-targets -- -D warnings` → clean.

## Targeted evidence runs

| Suite | Count | Result |
|---|---|---|
| arb-tx dlmm_bin_discovery (incl. NEW bitmap-extension boundary tests) | 11 | PASS |
| arb-math python_oracle_differential | 50 | PASS |
| arb-math differential (property) | 12 | PASS |
| arb-math dlmm_bin_array_boundary_tests | 2 | PASS |
| arb-engine lib | 41 | PASS |
| arb-engine phase17_pda_differential + phase17_two_pool_flash_arb_tests | 20 | PASS |
| arb-engine three_leg_net_economics_tests | 5 | PASS |
| arb-engine meteora_sdk_differential_tests | 9 | PASS |
| Python oracle self-verify (`oracle.py --verify`) | 19 vectors | PASS |

## Economic case matrix (18 required cases)

Covered by the workspace suites: gross+/net+ (NET-PROFIT-0), gross+/net− (NET-PROFIT-1,
EDGE_BELOW_HURDLE live evidence), flash-fee increase (KaminoFeeError + CSV vectors),
priority-fee/tip increase (adaptive price + tip cap tests), slippage increase
(min_amount_out binding tests), liquidity decrease (leg capacity rejection),
borrow > liquidity (fail-closed verify), missing BinArray (MissingArray test),
active-bin change (freshness gate), reserve change (fail-closed liquidity),
>1232 bytes (TransactionTooLarge tests), >4096 bytes (V1Oversized), >64 accounts
(V1AccountLimitExceeded), incorrect repayment (flash_repay_checks / atomic integrity),
unsupported Token-2022 extension (fail-closed transfer-fee tripwire), stale RPC data
(freshness gates). Cases 13/15 additionally covered at the compiler level in
v1_header_differential.rs.

## Live-mainnet evidence (previous phase, no broadcast)

163 pools registered, 245 slot scans (p50 370ms), fresh-pipeline cycle borrow 0.1 USDC →
net −1,906 raw → rejected; CU 76,549/400,000; wire 942/1,232 B; slot lag 1; simulation
success; zero broadcast. Honest rejections, not fabricated opportunities.
