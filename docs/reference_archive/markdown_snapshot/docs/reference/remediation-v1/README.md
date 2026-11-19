# Solana Arbitrage Remediation Pack v1

## What this package is

A compact, review-first remediation overlay derived from the supplied `solana-arb-bot.zip`, `gp.md`, `gm.md`, and `rep.md`.
It is **not** a replacement for the original 14-crate workspace and it does not claim that the original bot has been fixed.
It provides a small Rust reference core, acceptance tests, an evidence-based audit, and an integration sequence.

## Design decision

Use a **Proof-Carrying Opportunity Pipeline**:

1. A candidate route carries a coherent on-chain snapshot (slot and age).
2. Each leg carries its own pool, mints, fee, input, output, and minimum output.
3. Split routes are re-quoted per pool; no first-hop metadata is copied to sibling legs.
4. All economic amounts are in one explicitly identified base mint's raw units.
5. Every cost is itemized and counted once.
6. A disabled, missing, stale, or failed simulator is a hard rejection.
7. The risk gate consumes the exact quote and cost ledger that the transaction builder consumes.
8. Actual PnL is reconciled from confirmed token-balance deltas, not scanner estimates.

## Package layout

```text
solana-arb-remediation-v1/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs
│   └── proof_pipeline.rs
├── tests/
│   └── proof_pipeline.rs
└── docs/
    ├── AUDIT.md
    ├── ARCHITECTURE.md
    ├── INTEGRATION.md
    └── VALIDATION.md
```

## Use

This crate is deliberately standalone and has no RPC, signer, wallet, or transaction-broadcast dependency.
Copy/adapt its domain contracts into the existing workspace only after reviewing `docs/INTEGRATION.md`.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

The authoring environment used for this review did not contain `rustc` or `cargo`; therefore these commands are provided but **were not executed here**.

## Safety boundary

No live execution is enabled. No private key is read. No transaction is built, signed, or broadcast. Keep the original live gates closed until the acceptance criteria in `docs/VALIDATION.md` are independently met.
