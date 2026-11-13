# 08 — V0 / V1 Transaction Format Audit

Evidence: `audit_work/07_ALT_V0_V1.md`; official status in `14_EXTERNAL_RESEARCH_REFERENCES.md` §1-2.
Note: `cargo test -p arb-tx --no-run` completed at 14:57 UTC producing `v1_header_*` executables
(current tree); the workspace test run and clippy at 14:4x saw transient compile failures in those
two untracked files (fixed on disk mid-audit by an external process). Results recorded as observed.

## 1. Support matrix (as shipped, not as documented)

| Format | Code emits? | Runtime uses? | RPC/Jito path accepts? | SIMD-0385 reality |
|---|---|---|---|---|
| Legacy | `build_signed_transaction` (lib.rs:3471 → `compile_message` :3347) | **YES — the only live format** | yes (any RPC) | n/a |
| V0 | `compile_v0_message` (lib.rs:3488), `build_signed_v0_transaction` (:3656), `compiler.rs:939 try_compile_v0`, `v0_compiler.rs:152` | **NO — tests only** | yes, but **not reachable** | n/a |
| V1 | `try_compile_v1` (compiler.rs:1014), `AdaptiveTxCompiler::compile_v1_message` (adaptive.rs:383), `v1_fallback::compile_v1_fallback` (v1_fallback.rs:62) | **NO — tests only; rejected by the live 1232 B gate before dispatch** | **NO** — no `maxSupportedTransactionVersion: 1` anywhere in arb-bot; Jito bundle engine rejects >1232 B | **SIMD-0385 is Activated** on mainnet (official docs/SIMD/agave agree) — but this codebase has no submission path for it |

## 2. V0 verification (as compiled, standalone)

- Version prefix: 0x80 prepended at parse (`extract_transaction_signature` lib.rs:315-320), message compiled without prefix; signer bitmaps from privileges (writable signers, readonly signers, writable non-signers, readonly non-signers) — `compile_v0_message` lib.rs:3535-3600. ✓ structurally correct.
- ALT section: `compact_u16(num_tables) + per table key/writable/readonly indices` (lib.rs:3637-3650). Zero-reference tables omitted. Lookup indices are `u8` positions into `table.addresses` — **wrong when the table is a filtered subset (the shipped bug, see 07 §4)**.
- Signers never ALT-loaded (lib.rs:3540-3547). ✓
- Writable/readonly split: writable indices loaded first (lib.rs:3593-3599). ✓ matches spec (loaded writable appended before loaded readonly).
- `pos > 255` silently inlines over-range members instead of erroring (lib.rs:3553-3565) — fail-open defect.
- `MAX_ALT_ADDRESSES = 256` enforced at parse (master_alt.rs:134-139); `key_to_index` u8 — de-facto 256 cap, no explicit check.
- Size accounting (`wire_size = wire_bytes.len()`) is exact. `byte_budget.alt_savings_bytes` may go negative (informational).
- ALT activation/warm-up enforced **in the V0 compiler** (`verify_activation`, compiler.rs:953-956; fail-closed) — but that path is unreachable in production.
- Integration test (`#[ignore]`d, requires `ARB_MAINNET_RPC_URL`) is the only test that compiles with the **full on-chain table** (`mainnet_alt_integration_tests.rs:355-358`) — correct either way it lands; never runs in CI.

Measured wire bytes (fixtures, `three_leg_meme_compiler_tests.rs`): V0-with-ALT ≤ 1232 B for small
fixtures; larger fixtures need V1 (≤4096 enforced, adaptive.rs:547-581; compiler.rs:1077-1082;
v1_fallback.rs:133-138). No on-chain V0 was ever produced by this system.

## 3. V1 verification

Implementation exists and is unit-tested: header `[0]=0x81, [1..4] header, [4..8] ConfigMask u32 LE,
[8..40] blockhash, [40] num_ix u8, [41] num_addr u8, then inline keys, config values, 4-byte
instruction headers, payloads]` (v1_fallback.rs:333-441, compiler.rs:859-937, adaptive.rs:370-595).
`MAX_V1_INSTRUCTIONS = 64`, `MAX_V1_ADDRESSES = 64` (adaptive.rs:30-34), enforced; size cap 4096;
ComputeBudget instructions are **no-ops** in v1 (SIMD-0385) — `try_compile_v1` drops them and moves
fee/limit/heap/loaded-accounts-data into `TransactionConfigMask` (compiler.rs:1014-1099).

**V1 is not supported end-to-end in this repository:**
1. Live path emits legacy only.
2. The 1232 B dispatch gate (dispatch.rs:170-198) would drop any V1 before it could be sent.
3. Jito rejects >1232-byte bundles (stated in-repo at v1_fallback.rs:9-10).
4. No `maxSupportedTransactionVersion` is set on any RPC call in arb-bot (grep: zero hits).
5. `v1_fallback` keeps ComputeBudget ixs in the message (violating the SIMD-0385 no-op rule the
   sibling `try_compile_v1` obeys), has a **tip-strip predicate bug** (`accounts.len() == 2` never
   matches the marked 3-account tip ix ⇒ tip is never stripped ⇒ double pay if this path ever ran),
   computes `compute_v1_header` heuristically ignoring its own canonical sort (v1_fallback.rs:284-311,484-523),
   and its remap pass (`remap_instructions`) computes and discards `_prog_idx/_acc_idx` (:284-311).
6. `arb-execution::DispatcherConfig::default()` sets `v1_transport_supported: true` — a default that
   claims a capability nothing downstream honors.
7. Layout is self-consistent (parse-back asserts in `measure_v1_message_components`,
   compiler.rs:928-935) but **unverified against any SIMD-0385 implementation**; no official golden
   vector exists in-repo. The `solana-message` v4.6.0 differential tests
   (`v1_header_differential.rs`, untracked) compare *logical* content, not bytes, and currently
   (post-fix) compile but the file set still fails `fmt --check` and `clippy -D warnings`
   (10 lint errors).

**Verdict: V1 = UNSUPPORTED in the execution path. Do not wire it in. Treat any doc claiming
"V1-ready" as false.**

## 4. V0→V1 transition design (if ever undertaken)

| Aspect | V0 | V1 | Consequence |
|---|---|---|---|
| Prefix | 0x80 | 0x81 | parsers must branch |
| Accounts | static + ALT lookups (≤256 via u8) | **≤64 inline** (96 only behind SIMD-0596, Draft) | deep routes do not fit V1 either |
| ALTs | yes | **none** | lookup tables become inline 32 B keys — V1 message grows |
| Size | 1232 | 4096 | V1 bigger headroom |
| ComputeBudget | ixs (one per type) | no-ops; config in message mask | CU/fee/heap/loaded-accounts-data must move into config |
| Fees | per-CU price×limit, rounded | **absolute lamports** | fee model changes; `calculate_net_profit` assumptions change |
| Loaded accounts | 64 MiB default | config-masked | must set explicitly |
| RPC | `maxSupportedTransactionVersion: 0` | `: 1` | required on every call incl. simulation |
| Jito | ≤1232 only | never | Jito path dead for V1 |
| Signatures | size-bound | ≤12 explicit | fine (1 sig) |

Equivalence test to run offline (not run — would require building both compilers and a full-table
ALT fixture that exists only as `#[ignore]`d test data): compile identical instruction vector as
legacy/V0/V1; assert (a) same instruction data bytes, (b) same account privilege partition,
(c) v0 keys resolved from a real-table fixture equal the inline keys of legacy, (d) V1 config mask
carries limit/price/loaded-data-size/heap exactly. Do not ship any of this until the dispatch gate,
Jito transport, and RPC simulation agree on `maxSupportedTransactionVersion`.

## 5. Signer handling (both formats)

Signers always serialized in the static list before any loaded address; signature count ≤ 12 for v1
(SIMD-0385); v0 count size-bound; fee payer is static writable signer index 0 in every builder.
`jitodontfront` is a non-existent account attached **read-only** to the tip instruction — correctly
never an ALT member, correctly inline everywhere.
