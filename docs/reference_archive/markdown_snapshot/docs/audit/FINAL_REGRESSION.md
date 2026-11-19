# PHASE 14 — Final Regression & Production Simulation

## Full suite on the audit branch (post oracle-integration)

- `cargo fmt --all -- --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test --workspace --all-features`: 837 PASS / 2 BLOCKED (funded-key live sims)
- Oracle differential: 19/19 MATCH

## Controlled lifecycle simulation (shadow)

Executed as 3-minute live shadow soaks against mainnet (publicnode), repeatedly:
data ingestion → state validation → discovery → route generation → quoting →
fee computation → net-profit gating → (dispatch never reached: no opportunity passed
the economic gate — honest behavior, not a failure).

| Soak | Edges | Pairs | Phantom rejections | 429s | Verdict |
|---|---|---|---|---|---|
| soak4 (pre-fix baseline) | 49 | 0 | 0 (nothing quoted) | 39,519 | broken pipeline |
| soak10 (batching) | 98 | 24 | 4,134 | 83 | phantom-ROI flood |
| soak15 (all fixes) | 95–96 | 15–21 | **0** | 6 | clean, first real 3-hop cycles |
| soak16 (CLMM canonical) | 96 | 15–21 | **0** | 9 | clean, 0 orientation WARNs |

## Atomic repayment / rollback validation

- Structurally verified (instruction ordering + on-chain profit guard + runtime
  atomicity). No live execution — **NOT LIVE-PROVEN** (requires authorized live mode).

## Unavailable tests & blockers

| Test | Blocker |
|---|---|
| mainnet non-broadcast sims (arb-tx ×2) | funded `SOLANA_PRIVATE_KEY` prohibited in automated runs |
| 24h shadow soak | runtime budget (3-min soaks executed; 24h = operator VPS task) |
| Criterion P50/P95/P99 suite | not run this pass (bench exists; backlog) |
| cargo-fuzz / Trident | not run (backlog P2) |
