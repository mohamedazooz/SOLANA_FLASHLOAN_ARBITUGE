# Economic Cost & Quote Revalidation — 2026-11-27

**Branch:** `fix/economic-accounting-audit-20261127`
**Base:** `origin/main` at `6cbc9b6`
**Scope:** Price impact, protocol swap fees, Kamino flash-loan premium, Solana network fees, Jito tip, and canonical net-profit accounting.

## Executive result

The production quote/economic formulas reviewed in this pass are consistent with the current protocol references and the repository's canonical accounting rules. No production fee formula was changed. Two focused Raydium CPMM regression tests were added to pin the official integer-rounding behavior and fee-exclusive price impact, where documentation prose can otherwise be misread.

This is code/test verification only. It is not an RPC simulation, a landed transaction, or proof of realized profitability.

## 1. Raydium CPMM fee parity

Official references:

- https://github.com/raydium-io/raydium-docs-v1/blob/main/products/cpmm/fees.mdx
- https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/curve/calculator.rs
- https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/curve/fees.rs

The current CP-Swap input-side implementation does **not** independently ceil the trade and creator rates. It adds the rates for one total fee ceiling, then splits the rounded total:

```text
total_fee   = ceil(amount_in * (trade_rate + creator_rate) / 1_000_000)
creator_fee = floor(total_fee * creator_rate / (trade_rate + creator_rate))
trade_fee   = total_fee - creator_fee
```

Protocol and fund allocations are calculated from `trade_fee` only, rounded down; LP receives the remainder. When the creator fee is charged on output, it is independently rounded up against the curve output. The local exact-in and exact-out code matches the official `CurveCalculator::swap_base_input` / `swap_base_output` flow, so the initially suspected independent-ceil rewrite was rejected and reverted.

The added tests pin:

- exact-in: input `100_001`, trade rate `2_500`, creator rate `1_000`, denominator `1_000_000` → total fee `351`, creator `100`, trade `251`, protocol `30`, fund `20`, LP `201`;
- curve input after fees `99_650`, output reserve debit `90_619`, and fee-exclusive CPMM impact `907` bps;
- exact-out: target output `1_000` → required curve input `1_002`, gross input `1_006`, total fee `4`, creator `1`; exact-in with that gross input returns exactly `1_000`.

## 2. Price impact

- CPMM impact is computed from the amount that actually reaches the curve after input-side pool fees, against the initial spot price. It uses fee-free curve math for the impact measurement; swap fees are not mislabeled as price impact.
- Output-side creator fees and Token-2022 transfer fees affect the user's net output but are not curve impact. They remain included in the quote output.
- CLMM impact is derived from the protocol's concentrated-liquidity price movement/step math; DLMM impact uses bin traversal and starting-bin price. The existing tests explicitly cover fee exclusion and monotonic/boundary behavior.
- The legacy `gamma` argument on `cpmm_price_impact_bps` is retained for API stability; the function intentionally prices the curve effect only. Callers must pass the curve input after all input-side fees.

## 3. Flash-loan premium

The active lender path remains Kamino. The local canonical calculator uses the Reserve's `flashLoanFeeSf` fixed-point value with checked wide-integer arithmetic and the existing tie-breaking/disabled-reserve guards. It is not replaced by a percentage constant or a second calculator. Existing regression/differential coverage includes `economic::tests::flash_fee_uses_u68f60_sf_exactly`, `kamino_fee_boundary_tests`, and `kamino_klend_differential`.

Official source: https://github.com/Kamino-Finance/klend/blob/master/programs/klend/src/handlers/handler_flash_borrow_reserve_liquidity.rs

## 4. Network fees and Jito tip

These are separate liabilities:

- Solana base fee: `5_000` lamports per transaction signature (the current flash-arbitrage message uses the payer signer).
- Priority fee: derived from the requested Compute Unit limit and micro-lamports per CU, with ceiling arithmetic; it is not calculated from actual CU consumption.
- Jito tip: a separate System Program transfer, selected by the adaptive bidder from a fresh tip-floor snapshot and constrained by the configured retained-edge hurdle and tip ceiling.

The active opportunity path computes the edge available before the Jito tip, asks the bidder for the actual tip, and recomputes the final net with that tip. The canonical economic ledger keeps base/network fee, priority fee, flash-loan premium, and Jito tip in distinct fields. DEX swap fees and Token-2022 transfer fees already reflected in the hop quote's net output must not be deducted a second time.

References:

- https://solana.com/docs/core/fees
- https://docs.jito.wtf/lowlatencytxnsend/
- https://bundles.jito.wtf/api/v1/bundles/tip_floor

## 5. Legacy path reachability

The legacy Bellman-Ford economic helper contains zero-cost fallbacks that would be unsafe if used as an authoritative execution quote. A workspace-wide call-site search found no production Supervisor/runtime caller; the active path uses the canonical route graph and Supervisor economic evaluation. Per `PROTOCOL_PARITY_RULES.md`, the legacy helper was not wired into production or modified as a shortcut. Its retirement/deprecation remains a separate cleanup decision.

## 6. Verification and limits

Final local verification: `cargo test -p arb-math -p arb-dex` passed 557 tests (176 `arb-dex`, 381 `arb-math`); `cargo test -p arb-tx -p arb-land -p arb-bot` passed 701 tests (487 `arb-tx`, 72 `arb-land`, 142 `arb-bot`). Total: 1,258 passed, 0 failed; 5 live-RPC tests remained ignored by design. `cargo fmt --all -- --check` and Clippy for `arb-math`/`arb-dex` passed. Clippy for `arb-tx`/`arb-land`/`arb-bot` did not complete because Cargo remained blocked on a build-directory lock; no pass is claimed for those crates.

- Added two unit tests in `crates/arb-dex/src/engines/raydium_cpmm.rs` under `official_fee_rounding_parity_tests`.
- The tests are deterministic local quote tests. They do not prove account hydration, transaction compilation, RPC simulation, Jito ingress acceptance, landing, or realized PnL.
- `.env`, wallet files, credentials, runtime configuration, and live-execution flags were not modified.
- No transaction was broadcast and no PR was merged.
