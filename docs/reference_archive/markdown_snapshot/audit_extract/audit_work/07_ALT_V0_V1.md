# Audit 07 — ALT Architecture & Transaction Version Handling

**Scope:** `crates/arb-tx/src/master_alt.rs`, `alt.rs`, `compiler.rs`, `adaptive.rs`, `template.rs`, `lib.rs`;
`crates/arb-execution/src/{lib.rs,v0_compiler.rs,v1_fallback.rs,sizing.rs,merger.rs}`;
tests: `mainnet_alt_integration_tests.rs`, `adaptive_compiler_tests.rs`, `challenger_m2_tests.rs`, `three_leg_meme_compiler_tests.rs`;
live path: `crates/arb-bot/src/supervisor/{run.rs,dispatch.rs}`.
READ-ONLY audit. No code changes, no cargo run.

**Bottom line up front:** The static-master-ALT design is carefully engineered on paper (good parse/verify logic, dedup, warm-up checks). But (1) the *compiled-V0* code path passes a **filtered** account subset as the `AddressLookupTable.addresses` to `compile_v0_message`, which emits **wrong on-chain ALT indices** — any V0 actually produced this way would reference arbitrary wrong accounts; (2) the **live signing path never uses the ALT or the compiler at all** — it builds an inline **legacy** transaction and drops it at the 1232-byte gate; (3) **V1 is not real support** — it compiles in unit tests only, is rejected by the live MTU gate, is rejected by Jito, and its fallback engine has a tip-stripping bug that guarantees the marked tip instruction is never stripped; (4) ALT authority/activation/deactivation is **verified in tests and in the (dead) compiler path, never at runtime** — nothing in `arb-bot` fetches the ALT account before signing; (5) ALT writable signers are structurally impossible, but **Jito tip accounts are deliberately placed inside the master ALT**, and the tip instruction marks them writable.

---

## 1. Master ALT address selection; static vs dynamic contents; pool-specific accounts; signers/tips; duplicates; stale addresses

### 1.1 What the master ALT is designed to hold
Module doc (`master_alt.rs:1-17`): "a single, operator-provisioned Mainnet ALT" holding "program IDs, the DLMM event authority, memo program, token programs, common mints (WSOL/USDC/USDT), wallet-specific bot ATAs, **all eight Jito tip accounts**, and operator-configured static bridge-pool addresses." Bin arrays / meme pool accounts are explicitly excluded (`master_alt.rs:8-9`, `1050-1073`). Safety invariants claimed (`master_alt.rs:11-17`): ALT-loaded accounts can never be signers; ALT never auto-extended for new meme pools; controlled admin refresh only; table must be activated on-chain before compiled txs reference it.

### 1.2 Contents generation — static only, deterministic
`StaticMasterAlt::default_static_entries` (`master_alt.rs:375-569`) emits, in order:

| # | Category | Address | Source line |
|---|----------|---------|-------------|
| 1 | MeteoraProgram | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | master_alt.rs:33, 406-412 |
| 2 | MeteoraEventAuthority | derived `[b"__event_authority"]` PDA, hard-checked == `D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6` | master_alt.rs:38-41, 414-430, 716-723 |
| 3 | MemoProgram | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | master_alt.rs:36, 432-438 |
| 4 | SplTokenProgram | `TOKEN_PROGRAM_ID` (arb-constants) | master_alt.rs:441-457 |
| 5 | Token2022Program | stale check only — `_ = (token_2022, ata_program)` silenced at master_alt.rs:447 | master_alt.rs:443-464 |
| 6 | AssociatedTokenProgram | arb-constants | master_alt.rs:465-471 |
| 7 | SystemProgram | `SYSTEM_PROGRAM_ID` raw bytes | master_alt.rs:449, 472-477 |
| 8 | ComputeBudgetProgram | `COMPUTE_BUDGET_PROGRAM_ID` raw bytes | master_alt.rs:448, 478-483 |
| 9 | WsolMint / 10 UsdcMint / 11 UsdtMint | `So111…`, `EPjFWd…`, `Es9vMF…` | master_alt.rs:486-509 |
| 12–14 | BotWsolAta / BotUsdcAta / BotUsdtAta | derived via `derive_ata(payer, mint, spl_token)` | master_alt.rs:511-535, 725-731 |
| 15–22 | **JitoTipAccount × 8** | `JITO_TIP_ACCOUNTS` from arb-constants | master_alt.rs:537-548; arb-constants/src/lib.rs:57-66 |
| 23–24 | StaticBridgePool (optional) | operator-configured SOL/USDC, SOL/USDT, owner=meteora program | master_alt.rs:550-566 |

Total: **22 required + up to 2 optional = 24 max** of 256 capacity (`MAX_ALT_ADDRESSES = 256`, master_alt.rs:30).

### 1.3 Pool-specific / bin-array / bitmap-extension accounts — NOT in ALT
- `default_static_entries` has **no** LbPair, reserve_x/y, oracle, bin_array_pdas, bitmap_extension, meme mints, or meme ATAs (master_alt.rs:375-569 — none of those parameters exist).
- Design intent confirmed by test `test_default_static_entries_excludes_meme_pool_accounts` (master_alt.rs:1050-1073).
- `partition_accounts` (compiler.rs:639-711) classifies any account not in `master_alt` (i.e., all meme-pool state, bin arrays, bitmap extensions — `compiler.rs:670-695`) as **inline**, consistent with the doc.
- Real Mainnet Meteora DLMM ALT snapshot embedded in the test fixture (`JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz`, fetched read-only via RPC) contains exactly 7 addresses: Token, Token-2022, System, event authority, Memo, WSOL, `FiqLtg6z5ED2TR3hWrRaB6uRLcPqXFZpSVQutSznGfS` (master_alt.rs:891-953; integration test `mainnet_alt_integration_tests.rs:52-53,184-197`). No bin arrays, no pool state — confirms ALT is for shared static keys only.
- Note: that Meteora official ALT does **not** contain USDC/USDT mints or any bot ATA; it is **not** the table this codebase provisions. The code's master ALT is operator-provisioned separately; the Meteora ALT is used only as a parse/warm-up fixture.

### 1.4 Signers and tip accounts
- **Signers:** none of the ALT categories are signers. `StaticMasterAlt::verify_no_signers` (master_alt.rs:322-332) rejects ALT ∩ signer set; enforced in the V0 compilers with `signer_addresses = [ctx.payer]` (compiler.rs:947-951; v0_compiler.rs:159-163). Structurally, `compile_v0_message` routes any `is_signer` account to static signer buckets *before* the ALT lookup (lib.rs:3540-3547) — ALT-loaded keys can never be signers on the wire. Good.
- **Tip accounts: YES, included.** All eight `JITO_TIP_ACCOUNTS` (arb-constants/src/lib.rs:57-66) are pushed into the master ALT as `AddressCategory::JitoTipAccount` (master_alt.rs:537-548). This is deliberate ("all eight Jito tip accounts", master_alt.rs:5-6) and confirmed by test asserting exactly 8 present (master_alt.rs:1039-1046). The tip instruction writes to the tip account (`AccountMeta::new(tip_account, …)` ⇒ `is_writable = true`, lib.rs:257, AccountMeta::new at lib.rs:189-195). On the wire the tip account therefore becomes a **writable ALT lookup** in the V0 path. On Solana this is legal (ALTs may carry writable, non-signer keys), and Jito does accept v0+ALT transactions, but: (a) it contradicts the audit requirement that tip accounts be excluded from ALT content ("they must NOT be"); (b) it risks the tip account's writable index being merged/dedup incorrectly if the same tip key is also referenced elsewhere (see §6 on privilege merging); (c) the `v1_fallback` strip logic assumed the tip ix is *not* marked (see §4.4/8.3).
- **`jitodontfront` marker account** (`arb-constants/src/lib.rs:123-142`, `jitodontfront` = `jitodontfront111111111111111111111111111111`): appended read-only to the tip instruction by `attach_jitodontfront` (lib.rs:274-286) / `build_marked_jito_tip_instruction` (lib.rs:292-300). It is **not** in the master ALT (good — a non-existent on-chain account must stay inline-readonly). Any tx carrying it through `partition_accounts` gets it classified inline (compiler.rs:698-708 — not in ALT ⇒ inline). OK.

### 1.5 Duplicates
- On-chain parse rejects duplicate addresses outright: `parse_alt_account_data` uses a `HashSet` and errors `DuplicateAddress` (master_alt.rs:126-133).
- `populate_from_entries` dedups against existing set and within the entry list (master_alt.rs:585-609).
- `build_or_extend_master_alt_ixs` dedups by first-seen order (master_alt.rs:762-772).
- `AltManager` (alt.rs) does not dedup — caller's responsibility; chunking `MAX_ALT_EXTEND_ADDRESSES = 30` (alt.rs:16, `addresses.chunks(30)` at alt.rs:143,163) matches the 960-byte extend-ix budget noted there.
- `partition_accounts` dedups refs via `seen` (compiler.rs:645,698-708).
- **Gap:** `compile_v0_message` does not reject duplicate indices across multiple tables and silently does `found_in_table=true; break` on first table hit (lib.rs:3550-3566); multi-table conflicts are out of scope for a single master ALT but unguarded.

### 1.6 Stale addresses (expanded)
- `StaticMasterAlt` also exposes `category_of`/`index_of`/`contains`/`len` (master_alt.rs:256-283) — `index_of` is O(n) and fine for ≤256 entries; nothing caches an "ALT generation" alongside pre-compiled templates.
- `Category` ordering in the cached vector defines insertion order used by provisioning (`populate_from_entries` pushes in entry order, master_alt.rs:605-608); it is also the order used by `build_or_extend_master_alt_ixs` for on-chain extension (master_alt.rs:801-820) — so provisioning order == reference order, deterministic across runs (given identical entry lists).
- No entry carries a created/last-verified slot; `mark_activated` only sets table-level slots (master_alt.rs:622-626). If the table is extended on-chain *after* the bot cached indices, the cached table is stale in **content** but still warm-valid — silent correctness failure for newly appended indices (they only shift existing semantics if the table is rebuilt; pure appends are index-stable, which is the one lucky property).
- Jito tip round-robin selection (`jito_tip_account_for_slot`, arb-constants/src/lib.rs:112-117) means any of 8 tip accounts may appear; all 8 are ALT members, so all stay resolvable — but a table missing tips (Class C removal without inline fallback) would break 7 of 8 slots.

### 1.6a Provisioning flow end-to-end (alt.rs)
`AltManager::create_alt` (alt.rs:132-153) derives the ALT PDA from `[authority, recent_slot]` (alt.rs:25-30), emits `CreateLookupTable` (alt.rs:44-70, instruction index 0, accounts: ALT writable / authority readonly-signer / payer writable-signer / system program — alt.rs:60-65) followed by chunked `ExtendLookupTable` ixs capped at `MAX_ALT_EXTEND_ADDRESSES = 30` (alt.rs:16, 74-115, chunks at alt.rs:143,163). Authority and payer are the **same key** in `create_alt` (alt.rs:139 passes `payer` as both) — the created table is single-authority = payer; `master_alt.rs` helpers instead take a separate `authority` (build_or_extend_master_alt_ixs, master_alt.rs:748-830). Neither path is ever invoked at runtime (grep `create_alt|extend_alt` finds only alt.rs itself + test at alt.rs:269-289). The cache in `AltManager` is keyed by an arbitrary string with no validation of on-chain state; it's a stub. Note the real provisioning entry point for the master ALT is `build_or_extend_master_alt_ixs`, reachable from nowhere in production either.

## 2a. Wire-format mechanics of the v0 ALT section (for §6 context)
`compile_v0_message` emits the lookup section as: `compact_u16(num_active_tables)` + per table `table_key(32B) + compact_u16(writable_count) + writable_indices(1B each, table-local u8) + compact_u16(readonly_count) + readonly_indices` (lib.rs:3637-3650). Tables with zero referenced indices are filtered out of the section entirely (lib.rs:3638-3641) — an ALT with 22 members referenced by zero instructions serializes as **no table section at all** (compact_u16(0), 1 byte). This means the "ALT savings" accounting (compiler.rs:849) is computed over *referenced* indices, which is correct, and `loaded_count` returned as `alt_resolved_count` reflects referenced keys, not table size.
Slot 5+: a table whose **only** references are writable gets its writable vector filled and readonly vector empty — fine.
`pos > 255` guard at lib.rs:3553-3565 silently **inlines** an over-range ALT member instead of erroring — a latent fail-open: an entry beyond index 255 would be double-counted as static inline rather than tripping an error. Since `MAX_ALT_ADDRESSES = 256` (master_alt.rs:30) and parse rejects >256 (master_alt.rs:134-139), this can only trigger for hand-built tables in tests, but it is fail-open where it should fail closed.

## 2b. `partition_accounts` vs `merger::partition_accounts` — two divergent classifiers
`arb-execution/src/merger.rs:312-390` is a near-verbatim copy of `compiler.rs::partition_accounts` (same referenced-set walk, same `master_alt.contains` gate at merger.rs:375-382) **without** the bitmap-extension/bin-array handling being any different, and without the signer-violation comment. Both feed the same broken table construction downstream (v0_compiler.rs:175-191 consumes merger's output). Neither dedups against `jitodontfront` (which lands inline — correct). Divergence risk: fixes applied to one classifier will not propagate to the other — they are two copies maintained in parallel with no shared implementation.



---

## 2. Account classification: reusable-static vs pool-specific vs dynamic vs wallet-specific; missing required accounts

Classification is consistent between the two compilers and the dispatcher:

| Class | Members (as coded) | Where classified |
|-------|--------------------|------------------|
| **Reusable-static (ALT-eligible)** | programs (Meteora DLMM, SPL Token, Token-2022, ATA, System, ComputeBudget, Memo), event authority, WSOL/USDC/USDT mints, bot ATAs, 8 Jito tips, optional static bridge pools | `default_static_entries` master_alt.rs:375-569; `verify_required_present` master_alt.rs:286-309 |
| **Pool-specific (inline)** | lb_pair, reserve_x/y, oracle, token mints of the pool legs, host_fee_in, 2 bin arrays per leg, bitmap_extension | not in ALT ⇒ inline; compiler.rs:670-692, 754-756 |
| **Dynamic / meme-specific (inline)** | meme_a_mint, meme_b_mint, meme_a_ata, meme_b_ata | compiler.rs:659-663 (not in static ALT) |
| **Wallet-specific (ALT-adjacent)** | bot WSOL ATA (`user_token_in/out` legs 1 & 3), bot USDC/USDT ATAs | in ALT via `BotWsolAta` etc., master_alt.rs:511-535 |
| **Signer / payer (static, must NOT be in ALT)** | `ctx.payer` | compiler.rs:948, lib.rs:3499/3533 |
| **Top-level programs (v0_compiler forces STATIC, compiler.rs allows ALT-loaded)** | compute-budget, system, SPL token, Token-2022, ATA program, memo, Meteora DLMM | v0_compiler.rs:22-34 vs compiler.rs (no such exclusion — SPL/System/ComputeBudget/Meteora WILL be ALT-loaded if in ALT, because `compile_v0_message` loads any non-signer key found in the table, lib.rs:3550-3575) |

Required-but-possibly-missing:
- `verify_required_present` demands 11 categories (master_alt.rs:287-299). **Bot ATAs, Jito tips and bridge pools are NOT required** — a provisioned ALT missing all three passes verification.
- **`bin_array_bitmap_extension` and bin arrays are expected inline** — correct for meme pools. But note `DlmmSwapAccountSet.bin_array_bitmap_extension` is optional and `leg.bitmap_extension` is pushed inline (compiler.rs:689-691) — correct.
- **Meme ATAs** (`meme_a_ata`, `meme_b_ata`) are created inline via `CreateIdempotent` and carried inline — correct; they can never be ALT'd (they don't exist until created).
- **Host fee recipients** (`host_fee_in`) inline — correct.
- Potential miss: **Token-2022-owned bot ATAs** would be derived with the wrong token program — `derive_ata` hard-checks SPL `TOKEN_PROGRAM_ID` in `default_static_entries` (master_alt.rs:511-535, `owner: Some(spl_token)`) although mints may be Token-2022; `owner_program` is recorded but **never verified on chain** anywhere (see §3).
- **Missing from ALT vs required:** nothing referenced by instructions is required to be in ALT; inline is always legal. The "required" set is a provisioning sanity check only, and it is only enforced in the dead compiler path (§7).

Duplicates in the referenced set are collapsed before wiring (compiler.rs:645,698-701). Stale meme-pool accounts from a previous route are never persisted to the ALT (by design).

---

## 3. ALT verification: authority, activation, warm-up, deactivation

### 3.1 What exists (all correct, all off-by-default)
`parse_alt_account_data` (master_alt.rs:77-148):
- LOOKUP_TABLE_META_SIZE = 56 (master_alt.rs:73) — matches `solana-address-lookup-table-interface` `LOOKUP_TABLE_META_SIZE`.
- Rejects: short data (<56, master_alt.rs:78-84); `ProgramState::Uninitialized` (discriminant 0, master_alt.rs:87-91); unknown discriminant (master_alt.rs:93-97); address section not 32-byte aligned (master_alt.rs:118-124); duplicates (master_alt.rs:126-133); >256 addresses (master_alt.rs:134-139); bad authority option tag (master_alt.rs:104-115). Padding bytes 54..56 tolerated, **not** checked for zero (master_alt.rs:116) — minor.
- Authority: parsed as `Option<Pubkey>` (master_alt.rs:105-115); `from_onchain_readonly` enforces expected authority if `expected_authority: Some(_)` (master_alt.rs:646-662) and **rejects frozen ALTs when an authority is expected** (master_alt.rs:655-660).
- Deactivation: `from_onchain_readonly` rejects `deactivation_slot != u64::MAX` (master_alt.rs:640-645).
- Warm-up: `verify_warmup` requires `current_slot > last_extended_slot` (master_alt.rs:687-701); `usable_addresses_len` gates on it (master_alt.rs:704-707); `from_onchain_readonly` sets `activation_slot = last_extended_slot + 1` (master_alt.rs:678-680).
- Activation: `verify_activation` requires `last_extended_slot >= activation_slot` (master_alt.rs:335-352); `is_activated` (master_alt.rs:247-253).

### 3.2 Test coverage of verification
Real mainnet ALT snapshot parse + authority `5unTfT2kssBuNvHPY6LbJfJpLqEcdMxGYLWHwShaeTLi` + warm-up boundary `verify_warmup(376_938_276) == ok`, `verify_warmup(376_938_275) == err` (master_alt.rs:891-954). Live RPC integration test validates owner == ALT program, not executable, `deactivation_slot == u64::MAX`, `last_extended_slot < current slot` (`mainnet_alt_integration_tests.rs:112-197`, `#[ignore]`d, requires `ARB_MAINNET_RPC_URL`).

### 3.3 — **ENFORCEMENT GAP: verification is never executed on the live path**
- `verify_activation` / `verify_no_signers` / `verify_required_present` are called only inside `try_compile_v0` (compiler.rs:947-961) and `v0_compiler::compile_v0` (v0_compiler.rs:159-173) — i.e., only when building a V0 transaction.
- `arb-bot` **never references `StaticMasterAlt` at all** (grep `StaticMasterAlt|master_alt|alt_address` in `crates/arb-bot` → only a Telegram label string at `dispatch.rs:550` and the comment at `run.rs:1258`). It signs an inline **legacy** transaction (`build_signed_transaction` → `compile_message`, lib.rs:3471-3485, 3347-3446) and never consults any ALT.
- Nothing on the send path calls `from_onchain_readonly`/`mark_activated`/`verify_warmup` against the live chain; the integration test is `#[ignore]` (`mainnet_alt_integration_tests.rs:17`) and does not run in CI.
- If the operator ALT is deactivated, expired, or replaced, **no code path notices**. The V0/V1 compilers *would* fail closed (compiler.rs:953-961) — but they are unreachable from `arb-bot`.
- `mark_activated` is only ever called in tests (compiler.rs test at 1260; arb-execution test at lib.rs:475; arb-engine pipeline test at three_leg_meme_pipeline_tests.rs:342). **No production code ever sets `last_extended_slot`/`activation_slot` from chain state.**
- **Stale ALT references after table replacement:** `administrative_refresh` requires `force_replace=true` and rebuilds cached state, but any pre-compiled/pre-signed template (`PreWarmedTxTemplate`, template.rs:197-360) retains the old table key — nothing re-validates templates against a new ALT.

---

## 3a. Lifecycle walk-through: what happens when the operator ALT deactivates at slot N
Solana semantics (as modeled by this code): a table extended at slot S becomes reference-able at S+1 (`verify_warmup`, master_alt.rs:687-701). A deactivation sets `deactivation_slot`; `from_onchain_readonly` hard-rejects any `deactivation_slot != u64::MAX` (master_alt.rs:640-645) — strictly conservative, fine. What the code does **not** model: (a) scheduled-but-not-effective vs effective deactivation; (b) table **closure** — `addrs.len() == 0` is accepted silently (master_alt.rs:118-139), and `from_onchain_readonly` adopts a drained 0-member table without complaint (master_alt.rs:672-683; `verify_required_present` is documented off-limits there, master_alt.rs:629-632); it only surfaces as `is_provisioned() == false` (master_alt.rs:243-245) and the "No ALT address configured" error in the dead compile path (compiler.rs:964-966) — fail closed, but only there; (c) **re-activation is impossible on chain** — `administrative_refresh` (master_alt.rs:836-859) clears *cached* slots but cannot re-create the on-chain table; every pre-compiled template referencing the dead table key stays invalid forever, with no refresh step in production. Failure-ordering in production today: (1) table deactivated → (2) `AltNotActivated` would refuse V0 compiles — but (3) production never signs with ALT anyway (§7), so (4) if the compiler were wired in later without fixing F1/F5, cached Tier-1/2 templates would still carry the dead key, `sigVerify:false` simulation (dispatch.rs) would pass them, and only the Jito block engine would reject the bundle — tip lost, opportunity missed, nothing logged beyond a rejection callback.

## 3b. `AltManager` (alt.rs) vs `StaticMasterAlt` — two ALT abstractions, neither wired
| Aspect | `AltManager` (alt.rs:118-189) | `StaticMasterAlt` (master_alt.rs:209+) |
|---|---|---|
| Purpose | generic create/extend/cache helper | master table cache + verification |
| State | `HashMap<String, Pubkey>` cache | ordered addrs + categories + slots + authority |
| Create | PDA from `[authority, slot]`, create+extend ixs (alt.rs:132-153) | same via `build_or_extend_master_alt_ixs` (master_alt.rs:748-830) |
| Dedup | none | first-seen (master_alt.rs:589-598, 762-772) |
| Verification | none | full (§3.1) |
| Runtime use | none | none |
Duplication means fixes don't propagate; e.g. `AltManager::create_alt` uses `payer` as both authority and payer (alt.rs:139) while `build_or_extend_master_alt_ixs` separates them (master_alt.rs:748-752) — two differing provisioning semantics for the same on-chain operation, neither matching the test fixture (payer != authority, master_alt.rs:870-872).

## 4. Legacy vs V0 vs V1 decision; actual V1 support status

### 4.1 Decision logic (compile-time only)
`compile_3leg_meme_dlmm_arb` (compiler.rs:372-480):
1. Validate ctx → build ixs (CU limit, CU price, create-ATA-idempotent, 3 swaps, close-ATAs, marked tip).
2. Partition ALT vs inline (compiler.rs:445).
3. Compile V0; if `wire_size <= 1232` (`SOLANA_PACKET_MTU_LIMIT`, arb-constants/src/lib.rs:367) → return `V0JitoCompatible` (compiler.rs:457-459).
4. Else log and, if `ctx.v1_transport_supported`, `try_compile_v1` → `V1DirectTransport` if ≤ 4096 (compiler.rs:470-479, 1017-1099).
5. Else `Rejected`.

`AdaptiveTxCompiler::compile_adaptive` (adaptive.rs:678-860+): preference → legacy-size test → V0-with-ALT → V1 if `supports_v1` (adaptive.rs:741-760+); `CompilerCapabilities::v0_standard()` has `supports_v1: false` (adaptive.rs:108-116); `v1_capable()` sets it true (adaptive.rs:119-127); `max_supported_transaction_version()` → `Some(1)` only if supports_v1 (adaptive.rs:141-150).

### 4.2 Actual V1 support status: **not real / test-only**
Evidence:
- **Live path never emits V1 or V0.** `arb-bot` builds legacy via `build_signed_transaction` (run.rs:1355-1357 → lib.rs:3471 → `compile_message` lib.rs:3347). The only wire bytes that can reach dispatch are legacy (or a `PreWarmedTxTemplate` derived from those same legacy bytes, template.rs:233-255 → `build_kamino_bundle` — but that is only populated from Redis/RAM copies of what was already sent; run.rs:1366-1381, and never with an ALT in the live flow).
- **`dispatch_two_dex_arbitrage` (arb-execution/src/lib.rs:153) is never called anywhere** in the workspace (grep for `dispatch_two_dex_arbitrage|arb_execution::` → only its own tests). The entire dual-path dispatcher is dead code: its V0 estimate → `v0_compiler::compile_v0` → V1 fallback never runs.
- **V1 instructions would exceed the live gate anyway:** `dispatch.rs:170-198` hard-rejects any `tx_bytes.len() > 1232` before simulation/submission; a 4096-byte V1 can never land. And `validate_jito_bundle` enforces ≤1232/tx (lib.rs:325-341). Jito bundles: >1232-byte txs are rejected by the block engine (stated in v1_fallback.rs:9-10).
- V1 wire format itself: `try_compile_v1` drops ComputeBudget ixs and embeds priority fee / CU limit / 256 KiB loaded-accounts-data-limit natively in the message config (compiler.rs:1014-1099; constant set at compiler.rs:1053-1058). Tests cover the V1 wire layout, bitmask, sanitizers, and differential signature verification (`adaptive_compiler_tests.rs:39-440`) — so V1 is **implemented and unit-tested**, not merely stubbed.
- V1 **is not** integrated with anything that ships transactions, and the SIMD-0296/0385 4 KiB transaction format is **not deployed/enabled on Solana mainnet** as of this audit's assumed environment (the codebase itself treats V1 as "direct RPC/TPU transport… NOT Jito", compiler.rs:41-43, 365-367; v1_fallback.rs:9-10). There is **no RPC submission path** in the repo that sets `maxSupportedTransactionVersion: 1`.

### 4.3 False/overclaimed V1 support
- `RejectionReason::V1TransportUnsupported` is wired but V1 "support" exists end-to-end only on paper. Claiming "V1DirectTransport … suitable for direct RPC/TPU transport" (compiler.rs:40-43) is **aspirational**: no transport in the repo can send it.
- `arb-execution` `DispatcherConfig::default()` sets `v1_transport_supported: true` (arb-execution/src/lib.rs:130-140) — default config claims V1 capability that nothing downstream honors; the dispatcher's V1 output would be dropped by the 1232-byte gate in `dispatch.rs:170`.
- Doc comments say V1 uses "native message config" and ComputeBudget ixs must NOT be present (`compiler.rs:1014-1016`), but **`v1_fallback::compile_v1_fallback` keeps the ComputeBudget instructions in the message** (it never filters them — compare compiler.rs:1024-1029 which does filter). So the two V1 builders disagree; the fallback one emits a V1 message containing legacy ComputeBudget ixs, which violates the very SIMD-0385 semantics the module doc describes (v1_fallback.rs:14-20).
- `v1_fallback` claims to "Strip the standalone Jito Tip transfer … `accounts.len() == 2`" (v1_fallback.rs:213), but the tip instruction built by this codebase is always the **marked** one with **3** accounts (payer, tip, jitodontfront marker — lib.rs:292-300 → `attach_jitodontfront` pushes the marker, lib.rs:283-285). The strip predicate `ix.accounts.len() == 2` (v1_fallback.rs:213) therefore **never matches**, the tip is never stripped, and `effective_cu_price` roll-over is applied anyway (v1_fallback.rs:221-229) — i.e., Jito tip would be double-paid (once as transfer, once as CU price) if this path ever ran. Additionally, the marker account itself would remain in the flat account pool.
- `compute_v1_header` is a heuristic — `num_signers = signer_set.len().max(1)` (v1_fallback.rs:484) and readonly counts inferred from writable-set appearance (v1_fallback.rs:498-523), not from actual privileges of the canonical sort; the sorted list's bucket boundaries (v1_fallback.rs:235-274) are computed but **unused for the header** — inconsistent with `canonical_sort_accounts`. Wire bytes could carry a wrong header.
- `remap_instructions` computes `_prog_idx`/`_acc_idx` but **discards them** (v1_fallback.rs:284-311) and re-derives indices inline in `build_v1_message` via `key_to_index_fast` (v1_fallback.rs:373-399, 444-455) — dead remap pass; harmless but indicates untested invariants.

### 4.4 V1 message format fidelity vs real SIMD-0385
Header: `[0]=0x81, [1..4] header, [4..8] config mask u32 LE, [8..40] blockhash, [40] num_instructions u8, [41] num_addresses u8, [42..] inline 32-byte keys, then config values, then 4-byte instruction headers (prog_idx, num_acc, data_len u16 LE), then contiguous payloads` (v1_fallback.rs:333-441; mirrored in compiler.rs `measure_v1_message_components`, compiler.rs:859-937 and adaptive.rs:370-595). This is self-consistent (parse-back asserts), but **unverified against any Agave SIMD-0385 implementation**; no golden vector from the SIMD is present in tests (`adaptive_compiler_tests.rs:39-95` tests only internal round-trip). Also V1 here uses **trailing signatures** (adaptive.rs:557-581), and `extract_adaptive_transaction_signature` (lib.rs:315-320) supports it — internally consistent, externally unvalidated.
- `MAX_V1_INSTRUCTIONS = 64` (adaptive.rs:30-31) and `MAX_V1_ADDRESSES = 64` (adaptive.rs:33-34) are enforced (adaptive.rs:389-460; compiler.rs:1031-1044; v1_fallback.rs:93-104) — 3-leg swaps fit easily.
- `SOLANA_PACKET_V1_MTU = 4096` (adaptive.rs:27-28) enforced (adaptive.rs:547-581; compiler.rs:1077-1082; v1_fallback.rs:133-138).

---

### 4.5 Format-selection internals in `compile_adaptive` (adaptive.rs) — what "selection logic" really does
Beyond the four documented rules (adaptive.rs:661-677 comment), the implementation at adaptive.rs:678-860+ contains:
- `count_unique_accounts` near adaptive.rs:874 ("Public so the V1 fallback can pre-flight the SIMD-0385 account limit").
- Fast-path: first tries `TxBuilder::compile_message` (legacy) purely to measure size; if `config_is_active && capabilities.supports_v1` it returns V1 directly (adaptive.rs:741-760); otherwise, with ALTs supplied and v0 supported, it returns v0 (adaptive.rs:673-677 branch).
- Failure mode: if the legacy probe compile fails, there is no graceful degrade to v1 or v0 — the error propagates.
- `FormatSelectionReason` is recorded (ExplicitRequested / ProviderRestricted / AltRequiredForMtu) — never surfaced in production; only tests assert on it (adaptive_compiler_tests.rs).
- `max_supported_transaction_version()` returns `Some(1)` only when `supports_v1`, else `Some(0)` if v0, else `None` (adaptive.rs:141-150) — consumed only in tests (adaptive_compiler_tests.rs:64,311,336,352).

None of this is reachable from production. The sole fact the live path needs — "what does the RPC get?" — is: an unversioned legacy transaction submitted via normal send flow with no `maxSupportedTransactionVersion` set anywhere in arb-bot (grep: zero hits).

### 4.6 SIMD status (contextual, not asserted by code)
The code repeatedly invokes SIMD-0160 (instruction limit), SIMD-0296 (4 KiB page MTU), SIMD-0385 (v1 message format + trailing signatures + inline-only addresses) — e.g. adaptive.rs:2-13, 27-52, 118-127. As of this audit those SIMDs are not a universally deployed mainnet transaction format; Agave still enforces 1232-byte packets and v0 ALTs in production, and Jito's block engine rejects >1232-byte bundles (stated inside v1_fallback.rs:9-10 and reflected by adaptive.rs's own `v0_standard()` default, adaptive.rs:107-116). Any path requiring these SIMDs is conditional on a network upgrade that has not happened inside this codebase's assumptions. Treating V1 as "transport-ready because the compiler emits it" confuses local serialization with network capability — and no submission layer in this repo requests `maxSupportedTransactionVersion: 1` regardless.

### 4.7 Dead-code map (whole workspace)
- `arb-tx::compiler::compile_3leg_meme_dlmm_arb` — tests only (three_leg_meme_compiler_tests.rs:382 etc.).
- `arb-tx::adaptive::AdaptiveTxCompiler::compile_adaptive` — tests only (adaptive_compiler_tests.rs).
- `arb-execution::dispatch_two_dex_arbitrage` — tests only; `arb-bot` does not `use arb_execution` anywhere (grep `arb_execution` over arb-bot .rs files: zero).
- `arb-tx::master_alt::build_or_extend_master_alt_ixs` / `administrative_refresh` — tests only.
- `arb-tx::alt::AltManager` — tests only (alt.rs:268-289).
- `arb-bot` uses: `TxBuilder::{build_signed_transaction, build_marked_jito_tip_instruction, assemble_split_flash_arbitrage, ...}`, `PreWarmedTxTemplate`, `extract_transaction_signature`, `serialize_instructions`. `TxBuilder::build_kamino_bundle` → `compile_v0_message` **can** emit v0+ALT output when the caller passes non-empty `lookup_tables` (KaminoBundleParams.lookup_tables, lib.rs:2689) — but no production caller passes one; only tests (challenger_m2_tests.rs:613,691; bundle_tests.rs; adversarial_tests.rs) and they all use the fabricated ALT111… (lib.rs:3680).

---

## 5. Serialized transaction size: estimation code, measured size, runtime limits

### 5.1 Size estimation code (four independent implementations — inconsistent)
1. **`sizing.rs`** — `estimate_v0_wire_size` (sizing.rs:53-108): builds static keys (sizing.rs:166-194), builds **one** `AltLookup` from *referenced* ALT accounts (sizing.rs:238-272), estimates bytes. **Wrong on two counts:** (a) it counts table size using the referenced subset, like the compilers (see §6), and (b) its `count_header_fields` is acknowledged "conservative" garbage — it adds **all** static non-payer keys and **all** ALT keys to `num_readonly_unsigned` (sizing.rs:196-228), i.e. writable accounts are counted as readonly, so the estimate is not authoritative. It's used only as a pre-filter (arb-execution/src/lib.rs:168-173) in dead code.
2. **`compiler.rs measure_v0_message_components`** (compiler.rs:761-856) and **`v0_compiler.rs measure_v0_components`** (v0_compiler.rs:231-326) — byte-exact by parsing the real compiled message; `alt_savings_bytes = loaded*32 - lut_bytes` (compiler.rs:849; v0_compiler.rs:319). These are correct *as measurements* of whatever the compiler emitted.
3. **`compiler.rs measure_v1_message_components`** (compiler.rs:859-937) — parse-back check with exact expected-length assert (compiler.rs:928-935). Good.
4. **`template.rs`** — `PreWarmedTxTemplate` operates on pre-compiled bytes; no size estimation.

### 5.2 Actual compiled size with/without ALT
- Fully-inline V0 (no tables): measured in tests, e.g. `alt_wire <= inline_wire` asserted in `mainnet_alt_integration_tests.rs:385-388` and `adversarial_tests.rs:784-807`; no constant recorded for the fixture (println! only, three_leg_meme_compiler_tests.rs:615-616,1068).
- With 22-address static master ALT: V0 fixture fits ≤1232 per boundary tests (`three_leg_meme_compiler_tests.rs:628-645`, `arb-engine/tests/three_leg_meme_pipeline_tests.rs:352-353`), but **only when the compiled indices are correct** — see §6; sizes themselves are not affected by the index bug.
- **Legacy transaction in the live path** (all keys inline + no ALT): a 3-leg DLMM route carries ~45–55 unique accounts (7-instruction vector with 2-bin-array DLMM legs ⇒ ~50 × 32 B ≈ 1600 B of keys alone) → **legacy compile of a real 3-leg route will essentially always exceed 1232 bytes**, and the live dispatch gate at `dispatch.rs:170-198` then silently drops the opportunity (returns early, records a rejection, sends nothing). The bot is thus structurally unable to land 3-leg meme-DLMM arbs despite "succeeding" pipeline stages. This is the single largest operational consequence of the legacy-only live path.
- Inline vs ALT delta is roughly: each ALT-resolved account saves ~31 bytes minus ~1-2 bytes of lookup indices; with ~10-15 ALT references the delta is ~300-450 bytes — the difference between fitting and not fitting 1232.

### 5.3 Runtime loaded-account/MTU limits found in-code
- `SOLANA_PACKET_MTU_LIMIT = 1232` (arb-constants/src/lib.rs:367) — enforced in compiler.rs:457, `validate_jito_bundle` lib.rs:330, `dispatch.rs:170`, `arb-execution` lib.rs:133.
- `SOLANA_PACKET_LEGACY_MTU = 1232` / `SOLANA_PACKET_V1_MTU = 4096` (adaptive.rs:24-28) — enforced adaptive.rs:606-611, 645-650, 547-581.
- **`MAX_ALT_ADDRESSES = 256`** (master_alt.rs:30) and parse-time enforcement (master_alt.rs:134-139). ALT extend chunk = 30 (alt.rs:16).
- **No `MAX_ACCOUNTS_PER_TX` constant exists anywhere in the workspace** (grep: zero hits). Account indices on the wire are `u8` (compile_v0_message pushes `pos as u8` with `pos > 255 ⇒ skip` at lib.rs:3553-3564; key_to_index values are `u8` at lib.rs:3588-3599) — so **256 total accounts (static + loaded) is a de-facto cap enforced by construction, not by an explicit runtime check**. Solana's actual on-chain transaction account lock limit (256 per tx) is not modeled.
- **Loaded-accounts data size:** modeled only for V1 (`loaded_accounts_data_size_limit: Some(256*1024)` — 256 KiB "covers this route's footprint (3 x BinArray ~= 36 KiB…)", compiler.rs:1051-1058). No equivalent is computed or checked for the legacy/v0 path — the runtime default/actual per-tx loaded data budget on mainnet is not queried or asserted anywhere. **Missing check.**
- **64 account locks / writable-lock accounting:** no code models per-account lock limits or the "64 writable account" pre-checks that some relays apply; nothing to report beyond absence.
- `byte_budget.alt_savings_bytes` can go negative and is reported as-is (compiler.rs:849, `i64`) — informational only.

---

### 5.4 Worked byte accounting (fixture route, from the code's own builders)
Three-leg route fixture (`build_test_ctx`, three_leg_meme_compiler_tests.rs) — unique referenced accounts ≈ 60 (payer + 7 programs + 2 token programs + 2 meme mints + 2 meme ATAs + 3 bot ATAs + 3 lb_pairs + 6 reserves + 3 oracles + 2 mint sets x3 + 6 bin arrays + tip + marker + ATAs created/closed…). Inline legacy compile would need ≈ 1 + 64 (sig) + 1 (legacy header prefix absent — legacy has no version byte) + 3 (header) + compact_u16(keys) + 60×32 (keys) + 32 (hash) + instruction bytes ≈ **2,200+ B** — never legal for the 1232 gate. V0 with the 22-member master ALT: ~14 of those keys resolve via **1-byte indices** instead of 32 B (save ≈ 14×31 ≈ 434 B) → ≈ **1,700-1,800 B** — *still usually > 1232*, which is why the tests either accept V1 output or assert only `<= SOLANA_PACKET_MTU_LIMIT` when it fits (three_leg_meme_compiler_tests.rs:632-645 asserts V0 fits for their fixture — their fixture uses 2 bin arrays uniformly and 320k CU; production meme routes with Token-2022 transfer fees and 3 × `sweep_enabled` legs are larger). Practical consequence: even a correctly-wired V0 path may frequently exceed 1232 and fall to the V1 branch — which is unreachable/gated (§4, §7) — so the expected live outcome for deep routes is **silent skip** (dispatch.rs:170-198), with only a Telegram rejection recorded.
- Lookup overhead when everything fits: `compact_u16(1 table) + 32 (table key) + compact_u16(w) + w×1 + compact_u16(r) + r×1` ≈ 36 + referenced count bytes — the reason `alt_savings_bytes` (compiler.rs:849) is ≈ 31×loaded − 36.
- `wire_size` in `CompiledV0Transaction`/`CompiledV1Transaction`/`CompiledAdaptiveTransaction` is always `wire_bytes.len()` (compiler.rs:987, 1075; adaptive.rs:754 etc.) — size accounting is consistent with actual bytes, unlike the `sizing.rs` estimate (§5.1 #1).
- `ByteBudgetReport::summarize()` (compiler.rs:156-179) is the only diagnostic surface; it is printed via `eprintln!` at compiler.rs:462-467 but **never logged or shipped to telemetry** in any live path (dead code again).

## 6. Do ALT lookups reduce account metas correctly, or merely compress addresses?

**Mechanism in `compile_v0_message` (lib.rs:3488-3653) — correct in isolation:**
- Non-signer keys found in a table are removed from static keys and pushed onto that table's `writable_indices`/`readonly_indices` (lib.rs:3549-3575); writable loads are emitted before readonly loads (lib.rs:3593-3599) as v0 requires; lookup section is appended last (lib.rs:3637-3650); instruction indices map loaded keys to `static_len + i` / `writable_offset + i` (lib.rs:3588-3599). So it genuinely **re-bases account references onto loaded addresses** — not merely "compressing" 32-byte keys into the message while keeping indices bogus. Byte savings are real (lookup costs 1 index byte per referenced account vs 32).

**But the call sites corrupt the indices — CRITICAL BUG:**
- `compiler.rs try_compile_v0` builds the lookup table as `AddressLookupTable { key: alt_key, addresses: alt_accounts.to_vec() }` where `alt_accounts` is the **referenced-only subset** produced by `partition_accounts` (compiler.rs:639-711, 967-970). `compile_v0_message` then uses `table.addresses.iter().position(...)` as the **on-wire ALT index** (lib.rs:3552-3554). Positions within the filtered subset (0..k) are generally **not** the positions in the on-chain ALT (e.g. WSOL mint is index 5 in the snapshot at master_alt.rs:925-927, but would be index 0-2 in a filtered list). Every ALT-loaded account reference in the signed message would resolve to **the wrong on-chain account**.
- Same bug in `v0_compiler::compile_v0`: builds `alt_table` from `partitioned.alt_writable + alt_readonly` (v0_compiler.rs:183-191) — again filtered.
- Same bug in `sizing.rs build_alt_lookups`/`estimate_v0_wire_size` (sizing.rs:230-272) — estimates against the subset (size happens to be table-count dependent: `total += 32` per table once + indices bytes, so the estimate excludes unused table bytes, but header/keys accounting uses the same filtered membership — estimate ≠ actual signed message).
- **Contrast with the correct usage:** the mainnet integration test passes `addresses: parsed.addresses.clone()` (full on-chain table) — `mainnet_alt_integration_tests.rs:355-358`. And `TxBuilder::create_default_mainnet_alt` passes a full list (lib.rs:3681-3707). `compiler.rs`/`v0_compiler.rs` do not.
- Consequences: any V0 produced by these paths is **deterministically invalid** — indices wrong, accounts loaded ≠ accounts intended; at best simulation fails (unknown account / constraint violation), at worst (if an index coincidentally lands on a valid account with a compatible owner) wrong-account invocation. The unit tests pass because they never compare against on-chain resolution — `three_leg_meme_compiler_tests.rs` only asserts `wire_size == wire_bytes.len()` and presence of fields.
- Note the code even *comments* "`alt_accounts` may contain ALT members that are never referenced … they cost nothing and are not serialized" (compiler.rs:992-994) — the author understood membership vs reference, but still passed the referenced subset as the table. The fix direction (full table in, referenced indices emitted) is demonstrated by the integration test but not by the compilers.
- Privilege merging into the message is otherwise right: a loaded key referenced both writable and readonly across ixs is classified writable (writable pass collects first — actually writable/readonly partition is per-reference: each account is pushed to exactly one bucket because classification happens once on the merged privileges map; lib.rs:3549-3574, 393-398 of v0_compiler.rs handles merge with |=). `sizing.rs` additionally removes writable from readonly explicitly (sizing.rs:258-261).
- **Signer keys are never ALT-loaded** (lib.rs:3540-3547, partitioned out before table lookup; v0_compiler.rs:116). Correct.
- **Top-level program IDs:** v0_compiler forces them static ("must NEVER be placed in an ALT", v0_compiler.rs:21-34,113-118) while `compiler.rs` lets them be ALT-loaded (they're non-signers and in the ALT, compiler.rs:648-657 has no exclusion; lib.rs:3550-3575 loads them). Inconsistent — but both are legal in v0 (program ids may reference loaded keys). Not a correctness bug; a design divergence.

**Answer:** The wire machinery is a genuine re-basing (correct); the two in-repo compilers feed it the wrong table contents (indices wrong); only the full-table path (integration test / `create_default_mainnet_alt` consumers) is correct — and none of this reaches production because the live path is legacy-only (§7).

---

## 7. Does the runtime use `compiler.rs`/`AdaptiveTxCompiler` at all? Live signing path vs test-only compiled paths

**Live path (`arb-bot`) — verified line by line:**
1. `run.rs:1258-1352`: "P0 #1: do NOT use the fabricated ALT (ALT111… does not exist on-chain). Always build a legacy (v0-less) signed transaction with a real blockhash."
2. Tier-1 RAM template (`ram_templates`) → `patch_and_sign_with_swaps` (run.rs:1302-1316); Tier-2 Redis (`redis_cache.get_json::<CachedTemplate>` → `from_precompiled_bytes`, run.rs:1321-1349); Tier-3 full compile via **`TxBuilder::build_signed_transaction(signer, &ixs, blockhash)`** (run.rs:1355-1357).
3. `build_signed_transaction` (lib.rs:3471-3485) → **`compile_message`** (lib.rs:3347-3446) — a pure **legacy** compiler: all account keys inline, header = num_required_signatures/readonly counts, **no version byte, no ALT, no lookup section**. It then signs and prepends `compact_u16(1) + sig` (lib.rs:3479-3483).
4. The template caches are populated only from Tier-3's legacy bytes (`from_precompiled_bytes(bytes.clone(), …)` run.rs:1367-1374; `redis_cache.set_json(&route_sig, &cached, 60)` run.rs:1375-1381) — so even the fast path re-signs **legacy** bytes. Tier-1/2 are effectively legacy-template caches (and `route_sig` keys them per route; set at run.rs:1266-1273 with `prewarm:` prefix).
5. Dispatch gate: `dispatch.rs:170-198` hard-drops the tx if `len > 1232`. No ALT anywhere; only the literal string `"Tx Assembly & ALTs"` appears in a Telegram template (`dispatch.rs:550`).
6. `arb-bot` contains **zero** references to `StaticMasterAlt`, `compile_v0_message`, `compile_3leg_meme_dlmm_arb`, `AdaptiveTxCompiler`, `dispatch_two_dex_arbitrage`, or `compile_v0/compile_v1_fallback` (grep verified).

**Test-only paths:**
- `compile_3leg_meme_dlmm_arb` (compiler.rs:372) — invoked only from `arb-tx/tests/three_leg_meme_compiler_tests.rs:382,602,637,677,724,809,1061`.
- `AdaptiveTxCompiler` (adaptive.rs:~250-900) — invoked only from `arb-tx/tests/adaptive_compiler_tests.rs:13,39,95,…`.
- `arb-execution::dispatch_two_dex_arbitrage` (arb-execution/src/lib.rs:153) — invoked only from its own unit tests (arb-execution/src/lib.rs:486,499). **No production crate depends on it** (grep `arb_execution|dispatch_two_dex_arbitrage` workspace-wide: zero non-test hits).
- `challenger_m2_tests.rs:613,691` signs v0 txs via `TxBuilder::build_signed_v0_transaction` with `create_default_mainnet_alt()` — the **fabricated** ALT `ALT1111111111111111111111111111111111111111` (lib.rs:3680), which does not exist on-chain — the exact thing run.rs:1258 forbids in production. Tests therefore exercise a v0 encoder whose ALT key would never resolve on mainnet.
- `adversarial_tests.rs` (multiple) and `bundle_tests.rs:174,317,444` do the same.

**Discrepancy summary (the audit's central finding):** All ALT/V0/V1 machinery lives in `arb-tx`/`arb-execution` and is exercised **exclusively by tests**. The production signer in `arb-bot` (a) never uses ALTs, (b) never uses the versioned compiler, (c) always emits legacy inline transactions, and (d) is then size-gated at 1232 B, which a real 3-leg route will basically never pass (§5.2). The system currently cannot ship the transaction shape the tests validate, and the tests cannot validate the shape the system ships. Any claim that "ALT packing" happens in production (e.g., `latency_bench.rs:6` "Ed25519 Cryptographic Signing & ALT Packing with Real Authorized Keypair") describes code not wired into the hot path.

---

## 7a. Template machinery in the live path — why even Tier-2 can't rescue v0
`PreWarmedTxTemplate::from_precompiled_bytes` (template.rs:197) re-locates the blockhash and tip-lamports byte ranges inside already-signed wire bytes; `patch_and_sign_with_swaps` (template.rs:269) patches `size_in`/`min_out`/tip/blockhash in place and re-signs. Because the stored wire bytes are always produced by `build_signed_transaction` (legacy, no ALT section), the template layer **inherits the legacy layout**: there is no ALT lookup section to locate, and any attempt to feed a v0+ALT template through this locator would search for a 32-byte blockhash at the wrong offset — `locate_offsets` returns `TemplateOffsetError` (error.rs:48-49) if not found, and the callers treat failure as skip (run.rs:1360-1364). Tier-1 RAM templates are promoted from Tier-2 Redis (run.rs:1344-1346), which is populated at run.rs:1375-1381 from the same legacy bytes — there is **no code path, config flag, or env var that inserts ALT-carrying wire bytes into these caches**. Additionally `CachedTemplate` (run.rs:1375-1381) serializes `wire/blockhash/tip` only — it persists no ALT generation or table key, so even a hypothetical v0 template would have no way to detect that the referenced master ALT was refreshed/deactivated between cache write and replay.

One residual risk: `template.rs` offsets are computed on **legacy** bytes with the assumption of a single-signature layout; `patch_and_sign_with_swaps` re-encodes the signature at a fixed offset. No verification that the byte layout still corresponds to the intended instruction set occurs at patch time — if a route template is reused for a different meme pair whose inline account list differs in length, patch offsets are invalid and the tx silently corrupts (tests cover only the happy-path fuzzing at challenger_m2_tests.rs:607+ and adversarial_tests.rs:613+, always with the same fixture). This is an ALT-adjacent integrity gap: the ALT/v0 path was presumably designed to sidestep exactly this (fixed ALT ⇒ stable account count? no — meme pools change every route, so no) — but since production never uses it, the gap remains live.

## 7b. What the tests actually assert (and don't)
- `three_leg_meme_compiler_tests.rs:382,602,637,677,724,809,1061` — asserts `wire_size == wire_bytes.len()`, `alt_resolved_count > 0`, boundary ≤1232 passes/fails (`V0Oversized` at 655-663, `V1Oversized` at 702-708). Never decodes the lookup section, never asserts on-chain resolution, never rejects a filtered table.
- `adaptive_compiler_tests.rs:39-440` — V1 wire layout/bitmask/sanitizers/differential signature verification; `compile_adaptive` selects legacy-vs-v0 for small payloads (line ~318), v1 for large (line ~260). All against in-memory tables; none touches mainnet semantics of lookup resolution.
- `challenger_m2_tests.rs:607-730` — prewarmed-template fuzzing with `create_default_mainnet_alt()` (fake ALT111…); asserts patch soundness, not ALT validity.
- `mainnet_alt_integration_tests.rs` (`#[ignore]`d; requires live RPC) — the only test that (a) validates ALT owner == ALT program (line ~128-129), (b) non-executable (line ~129), (c) `56 + 32*N` size invariant (line ~140), (d) deactivation slot == u64::MAX (line ~145), (e) warm-up vs live slot (lines ~166-181), (f) event authority present (lines ~184-192). It compiles a v0 with the **full** table (lines 354-362) — i.e., it is the only artifact in the repo that exercises ALT lookup resolution correctly, and it is manual/ignored.
- Nothing asserts that a *referenced* ALT member is writable in the merged privilege set (e.g., tip account in ALT must remain writable in the lookup section); nothing asserts index < 256 for loaded lookups beyond the silent `pos > 255` skip.

## 8. Jito tip accounts — writability and ALT exclusion



8.1 **Writability:** tip instruction marks the tip account **writable** (`AccountMeta::new(tip_account, …)` = writable, lib.rs:256-257; `AccountMeta::new` sets `is_writable: true`, lib.rs:189-195). System program transfer, payer is writable-signer (lib.rs:255-258). This is required for the tip to move lamports and matches Jito's requirement.

8.2 **ALT contents:** all eight tip accounts are **deliberately included** in the master ALT (master_alt.rs:537-548) — see §1.4. The audit requirement ("tip accounts must NOT be in the ALT") is **not met by the code as written**. Legal on Solana (writable lookup), but it means (a) any ALT staleness/deactivation silently orphans tip references, and (b) the only reason tips work out is §7 — production never loads them from the ALT at all.

8.3 **Tip-marker interplay:** `build_marked_jito_tip_instruction` appends the `jitodontfront` marker **read-only** (lib.rs:274-300; marker constant arb-constants/src/lib.rs:138). Marker is never in the ALT (§1.4) — correct. But the V1 fallback tip-strip cannot remove the marked tip (3 accounts vs expected 2 — §4.3), and `dispatch_two_dex_arbitrage` dead code would thus never strip tips; V0 path keeps the tip inline-writable-in-ALT as designed.

8.4 **Tip selection:** production rotates tip accounts by slot/leader (`jito_tip_account_for_slot`, arb-constants/src/lib.rs:112-117; used at run.rs:619). All 8 are in the ALT, so whichever is selected is resolvable — again, only in the (dead) V0 path.

8.5 **ALT loading of writable tip + `jitodontfront` readonly marker:** the marker stays an inline readonly key; tip becomes a writable loaded key; both can coexist — no constraint violation, but it forfeits the intended "ALT holds only reusable readonly/statics" hygiene and bloats the writable-ALT surface (a writable loaded account whose index is shared across the table could alias with another consumer of the same table if privileges are mis-merged elsewhere).

8.6 **Simulation guard:** `dispatch.rs` simulates with `sigVerify: false` (dispatch.rs:507-512 area; comments at 508-512) and skips RPC sim under env flags (dispatch.rs:217-225) — tip correctness is never verified at runtime.

---

## 9. Proposed ALT manifest classification (static list), based on what's actually in the code

Derived strictly from `default_static_entries` (master_alt.rs:375-569) with risk annotations:

**Class A — keep in master ALT (runtime-agnostic constants, verify on provisioning):**
1. Meteora DLMM program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` — verify program exists & is executable (currently only hardcoded, never validated on-chain).
2. Meteora DLMM event authority PDA `D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6` — derivation verified against seed (master_alt.rs:414-424, 716-723) ✔.
3. Memo program `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`.
4. SPL Token program. 5. Token-2022 program. 6. ATA program. 7. System program. 8. ComputeBudget program.
9. WSOL mint. 10. USDC mint. 11. USDT mint (local constant duplicates arb-constants; master_alt.rs:44 vs arb-constants — inconsistent mint source, same value).

**Class B — wallet-specific, keep but scope to operator (re-derive on key rotation; never share table across wallets):**
12. Bot WSOL ATA (SPL-token derived). 13. Bot USDC ATA. 14. Bot USDT ATA.
→ Risk: if any bot ATA is ever Token-2022-owned, entries derived for SPL Token are wrong (master_alt.rs:512-535 hard-assume `Some(spl_token)` owner and programs). Add on-chain owner check at provisioning.

**Class C — REMOVE from ALT per policy (writable, and tips must stay inline for the Jito marker to stay meaningful / to avoid ALT-coupling of the bribe path):**
15–22. All eight Jito tip accounts. Recommendation: evict from master ALT; reference tips inline. (Code includes them deliberately; if the operator decides to keep them, they must at minimum be marked as Class C exceptions and independently re-verified at each refresh — current code does neither.)

**Class D — operator-configured static bridge pools (keep only with re-verification):**
23. Static bridge pool SOL/USDC (owner asserted = Meteora program, master_alt.rs:551-558).
24. Static bridge pool SOL/USDT (master_alt.rs:559-566).
→ Risk: pool state migrates silently on new deployments; nothing re-validates pool account owner/discriminant on refresh (`administrative_refresh` re-adds blindly, master_alt.rs:836-859). Verify owner == Meteora DLMM program + `lb_pair` discriminant at provisioning and at every refresh; drop stale bridges instead of silently keeping them.

**Class E — never in ALT (current code complies):**
- meme_a/meme_b mints, meme ATAs, any meme pool LbPair/reserves/oracle/bin arrays/bitmap extension/host-fee accounts, the `jitodontfront` marker, the payer/signer, any new pool discovered at runtime.

**Required-verification additions the manifest should carry:** for every Class A–D entry: `address, category, expected owner program, expected discriminant (mint/pool/pda), last-verified slot`. The current `StaticAddressEntry.owner_program` (master_alt.rs:199-208) records intent but **is never checked at runtime or provisioning**; wire it into `populate_from_entries`/`from_onchain_readonly` and into a CI live-chain test (model: `mainnet_alt_integration_tests.rs`, un-ignored behind an env flag).

**Concrete reference list (operator checklist):** Class A = {`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (Meteora DLMM), `D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6` (event authority PDA, seed-verified), `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` (memo), SPL Token `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`, Token-2022 `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`, ATA `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL`, System `11111111111111111111111111111111`, Compute Budget `ComputeBudget111111111111111111111111111111`, WSOL `So11111111111111111111111111111111111111112`, USDC `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`, USDT `Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB`} — 11 entries, matching `verify_required_present` (master_alt.rs:287-299). Class B = operator-derived ATAs (3). Class C = 8 Jito tips `ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49`, `DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh`, `DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL`, `ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt`, `Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY`, `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5`, `3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT`, `HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe` (arb-constants/src/lib.rs:57-66) — candidate removals. Class D = operator bridge pools (0-2). Expected on-chain table after pruning Class C: **14 required + 0-2 bridges = 14-16 entries** (fits trivially in 256; the 22-member default fits too, but policy alignment argues for Class C out).
Also note the **Meteora official public ALT** `JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz` — 7 members (Token, Token-2022, System, event authority, Memo, WSOL, `FiqLtg6z5ED2TR3hWrRaB6uRLcPqXFZpSVQutSznGfS`, see master_alt.rs:909-931). The code treats it as a read-only verification fixture only; it is **not** wired into any provisioning path — the operator's table is provisioned separately and must carry the Class A list independently. Do not confuse "the master ALT the code builds" with "the ALT Meteora publishes": they overlap on {Token, Token-2022, System, memo, event authority, WSOL} but differ on everything else (Meteora's lacks USDC/USDT mints, bot ATAs, tips, bridges — and contains `FiqLtg…` which this codebase never models).

---

## 10. Consolidated call-site table-construction inventory (F1 surface)
Every place a v0 message is built with ALTs, and whether the table passed is full or filtered:

| Call site | Table built from | Full on-chain table? | Verdict |
|---|---|---|---|
| `TxBuilder::compile_v0_message` (lib.rs:3488) | receives `&[AddressLookupTable]` — `table.addresses.iter().position()` used as on-wire index (lib.rs:3552) | caller's choice | encoder is correct; trusts caller |
| `compiler.rs::try_compile_v0` (compiler.rs:967-970) | `alt_accounts` (referenced-only subset from `partition_accounts`, compiler.rs:639-711) | **NO** | F1-critical |
| `v0_compiler.rs::compile_v0` (v0_compiler.rs:183-191) | `partitioned.alt_writable + alt_readonly` (referenced-only) | **NO** | F1-critical |
| `merger.rs::partition_accounts` (merger.rs:312-390) | same referenced subset (feeds v0_compiler) | **NO** | F1-critical |
| `sizing.rs::estimate_v0_wire_size` (sizing.rs:53-108) | `build_alt_lookups` from referenced subset (sizing.rs:238-272) | **NO** | estimate only, still wrong |
| `adaptive.rs::build_signed_v0_transaction` (adaptive.rs:623-659) | passes through caller's `lookup_tables` | caller's choice | OK if full table passed |
| `lib.rs::build_kamino_bundle` via `KaminoBundleParams.lookup_tables` (lib.rs:2690, 2836-2840) | passes through | caller's choice | OK if full table passed; **no production caller passes any** |
| `create_default_mainnet_alt` (lib.rs:3679-3708) | full hardcoded 23-entry list, fake key ALT111… | "full" but nonexistent on-chain | test-only crutch |
| `mainnet_alt_integration_tests.rs:354-362` | `parsed.addresses.clone()` (live RPC) | **YES** | the only correct usage; ignored test |
| `template.rs::from_bundle_params` (template.rs:233-255) → `build_kamino_bundle` | passthrough | caller's choice | test-only |

Additionally, **all three** in-crate V0 paths disagree on privilege handling for top-level programs (v0_compiler forces static; compiler.rs ALT-loads them; lib.rs happily loads any non-signer found in the table), and **two** independent byte-budget measurers exist (compiler.rs:761 vs v0_compiler.rs:231) with subtly different handling of `jito_tip_instruction_bytes` (both assume tip ix is last — compiler.rs:814, v0_compiler.rs:285 — true today because `build_marked_jito_tip_instruction` is pushed last in both builders, but silently wrong if any later ix is appended).

One more latent encoder hazard in `compile_v0_message`: `static_account_keys` is capped implicitly at 256 by the u8 index type; there is **no explicit check** that more than 256 unique keys (static + loaded) were collected — an overflow would truncate `i as u8` indices silently at lib.rs:3588-3599 rather than erroring. The `pos > 255` skip (lib.rs:3553) protects table indices, not the merged key_to_index map. For the current route sizes (~60 keys) unreachable; for any future route with >200 inline keys plus >56 loaded, silent index wrap. Recommend a hard `anyhow!` guard at the point static keys exceed 255-ish, or key_to_index u16 at build time with a final u8 range assert.

## Summary of severity-ranked findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| F1 | **Critical** | V0/V1 compilers pass **referenced-only subset** as `AddressLookupTable.addresses`; wire indices are positions in that subset, not the on-chain ALT ⇒ every ALT reference in a compiled v0 points at the wrong account. Unit tests can't catch it. | compiler.rs:443-445,947-970; v0_compiler.rs:175-200; lib.rs:3550-3566; contrast correct usage mainnet_alt_integration_tests.rs:355-358 |
| F2 | **Critical** | Live signing path is legacy-only inline; ALT/V0/V1/adaptive compiler never used in production; 3-leg legacy tx will not fit 1232 B and is silently dropped at the dispatch gate | run.rs:1258-1357; lib.rs:3347-3446; dispatch.rs:170-198; arb-execution dispatcher unreferenced (arb-execution/src/lib.rs:153) |
| F3 | High | Tip accounts included in master ALT (against policy) and writable; v1_fallback tip-strip predicate never matches the marked tip (3 accts vs `== 2`); tip would double-pay if V1 path ran | master_alt.rs:537-548; lib.rs:256-258,292-300; v1_fallback.rs:199-232 |
| F4 | High | V1 is test-only, unreachable, gated off (1232), Jito-incompatible, and unverified against real SIMD-0385; `DispatcherConfig` default claims `v1_transport_supported: true` | adaptive_compiler_tests.rs (unit-only); dispatch.rs:170; arb-execution/src/lib.rs:130-140; v1_fallback.rs:9-10 |
| F5 | High | ALT authority/activation/deactivation/warm-up enforced only in dead code paths; nothing in arb-bot fetches or verifies the ALT; `mark_activated` only in tests | compiler.rs:947-961; v0_compiler.rs:159-173; arb-bot grep; master_alt.rs:624-700 |
| F6 | Medium | `v1_fallback` emits V1 with legacy ComputeBudget ixs still present and a heuristic header (`compute_v1_header`) inconsistent with its own canonical sort; remap pass is no-op | v1_fallback.rs:199-232, 315-331, 457-523, 277-313 |
| F7 | Medium | Size estimation triplicated and inconsistent (`sizing.rs` wrong header counts; compilers correct-as-measured); no loaded-accounts-data-size or 256-account cap checks on v0/legacy; no `MAX_ACCOUNTS_PER_TX` anywhere | sizing.rs:53-228; compiler.rs:761-937; grep MAX_ACCOUNTS_PER_TX = ∅ |
| F8 | Medium | `create_default_mainnet_alt()` ships fabricated `ALT111…` key used by 15+ tests to sign v0 txs — normalizes an impossible ALT; run.rs correctly refuses it but tests don't | lib.rs:3679-3708; challenger_m2_tests.rs:613,691; adversarial_tests.rs; bundle_tests.rs |
| F9 | Low | Duplication/inconsistency: SPL-ATA-as-Token-2022 owner mismatch for mints/ATAs; local USDT const duplicates arb-constants; padding bytes 54..56 unchecked; multi-table `break` first-match; `owner_program` never verified | master_alt.rs:44,447,512-535,116; lib.rs:3551-3566 |
| F10 | Info | ALT machinery (parse, dedup, capacity, deactivation detection, event-authority seed check) is genuinely solid — the only broken links are call-site table contents (F1) and total live-path disuse (F2/F5). | master_alt.rs:77-148, 414-430, 716-731 |

**What must happen before any ALT-based execution is enabled:** (1) pass the *full* on-chain ALT into `compile_v0_message` (as the integration test does) or map referenced keys → real table indices inside the callers; (2) wire a runtime ALT preflight (fetch ALT account, `from_onchain_readonly`, `verify_warmup`) ahead of fast-path signing, or at minimum at template-refresh time; (3) evict tip accounts from the ALT or accept and document them as writable loaded indices with the marker readonly; (4) drive the live path to the V0 compiler (or formally retire ALT/V0/V1 and delete the dead dispatcher), and only then re-measure 1232-byte feasibility; (5) re-point size estimation at the compiled bytes, not at `sizing.rs`.
