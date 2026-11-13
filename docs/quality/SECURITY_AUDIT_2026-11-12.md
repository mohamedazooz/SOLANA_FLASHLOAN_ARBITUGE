# Security Audit Report — Three-Pool Meteora DLMM / Kamino Flash-Loan Arbitrage Engine

**Date:** 2026-11-12
**Scope:** full workspace at main `7c14cc9` (post PR #3 + PR #4 merges)
**Method:** source-level review of the live execution pipeline (discovery → accounting → gating → transaction build → V1 compile → simulation → dispatch → settlement), dependency/usage grep sweeps, adversarial and differential test-suite review, and verification runs on the audit branch.
**Safety posture during audit:** `ARB_LIVE=0`, `SOLANA_TX_V1_DIRECT_ENABLED=false`, `SOLANA_TX_V1_JITO_ENABLED=false` — no mainnet broadcast, no live signing, no Jito submission at any point.

---

## 1. Executive summary

The canonical pipeline is a single, connected path (see
`docs/PIPELINE_AUDIT_2026-11-12.md` §1 for the verified call map). All
critical and high-severity issues identified in the three audit rounds are
fixed and covered by tests. Two findings remain open by explicit decision
(O-16 provider degradation — infrastructure decision; directive gap register —
evidence items that cannot be produced without live/shadow soak time).

**Verdict: DEVELOPMENT COMPLETE.** Not SHADOW VALIDATED: no continuous shadow
soak or mainnet read-only simulation evidence has been produced yet. Live
remains blocked.

---

## 2. Verified security controls

### 2.1 Atomicity and capital safety
| Control | Location | Status |
|---|---|---|
| Borrow + 3 swaps + repay in ONE transaction; any failure reverts atomically | `arb-tx/src/lib.rs` `assemble_flash_arbitrage`; `dlmm_3leg_flash_arb.rs` | ✅ tests (`three_pool_flash_arb_tests.rs`, 17 tests) |
| Repayment = principal + real Kamino fee (`FeeCalculation::Exclusive`, round-half-away-from-zero, 1-unit floor) | `arb-tx/src/kamino.rs` `calculate_kamino_flash_fee` | ✅ canonical, single source |
| Repayment-guaranteed gate (conservative slippage floor must cover `size_in + flash_fee`) | `three_leg_meme_pipeline.rs` `RepaymentNotGuaranteed`; `run.rs` `validate_economic_invariants` | ✅ dual gate (pipeline + supervisor) |
| Actual supply-vault liquidity decoded fail-closed (mint match, u64 LE @64) | `arb-tx/src/kamino.rs` `decode_supply_vault_liquidity` | ✅ 4 tests |
| Borrow ceiling enforced against `kamino_available_liquidity` | `three_leg_meme_pipeline.rs` `InsufficientFlashLiquidity` | ✅ |
| No owned-capital fallback anywhere in the live path | `crates/arb-execution/` (owned-capital dispatcher) **deleted** (O-4) | ✅ removed |

### 2.2 Token and account safety
| Control | Location | Status |
|---|---|---|
| Token-2022-aware intermediate ATA derivation (`get_associated_token_address_with_program_id` per `hop.token_programs`) | `arb-tx/src/lib.rs:3169-3190` | ✅ (F1) |
| Settlement ATA derived with the correct token program | `dlmm_3leg_flash_arb.rs:367` | ✅ (F2) |
| Guarded CloseAccount loop: pre-existing payer ATAs excluded via RPC probe; RPC failure skips ALL closes (fail-safe) | `supervisor/execution.rs` `preexisting_intermediate_atas`; `assemble_flash_arbitrage(skip_close_intermediate_mints)` | ✅ (O-2) |
| Transfer-fee pools rejected before quoting (no BPS approximation) | `meteora_quote_engine.rs:108-139` | ✅ |
| Idempotent ATA creation (no failure on re-run) | `build_create_ata_idempotent[_for_program]` | ✅ |

### 2.3 Economic integrity
| Control | Location | Status |
|---|---|---|
| Single authoritative accounting model (CostLedger / EconomicQuote / UnifiedEconomicCalculator, integer) | `arb-math::economic` | ✅ |
| Capture carries canonical NET profit (not gross) | `run.rs` capture (O-1) | ✅ |
| No double-counted DEX fees (fees embedded in DLMM quote output) | `arb-math/src/dlmm.rs:680-720` | ✅ (F5) |
| f64 used only for initial screening; pricing/sizing/gating integer-only | `bellman_ford.rs` `evaluate_cycle` (documented, O-5) | ✅ (documented) |
| Implausibility gate (ROI ceiling) kills phantom quotes | `run.rs` `ARB_MAX_IMPLAUSIBLE_ROI_BPS` | ✅ |
| Min-profit floor + tip < net gate | `validate_economic_invariants` | ✅ |

### 2.4 Operational safety
| Control | Location | Status |
|---|---|---|
| Live mode requires explicit `ARB_LIVE`; default shadow/simulate | `main.rs`, `.env.example` | ✅ |
| `BYPASS_RISK_GATE` ignored in live mode + `warn!` on activation | `run.rs` (O-6) | ✅ |
| Kill switch: durable file marker, admin API loopback-only, bearer/X-token auth | `arb-risk`, `main.rs` admin server | ✅ |
| Mandatory RPC preflight simulation before dispatch; cannot be disabled by env | `dispatch.rs` (`sim_enabled = true`) | ✅ (O-8 comment corrected) |
| V1 transport opt-in per endpoint, default off; V0 ≤ 1,232 B; V1 ≤ 4,096 B; no silent downgrade | `adaptive.rs`, `V1_ONLY_RELEASE_POLICY.md` | ✅ |
| Split-route legs verified to trade the first-hop mint pair (registry check, fail-closed) | `run.rs` (O-14) | ✅ |
| Settlement reporter fails closed on missing hops (`let-else`) | `run.rs` (O-17) | ✅ |
| Duplicate-send protection, blockhash freshness, bounded retries, landing reconciliation (never "RPC accepted = landed") | `dispatch.rs`, `arb-land` | ✅ |
| Dead credential keys removed from `.env.example`; `SENTINEL_ADMIN_KEY` documented (read via `token_env`) | `.env.example` (O-10) | ✅ |
| Guard scripts refuse env files containing private keys / live flags for shadow | `run-shadow.sh:14`, `verify-shadow-package.sh:27` | ✅ |
| CI pinned to repo toolchain (`1.99.1`) — no stable-drift lint divergence | `.github/workflows/rust-ci.yml` (O-11) | ✅ |

---

## 3. Residual risks (accepted / documented)

| # | Risk | Severity | Mitigation / status |
|---|---|---|---|
| R-1 | Ingest provider degradation: all configured WS endpoints failed in the last live probe; system runs HTTP-polling only → higher discovery latency, weaker slot freshness | Medium | Freshness gates fail closed (`max_state_age_slots`); O-16 open — requires either real Yellowstone gRPC credentials or an operator decision to accept HTTP-polling |
| R-2 | Pre-existing intermediate ATA with external balance: closes are skipped (rent stays parked) — no capital loss, but the account is not reclaimed | Low | O-2 design; dust from a favorable DLMM rounding would revert the tx atomically (no loss) |
| R-3 | `dlmm_3leg_flash_arb::build_flash_funded_three_pool_transaction` is a reference builder, not wired into dispatch (live path uses `assemble_flash_arbitrage`) | Low | Explicitly documented in the module header (O-3); wiring requires per-hop live bin-array discovery |
| R-4 | No final-hop profit-floor instruction (per-hop `min_amount_out` is a slippage bound, not a profit floor) | Medium | On-chain Kamino repay fails if the balance is short → atomic revert, no capital loss; a profit-floor needs quote-API support (documented in `run.rs`) |
| R-5 | Durable execution reconciliation after restart is not implemented; a bundle in flight during restart must be checked manually | Low | Documented in runbook §7 |
| R-6 | Directive gap register outstanding: 3-pool SDK differential tests, property/fuzz suites, P50/P95/P99 benchmark report, mainnet read-only simulation evidence, shadow soak | Medium | Blocks SHADOW VALIDATED / PRODUCTION READY; tracked in `docs/PIPELINE_AUDIT_2026-11-12.md` §3 |

---

## 4. Secrets and repository hygiene

- No private keys, seed phrases, `.env`, or API credentials are committed. `.env.example` contains `REPLACE_ME` placeholders only.
- `OPERATOR_PRIVATE_KEY`, `AQ_LIVE_*`, `JWT_SECRET`, `EXECUTION_ADMIN_KEY`, `PUMP_PORTAL_API_KEY` were removed from `.env.example` after grep-verified zero code readers; `SENTINEL_ADMIN_KEY` retained (resolved at runtime via `config.toml` `token_env`).
- Admin API binds to loopback and is disabled unless the admin key is non-placeholder.
- Shadow guard scripts actively reject env files containing wallet keys or live-execution flags.

## 5. Test evidence (actually executed on the audit branch)

- `cargo check --workspace --all-targets` — 0 errors.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.
- `cargo test --workspace --all-targets` — **903 passed / 0 failed** across 64 suites (54 tests removed with the intentionally deleted `arb-execution` crate; 8 with never-compiled orphan files; no unintended test loss — attribute counts reconciled).

## 6. Final verdict

**DEVELOPMENT COMPLETE.**

Path to SHADOW VALIDATED: run the shadow package (`./run-shadow.sh`) ≥ 24–72 h on valid provider credentials, produce the soak report, and execute the mainnet read-only simulation evidence run. Path to PRODUCTION READY additionally requires the gap-register items (differential/fuzz/benchmarks) and an operator release review per `docs/RELEASE_READINESS.md`.
