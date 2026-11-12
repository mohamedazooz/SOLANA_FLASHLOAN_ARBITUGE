# Unified Solana Atomic Arbitrage — Consolidated Engineering Package

**Snapshot:** 2026-11-11  
**Source baseline:** `work_22new.zip`  
**Mode:** shadow / live execution disabled  
**Release classification:** documented integration candidate — **not production/live certified**

## What this package consolidates

This package keeps the existing multi-venue source tree and adds the unified route/execution research and integration contract in `docs/RESEARCH_AND_UNIFIED_ROUTE_INTEGRATION_2026-11-11.md`. The intended system is one route graph and one execution pipeline, not separate bots with incompatible accounting or dispatch logic.

### Unified route graph

Pool snapshots from all configured venues feed a common token/pool graph. Candidate cycles are enumerated across supported hops, then filtered by adapter capability, freshness, account dependencies, liquidity, token extensions, lender capacity, transaction format, and risk policy.

Supported venue families represented in the source inventory:

- Meteora DLMM (bin-based pools)
- Raydium AMM v4
- Raydium CPMM / CP-Swap
- Raydium CLMM
- Orca Whirlpool
- Pump.fun bonding curve (quote/discovery only; swap builder disabled)
- PumpSwap (quote/discovery only; transaction adapter disabled)
- Kamino Klend flash-loan lender

The graph can discover combinations; this does **not** mean every combination is accepted for signing. A route is executable only if every hop's adapter is explicitly accepted and all route-level gates pass.

### Intended atomic route shape

`flash_borrow → hop_1 → hop_2 → ... → hop_n → flash_repay`

Borrow, all swaps, and repayment must be in the **same Solana transaction**. A Jito bundle must not be used to split a flash loan across multiple transactions. Any failed instruction reverts the atomic transaction. Jito tip and priority fee are separately modeled costs; bundle acceptance is not landing confirmation.

### Route rollout order

1. Meteora DLMM pool A ↔ Meteora DLMM pool B, same mint pair, with exact bin-array coverage.
2. Meteora DLMM ↔ Raydium CPMM.
3. Raydium CPMM ↔ Raydium CLMM.
4. Orca Whirlpool ↔ an individually accepted adapter.
5. Pump.fun / PumpSwap ↔ DEX only after their disabled builders are implemented and independently accepted.
6. Generalized multi-hop cycles only after every hop and the complete composed transaction pass the same acceptance gates.

## Supervisor and route repairs applied in this package

The latest repair pass is documented in [`docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md`](docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md). It removes the unbounded supervisor sizing sentinel, pins the flash-loan route graph to WSOL, removes risk/Chronos/preflight bypasses, makes dispatch format-aware, adds an opt-in direct V1 compiler path, fixes CPMM reverse-direction state mutation and exact fee ratio math, corrects Raydium CLMM fee scaling, and corrects Meteora DLMM `index_reference` / `collect_fee_mode` handling.

The V1 path is opt-in and disabled by default. Legacy/V0 remain 1,232 bytes; V1 may reach 4,096 bytes only over the explicitly enabled direct RPC/SWQoS path. Jito bundle ingress remains 1,232 bytes pending independent V1 ingress verification.

## Remaining source blockers

1. Kamino available supply-vault liquidity is not yet included in `VerifiedQuoteInputs`; the finite USD ceiling is not a lender-liquidity check.
2. The supervisor still uses legacy profit arithmetic; the signed mint-aware `EconomicQuote` / `CostLedger` is not the sole decision source.
3. Conservative per-hop `min_out` and the final principal + flash fee + profit repayment guard still need end-to-end integration.
4. Guarded quote and pre-dispatch dependency revalidation are not yet mandatory on every route path.
5. Token-2022 epoch/max transfer-fee schedules and transfer hooks remain unsupported; those pools are rejected.
6. PumpSwap, Meteora DAMM v2, Raydium LaunchLab and incomplete Pump.fun paths remain discovery-only until dedicated quote/build acceptance.
7. V1 custom serialization and direct transport still need upstream SDK differential tests, RPC/QUIC end-to-end tests, and landed V1 evidence. Jito V1 ingress remains disabled.
8. Jito acceptance/landing reconciliation and realized balance-delta accounting still need complete verification.
9. Rust formatting, compilation, Clippy, tests, fuzzing and fork simulation have not been run in this environment because `cargo` and `rustc` are unavailable.

## Release gates — all required before live mode

- One immutable execution plan and one canonical compiled transaction representation from quote through dispatch.
- One signed, mint-aware economic ledger with explicit fee inclusion provenance; no mixed-mint raw-unit arithmetic.
- Exact integer math and conservative min-out propagation for every hop.
- Fresh, slot-coherent pool snapshots and complete dependency manifests; revalidate immediately before signing/dispatch.
- Finite size bounds from lender, route liquidity, token risk, and configuration; no unbounded sizing sentinel.
- Official SDK differential fixtures and fork simulation for every protocol adapter and route composition.
- Exact account-meta/order/privilege verification for every instruction builder.
- Transaction format selected by end-to-end capability: legacy/V0 at 1232 bytes; V1 only when compiler, signer, RPC readers, simulator, transport, and leader are proven compatible. No silent downgrade.
- Flash borrow + swaps + repay in one transaction; verify repayment and final balances.
- Jito bid cap, tip account policy, timeout handling, landed-status reconciliation, and realized-PnL accounting.
- `cargo fmt`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, full workspace tests, property/fuzz tests, fork tests, and shadow soak evidence.
- Explicit operator approval before changing `mode=shadow` or enabling live dispatch.

## Verification limitations for this delivery

The Rust toolchain (`cargo` and `rustc`) is not installed in the available build environment. Source repairs are present, but no Rust formatting, compilation, Clippy, unit/integration test, fork simulation, RPC submission, or live execution result is claimed. This delivery is an improved source candidate with explicit remaining blockers, not a verified executable release. See [`docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md`](docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md).

The archive intentionally excludes `.env`, `.mcp.json`, runtime logs, build caches, `target`, `node_modules`, and `.git`. `.env.example` is retained with credential-like values sanitized. Keep secrets in your deployment secret manager and never commit live keys.

## Operator decision

**Safe action now:** deploy only in shadow mode for data/quote/route-observation validation after independently building in a Rust 1.99.1+ environment.  
**Unsafe action now:** enabling live flash-loan execution or treating all discovered venue combinations as production-ready.
