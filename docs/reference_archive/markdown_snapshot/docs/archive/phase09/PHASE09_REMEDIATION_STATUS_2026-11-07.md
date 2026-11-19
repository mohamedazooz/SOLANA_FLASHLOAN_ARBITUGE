# Phase 09 remediation — implementation status (2026-11-07)

## Scope and source provenance

This patch is based on the uploaded `solana-arb-bot.zip` source tree (162 archive entries, 88 Rust files), not on the previously described continuation archive. The latter was not present in the current execution workspace. Do not merge this patch blindly into a newer checkout; compare the touched files first.

## Applied changes

### 09A — freshness / no stale fallback / execution status

- `RpcPool` now uses deterministic provider order: `SOLAMI_RPC_URL` → `DRPC_RPC_URL` → `HELIUS_RPC_URL` → `SOLANA_RPC_URL` → `SOLANA_RPC_URL_2`.
- Removed round-robin starting index and public Solana RPC fallback. If no configured providers exist, the request fails.
- Provider rate/burst settings are read from `RPC_PROVIDER_RPS` and `RPC_PROVIDER_BURST`. Missing/invalid production values yield zero permits (fail-closed); the convenience constructor used by unit tests retains explicit test limits.
- Removed the five-minute verified-input cache path and the stale-cache retry path. Verified flash-fee/mint-decimal reads now require a live RPC response.
- Removed reuse of the previous SOL/USD value between market cycles. SOL/USD must be recomputed from a pool whose slot age is within `max_state_age_slots` in the current snapshot.
- Jito `processed` status is no longer mapped to `Landed`.
- SWQoS `sendTransaction` signatures now use `getSignatureStatuses`; only Jito bundle IDs use `getBundleStatuses`. `processed` and `confirmed` are logged as provisional states; only `finalized` maps to the success path in the direct-signature branch.
- Removed hardcoded Helius HTTP URL/API key and the invalid behavior of passing an HTTP RPC URL as a Yellowstone gRPC endpoint. Yellowstone gRPC URL must be configured explicitly.

### 09B — risk sizing / route correctness / EV

- Removed the fixed `$10` input to `evaluate_full`.
- Trade notional is computed from the actual raw input and base-token decimals. WSOL uses the current SOL/USD observation; USDC uses its USD denomination. Unknown base-token valuation rejects the route.
- Flash-loan risk evaluation now receives and enforces `MAX_TRADE_SIZE_USD`; it is no longer exempt from the cap.
- `MAX_TRADE_SIZE_USD` can override the TOML value at startup and is validated as positive and finite.
- Split routing is disabled until each leg's exact quote, fee, slippage, execution instruction and aggregate min-out are equivalent.
- Jupiter execution is disabled until quote/instruction/repayment equivalence is proven. This is a deliberate safety gate, not a claim that Jupiter is unavailable.
- Removed the profit-dependent Chronos age relaxation. High gross profit no longer makes stale quotes eligible.
- Chronos signed lamport profit arithmetic now uses `i128` intermediates and clamps to `i64`, avoiding wraparound on large values.
- Added environment overrides for Chronos lambda, maximum age and minimum EV.
- Successful RPC simulation now records `units_consumed` per route shape. This is explicitly simulation CU, not landed transaction CU.

### 09C — provider configuration

- RPC priority is Solami first, DRPC second, then explicitly configured providers.
- `.env.example` now has three provider-neutral WebSocket slots: `SOLANA_WS_1_URL`, `SOLANA_WS_2_URL`, `SOLANA_WS_3_URL`.
- These three WebSocket slots are configuration placeholders only in this patch. They are **not yet connected** to a unified WebSocket supervisor, nor are they currently part of ordered WebSocket failover. No claim of WebSocket failover is made.

### Credential hygiene

- Removed exposed-looking Helius/Shyft credentials and the hardcoded webhook token from tracked configuration and `.env.example`.
- Webhook tests now use a test-only fixture token.
- `config.toml` starts in Shadow mode and contains no provider credentials.
- Any credential that was previously valid in the uploaded source should be revoked/rotated at its provider; redacting it from this patch does not invalidate it.

## Explicitly not complete

- Yellowstone Bootstrap/Dynamic Account Updates still need full protocol decoding and promotion into the quote-critical `MarketStateCache`/`MarketGraph` with slot/write-version/fork coherence. The stream is not yet a complete authoritative quote-state source.
- Kafka has not been integrated. It needs a defined event schema, partition key, ordering, idempotency and replay semantics; it is not an RPC substitute.
- The three generic WebSocket slots are not yet consumed by runtime code.
- Pump.fun on-chain FeeConfig, PumpSwap as an independent adapter, Token-2022 fee extensions and protocol-exact fee rounding are not completed by this patch.
- CLMM/DLMM adapters need protocol-version differential tests against official SDK/on-chain quote outputs. Existing local formulas are not evidence of execution parity.
- Actual landed CU from transaction metadata is not yet recorded. Simulation CU is not a substitute.
- Chronos lambda is still a configured hazard-model parameter, not empirically calibrated. At lambda=3/s and age=400ms, `exp(-1.2) ≈ 30.1%`; that is a model output, not an observed landing probability. Calibration requires a durable dataset of send-to-land outcomes grouped by age, route, provider, slot/leader and congestion.
- Transaction status polling currently stops after its configured attempt window. A durable background reconciler and transaction metadata accounting remain required.
- Full Rust compilation, unit/integration tests, Clippy, Docker build, live provider tests and differential quote tests were not run in this environment because Rust/Cargo/Docker are unavailable.

## Deployment contract

1. Copy `.env.example` to `.env` and replace all `REPLACE_ME_*` values locally on the server. Never paste secrets into chat or commit `.env`.
2. Set the exact Solami and DRPC RPC URLs, rate/burst limits, Yellowstone gRPC endpoint and token, plus the three WebSocket URLs when their integration lands.
3. Keep execution in Shadow/Simulate until the outstanding items above are closed and validated on a Rust-enabled machine.
4. Run `cargo fmt --all --check`, `cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, then provider replay and differential quote tests.
