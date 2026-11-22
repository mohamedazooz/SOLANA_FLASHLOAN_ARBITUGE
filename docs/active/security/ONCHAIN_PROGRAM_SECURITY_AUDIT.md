# On-Chain Program Security Audit — Phase 6 (Mission §11)

- **Date:** 2026-11-22 · Target: `programs/arb-executor` (native Agave split crates, **no Anchor framework** — despite mission wording, the program is a hand-rolled native entrypoint; this is itself documented).
- Tests this branch: `cargo test -p arb-executor` → **36 passed / 0 failed**. SBF build NOT run this session (toolchain absent — `cargo-build-sbf` unavailable; recorded honestly below).

## 1. Instructions audited

`execute_route` (354 L), `execute_three_leg` (468 L), `execute_three_leg_with_tip` (233 L), `profit_guard` (144 L), entry dispatcher (`lib.rs:60–80`).

## 2. Security property verification

| Property | Verdict | Evidence |
|---|---|---|
| Signer validation | ✅ `user.is_signer` enforced (`validation/accounts.rs:193`) | every instruction requires the payer/user signature |
| Protocol program substitution | ✅ hard-coded `METEORA_DLMM_PROGRAM_ID`, `MEMO_PROGRAM_ID`, `METEORA_EVENT_AUTHORITY` checked per instruction | `validate_protocol_programs` |
| Arbitrary pool injection | ✅ pool identity chain: WSOL↔MemeA↔MemeB↔WSOL mint closure resolved from actual pool accounts before any balance read (`resolve_route_mint_accounts`); pools must be distinct (`validate_distinct_pools`) | `execute_three_leg.rs:82–125` |
| Mint relationships | ✅ input mint appears exactly once per hop; owner programs of mints match (`output_mint_for_input`) | same |
| Token account ownership | ✅ `validate_user_token_account_for_mint` — owner == user AND mint match AND program-of-mint match (incl. Token-2022 resolution by mint owner) | `validation/accounts.rs` |
| Unauthorized draw of user funds | ✅ CPI swaps only between the user's two hop token accounts; reserves/vaults passed to fixed CPI layout | `cpi/` modules |
| Profit enforcement vs tx-generated balances | ✅ DELTA-BASED: `wsol_baseline = wsol_initial − leg1_amount_in`; per-leg sequential delta guards; final `wsol_final ≥ baseline + min_final_wsol_delta` ⇒ **unrelated pre-existing balances cannot subsidize a losing route** | `execute_three_leg.rs:214–240` |
| Intermediate-token subsidy | ✅ `IntermediateInventorySubsidized` isolation semantics: intermediate hop deltas checked against each hop's own inflow | per-leg guards + tests |
| Jito tip validation | ✅ `validate_jito_tip_account`: owner == System Program AND address ∈ the 8 canonical tip accounts (dumped as on-chain constants; **workspace test cross-checks each constant against `arb-constants` so drift fails CI**) | `validation/jito.rs:29–60` |
| Tip arithmetic | ✅ `tip = floor(edge·bps/10_000)` in u128, `tip_bps ≤ 10_000`, `tip ≤ edge` | `jito.rs:63+` |
| Reentrancy | ⚠️ stateless program (no accounts owned by the program, no PDAs, no state mutation) ⇒ classical reentrancy vulnerability class is structurally absent; all state is the user's token accounts, mutated only via invoked token programs | program data model |
| Rent/ATA handling | ✅ ATA creation happens off-chain (`CreateIdempotent` before the flash loan); program never manages rent | assembly in arb-tx |
| Arithmetic | ✅ `checked_sub`/`checked_add` everywhere; overflow → `CalculationOverflow` error, never wrap | all instructions |
| Replay | ℹ️ blockhash/signature layer outside the program (Solana protocol); instruction data has no nonce fields to replay beyond tx-level guarantees | design note |
| Entrypoint hygiene | ✅ 1-byte discriminator routing, rejects empty data, unknown discriminators rejected | `lib.rs:60–80` + tests (`rejects_truncated_data`) |
| `unsafe` code | ✅ none in `programs/arb-executor/src` (grep audit; only test-module constructor patterns) | grep evidence |
| Panics in production path | ✅ no `unwrap()`/`expect()` outside test modules (grep-verified) | grep evidence |

## 3. Trust boundaries & trust model

- **Trusted fixed addresses:** Meteora DLMM program, Memo v2, Meteora event authority, 8 Jito tip accounts, canonical WSOL mint. Nothing is trusted from instruction data except amounts/counters, all of which are validated against on-chain balances.
- **Never trusted:** instruction-supplied program ids (verified), instruction-supplied pools (verified by mint closure + distinctness), token account balances as claims (always read from account data after CPI).
- **Scenario coverage for account substitution:** arbitrary-programs, arbitrary-pools, wrong-owner token accounts, and Token-2022 combinations each have negative tests (`arb-executor_cpi_tests`, boundary tests, on-chain unit tests).

## 4. Deployment verification — see companion doc

`docs/active/security/PROGRAM_DEPLOYMENT_VERIFICATION.md` — conclusion: **not verified** (no on-chain lookup performed; vanity placeholder id in `arb-constants` cannot establish binary↔source correspondence). No deploy/upgrade performed per safety rules.

## 5. Findings

- **EXEC-F-2026-11-22-N1 (P2):** `ARB_EXECUTOR_PROGRAM_ID = "ArbExec1111111111111111111111111111111111111"` is a vanity placeholder in `arb-constants:658`. The workspace test only checks pubkey parseability, not identity with anything deployed. No live execution path may cite "deployed program verified" until a real deployment identity is recorded and cross-checked.
- **EXEC-F-2026-11-22-N2 (P3):** SBF toolchain absent in this session — binary build and size/CU budget not re-verified here (was verified in earlier sessions per repo logs); re-verify before any deployment attempt.

## 6. Evidence commands

```text
cargo test -p arb-executor → 36 passed / 0 failed
grep unsafe/unwrap/expect over programs/arb-executor/src → none outside tests
```
