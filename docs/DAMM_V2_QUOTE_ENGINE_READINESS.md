# Meteora DAMM v2 Quote Engine — Readiness and Execution Gates

**Status:** exact-in quote path available for analysis; production cycle discovery remains quote-only and must not generate an executable opportunity.

**Protocol reference:** `MeteoraAg/damm-v2`, source revision `8ea1d74d92ea843f0348d6875654ee7fbe765dc5`.

## Implemented

- A dedicated `MeteoraDammV2QuoteEngine` is registered with the universal quote dispatcher. It does not fall back to generic CPMM or CLMM pricing.
- The Pool snapshot preserves the scheduler, fee version, layout version, collect-fee mode, dynamic-fee state, concentrated-liquidity state, Compounding tracked reserves and the Clock point.
- Time Linear, Time Exponential, deprecated RateLimiter, Market-Cap Linear and Market-Cap Exponential base-fee modes are represented by the shared DAMM v2 math. Unknown modes and fee versions fail closed.
- Dynamic fee uses the on-chain volatility accumulator and the upstream ceiling formula. The total fee is capped by the Pool fee version.
- BothToken, OnlyB and Compounding fee placement is directional. Compounding uses the Pool V1 tracked token counters; it does not substitute physical Vault balances for those counters.
- The trading-fee allocation is reported once across LP claiming, Compounding, Protocol and Referral fields. The Compounding allocation has its own field. The dynamic-fee field is a diagnostic subset of the one total fee, not an extra amount to deduct.
- `total_fee_mint` names the denomination of the primary trading fee. `creator_fee_mint` separately names the denomination of the Creator Fee, because Raydium CPMM can charge that fee on the output token while its trade fee remains input-denominated. Raw amounts from different mints must not be added together.
- Exact-in price impact is calculated against the pre-swap spot rate and excludes pool and transfer fees.
- The HTTP hydrator and Helius WebSocket refresh share a strict account-graph validator. Pool, Vault A/B, Mint A/B and Clock are fetched as one `getMultipleAccounts` context; owner programs, Pool authority, exact Pool-referenced dependency addresses, Vault mint/authority and Clock slot are validated before a quote state is published.
- A DAMM v2 WebSocket Pool notification invalidates the old quote state while the coherent dependency refresh runs. An old fee/Clock snapshot is not left active.
- MarketGraph records DAMM v2 as quote-only and excludes it from executable cycle discovery until the deployed swap instruction has proven parity.

## Deliberately disabled

1. **Exact-out:** unsupported until a differential exact-out and minimum-output parity suite exists.
2. **Token-2022:** unsupported in this adapter until epoch-aware transfer-fee schedules and Transfer Hook extra-account resolution are carried through hydration, quote calculation and transaction construction.
3. **Executable cycle discovery:** DAMM v2 is excluded from the production executable graph. A successful exact-in quote is not proof that the transaction builder emits the exact deployed instruction/account metas.
4. **Live trading:** do not enable DAMM v2 execution from this implementation. Exact account-meta order, on-chain rounding, min-out semantics, instruction data and simulation parity still need to be proven against the deployed program.

## Verification performed

- `cargo fmt --all -- --check`
- `cargo check --workspace --jobs 1`
- `cargo test --workspace --jobs 1` — Exit Code 0; all non-ignored workspace tests passed. Opt-in Mainnet/live simulation tests remained ignored.
- `cargo clippy --workspace --all-targets --jobs 1` — Exit Code 0.
- DAMM v2 quote tests cover BothToken, OnlyB and Compounding in both directions, fee denomination, single-fee allocation, Compounding reserve selection, Clock/account-graph fail-closed cases, Token-2022 rejection, minimum output and price impact.
- Hydration tests cover a complete coherent account graph and reject Pool/Clock owner mismatch, Vault mint/authority mismatch, disabled pools, Clock-slot mismatch and dependency addresses that differ from the latest Pool bytes.
- A read-only Mainnet account snapshot was used to validate a live Pool/Vault/Mint/Clock account graph. This was not a full quote-vs-deployed-swap differential test and no transaction was signed or broadcast.

**Next release gate:** add an upstream/deployed-program differential suite for exact-in rounding, fee-side placement, Compounding reserve updates, account-meta ordering and min-out; then run non-broadcast simulation with a complete verified execution account graph. Only after these pass should DAMM v2 be considered for executable cycle discovery.

## Legacy adapter boundary

The older \`AmmPool\` / \`AdapterRouteGraph\` interface remains a reduced analysis surface. Its legacy \`quote() -> u64\` contract does not carry the per-mint fee breakdown in \`QuoteResult\` / \`HopQuote\`; DAMM v2 also returns no execution account bundle from this interface. Do not use that legacy surface for fee-ledger reconciliation or to claim execution readiness. Canonical production accounting must consume the hydrated \`PoolState\` through \`UniversalQuoteDispatcher\`, and executable cycle discovery remains gated as described above.
