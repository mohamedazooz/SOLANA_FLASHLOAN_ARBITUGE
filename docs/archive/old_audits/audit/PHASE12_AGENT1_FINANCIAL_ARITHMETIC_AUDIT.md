> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Phase 12 — Agent 1 Financial Arithmetic & Flash-Loan Audit (2026-11-17 session)

Scope: Agent 1/2/3 of the Phase 12→15 master prompt — financial arithmetic
integrity (`saturating_*` triage), Kamino flash-loan instruction identity,
dynamic sizing connectivity, landing state machine. Branch
`vibe/phase12-15-agents-0fcd5e` off main `8e2c464` (post PR #65 merge).

## A. `saturating_*` triage (415 occurrences)

Classification method: every occurrence was assigned to one of
BENIGN (counters, slot math, capacity bounds, monotone-up cost sums where
saturation is physically unreachable at lamport magnitudes),
CONSERVATIVE (saturation direction understates profit or clamps a loss
further negative), or UNSAFE (saturation could corrupt state used by a later
economic decision or mask a broken quote).

### UNSAFE → FIXED this session

| # | Location | Problem | Fix | Test |
|---|----------|---------|-----|------|
| 1 | `crates/arb-engine/src/adapters.rs` `quote_hop` CPMM reserve advance | `reserve_out.saturating_sub(amount_out)` would silently clamp a broken adapter's quote (out ≥ reserve) to 0 and corrupt the next hop's state; mutation also happened before validation | Fail-closed: `checked_add` for the input reserve, reject `amount_out >= reserve_out` with `ArbError::Overflow` **before** any mutation (error path is side-effect-free) | `cpmm_reserve_advance_is_exact_and_never_saturates` |
| 2 | `crates/arb-engine/src/dlmm_flash_arbitrage.rs` `evaluate()` economics | `(quote_b.amount_out as i64)` unchecked casts; `net_profit * 10_000` debug-panic overflow at net > ~922k SOL | i128 intermediates with signed-direction clamp (`clamp_i64`): positive overflow → `i64::MAX`, negative → `i64::MIN`; a loss can never be flipped into a profit | covered by existing 2-pool suite + clamp test below |
| 3 | `crates/arb-engine/src/flash_loan_sizing.rs` `net_profit_lamports` + 2× `best_bps` | `saturating_sub` chain and `(net * 10_000) / borrow` overflow/panic paths; `unwrap_or(i64::MAX)` could clamp a negative overflow upward | Single i128 expression, signed clamp; `best_bps` clamps by sign | `net_profit_clamp_never_flips_loss_into_profit` |

### CONSERVATIVE / BENIGN — verified, no change

- `arb-risk/src/lib.rs:345-357,653` — cost/tip sums; magnitudes ≪ u64.
- `arb-sim/src/profitability_gate.rs:53-58,270` — cost aggregation.
- `meteora_simulation_harness.rs:157` — i64 clamp at MIN (loss direction).
- `route_worker.rs:209-211` — plain `total_loan - sum` is branch-guarded
  (`sum < total_loan` / `sum > total_loan`); no underflow reachable.
- `10_000u128.saturating_sub(fee_bps)` gamma patterns — malformed fee_bps >
  10_000 degrades to zero output → route rejected (fail-safe direction).
- `flash_loan_sizing.rs:469` `unwrap_or(i64::MAX)` on borrow — reachable only
  above 9.2e9 SOL borrow; u64 repay guard remains the hard gate.

## B. Kamino flash-loan identity — VERIFIED against Anchor derivation

Recomputed independently (not copied from repo comments):

| Constant | Repo value | Recomputed sha256 | Verdict |
|---|---|---|---|
| `FLASH_BORROW_DISCRIMINATOR` | `[135,231,52,167,7,52,212,193]` | sha256("global:flash_borrow_reserve_liquidity")[0..8] | MATCH |
| `FLASH_REPAY_DISCRIMINATOR` | `[185,117,0,203,96,245,180,186]` | sha256("global:flash_repay_reserve_liquidity")[0..8] | MATCH |
| `KAMINO_RESERVE_DISCRIMINATOR` | `[0x2b,0xf2,0xcc,0xca,0x1a,0xf7,0x3b,0x7f]` | sha256("account:Reserve")[0..8] = `[43,242,204,202,26,247,59,127]` | MATCH |

Plus previously retained live evidence: `flash_loan_fee_sf` read from the live
WSOL reserve equals the repo constant (11,529,215,046,068), reserve owner
program and layout verified on mainnet (see
`reports/production/LIVE_PUBLIC_ENDPOINT_VERIFICATION_2026-11-17.md`).

## C. Dynamic sizing connectivity — VERIFIED

- 2-pool engine: coarse grid (`coarse_samples`) + fine refinement
  (`refine_steps`) over borrow size inside `evaluate()` callers.
- 3-leg pipeline: `size_search_sizes` multi-candidate search with
  repayment-guarantee gate (`RepaymentNotGuaranteed`), net profit delegated to
  the canonical `net_profit_lamports` (single source of truth).
- Supervisor folds a **dynamic** Jito tip (TipOracle p50, capped) into
  `network_cost_lamports` (`supervisor/run.rs:42-48,216`); base + priority fee
  are deducted once at the dispatch gate (documented anti-double-counting).
- `calculate_optimal_flash_loan` remains the standalone/test-facing API; the
  pipeline runs its own bounded search. Minor duplication, both delegate net
  profit to the canonical function — accepted.

## D. Landing state machine — VERIFIED

`arb-land::LandStatus` (Sent/Landed/Failed/Dropped/Expired),
`BundleResult`, `arb-types::InflightStatus` (Pending/Failed/Landed/Invalid with
`is_terminal` + `evidence()`), supervisor dispatch distinguishes
processed/confirmed/finalized, and G13/G14 telemetry (landed/dropped records +
dedicated rejection ring) already landed in PR #59.

## E. Verification

- `cargo test --workspace`: **1145 passed, 0 failed**.
- `cargo fmt --all`: clean. `cargo clippy --workspace --all-targets`: clean.

## F. G6 completion — automatic economic flash-reserve selection (same session, follow-up commit)

Previous state: guard + env override only; the reserve/vaults/fee/liquidity
block was constants. Now:

- `arb_tx::kamino::KAMINO_RESERVE_SUPPLY_VAULT_OFFSET` (8+152) and
  `KAMINO_RESERVE_FEE_VAULT_OFFSET` (8+184) — verified empirically against
  live mainnet on 2026-11-17: the WSOL reserve's bytes at these offsets
  reproduce the verified `KAMINO_WSOL_SUPPLY_VAULT` / `KAMINO_WSOL_FEE_VAULT`
  constants; the USDC reserve yields
  `Bgq7trRgVMeq33yt235zM2onQ4bRDBsY5EWiTetF4qw6` /
  `BbDUrk1bVtSixgQsPLBJFZEF7mwGstnD5joA1WzYvYFX` (now constants too).
- `decode_reserve_supply_vault` / `decode_reserve_fee_vault` /
  `decode_reserve_candidate` — fail-closed pure decoders (discriminator,
  length, vault-mint match).
- `select_kamino_flash_reserve` — economic selection: mint match, active,
  flash loans enabled (u64::MAX sentinel rejected), live supply-vault
  liquidity ≥ min_borrow; lowest fee rate wins, ties break on deepest
  liquidity; `NoEligibleReserve` error otherwise.
- `KaminoFlashParams.liquidity_mint` generalized (was hardcoded WSOL in
  `to_flash_accounts`); `from_candidate` builds params + per-mint ATA.
- Wired into `full_path_rehearsal` (`select_flash_reserve_live`, env-gated
  `FLASH_RESERVE_SELECT=0` escape hatch for offline dev).

Live evidence (public RPC, 2026-11-17):

```
[2c] live Kamino reserve selection: reserve d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q
     mint So11111111111111111111111111111111111111112 decimals 9
     fee_sf 11529215046068 liquidity 257399608499088 raw (2 candidate(s) swept)
```

- WSOL supply vault live: 256,932 SOL (first probe) / 257,399 SOL (rehearsal).
- USDC supply vault live: 34,431,493 USDC; USDC reserve active, fee_sf equal
  to WSOL's (1e-5 U68F60) — the mint-match filter correctly reserves USDC
  for USDC-entry routes.

Tests: `reserve_vault_decoders_read_verified_offsets`,
`reserve_candidate_decodes_all_fields_fail_closed`,
`reserve_selection_is_economic_and_fail_closed` (includes the tie-break
direction regression that caught the initial inverted comparator).
`cargo test --workspace`: 1148 passed / 0 failed. fmt + clippy clean.
