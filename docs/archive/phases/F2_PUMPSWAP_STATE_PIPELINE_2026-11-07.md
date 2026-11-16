# F2.1 — PumpSwap State Join & Shadow Promotion

Date: 2026-11-07  
Status: source changes implemented; Rust build/tests not run in this environment.  
Execution mode: Shadow only.

## Delivered

1. Yellowstone owner filters include PumpSwap and Pump Fees programs.
2. PumpSwap pool accounts are discovered from the owner-filtered Yellowstone account journal.
3. Dynamic tracked-account subscription includes each pool, both vaults, both mints, PumpSwap GlobalConfig, Pump Fees FeeConfig, Kamino WSOL reserve, and WSOL mint.
4. Tracked-account updates are sent only when the set changes; the set is deduplicated and capped at 4,096 accounts.
5. Strict join validation checks Pool owner/discriminator, canonical GlobalConfig and FeeConfig PDAs, owners and discriminators, and non-future config/mint snapshots.
6. Pool and both vaults must have the exact same slot.
7. Vault token mint and authority must match the Pool header; token accounts must be initialized and unfrozen.
8. Mint owner must match the vault token program; mint initialization, decimals and supply are decoded. All Token-2022 mint extensions are rejected until exact transfer/extension semantics are implemented.
9. FeeConfig decoding supports the legacy single-ladder prefix and current standard/stable ladders with optional exotic flat fees. Unknown non-zero trailing extensions fail closed.
10. Canonical pool classification uses `pool.creator == PDA([b"pool-authority", base_mint], Pump program)`, never `coin_creator`.
11. FeeConfig rates are decoded as on-chain BPS and normalized to PPM (`1 BPS = 100 PPM`) only at the adapter boundary. Fee tier selection uses quote-mint raw units and falls back to the first tier below its threshold. Creator fee overrides are applied only when GlobalConfig `creator_fee_configurable` is enabled and the pool fee does not exceed `max_configurable_creator_fee_bps` (current account offsets 940 and 941..949; legacy 940-byte account defaults the gate off).
12. PumpSwap GlobalConfig disable-buy and disable-sell bits gate promotion.
13. Coherent PumpSwap state is promoted into the shared market cache as a candidate-detection CPMM state.
14. The pricing reserve is `raw_quote_vault + signed virtual_quote_reserves`; real payout liquidity is `raw_quote_vault - protocol_fees - creator_fees`. Sells exceeding real liquidity are rejected.
15. The adapter's exact-in buy path follows the official fee math: floor the net from input and total BPS, ceil each fee on the unshaved net, shave any fee-rounding overrun, then swap `net - 1`. Sell math uses ceil fee components and checks `real_quote_reserve >= raw_quote - lp_fee`.
16. PumpSwap PPM fees are rounded UP to whole BPS for graph candidate discovery because the current graph representation only stores integer BPS. The adapter's PPM quote path remains separate.
17. Kamino flash-loan fee and WSOL mint decimals are now read from Yellowstone snapshots in the Supervisor's verified-input builder. The old HTTP JSON-RPC reserve reader was removed; missing or stale snapshots fail closed.
18. Pump.fun's hardcoded 125 BPS fee fallback was removed. The Pump.fun quote adapter now requires explicit fee BPS and token decimals; the generic registry skips Pump.fun until its curve, GlobalConfig and Pump Fees FeeConfig are joined. MCP Pump.fun quoting requires explicit caller-supplied reserves and fee inputs; fake multi-DEX, cross-arbitrage and latency telemetry outputs were removed.

## Correctness boundary

Graph promotion is a candidate detector, not an executable quote. Whole-BPS rounding is conservative and can miss small edges. Final route validation must use the PumpSwap adapter with current account state, exact fee components, exact min-out and the unified financial gate.

The adapter follows the current Pump Rust-client integer quote formulas for untaxed SOL/USDC-style quotes. Token-2022 extensions, mayhem, cashback and holder-reward pools are rejected. It has not yet been differential-tested against a corpus of official PumpSwap V2/V3 simulations and transaction metadata. It is not an authorization to execute trades.

## Not delivered in F2.1

- Pump.fun bonding-curve live account promotion and mint-to-curve discovery mapping. The adapter is dynamic-fee-only, but no live Pump.fun state join is active yet.
- Synthetic migration straddle quote (last curve buy plus PumpSwap remainder).
- PumpSwap V2 and Pump V3 instruction builders.
- Pump multi-hop instruction builder.
- Full fee-recipient, buyback, holder-reward, mayhem and cashback execution coverage.
- Token-2022 transfer-fee support; risky extensions are rejected.
- Jito bundle status reconciliation and atomic borrow/swap/repay builder.
- Supervisor decomposition and all >1,000-line file splits.
- Live execution. Live signing remains disabled.

## Required local verification

On Rust 1.85+:

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Then replay recorded Yellowstone account snapshots and compare adapter quotes with the current official Pump SDK/Rust-client and simulated transactions. No live signing until all comparisons pass.
