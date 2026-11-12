# 09 — Wallet and Profit-Destination Audit

Evidence: `audit_work/09_WALLET_FLASHLOAN.md` (401 lines; re-verified sections below by reading source).

## 1. Instruction-level fund flow — canonical flash route

| # | Instruction | Source | Destination | Mint | Amount | Authority |
|---|---|---|---|---|---|---|
| 0 | SetComputeUnitLimit | — | — | — | — | payer |
| 1 | SetComputeUnitPrice | — | — | — | — | payer |
| 2 | CreateATA(WSOL ATA, idempotent) | rent from payer | ATA | — | rent | payer |
| 3 | (Hop1=PumpFun only) CloseAccount(WSOL ATA) | ATA | **payer — but account holds borrowed WSOL** | WSOL | rent | payer |
| 4 | Kamino FlashBorrow(WSOL, B) | reserve vault | **payer's canonical WSOL ATA** | WSOL | B | payer (user_authority) |
| 5.. | Swap hops | payer ATAs ↔ pool vaults | payer ATAs | per-leg | per-leg | in-pool |
| n | (last hop Pump.fun) CreateATA+Transfer+SyncNative for repay | payer | WSOL ATA | SOL→WSOL | B+fee | payer |
| m | Kamino FlashRepay(B + fee), index = borrow slot | payer's WSOL ATA | reserve vault | WSOL | B + ceil(B·sf/2⁶⁴) | payer |
| m+1.. | CloseAccount(intermediate ATAs) | ATA | payer | per-mint | rent | payer |
| last | SystemProgram::Transfer → tip account (+`jitodontfront` marker readonly) | payer | tip account (round-robin of 8) | **SOL** | tip | payer |

**Every borrowed asset arrives in the payer's canonical WSOL ATA.** All intermediate tokens land in
the payer's ATAs (SPL-derived on the hot path). Final output (≈ E − B) lands back in the payer's WSOL
ATA (or native SOL if the last hop is Pump.fun; the repay re-wrap consumes exactly B+fee, residual SOL
remains in the wallet). **Residual profit never leaves the bot wallet.** No operator fee, referral,
treasury or shared account exists anywhere in the flow.

## 2. Repay account correctness

`user_wsol_ata` is both borrow source and repay sink; repay builder marks it writable, authority =
payer; `borrow_instruction_index` is bound at build time (`borrow_idx as u8`, lib.rs:3121/3205) —
**not** the hard-coded 2 (that is only in the legacy `build_kamino_flash_repay` wrapper, :422).
Kamino enforces on-chain (same-tx, not CPI, one borrow, repay present) — see §4, and the external
IDL evidence in `14_EXTERNAL_RESEARCH_REFERENCES.md` §8.

## 3. Canonical-ATA destruction (CONFIRMED CRITICAL)

`assemble_flash_arbitrage` closes the payer's **canonical** WSOL ATA at
`crates/arb-tx/src/lib.rs:3165-3167` — before Hop 1 executes — while the borrowed `B` WSOL sits in it.
SPL `CloseAccount` requires `amount == 0` ⇒ **every Pump.fun-first flash route reverts 100%**. Same
defect in `assemble_arbitrage` at :2968-2970 (close at :2963-2965 after hops is benign only if the
ATA was re-created; the pre-swap close is fatal either way).

Intermediate ATAs (meme mints): closed unconditionally at :3208-3211 and :2973-2975 with no
provenance/balance/withheld-fee checks — a pre-existing canonical zero-balance ATA is destroyed
(rent → payer); a pre-existing non-zero ATA bricks the route (revert). The correct policy exists
(`should_close_intermediate_ata`, compiler.rs:600-632 — tracks `meme_*_ata_exists`,
balance, Token-2022 withheld fees) but is wired **only** into the 3-leg meme compiler, which no live
path calls. The hot path uses `build_close_token_account` — SPL-only opcode 9
(lib.rs:533-550). Token-2022 account close via SPL program fails ⇒ revert; withheld fees never
inspected.

## 4. Atomicity proof status (not inferred from quotes)

| Property | Evidence | Verdict |
|---|---|---|
| Borrow & repay in the same transaction | instruction order + `borrow_instruction_index` binding (kamino.rs:85/124; lib.rs:3200-3206) | ✓ enforced by construction |
| Repay amount ≥ borrow + live fee | repay = B + ceil(B·sf/2⁶⁴), sf read live from reserve, fail-closed decode (kamino.rs:204-250) | ✓ |
| Wrong repayment mint | WSOL only enforced (`NonWsolFlashLoan`, lib.rs:3131-3133) | ✓ |
| Wrong repayment account | Same account as borrow destination by construction | ✓ |
| Reorder protection | Kamino sysvar check (external, verified in source) | ✓ |
| Insufficient final balance | **on-chain revert; tip unpaid** (tip is inside the same tx) | ✓ by atomicity — but never exercised by simulation |
| Partial hop execution | Impossible for same-tx instruction set | ✓ |
| Flash fee in `build_kamino_bundle` path | **`flash_loan_fee_sf = 0` hard-coded** (lib.rs:2831-2833) ⇒ that builder's repay is always short (Kamino requires amount ≥ borrow; the missing fee makes even a perfect route revert if anything routes through `assemble_atomic_bundle`) | **FAIL — confirmed defect** |
| Whole live path never simulated | hot path dispatches without `simulateTransaction` (run.rs:1389-1395); RPC preflight exists for Live mode (dispatch.rs:217-234) but guaranteed-revert shapes (F4/F5/F7) are structurally undetectable by this accounting | **FAIL** |

Test coverage: `bundle_tests.rs` asserts wire layout & discriminators; `intermediate_balance_isolation_tests.rs`
is a **model-based** (`WalletBalanceModel`) test of the Meteora two-swap builder only — hop 2 input
bound to hop1_min_out. **No test covers `assemble_flash_arbitrage` end-to-end**; no test covers WSOL
repay-source interplay; no test covers canonical-ATA close rules. `p0_golden_vectors.rs` = byte layouts.

## 5. Intermediate balance isolation

- Hop `i+1` input is bound to the previous hop's expected output discounted by slippage (`derive_swap_legs`,
  lib.rs:3018-3055; final hop floor = B + fee + tip + min_profit) — pre-existing inventory cannot be
  silently consumed beyond hop-1 delivery (fail-closed revert if short).
- But hop `i+1` is bound to hop-1 *expected* output, not hop-1 **min_out** (unlike the Meteora atomic
  builder, meteora_atomic_builder.rs:185-187,225,253) — adverse fills revert instead of degrading.
- Pre-existing WSOL sits in the same canonical ATA as the borrow; repay draws `B+fee` from it —
  a route with zero true arbitrage edge can still drain user WSOL up to the ATA balance. Isolation
  is therefore **all-or-nothing by atomicity**, which is fine for solvency but does not protect the
  user's pre-existing balance from over-generosity of the quote. The isolation tests prove the
  Meteora builder only; the flash path is untested for this property.

## 6. Where profit actually sits (final answer)

**Residual profit remains in the payer's canonical WSOL ATA as WSOL** (or, when the last hop is
Pump.fun, as native SOL after re-wrapping exactly B+fee). Spendable only by the bot keypair.
Reported profit fields (`ExecutionOpportunity.net_profit`) are `Lamports` computed from base-raw via
the verified rational rate; USD figures are `profit_sol × sol_price` with the same price driving both
directions of the 6-decimal conversion — self-consistent for WSOL, mislabeled for any other base.

## 7. Answers to the commissioned questions

1. Borrowed asset → payer's canonical WSOL ATA.
2. Wallet: bot keypair — the only signer anywhere.
3. Fees: swap fees in-pool; base/priority/tip/rent from payer native SOL; flash fee from WSOL ATA inside repay.
4. Intermediates → payer ATA of each intermediate mint (SPL-derived on hot path; token-program-aware in the Meteora standalone builder and PumpSwap).
5. Final output → payer WSOL ATA (or native SOL).
6. Repay → payer's WSOL ATA.
7. Residual profit → payer WSOL ATA / native SOL.
8. Mint of residual profit → WSOL (or SOL).
9. Wallet that can spend it → bot.
10. Same account as the profit calc? **No** — profit is a quote-time pure computation (`build_economic_quote`,
    wall-clock stamp, WSOL semantics), never reconciled against the actual ATA balance delta; `dispatch.rs`
    books only on `Landed` with the expected-not-actual delta (`net_profit_val = opp.net_profit`). **A positive
    reported profit cannot distinguish a real win from a subsidized fill.**

## 8. Wallet-level fund-flow diagram

```
reserve vault --borrow(B)--> [payer WSOL ATA] --swap legs--> (USDC/meme ATAs) --final hop--> [payer WSOL ATA]
                                     |                                                              |
                                     └──────── repay(B+fee) ←───────────────────────────────────────┘
                                                           │
                                   [tip] --native SOL transfer--> tip account (8-rotated)
                                                           │
                          ATA closes --> rent reclaim --> payer wallet
```
