# Integration plan for the existing workspace

This package is a reference contract, not a drop-in patch. Apply changes in small, reviewable commits.

## Phase 0 — credential containment (first)

1. Revoke/rotate the Helius API credential found in root `config.toml`.
2. Revoke/rotate the fixed webhook auth token found in `.env.example`.
3. Replace both with placeholders; load secrets from environment/secret manager.
4. Scan current tree, CI artifacts, container layers, logs, and Git history. The supplied ZIP has no `.git`, so history cleanup must happen in the actual repository.
5. Add secret scanning to CI and fail on non-placeholder credential patterns.

## Phase 1 — stop false approvals

1. Introduce an explicit simulation result enum in `arb-sim`; disabled is not success.
2. Make every production caller reject `Disabled`, `NotRun`, timeout, and failure.
3. Add an independent `ExecutionMode` guard so config `mode = live` cannot bypass unmet gates.
4. Preserve all existing live gates; do not enable broadcast as part of this remediation.

## Phase 2 — fix quote and arithmetic contracts

1. Resolve `min_profit_raw` semantics. Prefer distinct types: `Lamports`, `TokenAmount<Mint>`, or a runtime `(mint, decimals, atoms)` record. Rename the parameter to encode units.
2. Replace saturating/unchecked arithmetic in quote-critical code with checked operations returning typed errors.
3. Fix CLMM tick progression and add an iteration/progress invariant.
4. Fix DLMM Q64.64 overflow propagation; no fallback zero or max price.
5. Replace `as u16` fee casts with checked conversions; invalid fee metadata rejects the pool instead of defaulting.
6. Reconcile CPMM clamp semantics with the actual optimum and test just-below/equal/just-above boundaries.
7. Audit each protocol's fee direction and rounding against its deployed program/IDL and golden on-chain fixtures.

## Phase 3 — rebuild split routing

1. Make the split allocator return allocations only, not a final profitability claim.
2. Re-quote every allocation with the exact pool's protocol adapter and state snapshot.
3. Build each swap instruction from its own `PoolLegQuote` metadata.
4. Re-quote the return leg from the aggregate intermediate-token amount.
5. Recompute gross, all costs, net, minimum output, and aggregate impact for the final split route.
6. Feed the exact same quote and ledger into RiskGate and TxBuilder.
7. Reject split if any leg has stale state, unsupported protocol, mismatched mint, invalid fee, missing account, or unavailable exact quote.

## Phase 4 — unify decoders and state

1. Establish one canonical decoder implementation per DEX; `arb-ingest` should call `arb-dex`, not maintain shadow decoders.
2. Store decoder version, owner program, account hash, slot, and decode status with each pool state.
3. DexScreener/Gmgn/Bitquery data may discover candidates, but must not synthesize executable reserves. Candidate metadata is not pool state.
4. Require a verified on-chain account decode before a pool can enter the executable graph.

## Phase 5 — economic ledger and execution reconciliation

1. One cost ledger is built once and passed unchanged through quote, risk, and transaction planning.
2. Remove fallback constants from all Kamino builder entry points; require a verified fee snapshot parameter.
3. Ensure every public/legacy builder rejects missing fee data rather than inserting zero or a default.
4. After confirmation, compute realized PnL from token-balance pre/post deltas, repay amount, network fee, priority fee, tip, rent/account creation, and token transfer fees.
5. Record landed/reverted/dropped/unknown outcomes separately. Never label planned or simulated profit as realized.

## Phase 6 — observability and cleanup

1. Wire metrics for state age, gRPC lag, reconnects, update counts, quote failures, simulation outcomes, split-route rejection reasons, tip share, and Pump.fun lifecycle events.
2. Add bounded-cardinality labels; never label by arbitrary mint, signature, or wallet.
3. Only then remove dead code. Keep dev probes that support validation; archive unused modules with an owner and deletion date.
4. Evaluate Chronos ranking only with replay data and out-of-sample calibration.

## Suggested commit sequence

- `security/rotate-and-externalize-credentials`
- `sim/fail-closed-disabled-simulator`
- `math/typed-units-and-checked-arithmetic`
- `engine/protocol-exact-split-requote`
- `ingest/canonical-dex-decoder-registry`
- `economics/single-ledger-and-kamino-fee-contract`
- `ops/metrics-and-pnl-reconciliation`
- `cleanup/remove-proven-dead-code`
