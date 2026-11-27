# DLMM Quote Parity and Pricing-Path Audit — 2026-11-27

## Scope and verified architecture

- The authoritative protocol math is `arb-math/src/dlmm`; the DEX-facing adapter is `arb-dex::MeteoraDlmmQuoteEngine`.
- `arb-engine::MeteoraDlmmCrossPoolQuoteEngine` composes routes and delegates each hop to that same `QuoteEngine`; the route adapter/dispatcher also delegates to `UniversalQuoteDispatcher`. These are not separate DLMM math implementations and were not rewritten.
- `arb-dex/src/meteora_dlmm.rs` is not dead pricing code: the validation layer and transaction builder consume its account/PDA structures. It is retained.
- The legacy public `MeteoraDlmmPool` model is still referenced by validation/tests and is not safe to delete as dead code.

## Defect fixed

Exact-in normalized pool-side input/output to remove the DLMM trading fee before calculating curve price impact. Exact-out instead called `calculate_price_impact_bps(total_in, amount_out, ...)` directly, so its price impact included the fee. This made exact-in and exact-out incomparable and incorrectly showed non-zero impact for a constant-price single-bin fill.

A shared `calculate_fee_excluded_price_impact_bps` helper now performs the normalization for both directions:

- `InputOnly` / fee-on-input: subtract total fee from input.
- `OnlyY` / fee-on-output: add total fee back to output.
- Checked arithmetic fails closed on inconsistent fee totals or overflow.

Added regression coverage for exact-out under both collection modes. A constant-price bin must report zero curve impact while still reporting a non-zero trading fee.

## Protocol-parity review notes

- Meteora's official Rust integration guide documents `quote_exact_in` and `quote_exact_out`, direction-aware bin-array discovery, bitmap-extension traversal, current fee/volatility state, Token-2022 transfer-fee handling, and differential tests against the TypeScript SDK.
- The local math core already contains an exact-out bin-walk implementation and exact-out fee-invariant tests. The public `QuoteEngine` request/result contract is still exact-in shaped (`QuoteRequest.amount` is documented as input units and `QuoteResult` lacks a max-input field); enabling exact-out at the dispatcher without designing that contract would silently change semantics. It remains explicitly unsupported at the adapter boundary.
- Runtime DLMM quotes currently pass no current bank timestamp into the volatility-decay function. `PoolState.last_update_ms` is local receipt time, not Solana `Clock.unix_timestamp`; substituting it could understate dynamic fees. Preserve the fail-closed/conservative behavior until a trusted Clock snapshot is hydrated and bound to the same account context.
- Limit-order pools and unsupported function/collection modes remain rejected. Token-2022 requires verified epoch-selected transfer-fee schedules; unsupported extensions remain rejected.
- Do not remove historical reports or compatibility structures based only on duplicate names. Clippy's `-D warnings` gate passed for the changed math and DEX crates; no compiler-proven dead-code warning was emitted.

## Online references checked

- Meteora official Rust integration guide: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/rust-integration/library.mdx
- Meteora official DLMM SDK: https://github.com/MeteoraAg/dlmm-sdk
- Meteora DLMM account model: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/program/accounts.mdx
- Meteora TypeScript SDK quote reference: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/typescript-sdk/reference.mdx
- Rust math module reference: https://docs.rs/solana-protocols/latest/solana_protocols/protocols/meteora_dlmm/math/index.html

## Verification

- `cargo fmt --all -- --check` — passed.
- `cargo test -p arb-math -p arb-dex --all-targets` — passed; all 559 reported tests passed, 0 failed (including math, DEX, boundary, differential, fuzz, and bench smoke targets).
- `cargo clippy -p arb-math --all-targets -- -D warnings` — passed.
- `cargo clippy -p arb-dex --all-targets -- -D warnings` — passed.
- No RPC simulation, live transaction, Jito ingress, landing, or realized PnL is claimed.

## Remaining project work — not marked complete by this patch

The wider repository issue register still contains independent P1/P2 work: Raydium CLMM coherent runtime hydration/AmmConfig joins; PumpSwap V2 account/ATA execution parity; Orca adaptive-fee state; Meteora DAMM V2 quote/instruction parity; Kamino decoder differential limits; legacy economics reachability; and ingestion memory/telemetry items. Existing route/execution safety gates remain unchanged. These must be handled in separate evidence-backed slices rather than hidden inside a DLMM math patch.
