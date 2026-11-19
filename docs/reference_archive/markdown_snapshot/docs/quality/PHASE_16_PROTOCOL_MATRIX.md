# Phase 16 — Protocol Integration Matrix

**Date:** 2026-11-13
**Method:** source inspection of `crates/arb-tx/src/protocols/*` and
`build_swap_hop_instruction`; test inventory; reconciliation against
`docs/architecture/PROTOCOL_COMPATIBILITY.md` (2026-11-08), `docs/PROJECT_STATE.md`,
ADR-0007, ADR-0008.

Legend: **Exec** = a swap instruction builder exists and is reachable from the
single `build_swap_hop_instruction` dispatch. **Quote** = there is a decoder /
quote model. **Live evidence** = independent on-chain or fork acceptance evidence
that this repository can reproduce. "Source-level" never means "production
accepted".

## 1. Instruction-builder coverage (VERIFIED by source)

`build_swap_hop_instruction` (`crates/arb-tx/src/protocols/swap_hop.rs`) has a
match arm for **every** protocol below. There are no hard-disabled / "not
enabled" arms left in the file. Missing account matrices fail closed with
`MissingExecutionAccounts`, never with placeholder accounts.

| Protocol | Program ID (mainnet) | Exec | Quote | Tests | Notes |
|---|---|:--:|:--:|---|---|
| Raydium AMM v4 | `675kPX9…1Mp8` | ✅ | ✅ | `test_raydium_v4_swap_instruction_v2`, `test_build_swap_hop_instruction_for_all_supported_protocols` | versioned (`swap_version`) |
| Raydium CP-Swap (CPMM) | `CPMMoo8…P1C` | ✅ | ✅ | `test_raydium_cpmm_swap_layout_and_accounts` | |
| Raydium CLMM | `CAMMCzo5…grWqK` | ✅ | ✅ | `test_raydium_clmm_swap_layout_and_accounts`, tick/boundary suites | Q64.64 ticks |
| Orca Whirlpool | `whirLbMi…` | ✅ | ✅ | `test_build_swap_hop_instruction…`, `orca_sdk_parity` | sqrt-price |
| Meteora DLMM | `LBUZKhRx…` | ✅ | ✅ | golden vectors, bin-array boundary suites | primary flash route |
| Meteora DAMM v2 | `cpamdpZ…` | ✅ | ✅ (state) | `swap_hop` arm | discriminator + vault mapping |
| Pump.fun | `6EF8rrec…` | ✅ | ✅ | `test_pump_fun_buy_and_sell_layout_and_accounts` | bonding curve |
| PumpSwap | `pAMMBay…` | ✅ | ✅ | `test_pump_swap_*`, V1-legacy + V2 (17-account) | `pool_version` selects layout |
| Raydium LaunchLab | `Lan…` | ✅ | partial | `swap_hop` arm | newest; least exercised |
| Jupiter | aggregator | ⚠️ adapter only | ✅ client/quote | `test_jupiter_quote_with_dexes` | **not wired into the supervisor**; ADR-0008 remains `proposed` |

## 2. Documentation drift reconciled

| Document | Claim | Actual state | Action |
|---|---|---|---|
| `docs/architecture/PROTOCOL_COMPATIBILITY.md` (2026-11-08) | "Pump.fun / PumpSwap fail closed" (disabled) | Both have full builders and are reachable | **Stale.** Superseded by this matrix |
| `docs/PROJECT_STATE.md` | Pump.fun/PumpSwap verified | Matches source | Consistent with source |
| `ADR-0007` | PumpSwap adapter "not enabled", status `proposed` | Adapter enabled in `swap_hop.rs`; ADR never accepted | **Stale status**; code is ahead of the ADR |
| `ADR-0008` | Jupiter live path disabled | Still true: no Jupiter in supervisor | Consistent |
| Source comments | `arb-tx/src/lib.rs` UnsupportedProtocol for PumpSwap | No such arm remains | Removed/stale comment |

**Important:** "reachable builder" ≠ "live-accepted". No protocol in this
repository has a reproducible mainnet fork-acceptance record for every venue.
The only live mainnet evidence on file is a **simulate-only** V1 probe
(`reports/v1_mainnet_probe/probe_report_2026-11-13.json`) and it does not prove
each adapter's swap executes.

## 3. Regression surface

PR #13 changed **no** file under `crates/arb-tx/src/protocols/`, `flash/`,
`transport/`, or `crates/arb-bot/src/supervisor/`. The only execution-adjacent
change is `profitability_gate.rs` (F-1: reject structural loss), which is
strictly more conservative. Therefore the protocol matrix above describes the
same code as `main`.

## 4. Per-venue acceptance checklist (remaining work)

For each venue to move from "source-level" to "accepted", all of:

1. Differential quote vs official SDK on real pool state (tolerance-bounded).
2. Fork/litesvm `simulateTransaction` with real account data and a funded
   simulation payer.
3. Program-log confirmation that the target program was invoked.
4. Signed V1 transaction within SIMD-0385 (≤64 addresses) and ≤4,096 bytes.
5. Recorded evidence artifact committed under `reports/phase16/`.

Only Meteora DLMM currently has structural + size evidence; it still lacks
items 1–3 for a real route.
