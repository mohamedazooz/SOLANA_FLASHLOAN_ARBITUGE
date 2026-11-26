# Unified Swap, Vault, Token-2022 and Jito Parity Audit

Date: 2026-11-26
Base: origin/main at 7797561f (after PR #170)
Working branch: fix/unified-swap-proof-flashloan-jito-20261126

## Scope and safety decision

This change set improves local correctness boundaries but does NOT claim live/on-chain parity. DAMM v2, Token-2022 extension paths, and any adapter whose complete account dependency graph is not proven remain excluded from executable routing. No live transaction was submitted.

## Findings verified in source

1. The standalone MeteoraDammV2Pool adapter is referenced only by its tests; it is not wired into the production UniversalQuoteDispatcher/ingestion route. It must not be described as a live quote path.
2. The ingestion hydration path previously populated PoolExecutionAccounts::MeteoraDammV2 directly from pool byte offsets. Those offsets do not prove account owner, vault owners/mints, actual token balances, mint token programs/extensions, or a same-context Clock/fee snapshot. Hydration now returns no executable account set for DAMM v2 until those dependencies are fetched and validated coherently.
3. DAMM v2 CollectFeeMode::Compounding uses tracked token A/B reserve amounts, not the concentrated-liquidity (sqrt_price, liquidity) curve. The V1 Pool layout exposes token amount counters at account offsets 680..688 and 688..696, with layout version at 696. V0 does not provide these counters. The adapter now decodes V1 counters and fails closed for Compounding V0.
4. DAMM v2 concentrated A-to-B next-sqrt-price calculation must round up; B-to-A rounds down. The scalar exact-in helper cannot represent the upstream amount_left partial-fill result, so it now rejects quotes that cross the configured price boundary instead of silently clamping and reporting a misleading output.
5. DAMM v2 activation can be slot- or timestamp-based. A typed Clock sysvar decoder and context-slot binding helper now use Clock.slot or Clock.unix_timestamp and expose Clock.epoch for Token-2022 schedule selection, not local wall-clock time. This helper is not yet connected to the production hydration batch.
6. The production quote path still rejects Token-2022 extensions as risky. TransferFeeConfig epoch selection exists in the codebase, but it is not yet coherently joined with every pool mint, vault, route quote and swap instruction. Transfer Hook support additionally requires resolving the mint hook program and ExtraAccountMetaList into the actual instruction accounts. Until that full path exists, these mints must remain non-executable.
7. Jito Block Engine bundles contain up to five transactions, execute sequentially, and are all-or-nothing in one slot. The atomicity model incorrectly marked JitoBundle non-atomic; it now recognizes bundle atomicity. Flash-loan borrow and repay remain required in the same Solana transaction. Bundle byte validation and simulation now enforce the 1..=5 transaction bound.

## Local code changes

- arb-math/damm_v2.rs: compounding constant-product exact-in quote, ceiling fee rounding, concentrated A-to-B sqrt-price rounding-up, fail-closed range-crossing behavior.
- arb-dex/meteora_damm_v2.rs: V1 reserve counters, Compounding V0 rejection, verified quote gate, Clock snapshot parser/context binding, Anchor Pool discriminator validation.
- arb-ingest/hydration.rs: remove unverified DAMM v2 execution-account publication.
- arb-tx/primitives/atomic_integrity.rs: correct Jito bundle atomicity semantics while preserving same-transaction flash-loan borrow/repay.
- arb-land: enforce Jito 1..=5 bound in bundle simulation.

## Required next gates before enabling production routing

1. Wire the DAMM v2 adapter into the single canonical quote dispatcher only after same-context hydration of Pool + vaults + mints + Clock + fee state.
2. Verify pool and vault account owners, vault mint addresses, token-program owners, raw vault balances, protocol-fee counters, and V1 tracked reserve counters against one RPC context slot.
3. Add differential quote vectors against the exact upstream Meteora program/SDK for both Concentrated and Compounding, both directions, all collect-fee modes, scheduler boundaries, dynamic-fee state, min-out and rounding.
4. Implement epoch-aware Token-2022 transfer fee selection at the actual execution epoch and resolve Transfer Hook ExtraAccountMetaList accounts for every transfer in each swap instruction. Until then reject such routes.
5. Build transaction simulation fixtures with exact deployed program binaries and account snapshots; compare preflight post-token balances and fee deltas with the quote ledger. Mainnet read-only simulation is still required before enabling execution.
6. Unify all adapters behind one quote result contract carrying gross input, pool fee, transfer fee, gross output, net output, unfilled input, updated state/slot, and exact execution account dependencies.
7. Prove the full route ledger: per-DEX swap fees, Token-2022 fees, flash-loan fee, Jito tip, base fee, priority fee, account rent/setup costs, and residual token balances are charged exactly once.

## Evidence boundary

Unit tests prove local integer formulas and fail-closed guards only. They do not prove that a live transaction's output, vault deltas, token withheld fees, hook side effects, or Jito landing outcome match a deployed mainnet program. Do not enable production execution based only on this PR.

## Reference snapshots

- Meteora DAMM v2 upstream source inspected locally at commit 8ea1d74d92ea843f0348d6875654ee7fbe765dc5 (MeteoraAg/damm-v2).
- Jito bundle semantics: https://docs.jito.wtf/lowlatencytxnsend/ and https://docs.jito.wtf/bundles/
- Solana Token-2022 transfer fee rules: https://solana.com/docs/tokens/extensions/transfer-fees
- Solana Transfer Hook account resolution: https://solana.com/developers/docs/tokens/extensions/transfer-hook-integration
