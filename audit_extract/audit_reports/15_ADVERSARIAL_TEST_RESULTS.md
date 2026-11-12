# 15 — Adversarial Test Matrix and Results

Every row lists: the adversarial case, whether a test exists in the repo, what it proved, and the
**gap classification**. "PASS" = test exists and passed in `test_audit3.log`; "FAIL" = test exists
and failed; "MISSING" = no test exists at all; "NOT RUN" = requires live RPC/broadcast (forbidden).
Line numbers: `audit_work/13`, `/02`, `/06`, `/09`.

| # | Adversarial case | Test | Result | Evidence / gap |
|---|---|---|---|---|
| 1 | Wrong token decimals | none crossing decimals boundary in golden vectors | MISSING | cycle_profit_at zeroes decimals; only unit-level decimals tests in dimensional_currency_tests |
| 2 | Cross-mint raw arithmetic | wrapper types enforce | PARTIAL (types) | strong types exist; call sites cast across units (run.rs:1048) — untested at that boundary |
| 3 | Inverted price conversion | none | MISSING | dimensional.rs vs economic.rs rounding disagreement untested |
| 4 | Stale conversion rate | none for quote price; freshness gates exist | MISSING | `EconomicQuote` uses wall clock; no price-slot assertion |
| 5 | Negative bin IDs | `dlmm_bin_array_boundary_tests.rs:46-62, 54-61` | PASS | div_euclid floor verified |
| 6 | BinArray boundary crossing (69↔70, -70↔-71) | same suite :65-286 | PASS | |
| 7 | BinArray boundary — instruction path | `meteora_bin_array_indices` (lib.rs:1059) — **zero tests** | MISSING | under-provisioning counter-examples 1-3 in 06 §4 unguarded |
| 8 | Incorrect direction | directional array tests :559-641 (2 cases) | PARTIAL | opposite-direction array rejection untested |
| 9 | Missing BinArray | none (needs sim) | MISSING | quote⇒instruction set-equality untested |
| 10 | Wrong BinArray order | none | MISSING | ordering sensitivity asserted nowhere |
| 11 | Bitmap extension missing (required but absent) | none end-to-end | MISSING | atomic builder passes None always — untested |
| 12 | Stale pool state | `run.rs:455/497` age filters; WS/HTTP stamping | FAIL (design) | stamps inflate to current slot ⇒ stale reads as fresh — adversarial input undetected |
| 13 | Incoherent multi-pool snapshot | slot_integrity tests exist; live path doesn't call gate | MISSING (runtime) | one fresh + one stale pool indistinguishable live |
| 14 | Incorrect ALT index | none on-wire | MISSING | filtered-subset bug proven by code reading only |
| 15 | Inactive ALT | `parse_alt_account_data`/`from_onchain_readonly` unit tests | PARTIAL | only via dead V0 path; live path never checks |
| 16 | Signer in ALT | `verify_no_signers` unit tests | PARTIAL | dead path again |
| 17 | Tip account in ALT | compiler.rs:1339-1344 asserts 8-in-ALT (by design!) | FAIL (design) | artifact contradicts policy; no rejection test |
| 18 | V0 transaction overflow | `letter_tests` boundary 1232 in three_leg_meme_compiler_tests.rs:628-645 | PASS (compiler) | live path can't emit V0 anyway |
| 19 | Unsupported V1 | v1_header tests (post-fix compile) | PARTIAL | unit-tested serialization; zero runtime support |
| 20 | Runtime account limit (64) | none | MISSING | u8 indices + silent skip at pos>255 |
| 21 | Wrong writable flag | fuzzing_templates / book tests | PARTIAL | no ALT-privilege-merge test at message level |
| 22 | Wrong token program (SPL vs 2022) | intermediate_balance_isolation_tests (model) | PARTIAL | 2022 ATAs vs SPL derivation untested E2E |
| 23 | Token-2022 transfer fee | standalone fee tests PASS; in-quote tests MISSING | FAIL (in-quote) | double-subscription bug untested |
| 24 | Intermediate ATA contamination | intermediate_balance_isolation_tests (builder only) | PARTIAL | flash path untested |
| 25 | Profit sent to wrong wallet | none — structurally impossible by current wiring | PASS (by inspection) | only payer is referenced as destination |
| 26 | Wrong repayment account | bundle_tests discriminator/account layout | PARTIAL | both borrow/repay accounts asserted |
| 27 | Incorrect flash-loan fee | economic unit tests (sf math) | PARTIAL | no fixture with sf != 0 through builder |
| 28 | Jito tip double deduction | bundle_tests / adversarial | PARTIAL | CU-price double-elevator untested |
| 29 | Priority fee double deduction | none | MISSING | `calculate_fee_deduplicated_cu_price` never called |
| 30 | Jito submission without landing | dispatch.rs Landed-only PnL | PASS (accounted) | profit counted only post-confirmation |
| 31 | Duplicate bundle submission | transport dedups by signature; cross-tick dedup absent | PARTIAL | different-tip resend same slot unbounded |
| 32 | Expired blockhash | none tracking lastValidBlockHeight | MISSING | only post-hoc Expired detection |
| 33 | Partial route failure | flash atomicity (Case 4 model test) | PARTIAL | F4/F5/F7 guaranteed-revert shapes untested E2E |
| 34 | Optimizer accounting mismatch | trade_size_optimization_tests (2 tests) | MISSING (adversarial) | no multi-basin fixture; no golden-section test |
| 35 | Suspicious ROI regression | none | MISSING | no regression gate on ROI jumps |

## Executed verification (this audit)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | **FAIL** (4 files) |
| `cargo check --workspace --all-targets` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** (v1_header transient compile errors; re-run arb-tx: 10 lint errors) |
| `cargo test --workspace` (attempt 1) | **Aborted: no space on device** |
| `cargo test --workspace --no-fail-fast` (after freeing 27.7 GB) | **683 passed / 1 failed / 3 ignored** (failure = debug perf threshold only) |
| `cargo test -p arb-tx --no-run` | PASS at 14:57 (after external fix to v1_header files, mtime 14:57) |

## Independent reference checks performed

- Transfer fee 1%: independent ceil computation ⇒ bug reproduced by reading (renderer does not lie).
- Bin index floor vs C trunc division ⇒ divergence proven (negative IDs).
- v0 wire: filtered-subset position ≠ on-chain position ⇒ proven by construction.
- `jitodontfront` 3-account tip ix vs v1_fallback 2-account strip predicate ⇒ never matches (proven).

## Verdict

**Adversarial coverage is not merely incomplete — the most financially dangerous classes
(Token-2022 fees in quotes, instruction-array span, canonical-ATA destruction, ALT index
resolution, coherent multi-pool state) have zero executable tests.** The suite that passes measures
the math core well; it does not measure the system that runs.
