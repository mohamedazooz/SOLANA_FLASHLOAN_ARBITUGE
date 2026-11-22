# HFT Best Practices and Remediation Spec — CORRECTED, Evidence-Based (2026-11-22)

Label legend: `SOURCE_VERIFIED` (official docs/source), `REPOSITORY_VERIFIED` (inspected at HEAD `0f0707c`), `TEST_VERIFIED` (executed), `INFERRED`, `NOT_VERIFIED`.

## 1. Executive Summary

This revision replaces the proposed (and in-repo absent) remediation spec with an evidence-based one. Every requested finding was re-classified against HEAD. Result: **no new confirmed P0 code defective paths were found today**; the register's P0s (ECON-002, QUOTE-001, QUOTE-002) are `ALREADY_FIXED` with regression tests, and the SIMD-0385 priority-fee + DLMM traversal + bitmap-extension concerns are `ALREADY_FIXED` at HEAD with exact-source implementation discipline. The remaining open code work is CPI-003/CPI-001 `Phase 1C` parity closure, and the operator-gated V1 enablement decision — **not silently auto-enabling V1**, because mainnet activation Itself does not retire the repo's own stated gate (live on-cluster landing observation).

## 2. Findings Classification (authoritative table)

| Finding | Status | Actual file:function | Root cause / verification at HEAD | Correct remediation | Regression tests | Priority | Implemented |
|---|---|---|---|---|---|---|---|
| CPI-003 repayment economic risk | PARTIALLY_FIXED (register: CONFIRMED ECONOMIC RISK pending Phase 1C sign-off) | `crates/arb-tx/src/flash/kamino.rs:196-217` `calculate_kamino_flash_fee_checked` | fee is exact klend U68F60 `FeeCalculation::Exclusive`; disabled reserve → `u64::MAX` fail-closed; repay `amount` argument correctly carries principal only (fee collected at repay handler on top) `REPOSITORY_VERIFIED` | complete Kamino clone parity (Phase 1C) + keep checked helper | `test_calculate_kamino_flash_fee_checked_guards` at `kamino.rs:685` `TEST_VERIFIED`; 1,626/0 workspace | P2 | fee math: FIXED; parity sign-off: OPEN |
| CPI-001 source-path parity | NOT_REPRODUCIBLE as-stated (INTEGRATION GAP, OPEN) | register line 17 | no code defect demonstrated yet | Phase 1C parity audit | — | P2 | OPEN |
| DLMM-003 traversal | ALREADY_FIXED | `crates/arb-dex/src/dlmm_validation.rs:87` / `crates/arb-tx/src/protocols/meteora_accounts.rs:83` `bin_array_index_for_bin` | `(i64::from(bin_id)).div_euclid(70)` — correct floor division for negative IDs; array count derived from first/last bin, not naive `(bins_traversed/70)+1` `REPOSITORY_VERIFIED` | none (matches official math) | `dlmm_bin_discovery.rs` typed traversal errors (`ZeroTraversalDepth`, `BinArrayNotLoaded`, per-swap bin limit) `REPOSITORY_VERIFIED` | — | FIXED (verify) |
| DLMM-NEW-003 bitmap extension | ALREADY_FIXED | `crates/arb-tx/src/protocols/meteora_accounts.rs:37` + `crates/arb-constants/src/lib.rs:359` | PDA `[b"bitmap", lb_pair]` matches MeteoraAg/dlmm-sdk commons `pda.rs`; requirement rule = outside default window `[-512, 511]` (not `indices.len() > 1`) `SOURCE_VERIFIED` + `REPOSITORY_VERIFIED` | none | `dlmm_bin_discovery.rs` invariants `REPOSITORY_VERIFIED` | — | FIXED (verify) |
| DLMM-NEW-002 token-2022 transfer hook | NOT_REPRODUCIBLE → tracked token-2022 policy | none at drafting | repo policy = reject-unusual-token-program pools (`has_risky_token`), hooks not supported | keep reject-fail-closed; extend to explicit hook rejection at API | QUOTE-001 reject tests `TEST_VERIFIED` | P3 | policy present; explicit hook surface minor |
| ECON-002 profit-guard units | ALREADY_FIXED | `arb_math::economic::slippage_haircut_min_final` + `arb-engine` constructors | min-final now gets compound per-hop haircut, activates supervisor's conservative repayment gate `REPOSITORY_VERIFIED` | none (fix already canonical integer-only) | `arb-math` golden vector + haircut carry test `TEST_VERIFIED` | P0(re) | FIXED |
| QUOTE-001 transfer fees | ALREADY_FIXED | `meteora_quote_engine.rs` reject gates | reject any `transfer_fee_bps != 0` / `has_risky_token` (stricter-than-proposal) `REPOSITORY_VERIFIED` | none | `meteora_quote_engine_reject_tests.rs` (200 lines) `TEST_VERIFIED` | P0(re) | FIXED |
| QUOTE-002 dynamic volatility | ALREADY_FIXED | `meteora_quote_engine.rs` gates | reject `status != 0`, `function_type > 2`, `collect_fee_mode > 1`; decoded values propagated (incl. OnlyY mainnet parity commit `78de69c`) `REPOSITORY_VERIFIED` + `SOURCE_VERIFIED` (vanilla=0/OnlyY=1) | none | reject tests status 1/2/7, collect_fee_mode 2/5/255 `TEST_VERIFIED` | P0(re) | FIXED |
| SIZE-001 f64 sizing | ALREADY_FIXED | pub `golden_section_size` deleted; live f64 remains only in production-unreachable modules (`three_pool_router.rs`, `pump_raydium_*` quarantined) | register line 46, caller search at HEAD | integer joint search + integer multi-basin in live path `REPOSITORY_VERIFIED`; full removal in quarantined modules deferred | pipeline integer-grid interior-peak test `TEST_VERIFIED` | P1 | FIXED (live path) |
| SIMD-0385 V1 + priority fee | ALREADY_FIXED (semantics) + OPERATOR-GATED (enablement) | `crates/arb-tx/src/flash/compiler.rs:1040-1055` + `crates/arb-tx/src/transport/adaptive.rs::total_priority_fee_lamports` | V1 priority fee encoded as **total lamports** (8-byte LE u64 mask 0..1), computed `ceil(µlamports_per_cu × cu / 1e6)` with checked math + fail-closed overflow — matching the activated runtime (epoch 1035, 2026-09-15, Agave 4.2+) `SOURCE_VERIFIED` (SIMD-0385 spec; solders#187 empirical 1:1 lamports; ComputeBudget ixs become no-ops) + `REPOSITORY_VERIFIED` | V1 keep env-gated `SOLANA_TX_V1_DIRECT_ENABLED` / `SOLANA_TX_V1_JITO_ENABLED` (default OFF) until live on-cluster landing observation (repo gate TXC-F-N1); RPC nodes must be ≥ Agave 4.2.2 or v1 silently downgrades to v0 with zeroed compute budget | Phase-5 compiler suite 434/0 `TEST_VERIFIED` | P3 | code FIXED; production enablement NOT done (operator approval required) |
| Kamino atomicity/repay | ALREADY_FIXED (part) + OPEN (parity) | `kamino.rs:196-198` repay semantics | repay amount argument = principal only; fee collected at repay handler on top — matches klend handler `SOURCE_VERIFIED` (klend flash_borrow_reserve_liquidity/flash_repay) | Phase 1C clone parity closure | discriminators + layout + fee-guard tests `TEST_VERIFIED` | P2 | partial |
| Jito landing | NOT_VERIFIED (behaviorally) | `crates/arb-land/*` audited Phase 8 | landing rates/posteriors unpopulated (no mainnet evidence) — **do not claim any tip amount guarantees inclusion** | 24h+ shadow soak to populate per-rung Beta posteriors | arb-land 68/0 `TEST_VERIFIED`; landing metrics `NOT_VERIFIED` | P3 | code audited; metrics pending |

## 3. Verified Protocol Facts (this session, `SOURCE_VERIFIED`)

1. **SIMD-0385/TX-V1 activated on mainnet** at epoch 1035, 2026-09-15 (~01:00 UTC); feature gate `txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL`; size 1,232 → 4,096 bytes (SIMD-0296). Agave 4.2.2+ required on RPC nodes (0.4.2‑pre nodes silently downgrade v1→v0 with zeroed compute budget). Slot time 300ms since epoch 1024 (2026-08-28), SIMD-0525 stepping to 250/200ms.
   - https://solana.com/upgrades/larger-transaction-sizes (retrieved 2026-11-22)
   - https://github.com/solana-foundation/solana-dev-skill — `references/transactions-v1.md`
   - https://github.com/kevinheavey/solders/issues/187 (priority_fee = total lamports, empirical)
   - https://www.helius.dev/blog/agave-4-2-migration-checklist
2. **Meteora DLMM**: BinArray PDA `[b"bin_array", lb_pair, index.to_le_bytes()]`, 70 bins/array; BinArrayBitmapExtension PDA `[b"bitmap", lb_pair]`, required (error 6036 `BitmapExtensionAccountIsNotProvided`) for arrays outside `[-512, 511]`.
   - https://docs.meteora.ag/developer-guides/dlmm/program/accounts (retrieved 2026-11-22)
   - https://github.com/MeteoraAg/dlmm-sdk (pda.rs commons; CHANGELOG bitmap-extension fixes)
3. **Kamino klend**: flash-loan fee collected at repay (`Exclusive`), U68F60 `Fraction` (`1<<60` bits), min fee 1 atomic unit, `BorrowTooSmall` guard — all reproduced in repo code and covered by tests.
   - https://github.com/Kamino-Finance/klend (handler semantics per repo trailing references)

## 4. Discrepancies & Disambiguation

- The task assumed a source spec exists in-repo. It does not (see baseline). Corrections were therefore built from the `docs/audit/MASTER_FINDINGS_REGISTER.md` which alرeady carries per-finding `REPOSITORY_VERIFIED`/`TEST_VERIFIED` rows.
- Raw `(bins_traversed/70)+1` formula: classified **INCORRECT_PROPOSED_FIX**, superseded by exact first/last-bin derivation at HEAD.
- Blind `ceil(cu × µlamports / 1e6)` "applies to V1": **PARTIALLY confirmed** — formula applies only as a conversion step into ONE total-lamports u64; v1 itself takes the total directly (no per-CU semantics). Repo already implements this exact model.

## 5. Implementation Status

- **Code changes in this PR:** NONE that alter production behavior. Every requested finding was either `ALREADY_FIXED`/`TEST_VERIFIED` at HEAD, or its remaining work is operator-gated (V1 enablement, mainnet-evidence-dependent Jito benchmarks, CPI parity Phase 1C sign-off). Fabricating a code change where the register proves none is needed would violate the mission's own rules.
- **Docs produced:** this corrected spec; `reports/audit/HFT_REMEDIATION_BASELINE.md`; `reports/audit/HFT_REMEDIATION_FINAL_REPORT.md`.

## 6. Remaining Blockers (operator decisions)

1. Phase 1C CPI-001/CPI-003 parity closure (audit task, code verification without changing behavior).
2. V1 production enablement: requires operator decision + live on-cluster landing observation (Agave ≥ 4.2.2 node evidence).
3. 24h+ shadow soak for Jito posteriors + latency distributions.
4. quarantine cleanup of production-unreachable f64 sizing modules (tracked; owner: later phase).
