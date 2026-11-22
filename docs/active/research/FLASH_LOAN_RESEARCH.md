# Flash Loan Research (Kamino / Klend)

**S3 verification date**: 2026-11-22.

## Verified facts

| Item | Finding | Source |
|---|---|---|
| Program ID (mainnet) | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (staging = `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh`) | [klend architecture docs](https://www.mintlify.com/kamino-finance/klend/concepts/architecture) (S3) — matches `crates/arb-constants/src/lib.rs` (register S1) |
| Official flow | `getFlashLoanInstructions({ borrowIxIndex, lendingMarketAuthority, reserve, amountLamports, destinationAta, referrerAccount })` → `flashBorrowIx` + `flashRepayIx`; **`borrow_instruction_index` = position of the borrow ix in the final transaction** → must be rewritten after any V1 ComputeBudget stripping | [kamino.com flash-loans tutorial](https://kamino.com/docs/build/tutorials/borrow/flash-loans) (S3) |
| Rust crate | `klend-interface = "0.6.0"` with `solana-pubkey = "2.x"`, zero-copy `ReserveInfo::from_account_data`, `instructions_sysvar` required and validated by address | kamino.com tutorial + docs.rs/klend-interface (S3) |
| Fee formula | rate stored as U68F60 scale factor; fee = `max(amount·sf, 2^60)` raw bits with ties-away rounding → repo's `arb-math/economic.rs` klend implementation matches; community measurement of USDC reserve: 0.001% (10,000 USDC → 0.1 USDC) | community tooling [30daysmoney PR #14](https://github.com/bigmo6286/30daysmoney/pull/14) (S3, secondary) + in-repo oracle vectors (binding) |
| Zero-fee reserves | cbBTC, dSOL, EURC, JitoSOL, JupSOL, PYUSD, USDG, USDS report `flashLoanFeeRate = 0` on the Main market currently — must be read at runtime per reserve, never assumed | [kamino-tools README](https://github.com/Joyobroto/kamino-tools) (S3, secondary) |
| Repayment semantics | klend's flash repay can consume any available shortfall from the user's wallet balance ⇒ a wallet holding the loan token can silently subsidize a losing route | [30daysmoney PR #14](https://github.com/bigmo6286/30daysmoney/pull/14) (S3) |
| Sysvar scan scope | flash-repay validates repayment by scanning **top-level** instructions for the borrow; CPI-nested borrows can evade the scan — this is exploit research, never a pattern to use | [adpthegreat/bypassing-kamino-restrictions](https://github.com/adpthegreat/bypassing-kamino-restrictions) (register S1, inherited) |

## Existing implementation

- Canonical fee math: `crates/arb-math/src/economic.rs` (single source of truth; re-exported by arb-tx).
- Builders: `crates/arb-tx/src/flash/kamino_flash.rs` (real builder; `kamino.rs` is the wrapper namespace), assemblers `arb_executor.rs`, `dlmm_3leg_flash_arb.rs`, `cross_dex_flash_arb.rs`; `atomic_integrity.rs` enforces borrow→swaps→repay ordering.
- On-chain profit guard: `programs/arb-executor` asserts final balances against `borrow + fee + min profit`, which neutralizes the wallet-balance subsidy path (delta-based, not absolute-balance-based).

## Gaps / recommendations (REC)

1. Fee must be decoded from the live reserve at sizing time (U68F60 sf), never hardcoded — verify during Phase 2 forensic audit that sizing paths feed live reserve bytes into `arb-math` (tracked in FLASH_LOAN_FORENSIC_AUDIT work).
2. Explicitly require the wallet to hold zero of the intermediate/loan balances for flash routes, or rely solely on the on-chain delta guard; off-chain gross checks must not treat absolute balances as trade output.
3. Marginfi/Solend as alternative providers: SDKs exist but adding a provider multiplies borrow/repay account-lattice surface — keep single-provider until Kamino path is live-proven (decision per DECISIONS.md).

## Tests required

- Repayment invariant: a route that returns `amount + fee − 1` must fail the whole tx deterministically (LiteSVM + on-chain guard test).
- Fee-boundary vectors: amount·sf exactly at 2^60 floor, ±1 lamport above, ties-away rounding (oracle tests exist; keep vectors pinned).
- Same-reserve coupling: regression that changing the borrow reserve invalidates the compiled repay account set (compiler index-remap test).
