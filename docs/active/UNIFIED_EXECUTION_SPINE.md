# Unified Arbitrage Execution Spine

**Status:** active-path map and remediation contract
**Scope:** atomic Kamino flash-loan cycles of two or three pools; WSOL/USDC anchors; existing registered DEX adapters only.

## One authoritative path

Account streams + pool registry
-> MarketGraph::build_from_pools
-> primary find_cycles_with_anchor_caps scan
-> optional specialized DLMM seeders
-> evaluate_directed_route for every specialized seed
-> direction/hop-order-preserving route deduplication
-> Supervisor::process_opportunities
-> fresh account/owner/slot/generation checks
-> VerifiedQuoteInputs (Kamino fee, mint decimals, rational conversion, tip)
-> canonical route re-quote + bounded size search
-> signed net-profit gate
-> CU / priority fee / tip selection and re-price
-> minimum-output / final repayment floor
-> Supervisor::assemble_native_ixs_for_anchor
-> same-DEX TxBuilder OR cross-DEX adapter delegating to the same TxBuilder
-> Kamino borrow -> ordered 2/3 swaps -> Kamino repay -> Jito tip
-> atomic integrity verification -> V1 compilation/signing
-> durable dispatch intent persisted before send
-> Jito bundle OR direct/SWQoS transaction transport
-> Jito status + Solana signature status + block-height evidence
-> Confirmed/Finalized | Failed | Expired | still Pending
-> getTransaction metadata -> tracked base-mint token delta + payer lamport delta
-> signed PnL, provenance, durable outcome, metrics and Telegram

## Authority boundaries

- arb-engine::MarketGraph is the live route evaluator. Generic graph candidates and specialized DLMM candidates converge through evaluate_directed_route; specialized candidates do not independently authorize a trade.
- Supervisor::process_opportunities is the live economic/risk gate. Discovery estimates are not dispatch authority.
- arb-math::economic is the source of truth for flash fee, network fee, tip, denomination conversion, and signed net profit.
- arb-tx::TxBuilder::assemble_flash_arbitrage_inner is the canonical transaction assembler. The cross-DEX adapter validates hop continuity/protocol mix, then delegates to the same builder.
- arb-land / jito_service own transport and landing evidence. Processed is provisional and cannot book a fill.
- arb-sim owns transaction metadata decoding. Base-mint token movement and native fee-payer movement remain separate until denomination conversion.
- arb-store is the durable attempt/outcome/dispatch-intent record. State transitions are not separate attempts.

## Economic unit contract

| Value | Unit |
|---|---|
| size_in, hop inputs/outputs, flash fee, min-output floor | base-mint raw units |
| base/priority fee, Jito tip | lamports until converted with verified rational rate |
| lamports_per_base_raw and denominator | verified base raw units per lamport |
| canonical net profit / normalized PnL | signed lamports (SOL-equivalent) |
| landed token delta | signed raw units of the route's actual base mint |
| payer balance delta | signed lamports |

Never divide every borrow amount by 1e9. Convert base raw amount using the route mint's decimals and verified conversion. Never label a non-WSOL token delta as WSOL.

## Atomic transaction invariants

1. Borrow mint equals final route mint; route contains two or three hops and is closed.
2. Adjacent hop output mint and token program match the next hop input.
3. No pool is reused in a cycle.
4. Borrow and repay principal match; Kamino fee is funded separately on top.
5. Each intermediate hop input is clamped to the previous hop's guaranteed minimum output.
6. Final-hop minimum output is at least principal + flash fee + base fee + priority fee + tip + retained minimum profit.
7. Tip is last; transaction is compiled as V1 from current account metas.
8. No dispatch without a durable intent; no resubmission after ambiguous transport errors.
9. Only Confirmed/Finalized signature evidence is terminal success. Processed is not a fill.
10. Missing/malformed token-balance evidence never becomes fabricated zero or actual PnL.

## Intentionally not live execution authorities

The exported Bellman-Ford, three-pool router, DLMM flash evaluator, and legacy direct cross-DEX assembler are not the active supervisor dispatch path. The legacy direct cross-DEX assembler is disabled. Pump.fun boost surveillance is diagnostic and is not promoted into live dispatch. Do not reconnect these paths without an explicit audit and equivalence tests.

The optional split-routing feature has a distinct split assembly branch. It is outside the standard two/three-pool atomic path and must remain disabled unless separately audited against the same cost and final-floor invariants.
