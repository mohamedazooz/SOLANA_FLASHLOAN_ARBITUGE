# Hardcoded Values Audit — Forensic Scan (2026-11-26)

Baseline: `main` at `9e389e4`. Branch: `vibe/forensic-pricing-fee-audit-1b099c`.
Method: automated scan (`const` declarations, numeric literals in pricing/fee
contexts, f64 usage, unwrap/expect placement) across `crates/arb-constants`,
`arb-math`, `arb-dex`, `arb-engine`, `arb-tx`, `arb-bot`, `arb-ingest`,
followed by manual classification of every hit in the pricing/economic paths.

Classification (task-defined):
- **A — Protocol constant**: fixed by the protocol/runtime (fixed-point
  denominators, discriminators, tick bounds, instruction layouts). Kept as
  tested code constants; must NOT be moved to ENV.
- **B — Runtime on-chain value**: must be extracted from on-chain accounts.
  A hardcoded value here is a defect.
- **C — Application configuration**: policy value; unified in config/ENV with
  validation and ceilings.
- **D — Emergency fail-closed limit**: protective ceiling that must stay
  conservative and can only be tightened, never loosened, by config.

Verdicts: **OK** = correct and correctly classified; **QUARANTINED** =
exists but reachable only from test code; **REGISTERED GAP** = known issue
with a tracked register entry; **DEFECT** = must be fixed.

---

## 1. Findings

| # | القيمة | الملف والسطر | التصنيف | الحكم | المصدر الصحيح | التوحيد / الإجراء | الاختبار |
|---|---|---|---|---|---|---|---|
| 1 | `FEE_RATE_DENOMINATOR = 1_000_000` | `arb-math/src/clmm.rs:28` | A | OK | Raydium/Whirlpool fee rate is parts-per-million | Code constant; covered by parity suites | `raydium_clmm_swap_math_parity_tests`, `orca_sdk_parity` |
| 2 | `MIN/MAX_TICK_INDEX = ±443_636`, `MIN/MAX_SQRT_PRICE_X64` | `arb-math/src/clmm.rs:13-26` | A | OK | Protocol tick/sqrt bounds (Whirlpool `MIN/MAX_SQRT_PRICE`) | Code constant | `clmm_sqrt_bounds_tests` |
| 3 | `TICK_ARRAY_SIZE = 88`, Raydium `60`, struct sizes `113`/`168`, account size `10240` | `arb-math/src/clmm.rs:29-35` | A | OK | On-chain account layouts | Code constant | `clmm_tick_array_layout_tests` |
| 4 | `BASIS_POINTS_DIVISOR = 10_000`, `NANO_SCALE = 1e9` | `arb-math/src/cpmm.rs:18-19` | A | OK | AMM math denominators | Code constant | cpmm suites |
| 5 | `DAMM_V2_FEE_DENOMINATOR = 1e9`, max numerators 50% (V0) / 99% (V1) | `arb-math/src/damm_v2.rs:10-18` | A | OK | Meteora damm-v2 scheduler bounds | Code constant | damm_v2 suites |
| 6 | `DBC_FEE_DENOMINATOR`, `DBC_MAX_FEE_NUMERATOR = 99%` | `arb-math/src/dbc.rs:15-17` | A | OK | Meteora DBC bounds | Code constant | dbc suites |
| 7 | `LAUNCHLAB_FEE_DENOM(_PPM)` | `arb-math/src/launchlab.rs:10-11` | A | OK | LaunchLab fee denominators | Code constant; adapter is fail-closed without verified `PlatformConfig` fee | launchlab tests |
| 8 | `MAX_FEE_RATE = 100_000_000` (DLMM) | `arb-math/src/dlmm/mod.rs:58` | A | OK | lb_clmm `MAX_FEE_RATE` | Code constant | dlmm suites |
| 9 | `DEFAULT_FLASH_LOAN_FEE_BPS = 30`, `DEFAULT_RAYDIUM_FEE_BPS = 25` | `arb-math/src/pump_fun_raydium.rs:12-13` | B (misleading name) | QUARANTINED | Kamino `flash_loan_fee_sf` from the live reserve; Raydium fee from the decoded AmmConfig/PoolState | Verified: referenced ONLY from `#[cfg(test)]` code (`arb-dex/src/pump_fun.rs` tests, `arb-math/tests/*`). Production flash fee comes exclusively from `arb-math/src/economic.rs::calculate_kamino_flash_fee` with the reserve's U68F60 `sf`. **Action required by future maintainers:** rename to `TEST_...` or move into the test modules so the "default" wording cannot leak into production code. | `differential.rs:601-602` asserts the values; production path covered by `kamino_fee_boundary_tests` |
| 10 | `DEFAULT_PRIORITY_FEE_MICROLAMPORTS = 1_000`, `PUMP_FUN_RAYDIUM_CU_LIMIT = 260_000`, `tip_lamports = 10_000` defaults | `arb-tx/src/protocols/pump_fun_raydium.rs:23-24`, builder `Default` | C | REGISTERED GAP | Priority fee/tip policy must come from config/TipOracle; CU limit from measured CU with headroom | Verified: `PumpFunRaydiumArbitrageBuilder` has no production callers (grep over all crates: tests only). It is NOT wired into the Supervisor. Per AGENTS.md it must not be connected to the production path before its fee accounting is unified with the canonical CostLedger | None needed while unwired; wire-in PR must add fee-parity tests |
| 11 | `PUMP_GRADUATION_TARGET_LAMPORTS = 85e9`, `PUMP_BOOST_ALLOCATION_LAMPORTS`, `POOL_OPENING_QUOTE/BASE` | `arb-math/src/pump_boost_arb.rs:18-27` | A (with caveat) | OK (caveat) | pump.fun bonding-curve graduation constants (85 SOL target, ~1e6 boost, pumpSwap opening liquidity) | Code constant; used only for graduation-proximity gating, not final pricing. Caveat: if pump.fun changes its graduation config, these go stale — they gate, they never price | `pump_boost_arb_tests` |
| 12 | `SOLANA_BASE_FEE_LAMPORTS = 5_000` | `arb-constants/src/lib.rs:633` | A | OK | Solana per-signature base fee (5000 lamports/signature) | Code constant; economic engine multiplies by actual signature count | `arb-math` economic tests |
| 13 | `JITO_TIP_ACCOUNTS: [&str; 8]` | `arb-constants/src/lib.rs:95` | A | OK | Canonical Jito tip accounts | Code constant; ALT filtering keeps tips inline | `master_alt` tip-filter regression tests |
| 14 | `DEFAULT_MIN_PROFIT_LAMPORTS = 10_000`, `DEFAULT_MIN_KEEP_LAMPORTS` | `arb-constants/src/lib.rs:109-132` | C | OK | Config default, overridable; validated by `arb-config` (zero wallet reserve rejected) | Config default with validation | `arb-config` validation tests |
| 15 | `DEFAULT_HOP_SLIPPAGE_BPS = 10`, `HOP_SLIPPAGE_MAX_BPS = 1_000` | `arb-constants/src/lib.rs:552-558` | C + D | OK | Default is config (C); the 1_000 cap is fail-closed (D) — operators may lower, never raise | `is_valid_slippage_bps` + loader rejects > cap | `arb-config/validation.rs:229` tests |
| 16 | `LIVE_MAX_STATE_AGE_SLOTS = 2` | `arb-bot/src/config.rs:872` | D | OK | Fail-closed freshness for live evaluation | Config path treats it as a ceiling; stale snapshots are rejected | ingest slot-skew tests |
| 17 | `MAX_ROUTE_CANDIDATES_PER_TICK = 4096` | `arb-engine/src/lib.rs:91` | C | OK | Bounded candidate budget with observable truncation | Constant + truncation reported | `canonical_route_scanner_reports_candidate_budget_exhaustion` |
| 18 | `stale_after: Duration::from_secs(3)` provider policy | `arb-ingest/src/provider_supervisor.rs:58,213` | C | OK | Provider freshness policy; failure to find a fresh provider fails closed | Config-backed policy struct | `stale_or_lagging_provider_is_not_selected` |
| 19 | Fixed slices `d[8..40].try_into().unwrap()` style byte cuts | `arb-ingest/src/hydration.rs:1425-1646`, `helius_ws.rs:1112-1527` | A (layout offsets) | OK | On-chain account layout offsets | Every occurrence is guarded by a preceding `d.len() >= N` match guard, so slicing cannot panic on short data; offsets mirror the protocol layouts and are covered by decoder tests | decoder/hydration fixture tests |
| 20 | `BPS_DIVISOR_F64`, `MAX_F64_INPUT`, latency regression/`Z_95`, `SLOT_MS = 400` | `arb-math/src/chronos.rs:23-27`, `arb-math/src/latency.rs:18-30` | C (telemetry only) | OK | f64 here is confined to (a) `chronos_edge_weight` — a screening edge weight for graph search, and (b) latency projections. Verified: the authoritative profitability gates consume `build_economic_quote` integer output only; the Bellman-Ford f64 product is documented as screening-only (`bellman_ford.rs:428-436`) | Keep; do not feed these into profit decisions | `chronos_*` unit tests; economic integer suites |
| 21 | `KAMINO_*_RESERVE`, `RAYDIUM_SOL_USDC_*` addresses | `arb-constants/src/lib.rs:260-282` | A | OK | Documented canonical mainnet accounts | Code constants with drift tests (`kamino_usdc_reserve_matches_canonical_constant`) | arb-types conflict regression tests |
| 22 | `ECONOMIC_CALCULATION_VERSION = 2`, `LAMPORTS_PER_SOL` | `arb-math/src/economic.rs:16-19` | A | OK | Runtime/system constants | Code constant | economic suites |

## 2. Classes searched for and NOT found in production pricing paths

- **Fixed flash-loan percentage on the live path** — the only flash-fee
  implementation is the klend-formula one; a global bps assumption would fail
  the `flash_loan_fee_sf == u64::MAX` disabled-reserve guard.
- **f64 in authoritative decisions** — all f64 hits in `arb-bot`/`arb-engine`
  are display/telemetry (lamport→SOL log formatting, ticker spot prices,
  in-flight share heuristics) or screening (`bellman_ford`); gates consume the
  integer `CostLedger` output. `strategy.rs:259` f64 is the config-boundary
  USD→lamport ceiling conversion (rounds down, then clamped by live Kamino
  liquidity and env caps).
- **Fixed SOL/USD price in profit conversion** — conversion uses the verified
  rational `lamports_to_base_mint_rate(_denom)` from `EconomicConfig` bound to
  a fresh price observation; zero/invalid rates fail closed.
- **Fixed Jito tip in the profit gate** — tip comes from `EconomicConfig.jito_tip_lamports`
  (policy/market-derived); no constant tip is subtracted per-opportunity.

## 3. Regression evidence

- New typed-stage work in this branch adds
  `arb_types::opportunity_stage` unit tests and
  `arb-sim::opportunity_recorder::stage_derivation_tests` — covering that a
  non-positive net profit can never be recorded past `QUOTE_VALID` and that a
  failed simulation can never claim `SIMULATED` (task §12/§13 anti-inflation
  invariants).
- Existing suites proving the A-class constants: see
  `docs/active/audit/QUOTE_PARITY_REPORT.md` §1 for the per-venue list.

## 4. Unverifiable in this environment

- Whether the A-class layout offsets (finding #19) match the CURRENT deployed
  program versions on mainnet — decoder fixtures pin known-good snapshots; a
  live re-validation against freshly fetched accounts is required before any
  execution enablement (already a standing gate in `CURRENT_STATUS.md`).
- pump.fun graduation constants (finding #11) can drift if the protocol
  changes its config; re-verify against the live bonding-curve program before
  relying on graduation gating.
