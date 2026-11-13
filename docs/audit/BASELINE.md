# PHASE 1 — Repository Baseline

**Audit branch:** `vibe/forensic-audit-3adf0b` · **Base commit (main):** `f82c837` (Merge PR #17)
**Date:** 2026-11-13 · **Auditor:** independent forensic pass, evidence-mandatory rules.

## Git state at audit start

| Item | Value |
|---|---|
| Branch | `main` (fresh, up to date with origin) |
| Head | `f82c837` — Merge pull request #17 |
| Working tree | clean (audit branch created from it) |
| Recent merges | #17 (WS coverage + DLMM/CLMM orientation + collect_fee_mode), #16, #15, #14, #13 |

## Workspace inventory (measured, not estimated)

13 crates, edition 2024, `rust-version = "1.99"` (validated on 1.99.1 via `rust-toolchain.toml`).

| Crate | src LOC | Role |
|---|---|---|
| arb-bot | 12,219 | Supervisor, strategy, dispatch, control service, Telegram |
| arb-ingest | 13,237 | Registry, discovery, WS, RPC pool, DLMM/CLMM decoding |
| arb-tx | 17,008 | Flash loan, primitives, transport (V0+ALT, V1), Jito, SWQoS |
| arb-engine | 8,600 | Adapters, quoting, cycle finders, flash arbitrage engines |
| arb-math | 7,529 | CPMM/CLMM/DLMM fixed-point math |
| arb-dex | 4,913 | DEX protocol bindings |
| arb-sim | 2,641 | LiteSVM / RPC simulation |
| arb-types | 2,678 | Shared state types |
| arb-land | 2,545 | Jito landing / tip strategy |
| arb-store | 1,381 | Persistence |
| arb-risk | 918 | Risk gates |
| arb-constants | 465 | Program IDs, seeds, anchors |
| arb-metrics | 308 | Counters |
| **Total (src)** | **74,442** | |

### Files > 1000 lines (17 — maintainability P4 finding)

registry.rs (2362), rpc_pool.rs (1595), arb-types/lib.rs (1521), arb-math/dlmm.rs (1515),
arb-tx/flash/compiler.rs (1490), arb-engine/lib.rs (1437), transport/master_alt.rs (1417),
arb-math/clmm.rs (1242), discovery.rs (1227), helius_ws.rs (1193), arb-ingest/lib.rs (1187),
supervisor/opportunity.rs (1177), dlmm_flash_arbitrage.rs (1144), adapters.rs (1125),
transport/adaptive.rs (1114), primitives/builder.rs (1012), arb-tx/lib.rs (1010).

## Entry points

- `arb-bot` bin `arb-bot` (main supervisor) and bin `alchemy-webhook`.
- Mode gating: `ARB_MODE` / `ARB_LIVE` / `DRY_RUN` / `ARB_CANARY`; live requires explicit
  truthy `ARB_LIVE=1` (main.rs:349 bails otherwise; control_service.rs:535 double-refuses).
- `shadow-only` cargo feature: compile-time containment that removes the broadcast path
  entirely (verified: `cfg(feature = "shadow-only")` blocks in dispatch.rs:77/93/162).

## CI

`.github/workflows/rust-ci.yml`: fmt check, check, test, `clippy --workspace --all-targets -- -D warnings`.
Web app build workflow (React + Vite) — both green on PR #17.

## Test suites

839 test results collected this audit: **837 passed, 2 failed** — both are
live-mainnet integration tests requiring a funded `SOLANA_PRIVATE_KEY`
(mainnet_non_broadcast_simulation ×2, `InvalidAccountForFee` = unfunded payer).
Classified **BLOCKED (environmental)**, not regressions. No production key may be
used in automated tests (audit rule 16).

## External integrations verified present in code (not assumed)

| Integration | Where | Verified |
|---|---|---|
| Meteora DLMM | arb-ingest (decoders), arb-math (bin math), arb-dex | probe + oracle, this audit |
| Orca Whirlpools | arb-ingest (decoders @101/@133/@181/@213), arb-math (orca_whirlpools_core 2.1.1) | live probe slot 468876781 |
| Raydium V4/CPMM/CLMM | arb-ingest, arb-tx, arb-math | probe (CLMM) + discriminators |
| Kamino flash loan | arb-tx/src/flash/kamino.rs | discriminators verified earlier session |
| Jito | arb-land, arb-bot dispatch | submission path only; live landing UNPROVEN |
| WS (publicnode/Helius) | arb-ingest/helius_ws.rs | soak16: 3-min clean run |
| Yellowstone gRPC | IngestClient plumbing | present; provider exposure NOT VERIFIED |
| Telegram | arb-bot/telegram.rs | present, auth-gated |
