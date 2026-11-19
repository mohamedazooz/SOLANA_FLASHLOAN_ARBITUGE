# Acceptance criteria and validation matrix

## Hard blockers before any live mode

- [ ] All exposed credentials revoked/rotated; no secrets in source, examples, logs, images, CI artifacts, or history.
- [ ] All production simulation paths reject Disabled/NotRun/Failed/Timeout.
- [ ] No route is executable from third-party USD-liquidity estimates or fabricated reserves.
- [ ] Every amount has a declared mint and decimals; no raw token amount is stored as lamports.
- [ ] No unchecked cast, saturating fallback, or default fee can produce an executable quote.
- [ ] Split route is re-quoted per pool and return leg; risk and builder share identical quote data.
- [ ] All fees and costs are counted exactly once and sourced with slot/provenance.
- [ ] Every transaction has protocol-correct min-out and repay amount.
- [ ] On-chain simulation succeeds against a coherent recent state.
- [ ] Live execution remains disabled until all original gates are re-evaluated and signed off.

## Required tests

### Arithmetic

- boundary tests at zero, one, max integer, overflow, and denominator zero;
- property tests for monotonicity and conservation of input/output units;
- differential tests against deployed protocol math/IDL for CPMM, CLMM, DLMM, PumpSwap;
- exact fee rounding tests (input-side/output-side and protocol-specific).

### Routing

- two pools with different fees and different mint metadata;
- parallel split with uneven allocation;
- aggregate output that changes the return-hop price;
- stale leg among fresh legs;
- same pool accidentally duplicated;
- intermediate token mismatch and route that does not close.

### Execution

- simulation disabled, timeout, failure, and success;
- missing Kamino fee snapshot;
- transaction builder receives a different quote hash than RiskGate;
- min-out lower than repay requirement;
- landed, failed, dropped, and unknown confirmation outcomes.

### Security / operations

- secret scanner against repo, Docker image, CI logs, and `.env.example`;
- webhook auth rejects placeholder/default token;
- config live mode cannot override kill switch or missing approval gates;
- bounded queues under feed burst and reconnect storm;
- metrics cardinality and memory growth tests.

## Performance acceptance

Measure replay with the same captured account snapshots and candidate set. Report p50/p95/p99 for decode, quote, route search, split re-quote, simulation, build, send, and landing. Compare CPU, allocations, queue depth, stale-state rejection rate, and landed success rate. Do not optimize based on average latency alone.

## Profitability acceptance

Use out-of-sample replay and shadow/canary observations. Report gross estimate, each cost component, predicted net, actual balance-delta PnL, landing rate, revert rate, and confidence interval. No positive backtest or simulated result establishes future profitability.
