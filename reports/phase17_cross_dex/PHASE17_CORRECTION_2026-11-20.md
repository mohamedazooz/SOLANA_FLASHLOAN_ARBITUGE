# Phase 17 Evidence Correction — 2026-11-20

## What the available evidence establishes

The historical Phase 17 report records a non-broadcast simulation in which Kamino borrow began and Meteora DLMM returned `ExceededAmountSlippageTolerance` (`0x1773`). It establishes that the simulated instruction did not satisfy the minimum output encoded in that attempted transaction and that the SVM rolled the transaction back atomically. It does not establish a successful swap, a profitable opportunity, a Raydium execution, or a completed Kamino repayment.

The checked-in integration scenario uses synthetic/mock route state. It is useful as a builder and atomic-structure test, but it is not a replay of the mainnet failure. The historical evidence file and reports are retained as historical artifacts; their simulation claims must not be treated as proof of live execution readiness.

## Fixes in this branch

- Route deduplication now keys on ordered pool, protocol, input mint, output mint, and trade intent, and keeps the highest-net-profit quote for an identical directed route.
- RPC simulation outcomes are typed. A response without `err`, positive `unitsConsumed`, or a usable context slot cannot pass.
- Primary and fallback dispatch paths reject non-passed results and reject simulation context outside the configured quote freshness window.
- Simulation logs are bounded to 64 lines and 512 bytes per line before being carried forward.
- Slippage settings were not widened.

## Remaining proof required for `0x1773`

The repository does not contain enough original inputs to replay the recorded failure deterministically: the exact signed wire transaction, exact pool/bin/token account data at the simulation bank, the quote slot and per-hop quote outputs, and the encoded minimum outputs are not all available together. Until those inputs are retained and the failing transaction is replayed, the causal explanation remains open. A future forensic capture should bind those values to one attempt, plus simulation context slot and bounded program logs.

## Verification in this branch

`cargo test -p arb-sim --lib`: **33 passed**. `cargo check -p arb-bot`: **passed**. Route-dedup unit tests: **5 passed**. The focused supervisor integration test `test_matrix_13_supervisor_single_route_attempt`: **passed**. `cargo fmt --all` and `git diff --check` are also clean. No transaction was broadcast, and no mainnet simulation was run as part of this repair.
