# Phase 09B — risk, economics and execution correctness

This phase starts only after 09A produces protocol-exact quotes and coherent state snapshots.

1. Replace ambiguous numeric units with typed amounts carrying mint, decimals and raw atoms. Never store arbitrary token atoms in `Lamports`.
2. Create one immutable per-opportunity cost ledger used by discovery, RiskGate, transaction builder and reconciliation. Include DEX fees, Token-2022 transfer fees, flash-loan fees, base/network fees, priority fee, validator/Jito tip, rent and account creation.
3. Read flash-loan fees from verified on-chain reserve/config state at a recorded slot. Missing or stale fee state rejects the opportunity.
4. Enforce `MAX_TRADE_SIZE_USD` using a fresh, provenance-tagged price and the actual token decimals. Unknown mint pricing rejects.
5. Remove split-route heuristics. Allocate, then quote every leg with its own native adapter and fee; aggregate the route and re-run economics/risk on the exact combined result.
6. Jupiter routes remain disabled until every leg's pool, account list, fees, minimum output, slippage and transaction instructions are independently reconciled.
7. Distinguish signature lifecycle (`processed`, `confirmed`, `finalized`, `failed`, `unknown`) from Jito bundle lifecycle. Never query a transaction signature as a bundle ID.
8. Record realized PnL only from transaction metadata and pre/post token balances, including fees, tips, repayments, rent and transfer fees.
9. Compute CU estimates from route shape for ranking only; update calibrated estimates from simulation `units_consumed`, then actual landed metadata where available.

Acceptance: property/unit tests, transaction replay, exact ledger reconciliation, and no false `Landed` or realized-PnL labels.
