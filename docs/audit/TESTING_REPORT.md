# PHASE 10 — Testing Report

## Commands & results (this audit, real runs)

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS (exit 0) |
| Check | `cargo check --workspace --all-targets --all-features` | PASS |
| Clippy (CI gate) | `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0) |
| Clippy (--all-features) | + `--all-features` | 12 warnings — `shadow-only` containment artifact (documented, EXECUTION/ARCHITECTURE docs) |
| Tests | `cargo test --workspace --all-features` | **837 passed / 2 failed** |
| cargo audit | `cargo audit` | 0 vulnerabilities |
| Oracle differential | `cargo test -p arb-math --test python_oracle_differential` | **19/19 vectors MATCH** |

## The 2 failures = BLOCKED (environmental, not regressions)

`arb-tx::mainnet_non_broadcast_simulation::{orca_whirlpool, raydium_v4_v2}` — live-mainnet
non-broadcast simulations requiring a **funded** `SOLANA_PRIVATE_KEY`; unfunded payer →
`InvalidAccountForFee`, `Units Consumed: 0`. Running them requires a funded key, which is
prohibited in automated tests (audit rule 16). Reproduce on the VPS with a funded test key.

## New tests added by this audit

- `python_oracle_differential.rs` (arb-math): permanent Rust↔Python differential, 19 vectors.
- (Inherited this cycle: 4 CLMM orientation tests, 2 collect_fee_mode tests, 4 DLMM
  reserve fail-closed tests — commits `228c330`, `152ad88`, `ce810ae`.)

## Fuzzing

- proptest present as dev-dependency (arb-math); dedicated fuzz targets (cargo-fuzz /
  Trident) **NOT RUN** — backlog P2. Existing property tests counted in the 837.

## Simulation tests

- LiteSVM-based suites pass; **caveat P1-2**: empty-VM "succeeded" is not execution
  evidence. RPC non-broadcast sims: blocked on funded key (above).
