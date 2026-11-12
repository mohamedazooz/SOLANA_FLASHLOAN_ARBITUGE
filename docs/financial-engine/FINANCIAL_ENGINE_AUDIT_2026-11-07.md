# Financial Engine Audit — 2026-11-07

## Scope and baseline

Audited the full available `phase9-work` source tree from `solana-arbitrage-phase09a2-yellowstone-clmm-dlmm-2026-11-07.zip`, including `arb-math`, `arb-types`, `arb-risk`, `arb-engine`, `arb-dex`, `arb-tx`, `arb-land`, `arb-store`, `arb-bot`, the Pump.fun adapter, Kamino flash-loan reader, Chronos EV gate, and Jito landing ledger. This is a source audit, not a successful Rust build; this environment has no Cargo/rustc.

The intended strategy is **atomic, cyclic, cross-venue arbitrage** on Solana: borrow a base asset (normally WSOL), execute one or more swaps across Pump.fun bonding curve / PumpSwap / Raydium / Orca / Meteora and return enough of the same base asset to repay principal + lender fee in the same transaction. The route must be evaluated on exact per-adapter integer quotes, not a generic CPMM approximation.

## Executive verdict

The repository has the beginnings of a centralized economic engine, but it is not yet the sole source of truth. There are still multiple calculation paths, inconsistent units, unsigned-only PnL, a mismatch between expected and minimum output, stale Pump fee assumptions, and a Chronos probability model that is not calibrated as a real landing probability. These are correctness blockers, not cosmetic cleanup.

**Do not enable live execution based on the current financial calculations.**

## Critical findings

### FIN-P0-01 — Minimum-profit denomination is inconsistent

File: `crates/arb-math/src/economic.rs`, `EconomicConfig.min_profit_raw` and `build_economic_quote` around lines 150–165 and 273.

- `EconomicConfig` documents `min_profit_raw` as base-mint raw units.
- `build_economic_quote` treats the same argument as Lamports and converts it through `lamports_to_base_raw`.
- For non-WSOL base assets this double-converts / mis-scales the profit floor.
- The function argument type `TokenAmount` does not reveal whether it represents base-mint raw units or lamports.

Required correction: define the profit floor in an explicit denomination. For the first supported execution mode, use `BaseAssetRaw` (WSOL lamports when base mint is WSOL). USD policy floors must be converted once using a verified price with conservative rounding, then stored as base-asset raw units.

### FIN-P0-02 — Negative PnL is represented as an error, not a loss

`build_economic_quote` and `calculate_net_profit_raw` use `U256::checked_sub` for gross minus costs. When costs exceed gross, the function returns `Overflow` instead of a signed negative PnL.

Consequences:
- Loss magnitude is discarded.
- EV and daily-loss accounting cannot consume a complete economic result.
- Callers often collapse the error into `None` / `net <= 0`, losing the reason and amount.

Required correction: introduce a signed PnL domain type (e.g. `SignedBaseAmount(i128)` or a checked sign+magnitude representation). Keep unsigned raw token amounts for transfers; never use an unsigned token amount to represent a profit or loss.

### FIN-P0-03 — `minimum_final_amount_raw` is stored but not used for the profit gate

`EconomicQuote` carries both `expected_final_amount_raw` and `minimum_final_amount_raw`, but `build_economic_quote` calculates gross/net from `expected_final_amount_raw`. The `meets_min_profit` flag therefore may approve an expected-value quote whose enforceable `min_out` outcome does not meet the required profit floor.

Required correction:
- Keep `expected_net_pnl` for ranking / EV.
- Calculate `min_out_net_pnl` from `minimum_final_amount_raw`.
- Execution authorization must use `min_out_net_pnl >= configured_profit_floor`, after every cost that is guaranteed to be incurred on a successful transaction.
- If the route has no enforceable on-chain final minimum-profit assertion, reject it for atomic-arbitrage execution.

### FIN-P0-04 — SOL-cost conversion rounds down and silently repairs a zero denominator

`lamports_to_base_raw` uses integer division (floor) and `rate_denom.max(1)`. A zero denominator silently becomes 1; fractional base-mint cost is rounded down. This can understate network/priority/tip costs.

Required correction:
- Reject `rate_denom == 0`.
- For converting a liability/cost into base raw units, use checked ceiling division.
- Keep a separate floor conversion for assets received where the protocol's semantics require it.
- Never use one generic conversion function for both liabilities and assets.

### FIN-P0-05 — Cost ledger can add unlike units

`CostLedger::total_included_not_deducted()` sums every entry's `amount_raw` regardless of `CostUnit::{Lamports, TokenRaw}`. A sum of lamports and arbitrary token raw units has no economic meaning. It also uses unchecked `sum::<u128>()`.

Required correction: aggregate only after conversion to the selected base asset, with per-entry conversion provenance and checked arithmetic. Alternatively, return separate totals per unit and prohibit direct cross-unit addition.

### FIN-P0-06 — Pump.fun fee is hardcoded to 100 bps and PumpSwap is not a protocol variant

File: `crates/arb-dex/src/pump_fun.rs:15`; `DexProtocol` in `arb-types`.

- The adapter hardcodes `PUMP_FUN_FEE_BPS = 100` (1%).
- Pump.fun's current official fee page (updated 17 Oct 2026) lists a 1.25% bonding-curve fee for SOL/USDC pairs (0.30% creator + 0.95% protocol), while PumpSwap canonical pools have market-cap-tiered fees from 1.25% down to 0.30%; non-canonical pools have a separate 0.30% schedule.
- Fees are collected in the pool quote asset. Creator/Protocol/LP components and newer v2/v3 instructions can change where fees are held/paid without changing the charged amount.
- `DexProtocol` has `PumpFun` but no separate `PumpSwap` variant.

Required correction:
- Add a dedicated PumpSwap adapter and protocol variant.
- Decode current Pump fee config / pool state from on-chain account data and use the exact program/IDL version.
- Distinguish canonical vs non-canonical pool, SOL/USDC/custom pair, holder-rewards/creator fee mode, and curve vs graduated pool.
- Charge the exact quote-asset fee once; do not apply a frontend platform fee to direct on-chain program use unless the actual instruction path imposes it.
- Treat graduation as a state transition, not as a free arbitrage gap.

### FIN-P0-07 — Direct HTTP JSON-RPC remains in the flash-fee path

`crates/arb-tx/src/kamino.rs:261` has `read_reserve_flash_fee_sf(rpc_url, ...)` implemented with `reqwest` JSON-RPC. It is called from `verified_config.rs` and `live_single_exec.rs`. This conflicts with the project's agreed market-data architecture (Solami Yellowstone gRPC + WebSocket feeds, no HTTP JSON-RPC market reads).

Required correction: the Yellowstone account-state pipeline must provide the exact Kamino reserve account bytes and observed slot to the pure decoder `decode_reserve_flash_fee_sf`. Remove the HTTP reader from the production data path. Keep Jito bundle relay / control-plane HTTP only where required; those are not market-state providers.

### FIN-P0-08 — A live probe ignores the RiskGate result and has a fabricated size fallback

`crates/arb-bot/src/bin/live_single_exec.rs`:
- `optimal_two_pool(...).unwrap_or(10_000_000)` fabricates a 0.01 SOL input if optimization fails.
- `.max(1_000_000)` imposes a fixed 0.001 SOL floor.
- `trade_size_usd = usdc_out / 1e6` values the first-leg output rather than the actual input notional.
- `_risk_ok = evaluate_flash_loan(...)` discards the result and continues building/signing the transaction.
- Tip selection uses `unwrap_or(Lamports(10_000))`.

Required correction: remove all economic fallbacks; propagate sizing/price/tip errors; compute input notional from the actual input mint's raw amount, verified decimals and a fresh price; return immediately on RiskGate rejection. Keep this binary disabled as a diagnostic harness until it follows the same production financial gate.

### FIN-P1-09 — Duplicate financial formulas and type ambiguity remain

The supervisor has an ad-hoc net-profit path, `economic.rs` has a separate authoritative path, `arb-risk` computes a Chronos net/EV path, and transaction builders independently derive repay/min-profit values. Prior audit files also identified raw token values stored in `Lamports` in engine graph paths. A `TokenAmount(u64)` is not enough to identify which mint or decimals it belongs to.

Required correction: one `EconomicEvaluation` object must be created after exact route quoting and passed unchanged to Risk, TxBuilder, Jito bidding, reporting and settlement. The object must carry base mint, decimals, source slot, expected/minimum final output, per-hop quote IDs, fee ledger, signed expected/minimum PnL and calculation version.

### FIN-P1-10 — Chronos `exp(-lambda * age)` is a freshness-survival model, not a measured landing probability

`arb-risk::evaluate_chronos_ev` calculates `survival_probability = exp(-lambda * age)` and then treats it as `P_success` in EV. The default lambda is 3.0 s^-1. Unless lambda is calibrated from observed opportunity invalidation/landing outcomes by route and state-age bucket, 30% or any other output is a model assumption, not a measured probability.

It also combines outcomes too coarsely:
- Jito bundle not landed: no committed transaction, no on-chain base/priority fee or tip from the bundle.
- Bundle landed: transactions execute sequentially atomically; a successful transaction can pay network/priority fees and the tip instruction.
- A transaction that is processed and fails on-chain can still incur network fees, while state changes are rolled back.
- SWQoS/direct transaction and Jito bundle have different failure-cost distributions.

Required correction: model at least (a) quote remains valid, (b) submitted but not landed, (c) landed and succeeded, (d) landed and failed on-chain, (e) expired/rejected. Estimate separate probabilities from telemetry; keep the freshness hazard and landing probability separate. EV must use mutually exclusive outcome probabilities and route-specific cost cashflows.

### FIN-P1-11 — Jito Bundle acknowledgement is not landing and Bundle ID is not Signature

Jito `sendBundle` returns a bundle ID / acknowledgement. It does not prove processing or landing. Query bundle status with the bundle ID. Query transaction status with each transaction signature. A Jito bundle is up to five sequential transactions, atomically committed within one slot if successful; this does not allow a flash-loan borrow in transaction 1 and repayment in transaction 2 when the lender requires both instructions in the same transaction.

Required correction: one transaction must contain the flash-borrow, every arbitrage swap leg, the repay instruction, and the on-chain minimum-profit guard. A Jito bundle may carry multiple transactions for other use cases, but the flash-loan lifecycle must respect the lender's same-transaction rule. Track bundle ID and transaction signatures as different identifiers and states.

### FIN-P1-12 — Priority fee calculation and actual fee reconciliation must be separate

Solana priority fee is based on `ceil(CU_price_micro_lamports * CU_limit / 1_000_000)`, not `CU_consumed`. The transaction's actual charged fee and `computeUnitsConsumed` must be read from landed transaction metadata. Estimate for pre-trade economics; reconcile from metadata after landing. Failed on-chain transactions can still charge fees.

## Confirmed strategy model

### Atomic cycle

1. Select a base asset (initially WSOL; later allow USDC only after denomination-safe math).
2. Borrow base asset from a supported lender.
3. Route through independent protocol adapters: Pump bonding curve / PumpSwap / Raydium CPMM or CLMM / Orca CLMM / Meteora DLMM.
4. Return to the same base mint.
5. Repay principal + exact lender fee in the same transaction.
6. Enforce a final on-chain minimum profit guard in base raw units.
7. If any instruction fails or final amount is below the guard, the transaction reverts atomically.

### Important boundaries

- Bellman-Ford / negative-log marginal edge detection identifies candidate cycles; it is not proof of executable profit.
- Exact integer quote simulation with the real input size, every tick/bin crossing, fee and token transfer fee is the economic truth for the candidate.
- A Jito bundle is an inclusion/ordering mechanism, not an economic guarantee and not a substitute for lender same-transaction flash-loan semantics.
- CEX arbitrage cannot be assumed atomic with Solana on-chain execution. It needs pre-funded inventory / hedge / settlement-risk modelling; do not place it inside the same atomic flash-loan model.

## Required single-source formula

For base asset `B`, all amounts below are raw units of the same mint:

`gross_expected_B = expected_final_B - borrowed_principal_B`

`gross_min_B = minimum_final_B - borrowed_principal_B`

`net_expected_B = gross_expected_B - costs_not_included_in_quotes_B`

`net_min_B = gross_min_B - costs_not_included_in_quotes_B`

Where `costs_not_included_in_quotes_B` includes exactly once:
- flash-loan premium (protocol-native exact rounding)
- Solana base signature fee converted into B units
- priority fee (estimated before submission; actual after landing)
- Jito tip (only in the outcome where its instruction commits)
- Token-2022 transfer fees / transfer-hook costs not already represented in the adapter's net output
- ATA/account initialization cash costs, with recoverable rent tracked separately from economic expense
- any verified provider/other protocol charge

DEX swap fees, dynamic fees, creator/protocol/LP fees, and price impact must be included by each adapter's exact quote. They must not be deducted again in the central engine. Each quote must explicitly declare `fees_included` and `net_output` semantics.

**Execution gate:** `net_min_B >= minimum_profit_B` (not expected net alone). `expected_net_B` is used for ranking/EV only.

## Next implementation order

1. F0: Freeze current formula contract and golden test vectors; no Live.
2. F1: Strong unit types + signed PnL + exact liability conversions + cost-ledger unit safety.
3. F2: One economic evaluation object wired through supervisor → risk → builder → Jito → settlement.
4. F3: Pump.fun/PumpSwap fee-state adapters and cross-venue exact quotes.
5. F4: Optimal sizing per curve family; CPMM closed form only for eligible CPMM cycles; CLMM/DLMM/Pump use quote-driven piecewise/discrete optimization.
6. F5: Chronos probability calibration and outcome-conditioned EV.
7. F6: Jito/flash-loan execution reconciliation and realized PnL ledger.

No cleanup/deletion of potentially used modules until call-graph and integration tests prove them unused.
