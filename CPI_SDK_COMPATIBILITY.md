# CPI Phase 1 — SDK / IDL Compatibility Matrix

**Report date:** 2026-11-16
**Method:** official upstream sources fetched live (GitHub raw/API, crates.io,
docs.rs, npm, solana.com, docs.meteora.ag) plus local source inspection. Every
factual claim carries a source. Items that could not be verified are marked
**NOT VERIFIED**. No dependency was added and no lockfile was changed by this
campaign.

---

## 1. Compatibility matrix

| Component | Verified version / ID / commit | Compatibility with Rust 1.99.1 / edition 2024 / `solana-pubkey 2.2` / `solami 0.1.58` | Source |
|---|---|---|---|
| Kamino KLend program | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`; repo HEAD `686913ee421e0c5f3dc44c4c8344461ee0e4fb89` (release 1.28.0) | Flash borrow/repay must be **top-level** (CPI rejected) — builder-side integration OK | [README](https://raw.githubusercontent.com/Kamino-Finance/klend/master/README.md), [HEAD](https://api.github.com/repos/Kamino-Finance/klend/commits/master) |
| `klend-interface` (released) | **0.6.0** (crates.io 2026-06-03; docs.rs 2026-10-17); rust-version 1.81, edition 2021; deps `solana-pubkey ^2.1`, `solana-instruction ^2.1`; **no `anchor-lang`** | Best fit for the 2.x workspace | [docs.rs](https://docs.rs/klend-interface), [crates.io](https://crates.io/api/v1/crates/klend-interface) |
| `klend-interface` (git master) | Cargo.toml says **0.11.0** — drift vs crates.io 0.6.0 | Pin a version or commit; a Git dep silently changes API | [master Cargo.toml](https://raw.githubusercontent.com/Kamino-Finance/klend/master/libs/klend-interface/Cargo.toml) |
| Meteora DLMM program | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (mainnet = devnet); memo `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`; event-authority seed `b"__event_authority"` | Program ID stable; current interface is `swap2` | [CPI docs](https://docs.meteora.ag/developer-guides/dlmm/rust-integration/cpi), [events docs](https://docs.meteora.ag/developer-guides/dlmm/program/events) |
| DLMM IDL | `idls/dlmm.json`: name `lb_clmm`, **version 0.12.0**, spec `0.1.0`; last IDL commit `2f540c2107988e0f97efa5917100bb912c96c17a`; repo HEAD `1532e540db41ac405dbf4c5bb2ddfceaa1f98ff1` | New-format IDL; Anchor ≥0.30 can consume it. Compile-test with 0.12.0 **NOT VERIFIED** | [IDL](https://raw.githubusercontent.com/MeteoraAg/dlmm-sdk/main/idls/dlmm.json), [IDL commit](https://api.github.com/repos/MeteoraAg/dlmm-sdk/commits?path=idls/dlmm.json&per_page=1), [HEAD](https://api.github.com/repos/MeteoraAg/dlmm-sdk/commits/main) |
| DLMM TS SDK | `@meteora-ag/dlmm` **1.9.14** (`latest`), depends on `@coral-xyz/anchor 0.31.0`; local ADR pins 1.9.14 / Anchor ^0.30.1 | Reference/test only; client version ≠ program version | [npm](https://registry.npmjs.org/@meteora-ag/dlmm/latest), ADR-0014 |
| `MeteoraAg/cpi-examples` | HEAD `7f428b1fd7798ba4d7e9ec51a716479bbcef466a`; `anchor-lang 0.31.0`, Solana 2.1, rust 1.85; bundled IDL **0.8.2**, CPI uses legacy `swap` | Pattern useful; **stale vs IDL 0.12.0** — copying verbatim builds the wrong instruction | [repo](https://github.com/MeteoraAg/cpi-examples), [HEAD](https://api.github.com/repos/MeteoraAg/cpi-examples/commits/main), [its Cargo.toml](https://raw.githubusercontent.com/MeteoraAg/cpi-examples/main/programs/cpi-example/Cargo.toml) |
| Anchor in root workspace | **None** — no `anchor-lang`/`anchor-spl` in any workspace `Cargo.toml` or `Cargo.lock` | `SDK_PARITY.md` warns against adding old Anchor/Solana 1.x crates for quote parity | local grep; [SDK_PARITY.md](docs/reference/SDK_PARITY.md) |
| `lb_clmm` crate | **0.1.1** (2024-05-11, edition 2021), `anchor-lang ^0.29`, `solana-program ^1.16` | **Incompatible** with Solana 2.x — do not add | [crates.io](https://crates.io/api/v1/crates/lb_clmm), [deps](https://crates.io/api/v1/crates/lb_clmm/0.1.1/dependencies) |
| `meteora-dlmm-sdk` crate | **0.1.0** (Codama-generated); `solana-pubkey ^4.0.0`, `solana-instruction ^3.1`, `solana-program ^2.1.14`, `borsh ^1.5.5`, optional `anchor-lang ^0.30` | **Type-incompatible** with `solana-pubkey 2.2` | [crates.io](https://crates.io/api/v1/crates/meteora-dlmm-sdk), [deps](https://crates.io/api/v1/crates/meteora-dlmm-sdk/0.1.0/dependencies) |
| Official Rust `commons` | Unpublished workspace crate; `commons = { path = "../dlmm-sdk/commons" }` | Not a registry dep; path/git vendoring required | [Rust integration docs](https://docs.meteora.ag/developer-guides/dlmm/rust-integration) |
| `solami` | **0.1.58**, edition 2024; `solana-pubkey ^4`, `solana-instruction ^3`, `solana-message ^4`, `solana-transaction ^4`, `solana-client ^4.2` | Local lock has `solana-pubkey` 2.4.0, 3.0.0, 4.3.0 simultaneously → distinct Rust types, no implicit interop | [crates.io](https://crates.io/api/v1/crates/solami/0.1.58), [deps](https://crates.io/api/v1/crates/solami/0.1.58/dependencies), local `Cargo.lock` |
| Solana Transaction V1 | 4,096 B; 64 inline accounts; no ALT; duplicate addresses rejected; explicit CU limit + loaded-data-size limit required; base64 for >1,232 B; `maxSupportedTransactionVersion: 1` (integer) on reads; `txv1` epoch 1035 / 2026-09-15. SIMD-0596 (96 accounts) is a **draft**, not active | Consistent with repo V1 invariant | [larger transactions](https://solana.com/upgrades/larger-transaction-sizes), [v1 account limit](https://solana.com/upgrades/v1-account-limit) |

---

## 2. Kamino flash loan — CPI rejection CONFIRMED (directive §3)

Sources: [`flash_ixs.rs`](https://raw.githubusercontent.com/Kamino-Finance/klend/master/programs/klend/src/lending_market/flash_ixs.rs), [`ix_utils.rs`](https://raw.githubusercontent.com/Kamino-Finance/klend/master/programs/klend/src/lending_market/ix_utils.rs), [borrow handler](https://raw.githubusercontent.com/Kamino-Finance/klend/master/programs/klend/src/handlers/handler_flash_borrow_reserve_liquidity.rs), [repay handler](https://raw.githubusercontent.com/Kamino-Finance/klend/master/programs/klend/src/handlers/handler_flash_repay_reserve_liquidity.rs).

- Both handlers take `sysvar_info: AccountInfo` constrained to `sysvar::instructions::ID`.
- `InstructionLoader::is_flash_forbidden_cpi_call()` uses `load_current_index_checked` / `load_instruction_at_checked` plus `get_stack_height()`: if the current instruction's `program_id != crate::ID` **or** stack height > `TRANSACTION_LEVEL_STACK_HEIGHT`, it returns `true`.
- Borrow → `LendingError::FlashBorrowCpi` ("Flash Borrow was called via CPI!"); repay → `LendingError::FlashRepayCpi`.
- On-chain: unique borrow/repay pair, discriminator match, amount match, `borrow_instruction_index` ordering, reserve at account index 3, identical account lists.
- `flash_repay` `liquidity_amount` must equal the **borrow principal**, not principal+fee. Fee is collected by a separate transfer to the fee vault.
- Both handlers require `user_transfer_authority: Signer` → **a PDA cannot sign the top-level repay**; a user-owned WSOL token account with a real signer is the only proven model (directive §3.1).
- Published 0.6.0 API: `helpers::flash::flash_loan(user, &reserve, source, destination, amount, borrow_instruction_index, referrer) -> (Instruction, Instruction)` ([source](https://raw.githubusercontent.com/Kamino-Finance/klend/master/libs/klend-interface/src/helpers/flash.rs), [example](https://github.com/Kamino-Finance/klend/blob/master/libs/klend-interface/examples/flash_loan.rs)); low-level `instructions::flash::{flash_borrow_reserve_liquidity, flash_repay_reserve_liquidity}` ([source](https://raw.githubusercontent.com/Kamino-Finance/klend/master/libs/klend-interface/src/instructions/flash.rs)). **No `init_flash_borrow` / `flash_borrow` exist in 0.6.0.**

**Conclusion:** the directive's non-negotiable architecture is correct and is already what the repository does — borrow/executor/repay as top-level instructions in one transaction.

---

## 3. Meteora `swap2` exact interface (IDL 0.12.0)

- Discriminator `[65, 75, 63, 76, 235, 91, 91, 136]` — matches `arb-constants::METEORA_SWAP2_DISCRIMINATOR`.
- **16 accounts, exact order:** `lb_pair`(w), `bin_array_bitmap_extension`(w, optional), `reserve_x`(w), `reserve_y`(w), `user_token_in`(w), `user_token_out`(w), `token_x_mint`, `token_y_mint`, `oracle`(w), `host_fee_in`(w, optional), `user`(signer), `token_x_program`, `token_y_program`, `memo_program`, `event_authority`, `program`.
- **3 args:** `amount_in: u64`, `min_amount_out: u64`, `remaining_accounts_info: RemainingAccountsInfo`.
- `RemainingAccountsInfo { slices: Vec<RemainingAccountsSlice> }`; `RemainingAccountsSlice { accounts_type: AccountsType, length: u8 }`; `AccountsType` = `TransferHookX | TransferHookY | TransferHookReward | TransferHookMultiReward(u8) | TransferHookReferral`.
- Remaining accounts: transfer-hook groups in slice order, then bin arrays. No hooks → `slices: vec![]`, bin arrays first. Bin-array PDA seeds `bin_array` + lb_pair + index; bitmap extension needed when the route leaves the default bitmap range.
- Generated pattern: `dlmm::client::accounts::Swap2` + `args::Swap2`, `memo_program: spl_memo::ID`, bin arrays appended writable — [official docs](https://docs.meteora.ag/developer-guides/dlmm/rust-integration).
- **The official public CPI example uses legacy `swap`, not `swap2`** ([swap.rs](https://raw.githubusercontent.com/MeteoraAg/cpi-examples/main/programs/cpi-example/src/instructions/dlmm_cpi/swap.rs)) — it is a pattern reference only, not a source for the instruction to build.

---

## 4. Anchor `declare_program!` / `CpiContext`

- Macro reads `<name>.json` from `idls/`, generates `cpi`, `client`, `account`, `program`, `constants`, `events`, `types`, `error`, `parsers`; on-chain CPI via `CpiContext::new(program, accounts).with_remaining_accounts(...)` — [Anchor docs](https://www.anchor-lang.com/docs/features/declare-program).
- Pre-0.30 IDLs failed on mandatory `discriminator`; 0.31 added legacy conversion ([issue #2972](https://github.com/otter-sec/anchor/issues/2972), [0.31 notes](https://www.anchor-lang.com/docs/updates/release-notes/0-31-0)).
- **NOT VERIFIED (confidence: medium):** `declare_program!(dlmm)` compiling against the **0.12.0** IDL. Compile-test in a scratch crate before relying on it. This workspace pins no Rust Anchor crate.

---

## 5. Local program IDs (verified in `crates/arb-constants/src/lib.rs`)

| Program | ID | Line |
|---|---|---|
| Meteora DLMM | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | 161 |
| Kamino Klend | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | 430 |
| SPL Token / Token-2022 | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` / `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | 16 / 18 |
| Memo | `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr` | 349 |
| DAMM v2 / DBC / DLMM Pro | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` / `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` / `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n` | 356 / 369 / 376 |

> **DISCREPANCY to resolve:** `docs/production/kamino_flash_loan.md:12-15` lists
> borrow `[142,114,213,90,143,89,137,26]` / repay `[175,175,109,31,13,152,155,237]`,
> while `arb-constants:445-447` has `[135,231,52,167,7,52,212,193]` /
> `[185,117,0,203,96,245,180,186]`. The docs are likely stale; confirm against the
> deployed program before any live use.

---

## 6. Known limitations / incompatibilities

1. Kamino CPI is impossible — flash instructions must be top-level.
2. `klend-interface` crates.io 0.6.0 vs git master 0.11.0 — pin a version or hash.
3. Official DLMM CPI example is stale (IDL 0.8.2, `swap`, no memo) vs IDL 0.12.0 (`swap2`, memo).
4. Local `Cargo.lock` has `solana-pubkey` 2.4.0 / 3.0.0 / 4.3.0; `solami` and `meteora-dlmm-sdk` require v4 — distinct types, no implicit interop.
5. `lb_clmm` (Solana 1.x) and `meteora-dlmm-sdk` (pubkey v4) must stay out of the runtime crate graph.
6. Event-authority literal is derived, not officially published; only the seed `b"__event_authority"` is authoritative.
7. Upstream CPI examples CI pins rust 1.85 / Solana 2.1 / Anchor 0.31; compatibility with the 1.99.1 BPF toolchain **NOT VERIFIED**.
8. No `solana` or `anchor` CLI is installed here — no BPF build, CPI test, or `simulateTransaction` can be produced in this environment.

## 7. Integration decision

- Root workspace must **not** gain Anchor/Solana 1.x dependencies (would force a 14-crate upgrade — directive §2 forbids).
- Any new on-chain program must be an **isolated workspace** under `programs/` with its own `Cargo.toml`/`Anchor.toml`, excluded from `members = ["crates/*"]`.
- Preferred SDK pins if/when building: `klend-interface = "=0.6.0"` (crates.io, pinned); DLMM `idls/dlmm.json` at commit `2f540c21…` (IDL 0.12.0); Anchor `>=0.31` compile-tested against 0.12.0 before use.
- **No dependency or IDL was pinned in this campaign** — Phase 1 research is complete, integration is not.
