# Supervisor + Protocol Route Repair — 2026-11-11

**Baseline:** `work_22new.zip` / consolidated workspace  
**Runtime mode:** `shadow`  
**Live authorization:** disabled  
**Verification status:** source edits made; Rust toolchain unavailable in this environment, so build/tests are not certified.

## 1. Supervisor changes made

### Funding denomination and route anchors

- The runtime graph is now anchored to the configured Kamino WSOL reserve only.
- All other supported tokens remain intermediate vertices, so a route can traverse multiple pools and venues while starting and ending in WSOL.
- This removes the previous mismatch where a WSOL flash-loan fee / 9-decimal economic configuration could be applied to USDC or meme-token base anchors.
- This intentionally does **not** claim support for non-WSOL flash loans. That requires a separate lender reserve, fee decoder, mint-decimal contract, and repayment builder.

### Finite candidate sizing

- Removed `u64::MAX` as the supervisor's graph-sizing ceiling.
- The WSOL raw-unit ceiling is now derived from `risk.max_trade_size_usd / fresh_SOL_USD × 1e9`.
- The Meteora two-pool DLMM optimizer's configured `max_input` is clamped to that ceiling.
- Existing per-pool DLMM/CLMM capacity clamps remain in the graph/optimizer.
- This is a finite *search and risk* ceiling, not a lender-liquidity proof. The current `VerifiedQuoteInputs` does not yet include the live Kamino supply-vault available amount; that remains a P0 blocker before live execution.

### Risk and valuation

- Removed the `BYPASS_RISK_GATE` and `BYPASS_CHRONOS_GATE` supervisor escape paths.
- Removed the hard-coded `$190` / environment SOL price fallback from the decision path. Missing fresh SOL/USD data now rejects the candidate.
- Removed the false assumption that every unrecognized token has SOL's price. The current flash-loan route is WSOL-anchored; unsupported base-mint valuations fail closed.
- The non-flash `evaluate_full` path now also enforces inflight-bundle, tip-spend-ratio, and minimum-landing-rate controls, matching the flash-loan path.
- A zero/unknown gross-profit denominator now produces a maximally conservative tip-share result instead of zero bps.

### Preflight and dispatch

- Removed `FAST_PATH_SKIP_RPC_SIM` / `ENABLE_PREFLIGHT_SIMULATION=false` as live bypasses. Live dispatch always requires RPC preflight simulation.
- The dispatch size check now parses the actual transaction envelope:
  - Legacy/V0: 1,232 bytes.
  - V1: 4,096 bytes, only on the explicitly enabled direct SWQoS/RPC path.
  - V1 through Jito bundle ingress is rejected; the Jito path remains 1,232 bytes until its exact ingress compatibility is independently verified.
- The legacy transaction builder now rejects a serialized transaction above 1,232 bytes instead of emitting an invalid legacy packet and waiting for dispatch to reject it.

## 2. Transaction V1 integration

The Solana mainnet V1 transaction format is active. The network's V1 limit is 4,096 bytes; this is **not** a blanket replacement of the 1,232-byte limit for legacy/V0.

Changes made:

- The supervisor can use `AdaptiveTxCompiler` when `SOLANA_TX_V1_DIRECT_ENABLED=true` and the route is direct SWQoS/RPC.
- Adaptive selection retains V0/legacy for transactions that fit 1,232 bytes and falls back to V1 only when needed and permitted.
- V1 compilation strips ComputeBudget instructions because they are no-ops in V1 and still consume instruction/CU budget.
- Signed V1 construction requires an explicit CU limit (1..=1,400,000) and loaded-account data limit (1..=64 MiB).
- V1 priority fee is converted from micro-lamports/CU to an absolute total lamport value with ceiling arithmetic.
- V1 has no ALT support in this format path; addresses are inline, and the existing 64-address / 64-top-level-instruction limits are enforced.
- V1 is excluded from the existing legacy prewarmed-template patch path because that template format is not V1-aware.
- The optional local LiteSVM fast simulation is skipped for V1 because local parser support may lag; mandatory live RPC simulation remains required and fail-closed.

`SOLANA_TX_V1_DIRECT_ENABLED=false` is added to `.env.example`. Do not enable it until every configured RPC/sender endpoint and the downstream validator/leader path are confirmed V1-capable (Agave/Jito-Solana 4.2.2+). The direct path still needs end-to-end provider testing and V1 landing evidence. No automatic V1 downgrade or relabeling is performed.

## 3. Pool-specific quote path repairs

The mixed route adapter uses mutable per-pool state and protocol-specific hop dispatch. The following source-level repairs were made:

- CPMM reverse direction now swaps input/output reserves correctly and advances the correct reserve side after each hop.
- CPMM fee is calculated from the pool's exact integer numerator/denominator with ceiling arithmetic; it is no longer truncated through BPS before fee calculation.
- Raydium CLMM's fee unit is converted from BPS to the math engine's millionths (`1e6`) representation (`25 bps → 2,500 ppm`). The conversion fails closed if it cannot fit the current math type.
- Meteora DLMM cross-pool quoting now uses the decoded `index_reference` and `collect_fee_mode`, not `active_bin_id` / a hardcoded input-only fee mode.
- Meteora DLMM function types other than the implemented Spot function type are rejected rather than priced with the wrong curve.
- Graph discovery excludes pools marked risky or carrying the unsupported BPS-only transfer-fee approximation.
- Meteora DLMM pair discovery rejects risky/transfer-fee pools and unsupported function types.
- Meteora DAMM v2, Raydium LaunchLab, and PumpSwap remain quote/discovery-only until their dedicated fee/curve adapters have independent differential acceptance. They are no longer falsely treated as generic CPMM executors by the mixed-hop adapter.
- Pump.fun retains its dedicated bonding-curve quote adapter. Orca Whirlpool and Raydium CLMM use concentrated-liquidity tick math. Meteora DLMM uses bin traversal and its decoded dynamic-fee parameters. Raydium CPMM and AMM V4 use their pool fee ratio and constant-product reserve model.

## 4. Route composition

The runtime route graph is one WSOL-anchored graph across all configured pools. A route is not assigned one global pricing formula. Each hop dispatches through the adapter for that pool's protocol and mutates that pool's local quote state before the next hop.

### Current intended route families

1. **Meteora DLMM pool A ↔ Meteora DLMM pool B:** dedicated two-pool bin traversal and bounded integer sizing; the pair must be distinct and share the same WSOL/token pair.
2. **Meteora DLMM ↔ Raydium CPMM / AMM V4:** mixed-hop graph adapter; only accepted account matrices and fee models may proceed.
3. **Raydium CPMM / AMM V4 ↔ Raydium CLMM:** CPMM exact fee ratio plus CLMM tick-array math.
4. **Orca Whirlpool ↔ accepted pool adapter:** Whirlpool Q64.64 / tick-array quote plus the other pool's own adapter.
5. **Pump.fun ↔ accepted DEX:** bonding-curve quote only after the Pump.fun builder and all account dependencies are accepted.
6. **PumpSwap / DAMM v2 / LaunchLab:** discovery only pending their dedicated math and transaction acceptance.
7. **Multi-hop:** WSOL → token A → token B → … → WSOL, with each pool quoted by its own adapter and all swaps plus flash repayment kept in one transaction.

A route's presence in the graph is not live-execution authorization. Missing account dependencies, unsupported Token-2022 extensions, stale pool state, or an unaccepted adapter must reject the route.

## 5. Remaining blockers — not hidden

The source changes above do not make the bot live-ready. These items remain unresolved and are explicitly not represented as completed:

1. **Kamino lender capacity:** the current verified-input contract reads reserve fee/mint/status but does not yet decode the live available liquidity from the reserve supply vault. The finite USD cap does not replace this.
2. **One economic authority:** the supervisor still has legacy `calculate_net_profit_raw` decision logic. `EconomicQuote` / `CostLedger` is not yet the single signed, mint-aware source from quote through settlement.
3. **Minimum-output correctness:** the main opportunity model still needs a separately computed conservative `min_out` per hop and a final output guard that covers principal + flash fee + required profit. `expected_out == min_out` is not a safe substitute.
4. **Guarded quote integration:** `quote_route_guarded` and `revalidate_before_dispatch` are not yet mandatory on every route path immediately before signing/dispatch.
5. **Token-2022:** real per-mint epoch fee schedules, maximum fee, transfer hooks and account metadata are not integrated end-to-end. Those pools are intentionally rejected by the current quote graph.
6. **Protocol acceptance:** Pump.fun, PumpSwap, DAMM v2, LaunchLab, and every multi-protocol composition need official SDK differential fixtures, account-meta parity and fork simulation.
7. **V1 verification:** no V1 RPC/QUIC end-to-end test, signed-byte round trip against upstream SDK golden vectors, V1 simulation test, or landed V1 transaction was run in this environment. Jito bundle V1 ingress remains disabled.
8. **Landing and settlement:** send acceptance must not be reported as landing. The Unknown/timeout state and actual token/native balance reconciliation still need complete end-to-end verification.
9. **Rust verification:** `cargo fmt`, `cargo check`, `clippy`, workspace tests, fuzz/property tests and fork tests were not run because `cargo` / `rustc` are not installed here.

## 6. Mainnet V1 network facts (researched 2026-11-11)

- V1 is active on Solana mainnet since 2026-09-15 (epoch 1035).
- Legacy/V0 remain capped at 1,232 bytes; V1 raises the transaction wire limit to 4,096 bytes.
- V1 uses the `0x81` discriminator and trailing signatures; it has no Address Lookup Tables.
- V1 currently supports 64 inline addresses and 64 top-level instructions. SIMD-0596's proposed 96-account limit must not be assumed active.
- V1 resource limits live in the message config. CU limit and loaded-account data limit default to zero and must be explicit. Priority fee is total lamports, not micro-lamports per CU.
- Solana's docs state QUIC can segment V1 transactions larger than the MTU into multiple QUIC frames. This does not mean every RPC, sender, wallet, private relay or Jito ingress supports V1.
- Agave RPC and Jito-Solana operators need V1-capable releases (4.2.2+ per Solana's upgrade guidance). RPC transaction readers that fetch full transactions/blocks must opt in with `maxSupportedTransactionVersion: 1`.

## 7. Files changed in this repair pass

- `crates/arb-bot/src/supervisor.rs`
- `crates/arb-bot/src/supervisor/run.rs`
- `crates/arb-bot/src/supervisor/dispatch.rs`
- `crates/arb-tx/src/adaptive.rs`
- `crates/arb-tx/src/lib.rs`
- `crates/arb-risk/src/lib.rs`
- `crates/arb-engine/src/adapters.rs`
- `crates/arb-engine/src/lib.rs`
- `crates/arb-engine/src/meteora_quote_engine.rs`
- `crates/arb-engine/src/cross_pool_dlmm.rs`
- `crates/arb-engine/src/meteora_pair_discovery.rs`
- `crates/arb-ingest/src/registry.rs`
- `crates/arb-tx/tests/adaptive_compiler_tests.rs`
- `crates/arb-engine/src/adapters.rs` (regression test)
- `.env.example`

## 8. Release verdict

**Improved source candidate — not a certified executable release.** Keep `mode=shadow`, keep `SOLANA_TX_V1_DIRECT_ENABLED=false`, and do not deploy live. The next engineering gate is to add verified Kamino supply-vault liquidity to the risk sizing contract, then unify the signed economic quote and conservative min-out, then run the complete Rust and fork acceptance suite in Linux with the pinned Rust 1.99.1 toolchain.

## 9. Primary technical references

- Solana Foundation — Larger Transaction Sizes / V1 mainnet activation and transport notes: https://solana.com/vi/upgrades/larger-transaction-sizes
- Solana — Runtime constants (4,096-byte V1, 64 MiB loaded-account maximum, account/instruction ceilings): https://solana.com/docs/core/constants-reference
- Solana Cookbook — Versioned transactions (V1 config fields, no ALTs, `maxSupportedTransactionVersion`): https://solana.com/it/developers/cookbook/transactions/versions
- Solana — Account runtime and loaded-account data size: https://solana.com/docs/core/accounts/account-runtime
- Jito — Low-latency transaction sending and bundle semantics: https://docs.jito.wtf/lowlatencytxnsend/
- Meteora — Rust SDK integration guide: https://docs.meteora.ag/developer-guides/dlmm/rust-sdk
- Meteora DLMM SDK releases: https://github.com/MeteoraAg/dlmm-sdk/releases
- Orca Whirlpools Rust SDK: https://github.com/orca-so/whirlpools
- Raydium SDK V2: https://github.com/raydium-io/raydium-sdk-V2
- Kamino Klend source: https://github.com/Kamino-Finance/klend
