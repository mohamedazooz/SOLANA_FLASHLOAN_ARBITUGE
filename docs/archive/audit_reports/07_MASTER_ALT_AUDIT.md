# 07 — Master ALT Forensic Audit

Evidence: `audit_work/07_ALT_V0_V1.md`; commands run in §15; external pass in `14_EXTERNAL_RESEARCH_REFERENCES.md` §3.
No ALT was created, extended, deactivated or closed — this section analyzed code and the repo's own embedded mainnet ALT snapshot (read-only fixture at `master_alt.rs:891-953`).

## 1. ALT selection and contents (as provisioned by code)

`default_static_entries` (master_alt.rs:375-569) emits 22 required + 2 optional:

| # | Category | Address/source |
|---|---|---|
| 1 | MeteoraProgram | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (:33,:406) |
| 2 | MeteoraEventAuthority | `[b"__event_authority"]` PDA, hard-checked == `D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6` (:38-41,:716) |
| 3 | Memo | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` |
| 4-6 | SPL Token / Token-2022 / ATA program | arb-constants |
| 7-8 | System / ComputeBudget | raw bytes |
| 9-11 | WSOL / USDC / USDT mints | constants |
| 12-14 | Bot WSOL/USDC/USDT ATAs | `derive_ata(payer, mint, spl_token)` (:725) — **SPL-only even for Token-2022** |
| 15-22 | **8 Jito tip accounts** | JITO_TIP_ACCOUNTS (arb-constants:57-66) |
| 23-24 | Optional static bridge pools (SOL/USDC, SOL/USDT) | operator config |

**Total: 22–24 of 256 capacity.** No pool-specific, meme mint, LbPair, bin-array or bitmap-extension
address is ever included (by design; verified in tests and by the real mainnet fixture ALT which holds
7 addresses: Token, Token-2022, System, event authority, Memo, WSOL — *not* USDC/USDT/ATAs).

Duplicates: on-chain parse rejects duplicates (:126-133); provisioning dedups first-seen (:585-609,:762-772).
`compile_v0_message` does **not** reject duplicates across tables and silently `break`s on first hit (lib.rs:3550-3566) — multi-table unguarded (out of scope for single master ALT).

Signers: structurally impossible on the wire (non-signers only in tables, lib.rs:3540-3547); `verify_no_signers` enforced in V0 compilers (compiler.rs:947-951). Tip accounts **are** in the ALT (not signers — legal on-chain, but diverges from the required policy that tips stay inline).

## 2. Suitability classification

| Class | Members | Verdict |
|---|---|---|
| A. Reusable static programs | DLMM, Token, Token-2022, ATA, System, ComputeBudget, Memo, event authority | ✓ correct in ALT |
| B. Reusable protocol accounts | bridge pools (optional) | ✓ optional |
| C. Pool-specific state (lb_pair, reserves, oracle, bitmap ext) | never in ALT | ✓ correct — must stay excluded |
| D. Dynamic bin arrays | never in ALT | ✓ correct — pool/direction-specific |
| E. Wallet/user ATAs (meme ATAs created this tx) | never in ALT | ✓ correct |
| F. Bot ATAs (WSOL/USDC/USDT) | in ALT | Borderline ✓ (stable, payer-owned; but derived only for SPL) |
| G. Signers / payer | never in ALT | ✓ enforced at compile (V0 path) |
| H. Jito tip accounts | **in ALT** | **✗ must be inline writable system accounts; remove** |
| I. `jitodontfront` marker | not in ALT | ✓ correct (nonexistent account must be inline readonly) |

## 3. ALT lifecycle verification — exists, never runs at runtime

`parse_alt_account_data` (master_alt.rs:77-148) correctly models: meta size 56, discriminant, 32-byte
alignment, ≤256 addresses, authority `Option<Pubkey>`, padding tolerated (not checked for zero — minor).
`from_onchain_readonly` rejects deactivated tables (deactivation_slot ≠ MAX) and frozen tables when authority
expected (:640-660); `verify_warmup` requires `current_slot > last_extended_slot` (:687-701);
`verify_activation` (:335-352). **All callers are in the V0 compilers, which `arb-bot` never invokes.**
Nothing on the live path fetches the ALT; a deactivated/replaced ALT is never noticed. `mark_activated`
only in tests. Tier-1/2 template caches persist no table generation — a rebuilt ALT would not invalidate
cached templates (silent correctness failure if the V0 path is ever wired in).

Per official ALT semantics (14_EXTERNAL_RESEARCH §3): addresses extended at slot S are unusable at S
(`last_extended_slot_start_index`); deactivation is permanent ~512-slot cool-down then close; table owned
by ALT program; runtime loads ≤64 accounts regardless of table size. The audited ALT code models warm-up
and deactivation correctly but is dead code in production.

## 4. On-wire behavior (measured design, not live measurement)

`compile_v0_message` emits: `compact_u16(tables)` + per table `key(32B) + u16(writable) + indices + u16(readonly) + indices` (lib.rs:3637-3650). Zero-reference tables omitted entirely (should yield a `compact_u16(0)` section — 1 byte). Every referenced ALT-loaded account saves ~31 bytes (32B key → 1B index) minus ~1B index. Fixed lookup overhead ≈ 36 bytes per table; break-even ≈ 2 ALT references.

**BUG (CONFIRMED CRITICAL): filtered-subset index emission.** `try_compile_v0` builds
`AddressLookupTable { key, addresses: alt_accounts.to_vec() }` where `alt_accounts` is the
**referenced-only subset** from `partition_accounts` (compiler.rs:639-711, 967-970);
`compile_v0_message` indexes via `table.addresses.iter().position(...)` (lib.rs:3552-3554).
The on-wire index is therefore position-in-subset, not position-in-on-chain-table — **every ALT-loaded
reference points at the wrong account.** Same in `v0_compiler::compile_v0` (v0_compiler.rs:183-191).
Same skewed membership in `sizing.rs` estimates. The mainnet integration test (full table) is the only
artifact that resolves indices correctly — and it is `#[ignore]`d.

## 5. Serialized size with/without ALT (arithmetic proof from code)

Legacy live path (no ALT): 3-leg meme route ≈ 60 keys × 32 B ≈ 1,900 B + header/sigs ≈ **2,000–2,300 B** — exceeds 1232 → dispatch.rs:170 silently drops the opportunity.

V0 with 22-member ALT: ~14 references resolve via 1-byte indices → save ~434 B → ≈ **1,600–1,750 B** — still usually > 1232. Hence the code's own tests accept V1 output (≤4096) or assert only for small fixtures. **Even a correctly-wired V0 path for deep 3-leg routes will frequently exceed 1232 B and fall to the V1 branch — which is gated off by the same 1232 B dispatch check and by Jito's documented rejection of >1232-byte bundles.** The expected live outcome for deep routes is silent skip.

Runtime limits recorded in code: 1232 (constants:367) enforced in compiler.rs:457, dispatch.rs:170,
`validate_jito_bundle` lib.rs:330; V1 4096 (adaptive.rs:547-581); ALT 256 (master_alt.rs:30); extend chunk 30 (alt.rs:16); **no MAX_ACCOUNTS_PER_TX / loaded-accounts check anywhere; de-facto 256 via u8 indices; `pos > 255` silently inlines (lib.rs:3553) instead of erroring.**

## 6. Does the ALT actually reduce anything?

Yes — **address bytes on the wire only**. It does not: reduce runtime account loading; make signers
lookup-eligible; remove account locks (64/tx); reduce compute; guarantee fit. Data-driven tip
policy and an ALT are orthogonal; the Jito tip account **must not** be ALT-loaded (Jito recommends
inline tips; correct in live path today since it is legacy-only).

## 7. Architecture recommendation (NOT actioned — no ALT is created by this audit)

- **One shared static master ALT** (current design) is correct for A+B+F; **remove all 8 tip
  accounts** (Class H) → inline writable.
- Do **not** extend it with bin arrays; instead keep pool-specific state inline (Class C/D/E) —
  per-route ATAs and dynamic arrays cannot be shared across routes anyway.
- Optionally add a **second, protocol-shared ALT** for the 3 most common static DLMM constants if
  the operator ever wires V0 in — but note even then deep routes exceed 1232; the real unlock is
  **fewer per-leg accounts or signed-bundle economics, not ALT size**.
- **Proposed manifest (static, no create action):** Class A (11): DLMM program, event authority,
  Memo, SPL Token, Token-2022, ATA program, System, ComputeBudget, WSOL, USDC, USDT mints.
  Class B (3): bot WSOL/USDC/USDT ATAs. Class C (0 tips — evict). Class D: none beyond current.
  Total 14 entries of 256. Class C/D/E/H/I remain inline. `verify_required_present` should be
  updated to match (currently doesn't require the ATAs or tips — good for tips, bad for ATAs).
- Verification cadence: fetch the ALT once per **epoch** (not per tick), assert owner, activation,
  deactivation, authority, presence of Class A; hard-fail the V0 path on mismatch (already the V0
  compilers' behavior — they just need to become reachable).
