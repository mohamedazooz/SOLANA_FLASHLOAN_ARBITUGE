# Solana Core / Agave Best Practices

## 1. Transaction V1 (SIMD-0385 / SIMD-0296)

| Item | Status | Source |
|---|---|---|
| SIMD-0385 spec | **Activated** (frontmatter `status: Activated`) | [proposals/0385-transaction-v1.md](https://raw.githubusercontent.com/solana-foundation/solana-improvement-documents/main/proposals/0385-transaction-v1.md) — raw spec fetched 2026-11-22 (S3) |
| Mainnet activation | Epoch 1035, slot 447,120,000, ~2026-09-15 01:04 UTC | [solanacompass](https://solanacompass.com/news/transaction-v1-simd-0385-is-live-on-solana-mainnet-at-epoch-1035), [xroot.dev gate tracker](https://app.xroot.dev/upgrades/simd-0385-transaction-v1) 2026-11-22 (S3) |
| Wire format | VersionByte `129` (0x81); LegacyHeader(3B); ConfigMask (u32 LE); LifetimeSpecifier (32B blockhash); NumInstructions (u8); NumAddresses (u8); inline Addresses; ConfigValues (4B per set bit); InstructionHeaders (4B each); payloads; **trailing signatures** | SIMD-0385 raw spec (S3 primary) |
| Priority fee | Config mask bits **[0,1] both set**; value = 8B LE u64 = **absolute total lamports** (not micro-lamports/CU). One bit alone = invalid. | SIMD-0385 raw spec (S3) |
| Unset requests | Priority fee 0; **CU limit 0 → transaction fails immediately**; loaded-accounts-data 0 → cost model reserves one 32 KiB page; heap 32 KiB | SIMD-0385 raw spec (S3) |
| Heap | Must be 1 KiB multiple in [32 KiB, 256 KiB], else sanitization failure | SIMD-0385 raw spec (S3) |
| Constraints | size 4096; signatures 12; accounts 64; instructions 64; accounts/instruction 255 | SIMD-0385 raw spec (S3) |
| ALTs | **Not supported in v1** — all 64 addresses are inline (32B each) | SIMD-0385 raw spec (S3) |
| ComputeBudget ixs | Ignored for configuration; processed as successful no-ops that still consume CU (even invalid ones do not fail the tx) → builders must strip them and rewrite index-dependent args (e.g. klend `borrow_instruction_index`) | SIMD-0385 raw spec (S3) |
| Validator requirement | Agave ≥ 4.2.2 / Jito-Solana ≥ 4.2.2 — older nodes silently downgrade v1→v0 with zeroed compute budgets | [solanacompass activation note](https://solanacompass.com/news/transaction-v1-simd-0385-is-live-on-solana-mainnet-at-epoch-1035) 2026-11-22 (S3) |
| Draft SIMD-0596 | 96-account v1 limit — **not activated**; do not code against it | register Session 1 (inherited) |

### Existing implementation
`crates/arb-tx/src/transport/adaptive.rs` V1 compiler (0x81, 4,096-byte ceiling, 64 accounts, explicit CU limit / loaded-data limit / total-lamports priority fee, CB stripping + borrow-index rewrite). Verified line-by-line in register Session 1; spec re-verified today against the raw proposal (S3).

### Gap / action / test
- No protocol gap found. `compile_v1_message` borrow-index rewrite is the regression-critical surface — keep the wire-offset tests (`crates/arb-tx/src/transport/`, `crate: arb-tx wire_parity`) as the binding check.
- REC (P3): read-side `maxSupportedTransactionVersion` handling must include `1` (already enforced in repo preflight); add a transport-compatibility note in `TRANSACTION_COMPATIBILITY_MATRIX.md` citing the Agave 4.2.2 requirement for RPC nodes.

## 2. Fees and compute budget

| Item | Status | Source |
|---|---|---|
| Base fee | 5,000 lamports per signature | Solana fee docs pinned to Agave 3.1.8, fetched 2026-10-13 via [eco.com summary](https://eco.com/support/en/articles/17593745-solana-transaction-fees-base-fee-priority-fees-and-compute-units) (S3) |
| Max CU | 1.4 M per transaction (legacy/v0); default 200,000 per non-builtin instruction | [rpcedge priority fees](https://rpcedge.com/blog/solana-priority-fees-explained) 2026-07-22 (S3) |
| Priority fee charged on **requested limit**, not consumed CU | fee = price × requested limit | same (S3) — REC: keep tight CU limits; repo sets explicit limits in V1 message ✓ |
| Scheduler ranking | reward × 1,000,000 / (cost + 1); loaded account data contributes to cost | [eco.com Solana fee summary](https://eco.com/support/en/articles/17593745-solana-transaction-fees-base-fee-priority-fees-and-compute-units) (S3) |
| Fee markets are local | estimate must sample the accounts actually written (writable, locked) | [rpcedge](https://rpcedge.com/blog/solana-priority-fees-explained) (S3) |
| CU limit sizing | simulate first, add ~10% margin, cap | Solana compute-budget guide via [rpcedge](https://rpcedge.com/blog/solana-priority-fees-explained) (S3) |

### Existing implementation
Repo simulates before send (`arb-sim` typed outcomes), sets explicit CU limits, and treats Jito tip and priority fee as separate cost lines in `arb-math/economic.rs`.

### Recommended action (REC)
- Keep priority-fee estimation per-account-cluster (local fee market) and cap it; document `transaction-profile.toml` as the current approximation. Test: golden test that net-profit gate uses `price × requested_limit`, never consumed CU.

## 3. Blockhash / lifetime / reconciliation

| Item | Status | Source |
|---|---|---|
| Blockhash validity | ≈ 150 slots; repo uses margin 160 before declaring expiry | inherited (register); consistent with Agave MAX_PROCESSING_AGE model |
| Documented clone-of-record | not from this session — no reliable primary citation found today | open item |

### Existing implementation
`crates/arb-bot/src/supervisor/recovery.rs`: signature persisted pre-send, pending until (a) signature status resolves or (b) block-height progression proves expiry; never resubmits a new signature (ISSUE-P1-02 fix).

### Test required
- Keep `dispatch` timeout-reconciliation regression tests as release gates (already exist in `arb-bot`).
