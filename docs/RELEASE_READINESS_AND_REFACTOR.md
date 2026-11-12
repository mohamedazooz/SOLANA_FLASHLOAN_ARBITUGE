# Release readiness and source decomposition plan

Date: 2026-11-07

## Release status

**Not live-trading ready.** This package is a controlled-development/shadow candidate, not an approved mainnet execution release. `live_single_exec.rs` has been moved out of Cargo source discovery into `docs/archive/quarantined/`, and `autobins = false` means only explicitly reviewed binaries are built. No live transaction should be signed or submitted until every gate below passes.

## Critical blockers

1. Rust build, tests, clippy and Docker image have not been verified in this environment (Rust/Docker toolchains unavailable).
2. Financial F1 source changes still require compiler validation and complete migration of every execution caller to the same signed, min-out-based financial evaluation.
3. The active Supervisor's Kamino reserve fee and WSOL mint-decimal inputs now come from Yellowstone account snapshots. Manual diagnostic/discovery CLIs may still use RPC for one-off inspection; they are not runtime fallback paths.
4. Three WebSocket endpoints are configured but real runtime failover/health scoring/re-subscription is not complete. Provider failure must fail closed and move to the next configured provider; no stale-cache or synthetic fallback.
5. Yellowstone account storage is not yet fully fork-aware and reconnect-gap reconciled. The PumpSwap state join requires exact Pool/vault slot coherence, but processed-slot rollback and full stream resnapshot policy remain outstanding.
6. PumpSwap Pool/vault/mint/GlobalConfig/FeeConfig state is now joined and promoted to the shared candidate cache. Its exact quote adapter is not yet differentially validated against a full official SDK/simulation corpus, and it is not yet connected as the sole route quote authority.
7. Pump.fun no longer has a static 125 BPS fee fallback. Its quote adapter requires explicit fee BPS and mint decimals, and the generic registry skips Pump.fun until the separate Pump.fun Global + FeeConfig PDA (including its extra seed) + BondingCurve state are joined.
8. Pump.fun synthetic migration straddle quoting, Pump V2/V3 instruction builders, PumpSwap V2 instruction builders, and fee-recipient/sweep accounting are not complete.
9. Raydium CLMM and Meteora DLMM still need complete live account dependency traversal and differential tests against official SDK/protocol fixtures.
10. Jito send acknowledgement, bundle status, signature status, transaction metadata, expiry and reconciliation still need one durable execution state machine. Bundle ID must never be treated as a transaction signature or landing proof.
11. Kamino borrow/repay constraints must be verified for the exact instruction path. Keep borrow, swaps, repay and on-chain min-profit guard in one transaction where lender semantics require atomic same-transaction repayment; Jito bundle transport is not a substitute for on-chain atomicity.
12. Risk controls, KillSwitch persistence, operator authorization, daily loss and tip budgets require production configuration and fault-injection tests.
13. The requested source decomposition is not yet complete. The Supervisor and six other Rust source files remain above 1,000 lines and are listed below.

## File-size inventory (>1,000 lines)

The following source files exceed the requested 1,000-line ceiling and still require decomposition. The new `pump_state.rs` (909 lines) and `pump_swap.rs` remain below the ceiling. Large legacy modules have not been mechanically split in this change because moving Rust impl blocks without compiler/test feedback risks changing visibility, module paths or behavior.

| File | Lines | Proposed ownership split |
|---|---:|---|
| `crates/arb-tx/src/lib.rs` | 3,372 | `compute_budget.rs`, `jito_tip.rs`, `token_accounts.rs`, `kamino_instructions.rs`, `raydium_v4.rs`, `orca.rs`, `meteora_dlmm.rs`, `raydium_cpmm.rs`, `raydium_clmm.rs`, `pump_fun.rs`, `route_dispatch.rs`, `builder.rs`; retain `lib.rs` as re-export facade |
| `crates/arb-bot/src/supervisor.rs` | 2,114 | `supervisor/mod.rs`, `discovery.rs`, `candidate_pipeline.rs`, `quote_validation.rs`, `risk_decision.rs`, `execution_dispatch.rs`, `monitoring.rs`, `reconciliation.rs`; `Supervisor` orchestration should only coordinate services |
| `crates/arb-ingest/src/registry.rs` | 1,419 | `registry/mod.rs`, `pool_decode.rs`, `mint_metadata.rs`, `account_fetch.rs`, `fee_decode.rs`, `clmm_decode.rs`, `dlmm_decode.rs` |
| `crates/arb-engine/src/lib.rs` | 1,342 | `graph.rs`, `quote.rs`, `cycle_search.rs`, `sizing.rs`, `opportunity.rs`; `lib.rs` exports stable public facade |
| `crates/arb-types/src/lib.rs` | 1,264 | `amounts.rs`, `ids.rs`, `protocol.rs`, `pool.rs`, `route.rs`, `execution.rs`, `statistics.rs`, `errors.rs` |
| `crates/arb-math/src/dlmm.rs` | 1,115 | `dlmm/fee.rs`, `bin.rs`, `quote.rs`, `state.rs` |
| `crates/arb-math/src/clmm.rs` | 1,095 | `clmm/tick.rs`, `sqrt_price.rs`, `swap_step.rs`, `quote.rs`, `tick_array.rs` |

## Rules for the decomposition pass

- Preserve existing public paths through `pub use` facades during migration; do not create duplicate implementations or parallel aliases with different behavior.
- One canonical owner per protocol decoder, quote formula, fee formula, transaction builder and execution state transition.
- No cross-crate reach-through to private internals; expose narrow typed interfaces.
- Keep pure math and protocol decoding free of network I/O. Ingestion owns streams; execution owns signing/sending; reconciliation owns post-submit truth.
- No fallback amount, fee, price, slot, account state, or provider data when an authoritative source is missing.
- Each extraction must include call-graph review, compile, unit tests, integration fixtures and behavior-equivalence tests before deleting old code.
- Keep every production source file below 1,000 lines after extraction; tests may be split into dedicated test modules as well.

## Intended runtime topology

```text
Yellowstone gRPC + configured WS feeds
        -> provider health / slot & fork tracker
        -> account cache with provenance and freshness
        -> protocol decoders + dependency planner
        -> exact venue quote adapters
        -> route candidate graph (discovery only)
        -> discrete route sizing + unified signed economics
        -> risk gate + operator policy + persistent kill switch
        -> transaction planner (borrow + swaps + repay + min-profit guard)
        -> simulation / CU & fee estimator
        -> Jito bundle or explicitly configured non-Jito sender
        -> bundle + signature status tracker
        -> finalized transaction metadata + PnL reconciliation
        -> metrics / Telegram control / audit journal
```

The graph search is candidate discovery only. Executable profitability must come from exact integer venue quotes at the actual size and conservative min-out, after all protocol fees, transfer fees, flash fee, network fees, priority fee, tip and other verified costs.
