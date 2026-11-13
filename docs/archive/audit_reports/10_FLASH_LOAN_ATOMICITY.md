# 10 — Flash Loan Atomicity Audit

Provider: **Kamino Lend (`klend`)** — program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
(arb-constants/src/lib.rs:322); main market `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF`
(:324), authority PDA `9DrvZvy…` (:326 — derivation test with bump 248, kamino.rs:272-283),
WSOL reserve `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q` (:328), supply vault
`GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U` (:330), fee vault `3JNof8…` (:332).
Flash restricted to WSOL (`NonWsolFlashLoan`, lib.rs:3131-3133). USDC reserve catalogued, unused.

## 1. Instructions and pinning

No Anchor SDK/IDL is consumed at build time — instructions are hand-packed with hard-coded
discriminators:

| Instruction | Discriminator | Source check |
|---|---|---|
| `flash_borrow_reserve_liquidity` | `[135,231,52,167,7,52,212,193]` | = sha256("global:flash_borrow_reserve_liquidity")[..8] (arb-constants:337) ✓ matches official handler name |
| `flash_repay_reserve_liquidity` | `[185,117,0,203,96,245,180,186]` | (constants:339) |

Account offsets of the live reserve layout are **hard-coded byte offsets**:
`FLASH_LOAN_FEE_SF_OFFSET = 8+4896`, `STATUS_OFFSET = 8+4848`, `MINT_OFFSET = 8+120`,
`DECIMALS_OFFSET = 8+264` (kamino.rs:197-200), with the caveat that the deployed program predates
the master-branch layout and offsets must be re-verified on any upgrade (kamino.rs:190-196).
Fee is **read live from the reserve account** and decoded fail-closed: discriminator, length, mint,
active status all verified; any mismatch is an error, never a default (kamino.rs:204-250). Fee math:
`ceil(borrow × sf / 2^64)` (kamino.rs:170-179). SF is U68F60 fixed point — not bps.
Live sf ≈ 11_529_215_046_068 observed 2026-11-04 (kamino.rs:185-187) ≈ 6.25e-7 — consistent with the
official SOL/BTC market 0.001% claim order-of-magnitude-wise (0.006 bps vs 1 bp).

Data layouts documented and encoded at kamino.rs:82-128. Borrow 16 B, repay 17 B (amount+index).
`borrow_instruction_index` = actual build-time index (lib.rs:3121, 3205; split path :3301, :3316).

## 2. Atomicity — enforced by construction, not inferred

Order: `[CU limit] → [CU price] → (ATA creates) → borrow → swaps → repay → (closes) → tip`.
- Borrow and repay are top-level instructions in **one transaction** — Kamino forbids CPI flash loans
  (external source, §8). Repay carries `borrow_instruction_index`; Kamino's handler validates:
  current index > borrow index, borrow ix belongs to this program with the borrow discriminator,
  `accounts[3]` of borrow ix == this reserve, `liquidity_amount` matches exactly (official handler
  source, verified).
- One borrow per transaction only (`MultipleFlashBorrows`), repay mandatory (`NoFlashRepayFound`) —
  enforced on-chain.
- If any hop fails min-out or repay cannot be satisfied: the whole tx reverts; **nothing lands, no
  tip is paid, no fee is charged** (all inside one tx). This is genuine atomicity — verified from
  instruction ordering + Kamino semantics, not from a successful quote.

## 3. Confirmed defects (flash-path specific)

1. **`build_kamino_bundle`/`assemble_atomic_bundle` uses `flash_loan_fee_sf = 0`** (lib.rs:2831-2833).
   Repay = B, missing ceil(B·sf/2⁶⁴) ⇒ `InsufficientRepayAmount` on-chain against the live (non-zero)
   fee whenever this builder is used. Comments admit "callers must re-price". All fixtures pass 0
   (`:3959`, `:4017`). **Production footgun; untested against real sf.**
2. **`KaminoBundleParams` sets `min_profit_raw = TokenAmount(opp.net_profit.as_u64())`** — lamports as
   base raw (lib.rs:2824).
3. Same-zeroed-fee pattern in `assemble_atomic_bundle` test path.
4. **Guarantee-of-failure shapes that nothing catches before dispatch**: Pump.fun-first flash route
   closes the WSOL ATA pre-swap (lib.rs:3165-3167, re-verified in this audit) ⇒ `CloseAccount` on a
   non-zero account ⇒ revert, 100% of the time. No `simulateTransaction` on the hot path
   (run.rs:1389-1395) ⇒ signed, tipped-into-bundle bytes that can never land.
5. Jupiter flash route (`assemble_jupiter_flash_arbitrage`, :3228) creates **no ATAs at all** — cannot
   run on a fresh wallet; `route_via_jupiter = false` (run.rs:1110) — dead but compiled.
6. Flash route splits **no** instruction pairing across transactions; split route is disabled
   (run.rs:1009) — OK.

## 4. Failure-mode matrix (live flash path)

| # | Condition | On-chain | Off-chain visibility |
|---|---|---|---|
| F1 | Adverse fill, min-out met | lands, profit < expected | Landed; PnL = expected (not actual) |
| F2 | Hop min-out not met | revert; nothing lands; tip unpaid | Failed; 5_000 lamports modelled loss |
| F3 | Final ATA < repay | revert at repay | Same; silent economics leak |
| F4 | First hop Pump.fun | **always revert (§WALLET §3)** | Failed every time |
| F5 | Pre-existing intermediate ATA, bal>0 | revert at final CloseAccount | Failed |
| F6 | Pre-existing intermediate ATA, bal=0 | ATA destroyed; rent → payer | Landed but user account gone |
| F7 | Token-2022 intermediate | SPL-vs-2022 mismatch; revert | Failed |
| F8 | Reserve decode mismatch / sf unreadable | tick skipped pre-quote (fail-closed) | debug log only |
| F9 | t_max < 1_000 lamports | never dispatched | rejection alert |
| F10 | Verified inputs stale/missing | tick skipped | debug log |
| F11 | ProfitFloorNotMet at build | never signed | silent (`?` in callers) |
| F12 | Bundle rejected/expired | dropped; 0 loss | Dropped path |

## 5. What a passing audit of this section would require

- A compiled transaction for a real route passing `simulateTransaction` against current mainnet
  (blocked — audit forbids broadcast; simulation-only RPC is permitted by the brief if configured;
  not configured for this audit's environment).
- Repay-index check via the instructions sysvar on a fork (litesvm) — not in repo tests.
- Fixture with sf read from a **captured mainnet reserve snapshot** and repay exact-match.
- End-to-end `assemble_flash_arbitrage` test asserting the close-ATA ordering bug (would fail today).

## 6. Verdict

Atomicity semantics are correctly engineered (same-tx, index-bound, WSOL-only, fail-closed decode).
**Atomicity ≠ executability**: the assembled instruction sequence for common route shapes cannot
execute, and CI cannot discover this because the live path is never simulated and fixtures use sf=0.
