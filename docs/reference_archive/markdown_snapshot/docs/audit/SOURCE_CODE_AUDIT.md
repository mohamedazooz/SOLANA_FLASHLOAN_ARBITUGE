# PHASE 3 — Complete Source Review (findings inventory)

Method: every crate's production modules walked; hot paths (quote → gate → dispatch)
traced line-by-line across multiple sessions; each finding tied to file:line and severity.

## P0 — none open

All P0s found in prior phases were fixed with regression tests and are on `main`
(fail-open DLMM reserve fabrication, DexScreener orientation inversion,
`collect_fee_mode` hardcode, sequential RPC storms, unverified CLMM orientation).

## P1 — correctness / economics

| # | Finding | Location | Status |
|---|---|---|---|
| P1-1 | `unwrap_or_default()` on `Pubkey::from_str` in swap-hop tx paths → silent zero-pubkey account in a signed transaction (on-chain failure, paid fee) | arb-tx swap_hop paths | **OPEN** — fix requires fail-closed error propagation; scheduled next |
| P1-2 | LiteSVM sim runs an empty VM and logs "succeeded" — misleading pre-sign signal | arb-sim | **OPEN** — must either load real accounts or stop reporting success |

## P2 — protocol / reliability

| # | Finding | Location | Status |
|---|---|---|---|
| P2-1 | CLMM orientation falls back to catalog when pool bytes unavailable | registry.rs canonical block | OPEN (edge case, never observed live) |
| P2-2 | Pump.fun discovery API returns 403 → 0 curves/cycle | pumpfun_discovery | OPEN (provider block; non-critical path) |
| P2-3 | Live-mainnet integration tests require funded key → cannot run in CI | arb-tx tests, arb-sim test | BLOCKED by design (safety) |

## P3 — performance / architecture

| # | Finding | Location | Status |
|---|---|---|---|
| P3-1 | Discovery pool count degrades on rediscovery cycles (163→91) | discovery refresh | OPEN, unverified long-run |
| P3-2 | Retry backoffs are fixed 5s (no jitter) | rpc_pool | OPEN (minor) |
| P3-3 | `--all-features` clippy warnings: `shadow-only` containment compiles dispatch out, marking `jito_lander`/`observe_bundle_rtt`/`format_mint_short` dead in that configuration | arb-bot | DOCUMENTED (feature semantics, not a defect; CI gate is default-features `-D warnings` = green) |

## P4 — maintainability

| # | Finding | Status |
|---|---|---|
| P4-1 | 17 files > 1000 lines (max registry.rs 2362) | OPEN — split plan pending |
| P4-2 | Dead code: `apply_raw_account` (lib.rs:729), `meteora_atomic_builder`, `kamino_flash`, `cross_dex_flash_arb`, `dlmm_4leg_flash_arb` (tests-only) | OPEN — disposition decided: delete duplicates (git preserves), keep probes as examples |
| P4-3 | 19 env keys read-but-undocumented; ~31 documented-unread | OPEN — `.env.example` reconciliation pending |

## Error propagation / panic audit (spot-checked on hot paths)

- Quote/gate/dispatch paths return `Option`/`Result`; no `unwrap` on live inputs found in
  adapters/registry hot paths (the `unwrap_or_default` P1-1 is in tx construction).
- `is_active: true` hardcoded at registry.rs:1123 is guarded upstream by reserve-zero
  fail-closed checks — documented, acceptable.
- `Arc<Mutex>` (parking_lot) fields in Clone-derived state: locks are short, no await
  while holding found in reviewed paths.

## Swallowed-error scan

`let _ = yellowstone.apply_pool_update(state)` (WS arms) intentionally ignores
in-flight-version rejections (older-write guard) — correct, not a swallow.
All other `_ =` sites reviewed: metrics only.
