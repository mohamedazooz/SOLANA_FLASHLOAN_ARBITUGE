# EXTERNAL_BEST_PRACTICES — Protocol Research Register

**Mission**: Atomic Profitability & Execution Engineering
**Research date**: 2026-11-21
**Method**: Every finding below was checked against the cited URL during this session. Only findings actually observed in the cited source are recorded; items not inspected are explicitly marked `NOT INSPECTED`. No open-source implementation was copied into production.

---

## 1. Solana Transaction V1 (SIMD-0385 / SIMD-0296)

| Item | Finding | Checked |
|---|---|---|
| Spec status | **Activated** | [proposals/0385-transaction-v1.md](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md) (raw fetched 2026-11-21) |
| Activation | Mainnet epoch 1035, activation slot 447,120,000, ~2026-09-15T01:00 UTC | [solana.com/upgrades/larger-transaction-sizes](https://solana.com/upgrades/larger-transaction-sites) via [solana.com upgrade page](https://solana.com/upgrades/larger-transaction-sizes), 2026-11-21 |
| Wire format | VersionByte `129` (0x81); LegacyHeader (3 bytes); TransactionConfigMask (u32 LE); LifetimeSpecifier (32 B); NumInstructions (u8); NumAddresses (u8); Addresses (inline 32 B each); ConfigValues (4 B per set bit); InstructionHeaders (4 B each: program index u8, account count u8, data len u16 LE); contiguous payloads; **trailing signatures** | SIMD-0385 spec text |
| Priority fee | Config mask bits **[0,1] both set**, value = 8-byte LE u64 = **absolute total in lamports** (NOT micro-lamports/CU). If only one bit is set the transaction is invalid. | SIMD-0385 |
| Unset resource requests | Priority fee 0; **CU limit 0 (transaction fails immediately)**; loaded-accounts-data limit 0 (cost model reserves 32 KiB page); heap 32 KiB | SIMD-0385 |
| Heap size | MUST be a 1 KiB multiple in [32 KiB, 256 KiB]; otherwise sanitization failure | SIMD-0385 |
| Limits | 4,096-byte size, 12 signatures, **64 accounts**, **64 instructions**, 255 accounts/ix | SIMD-0385 constraints table |
| ALTs | **Not supported in v1** — all addresses inline (32 B each) | SIMD-0385 §Detailed Design |
| ComputeBudget instructions | Ignored for configuration in TxV1 (processed as successful no-ops that still consume 150 CU) → must be stripped from V1 messages and any instruction-index-dependent CPI data (e.g. klend `borrow_instruction_index`) rewritten against the executed list | SIMD-0385 + [solana.com v1 announcement](https://solana.com/news/transaction-v1-and-the-alt-trade-off) |
| Account headroom | 64-account limit "may remain the binding constraint" for multi-pool routes even inside the 4,096-byte envelope; dense ALT users gain the least | ALT trade-off analysis (Dune sample, 2026-08-17) |
| Draft SIMD-0596 | Would raise the account limit to 96 — **not activated**; do not code against it | solana.com upgrade page (2026-11-21) |

**Implementation decision**: The repository's `crates/arb-tx/src/transport/adaptive.rs` V1 compiler matches every point above (verified line-by-line this session). No change made. The `jitodontfront` MEV-protection marker (rebates nothing; blocks sand win) was noted from Jito docs but is deliberately **not** adopted without an explicit operator decision.

## 2. Kamino (Klend) Flash Loans

| Item | Finding | Checked |
|---|---|---|
| Program ID | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (mainnet) | [Kamino architecture docs](https://www.mintlify.com/kamino-finance/klend/concepts/architecture), 2026-11-21; matches `crates/arb-constants/src/lib.rs:463` |
| Flash-loan flow | Build `flash_borrow` + custom instructions + `flash_repay` (kamino.com docs tutorial); `borrow_instruction_index` **specifies the position of the flash borrow instruction in the final transaction** → any V1 ComputeBudget stripping must rewrite it | [kamino.com/docs/build/tutorials/borrow/flash-loans](https://kamino.com/docs/build/tutorials/borrow/flash-loans), 2026-11-21 |
| Rust client | `klend-interface = "0.6.0"` with `solana-pubkey = "2.1"`, `solana-sdk = "~2.3"`; zero-copy account deserialization; `instructions_sysvar` is required (unchecked account validated by address to `solana_program::sysvar::instructions::ID`) | [docs.rs/klend-interface](https://docs.rs/klend-interface/latest/klend_interface/), 2026-11-21 |
| Fee semantics | Sample community measurement: USDC reserve flash fee 0.001% (rate stored as U68F60 sf) — repo's `max(amount·sf, 2^60)` + ties-away rounding preserved | community tooling (30daysmoney PR #14) — classification: PARTIALLY VERIFIED, repo oracle tests are the binding check |
| Restrictions / sysvar scan | Flash-repay scans **top-level** instructions for the borrow CPI; CPI-nested borrows can evade the scan — repository must never rely on this as a feature (exploit research, not a pattern to copy) | [adpthegreat/bypassing-kamino-restrictions](https://github.com/adpthegreat/bypassing-kamino-restrictions), 2026-11-21 |

**Implementation decision**: Keep the existing canonical kamino fee implementation (`arb-math/src/economic.rs`) and the borrow-index rewrite in `compile_v1_message`. No protocol defect found; Kamino integration preserved per mission rule 7.

## 3. Jito Block Engine

Source: [docs.jito.wtf/lowlatencytxnsend](https://docs.jito.wtf/lowlatencytxnsend/) (fetched 2026-11-21; page dated 2026-11-06).

| Item | Finding |
|---|---|
| Bundles | Max **5 transactions**, executed sequentially & atomically in one slot; all-or-nothing |
| Auction | Parallel auctions at **50 ms ticks**; bundles prioritized by tip/CU efficiency; highest paying combination submitted to validator |
| Minimum tip | **1,000 lamports** (block-engine enforced). The repo default floor (10,000 lamports, `arb-constants`) is stricter — safe direction. Comment now clarified. |
| Tip accounts | 8 accounts, constant (see docs page). Repo's `JITO_TIP_ACCOUNTS` matches all 8 exactly |
| Tip placement | Tip = any instruction transferring SOL to one of the 8 tip accounts (top-level or CPI); random account selection recommended to reduce contention. **Warning: tip txs without ALTs** for validators not running Jito-Solana... (docs: tips must avoid ALT resolution path; repo uses inline accounts in V1 → compliant) |
| Status lifecycle | `getInflightBundleStatuses` (≤5 ids, 5-minute lookback): Failed / Pending / Landed / Invalid; `getBundleStatuses` returns slot + confirmation_status; idempotency via signature-status checks before any resubmit |
| Rate limits | Default 1 request/s/IP/region; 429 on exceed → repo's bounded retry policy is aligned (wait ≥1 s, single retry surface) |
| Regions |<mainnet block engines: global, amsterdam, dublin, frankfurt, london, ny, slc, singapore (shred receivers + NTP hosts per region) |
| `sendTransaction` path | Proxies directly to validator with MEV protection, `skip_preflight=true` always — pre-flight simulation must therefore be done by the bot itself (matches repo's mandatory arb-sim gate) |
| Sandwich mitigation | `jitodontfront`-prefixed read-only account accepted as bundle index-0 marker (not adopted) |

**Implementation decision**: Dynamic tip bounded by `ExpectedNetProfitAfterTip >= RequiredProfitFloor`; no fixed gross-percentage tip change made this session (existing share caps 70/85 bps-of-edge defaults retained and explicitly documented as defaults, not enforced constants).

## 4. Meteora DLMM

| Item | Finding | Checked |
|---|---|---|
| Official TS SDK | `@meteora-ag/dlmm` latest npm version **1.9.14** (published ~2026-07) | [npm](https://www.npmjs.com/package/@meteora-ag/dlmm), 2026-11-21 |
| Differential oracle pin | The repo's differential harness pins exactly `@meteora-ag/dlmm@1.9.14` over a captured mainnet snapshot — the pin is still current | `scripts/differential-harness` + npm, 2026-11-21 |
| DLMM Pro | Separate `dlmm-pro-sdk` (TS + Rust commons) exists — the repo's `dlmm_pro.rs` adapter tracks that protocol; parity testing for DLMM Pro is **NOT INSPECTED** this session | [github.com/MeteoraAg](https://github.com/MeteoraAg), 2026-11-21 |

**Implementation decision**: The layer-match source (`LayerFill`/`BinFillResult`, `Rounding::Up` full-layer input / `Rounding::Down` partial output) in `crates/arb-math/src/dlmm/swap.rs` mirrors the official SDK `FillResult` semantics and remains the parity reference. No change.

## 5. Raydium / Orca

- **NOT INSPECTED in depth this session** (points consumed by the release that were revalidated only at the math level via repository oracle vectors).
- Standing rule remains: Raydium CPMM fee rounding (`ceil` fee upfront, `floor` output), CLMM Q64.64 tick math, and Orca Whirlpool account ordering may only be modified with a fresh differential capture against `raydium-io/raydium-clmm`/`orca-so/whirlpools` program logic — capture requires network egress (BLOCKED here; command in `QUOTE_PARITY_REPORT.md`).

## 6. Testing Infrastructure

| Item | Finding | Checked |
|---|---|---|
| LiteSVM | In-process SVM used by the repo's sim strategy (arb-sim integration) — NOT INSPECTED at upstream this session | — |
| Mollusk | Anza test framework alternative — NOT INSPECTED | — |
| Differential harness | Repo's own JS harness captures byte-identical mainnet snapshots and SDK quotes → used as the SDK oracle for DLMM parity | repo source, this session |

## 7. Anti-hallucination ledger

Assertions in code comments/docs that could NOT be independently re-verified this session and remain classified `PARTIALLY VERIFIED`:

1. `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 64 MiB` justification cites a 2026-11-17 live Agave 4.4.1 measurement (10,714,291-byte klend spine) — plausible and conservative, but not re-measured here (no network).
2. Oracle test vectors in `audit/test_vectors/` are generated by the in-repo Python oracle — that oracle derives from protocol specs, not from deployed program execution. DLMM parity against the **official SDK** is covered only by the captured-snapshot differential; a broader parameter sweep against the deployed programs is a future capture task.

---

# Session 2 — Open-Source Sweep Applied to `arb-executor` Tip Configuration (2026-11-21)

Scope: evaluate (vs. in-repo native implementation) the frameworks and payment-flow
conventions used by production on-chain MEV programs before implementing the verified
best-practice change: an on-chain conditional Jito tip instruction
(`execute_three_leg_with_tip`, new variant `[124, 34, 60, 60, 78, 94, 133, 177]`).

## S2.1 Program frameworks (`anza-xyz/pinocchio`)

| Item | Finding | Checked |
|---|---|---|
| Project | `pinocchio` v0.11–0.12, maintained under `anza-xyz`; "no external dependencies", zero-copy, "optimized in terms of both compute units consumption and binary size"; `no_std`; `lazy_program_entrypoint!` for on-demand input parsing; companion crates `pinocchio-system`/`pinocchio-token` 0.5.0; upstream BPF target needs `solana-define-syscall` with `unstable-static-syscalls` | [raw README](https://raw.githubusercontent.com/anza-xyz/pinocchio/master/README.md), 2026-11-21 |
| Applicability | Migrating `arb-executor` would change program ID/wire-independent behavior for an already-met goal. The repo already implements pinocchio-class patterns natively on the Agave 3.x split crates (`solana-address`/`solana-account-info`/`solana-cpi` with `default-features = false`, zero-copy `64..72` balance reads, no Anchor). A migration buys no measured CU/binary win and adds supply-chain + feature-flag risk | repo diff + pinocchio docs, 2026-11-21 |

**Implementation decision**: NO framework migration. `arb-executor` stays native
zero-copy on split Agave crates (same patterns pinocchio packages), preserving the
verified program identity and wire formats.

## S2.2 Jito conditional-tip flow (open-source searcher references)

| Item | Finding | Checked |
|---|---|---|
| Official examples | `jito-labs/searcher-examples` builds the tip as a plain `system_instruction::transfer` to one of the canonical tip accounts and sends bundles via `send_bundle_with_confirmation` | [repo](https://github.com/jito-labs/searcher-examples), [cli/src/main.rs](https://github.com/jito-labs/searcher-examples/blob/master/cli/src/main.rs), 2026-11-21 |
| Tip placement | The tip must be present in the **last transaction of the bundle**; SOL transfer without ALT indirection; min 1,000 lamports (block-engine enforced) | [QuickNode guide](https://www.quicknode.com/guides/solana-development/transactions/jito-bundles), [dev.to bundle guide](https://dev.to/mohammedalabd/atom-transactions-using-jito-bundles-1m78), Jito docs (Session 1), 2026-11-21 |
| Min-tip reality check | Practitioner reports: "Bundle must tip at least 1000 lamports" — confirmed as block-engine validation, below-auction tips simply do not land | [zhe-t gist](https://gist.github.com/zhe-t/60938c69e29276b7a9f098e1b0672c79), 2026-11-21 |

**Implementation decision**: Conditional tip implemented **on-chain** (not off-chain
guessing): `execute_three_leg_with_tip` transfers `floor(gross_edge * tip_bps / 10_000)`
lamports from the signing user to a canonical tip account only after the canonical
three-leg body succeeds, guarded by (a) canonical tip-account validation,
(b) WSOL-mint dimensional guard (WSOL 1:1 lamport peg), (c) retained-edge floor
`gross_edge - tip >= min_final_wsol_delta - leg1_amount_in` so the tip can never
consume the flash-loan cost headroom or the operator's stipulated minimum profit.
Because it is the final instruction in the arbitrage transaction and Jito bundles are
all-or-nothing, a losing or under-profit execution never pays a tip.

## S2.3 Meteora DLMM CPI reference (unadopted, corroborating)

| Item | Finding | Checked |
|---|---|---|
| Official CPI guide | `docs.meteora.ag/developer-guides/dlmm/rust-integration/cpi.md`: call `lb_clmm` from Rust programs with generated bindings, event-CPI accounts, remaining accounts (bin arrays), and validation | [docs.meteora.ag llms.txt index](https://docs.meteora.ag/llms.txt), 2026-11-21 |
| `swap2` discriminator | third-party `solroute` pins `Swap2IxDiscm = [65, 75, 63, 76, 235, 91, 91, 136]` — byte-identical to the repo's `METEORA_SWAP2_DISCRIMINATOR` in `arb-executor/cpi/meteora_dlmm.rs` | [solroute meteora pkg](https://pkg.go.dev/github.com/solana-zh/solroute/pkg/pool/meteora), 2026-11-21 |

**Implementation decision**: No change to the swap2 CPI layer (already matches).

## S2.4 Anti-hallucination ledger (Session 2 additions)

1. Pinocchio's CU/binary advantages are VERIFIED claims from its docs for **programs
   that use it**; the assertion that `arb-executor` gains nothing from migration is an
   ENGINEERING JUDGMENT (PARTIALLY VERIFIED) — a CUs-delta would need a `cargo
   build-sbf` + on-chain/CU-measurement harness, which this sandbox cannot run
   (no SBF toolchain guarantee; command: `cargo build-sbf -p arb-executor`).
2. The `execute_three_leg_with_tip` tip economics assume `min_final_wsol_delta >=
   repay (principal + fee)` as documented in `FLASHLOAN_EXECUTION_REPORT.md`; the
   on-chain retained-edge floor uses exactly that operator stipulation.
