# 09 — Wallet / Flash-Loan Flow & Atomicity Audit

**Scope:** `crates/arb-tx` (lib.rs, kamino.rs, meteora_atomic_builder.rs, adaptive.rs, jupiter.rs, compiler.rs, pump_fun_raydium.rs), `crates/arb-engine` (lib.rs, three_leg_meme_pipeline.rs), `crates/arb-math/src/economic.rs`, `crates/arb-bot` (supervisor/run.rs, dispatch.rs, execution.rs, verified_config.rs, supervisor.rs), `crates/arb-types` (lib.rs, dimensional.rs), `crates/arb-constants/src/lib.rs`, plus the listed tests. Read-only; no code executed. Date: 2026-11-11.

Notation: `base = WSOL` throughout the live path — `build_verified_inputs` hard-codes `arb_tx::WSOL_MINT` as base and `KAMINO_WSOL_RESERVE` as the reserve (`crates/arb-bot/src/supervisor/execution.rs:159-160`). All "base-raw" and "lamports" coincide numerically (rate 1:1, `verified_config.rs:95-97`), which masks several latent unit-mismatch bugs that would bite a non-9-decimal base mint.

---

## 1. End-to-end asset flow (which token account, whose ATA, who pays what)

### 1.1 Where the borrowed asset arrives
- Kamino flash borrow deposits `amount` of WSOL into **the payer's canonical WSOL ATA** (`user_wsol_ata = get_associated_token_address(&payer, &WSOL_MINT)`, SPL-Token PDA):
  - account derived `arb-tx/src/lib.rs:3085`, `get_associated_token_address` at `:442-448` (SPL Token program id hard-wired at `:443`),
  - account set resolved `KaminoFlashParams::to_flash_accounts` (`:379-409`; `user_destination_liquidity` falls back to the payer's WSOL ATA at `:385-391`),
  - borrow instruction built `build_kamino_flash_borrow` `:412-419` → `kamino::build_kamino_flash_borrow` `kamino.rs:85-119`, `AccountMeta::new(user_liquidity_ata, false)` at `kamino.rs:110`.
- Borrowed-source vault: `KAMINO_WSOL_SUPPLY_VAULT = GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U` (`arb-constants/src/lib.rs:330`); lending market `KAMINO_MAIN_MARKET = 7u3HeHx…` (`lib.rs:324`); claim is signed by the payer (`user_authority` signer, `kamino.rs:104`).
- The borrow instruction sits at index `borrow_idx = ixs.len()` **after** ComputeBudget ixs plus any idempotent ATA creations (`arb-tx/src/lib.rs:3120-3122`, `321` for split path at `:3301`).

### 1.2 Who pays fees (and what kind)
- **Base signature fee (5,000 lamports)**: paid by `payer` from native SOL. Constant `BASE_FEE_LAMPORTS: u64 = 5_000` in `arb-bot/src/supervisor.rs:43`, applied at `run.rs:678`, `813`, `963`, and `1495` (cosmetic log claims "~0.000010 SOL").
- **Priority fee**: `payer` pays via `SetComputeUnitPrice`; `micro_lamports = min(desired, adaptive cap)` — `run.rs:655-661`; CU price actually injected is re-derived in the builder by `calculate_dynamic_cu_price` (`arb-tx/src/lib.rs:3076-3079`).
- **DEX swap fees**: taken in-pool from the swap amount (expected_out is net of them), not paid by wallet.
- **Kamino flash-loan fee**: repaid inside the repay amount (`borrow + fee`), debited from the payer's WSOL ATA (`arb-tx/src/lib.rs:3186`, `3201-3206`; `fee = calculate_kamino_flash_fee(borrow, fee_sf)` `:3143`, fee math `kamino.rs:170-179`, Q64.64 ceiling division).
- **Jito tip**: `payer` → tip account, **native SOL transfer as the last instruction**, wrapped in a `jitodontfront`-marked system transfer (`arb-tx/src/lib.rs:243-260`, `288-...`, emitted last in all assemble paths: `:2807-2814`, `:2977-2984`, `:3213-3220`, `:3269-3276`, `:3319-3326`). Tip account round-robin from 8 accounts (`arb-constants/src/lib.rs:57`, `:85`).
- **Rent for created ATAs**: `payer` pays (`build_create_ata_idempotent`, payer = wallet_owner, `arb-tx/src/lib.rs:464-486`).

### 1.3 Where intermediates go
- **Native (non-Jupiter) route**: hop N's output is credited to the payer's ATA **of the same mint**, derived via the SPL-Token ATA program (`get_associated_token_address`, `arb-tx/src/lib.rs:2216-2217`, `2220-2223`, `2260-2261`, `2278-2279`, `2295-2296`, `2338-2339`, `2421-2423`, `2491-2493`). Meteora/Orca/Whirlpool/Raydium-CPMM/Pump.fun-CPMM user ATAs are **all SPL-Token-program-derived**, even for Token-2022 mints (see §5.4). PumpSwap uses token-program-aware derivation (`:1828-1857`).
- The Meteora 2-swap standalone builder (`meteora_atomic_builder.rs`) derives user ATAs token-program-aware (`:177-182` with `get_associated_token_address_with_program_id`) — inconsistent with the hot path.
- **3-leg meme pipeline** (unused at runtime, tests only) uses explicit validated `meme_a_ata` / `meme_b_ata` accounts in `ThreeLegMemeArbitrageContext` (`arb-engine/src/three_leg_meme_pipeline.rs:583-601`, compiler validation `arb-tx/src/compiler.rs:310-339`).

### 1.4 Where the final output arrives / residual profit stays
- Final hop outputs back into the borrowed mint (WSOL), into **the payer's canonical WSOL ATA** (`user_wsol_ata`). Residual profit (output − repay_amount) remains there as WSOL.
- If the last hop is **not** a Pump.fun sell and the first hop was a Pump.fun buy, the canonical WSOL ATA may have been closed and re-created mid-tx (§3).
- If the **last** hop is Pump.fun, the output is native SOL in the payer wallet, then re-wrapped to WSOL for the repay (`:3188-3198`); any excess (profit) stays as native SOL in the wallet **only above the repay_amount** — the wrap transfers exactly `repay_amount`, so the profit residual stays as native SOL in the payer wallet.
- Only the bot keypair (payer/authority) can spend the residual: same authority derivation is used for all user ATAs and for ATA creation/close. No other wallet is referenced anywhere in the close/create paths.
- Note: when the first hop is Pump.fun buy, the flow closes **the canonical WSOL ATA** and burns the account mid-transaction; rent, if reclaimable, goes to `payer` (`build_close_token_account(user_wsol_ata, payer, payer)` `:2969` and `:3166`).

### 1.5 Which account repays the flash loan
`user_wsol_ata` of the payer. Repay account 6 (`user_liquidity_ata`) is marked writable (not signer) and is the same account that received the borrow (`kamino.rs:106` vs `:150`, built from the same `KaminoFlashAccounts`). Repay debits `borrow + fee` from it (`:3186`, `:3201-3206`).

---

## 2. Profit calculation — complete reconstruction

### 2.1 Authoritative on-paper formula (EconomicQuote)
`arb-math/src/economic.rs:274 build_economic_quote` computes, entirely in base-mint raw units with SOL-denominated costs converted via `lamports_to_base_mint_rate / lamports_to_base_rate_denom`:

```
gross_signed        = expected_final_raw − borrow_amount_raw            (:328)
total_costs         = flash_fee + dex_fees + transfer_fees + acct_cost
                      + other_costs + network_raw + priority_raw + tip_raw  (:333-341)
expected_net        = gross_signed − total_costs                          (:343-349)
minimum_net         = (min_output − borrow) − total_costs                 (:350-356)
meets_min_profit    = minimum_net ≥ min_profit_raw                        (:359)
is_profitable()     = meets_min_profit && minimum_net > 0                 (:190-194)
```
Conceptually documented at `:390-398`. All amounts are base-mint raw; network/priority/tip are converted lamports→base-raw (rounded up, `:199-218`), overflow-checked (checked U256), never saturating.

Call site (`arb-engine/src/lib.rs:365-385`, `make_economic_opportunity`): passes `expected_final = final_out`, `minimum_final = final_out` (same value!), **`dex_fees=0`, `transfer_fees=0`, `account_creation=0`, `other=0`, `network_fee=0`, `priority_fee=0`**, only `flash_loan_fee_sf`, `jito_tip_lamports` (= configured `max_tip_lamports` ceiling, `execution.rs:274`), and `min_profit_raw = config.min_profit_raw`. Then `gross/net` are converted back to lamports for `ExecutionOpportunity` (`arb-engine/src/lib.rs:391-402`).

### 2.2 Re-check at dispatch gate (run.rs:805-820)
`calculate_net_profit_raw` (same file as §2.1, re-export from `arb-math::economic`) is recomputed with the **actual planned tip**, base fee and priority fee:
```
net = expected_out − size_in − flash_fee_raw − dex(0) − xfer(0) − acct(0) − other(0)
      − base_fee(5000 L→raw) − priority_lamports(→raw) − tip(→raw)     (run.rs:805-818; formula economic.rs:439-452)
```
`.filter(|net| *net > 0)` then compared against a converted floor via `lamports_to_base_raw_checked` (`run.rs:853-861`, `arb-types/src/dimensional.rs:275`). This is the genuinely authoritative gate.

### 2.3 Gate order in the live loop (`arb-bot/src/supervisor/run.rs`)
1. Verified inputs (WSOL base, reserve decode, mint decimals) — fail closed (`:421-429`, `execution.rs:155-283`).
2. `EconomicConfig.min_profit_raw = TokenAmount(min_net_profit_abs)` — **lamports stuffed into a raw-unit field with no conversion** (`:430-436`). For WSOL (rate 1:1) harmless; for a 6-decimal base mint this is a 1e3-scale mislabel.
3. Opportunity must clear `make_economic_opportunity` (flash fee + configured tip ceiling only — **base/priority fees NOT deducted at opportunity level**), `arb-engine/src/lib.rs:387-389`.
4. `MIN_NET_PROFIT_LAMPORTS = 30_000` (`arb-engine/src/lib.rs:61`) and `DEFAULT_MIN_PROFIT_LAMPORTS = 50_000` (`arb-constants/src/lib.rs:69`) / `DEFAULT_MIN_KEEP_LAMPORTS` (`:81`) exist but are **not** on the live gate path.
5. Jito sizing: `net_before_tip = gross − flash_fee_raw − 5_000 − priority` (`:676-679`); `t_max = net_before_tip − min_net_floor − 10_000` contigency buffer (`:684-688`); opportunity rejected if `t_max < MIN_JITO_TIP_LAMPORTS` (`:690-707`). **Unit mix**: `gross` (lamports) minus `flash_fee_raw` (base raw) — numerically identical only for WSOL.
6. Tip is capped: `min(bid, t_max)` (`:735`).
7. Final go/no-go: `calculate_net_profit_raw` as in §2.2 (`:805-820`), `> 0` required, then `≥ min_net_base_raw` (proper conversion, `:890-908`). **This is the second, stricter net-profit gate; no double deduction occurs between §2.2 and §2.1 because they are alternatives, not sums** — but the codebase carries two divergent net-profit formulas and the opportunity-level one (§2.1) ignores base/priority/alt costs.
8. Risk gates: `evaluate_flash_loan` / `evaluate_full` with `Lamports(real_net_base_raw)` — passing a **base-raw-denominated value cast as `Lamports`** (`:1048`, `:1059`).
9. Chronos EV gate: `gross_profit_lamports: gross` (lamports, correct) but `flash_fee_lamports: flash_fee_raw` (base raw, mislabeled) (`:961-962`).

### 2.4 Three-leg meme path (test-only, not wired into run.rs)
`simulate_profitability_gate` (`arb-engine/src/three_leg_meme_pipeline.rs:619-629`) checks **gross** profit ≥ `min_profit_lamports` — no flash-loan fee, no tip, no base/priority deduction. Result struct even names the field `gross_profit_lamports` (`:146`) yet the gate is presented as the "profitability gate" (`:615-618`). `ThreeLegMemeSizeSearchResult::gross_profit_lamports`/`best_net_profit_lamports` (`:123-146`) likewise carry gross values. `quote.gross` is `out[2] − input` in WSOL lamports (`:208-210`), which is internally consistent but ignores all costs.

### 2.5 Does it deduct flash-loan fee, priority fee, Jito tip, base fee?
| Cost | Opportunity-level (`make_economic_opportunity`) | Dispatch gate (`run.rs:805`) | `net_before_tip` (`:676`) | Meme path |
|---|---|---|---|---|
| Flash fee | yes (Q64.64, on-chain verified) | yes | yes | no |
| Priority fee | **no (0)** | yes | yes | no |
| Base fee (5000) | **no (0)** | yes | yes | no |
| Jito tip | yes (configured ceiling, not the real bid) | yes (actual bid, capped by t_max) | excluded (by design) | no |
| Dex fees | **no (0; assumed inside expected_out)** | 0 (same assumption) | n/a | no |
| Transfer fees (Token-2022) | 0 | 0 | n/a | no |

- **No double deduction of the same cost within one formula.** However:
  - `build_economic_quote` counts `network_raw` and `priority_raw` *and* `tip_raw` — three separate lines (`economic.rs:338-340`), each converted from lamports; fine.
  - The derivation of `min_out` floor (`derive_swap_legs`, `arb-tx/src/lib.rs:3018-3055`) builds `floor = borrow + flash_fee_raw + tip_raw + min_profit_raw` — **again mixing lamport tip with base-raw borrow/fee** (`:3020-3024`). For WSOL base this is consistent; otherwise corrupt.
  - `build_economic_quote` is also called with `is_profitable()` on the same totals — no double counting arithmetically, but the opportunity filter and the dispatch gate can disagree because they include different cost sets (see above).

### 2.6 Cross-mint comparison without normalization
- `find_multi_hop_cycles` / ranking sort opportunities by `net_profit.0` (lamports) — `arb-engine/src/lib.rs:963`, `:998`, fine for WSOL-only live path.
- `derive_swap_legs` mixes `expected_out` (base raw) with `tip_raw` (lamports) in `floor` — §2.5.
- `run.rs:1457` computes `deductions_base = (gross_profit − real_net)/base_scale` — subtracts lamport-denominated `opp.gross_profit` from base-raw `real_net_base_raw` **before dividing by base_scale**. For WSOL this is exact (both lamports); for any non-9-decimal base mint it is a silent cross-mint subtraction.
- `run.rs:1697-1707` (`final_health_check`-like helper) compares `opp.net_profit` (lamports) directly with `min_net_profit_abs` (lamports) — consistent there, but the same `min_net_profit_abs` is used as **base-raw** units at `run.rs:432` (into `EconomicConfig.min_profit_raw`), at `run.rs:1700` for `opp.net_profit` compare (lamports — consistent), and again raw-cast at `run.rs:1629` (`Lamports(real_net_base_raw)`) and compared to lamports-typed `min_net_profit_abs` in `dispatch.rs:110` (`real_net_profit.as_u64() >= self.config.strategy.min_net_profit_abs`) and `arb-risk/src/lib.rs:592-596`. **Three different units are compared against the same constant in three places.**
- USDC-denominated strategy floor: no such path is active — base is hard-wired WSOL in verified inputs.

### 2.7 Stale prices
- SOL/USD reference used for USD displays and for 6-decimal conversion is derived fresh per tick from fresh SOL-USDC pool snapshots (`run.rs:449-480`); stale pools are excluded by `max_state_age_slots`; `last_known_live_sol_price` is reset to 0 every cycle (`:447`), and `live_sol_price()` returns `None` when 0 (`execution.rs:144-147`). Callers then either fail closed (verified inputs for 6-decimal base, `verified_config.rs:215-217`) or display $0.00.
- EconomicQuote stamps `quote_timestamp_ms = SystemTime::now()` (`arb-engine/src/lib.rs:360-363`) — wall-clock, not state slot; not used for staleness decisions in the gates (slot-based `max_state_age_slots` is, `run.rs:1005`, `max_state_age_slots` in `execution.rs:292-307`, fail-closed to `u64::MAX` on missing pool).
- Pool reserve quotes themselves are only as fresh as the Yellowstone snapshots (no re-quote at fire time; the hot path explicitly does **not** call `simulateTransaction`, `run.rs:1389-1395`).

---

## 3. ATA create/close behaviour — ephemeral vs canonical

### 3.1 Account kinds
`assemble_flash_arbitrage` (`arb-tx/src/lib.rs:3061-3223`) creates **idempotent ATA instructions for the payer's canonical SPL-derived ATAs** — the same addresses the swaps use — not dedicated ephemeral accounts:
- WSOL ATA create-idempotent at `:3094-3099`,
- intermediate (non-WSOL, non-USDC) mint ATA create-idempotent at `:3116-3118` (derived at `:3109` via SPL `get_associated_token_address`).

`assemble_arbitrage` (`:2867`) does the same at `:2917-2923` plus closing instructions at `:2973-2975`, and closes the **user's canonical WSOL ATA** when Hop1=Pump.fun and Hop2≠Pump.fun (`:2968-2970`). `assemble_flash_arbitrage` closes it at `:3165-3167` (every first-hop-Pump.fun flash route), re-creating it in the next hop slot if Hop2 exists (`:3173-3180`).

### 3.2 Close instructions and safety checks
- `build_close_token_account(account_to_close, destination_rent_receiver, owner_authority)` — SPL `CloseAccount` opcode 9, **hard-coded SPL Token program** (`arb-tx/src/lib.rs:533-550`). `build_close_token_account_for_program` (`:516-531`) takes an explicit token program (used only by the 3-leg meme compiler, `compiler.rs:418/427`).
- `owner_authority` passed is the payer; `destination_rent_receiver` is the payer. There is **no code-side check that the closed account was created by this transaction**, nor that it is not a canonical pre-existing ATA. The only zero-balance enforcement is the token program itself (CloseAccount fails on non-zero amount), and the token program itself enforcing owner = token program id.
- Consequences in the flash path:
  - **Canonical WSOL ATA closed while holding the borrowed funds**: when Hop 1 is Pump.fun buy, `:3166` emits `CloseAccount` on `user_wsol_ata` which at that moment contains `borrow_amount` WSOL (Hop 1 has not yet run — close is emitted **before** the swaps at `:3169`). SPL CloseAccount requires amount == 0 ⇒ **the transaction can never succeed on-chain**. The whole flash route with a Pump.fun first hop is dead on arrival (fail-closed, but 100% failure rate).
  - Same defect at `:2969` in `assemble_arbitrage`.
  - Even if borrow=0 the account would be destroyed and re-created later, but borrow=0 is rejected (`ZeroBorrowAmount`, `:2756`).
  - **Pre-existing canonical ATA for an intermediate meme mint, zero balance**: created by user before this trade; instruction at `:3208-3211` closes it and returns its rent to payer. Pre-existing account destroyed — violates “never close a canonical ATA” even though no funds are lost.
  - **Pre-existing canonical ATA with balance > 0**: CloseAccount fails ⇒ entire arbitrage reverts (fail-closed, route unusable until the user empties the ATA).
  - There is **no check of the token account’s amount before closing in the flash path** — unlike the 3-leg meme compiler which implements `should_close_intermediate_ata` (below).
  - **Token-2022 mints**: closes/creates in the flash path use the **SPL Token program only** (`build_close_token_account` `:534-550`; `build_create_ata_idempotent` `:464-486` with `token_program = TOKEN_PROGRAM_ID` `:471`; ATA derivation `:443`). A Token-2022 mint’s ATA created through the SPL path is invalid for Token-2022 programs, the swap instruction then references SPL-derived addresses alongside Token-2022 program ids (e.g. Meteora branch `:2260-2263`), and the on-chain result is failure. Fail-closed, functionally broken.
  - **Token-2022 withheld transfer fees**: never inspected anywhere in the flash/native paths (no `withdraw_withheld` handling). The meme compiler explicitly refuses to close when `withheld_fees > 0` (`compiler.rs:600-632`); the flash path closes blindly (subject only to SPL-amount==0 and program-owner checks, which themselves fail for 2022 accounts).
- The **3-leg meme compiler is the only implementation with a correct policy**: it tracks `meme_a_ata_exists`, `meme_ata_balance`, `meme_is_token_2022`, `meme_withheld_fees` (`arb-tx/src/compiler.rs:251-263`), refuses to close pre-existing accounts, non-zero balances, or 2022 accounts with withheld fees, and only closes accounts created by this route (`should_close_intermediate_ata`, `compiler.rs:600-632`). It also uses `build_close_token_account_for_program` (`compiler.rs:418/427`, token-program-aware) and `build_create_ata_idempotent_for_program` (`:392`). **But this compiler has no flash-loan integration and is not called by the live loop** (grep of `run.rs` finds no reference to `ThreeLegMemePipeline` / `compile_3leg_meme_dlmm_arb`; only tests use it).
- WSOL ATA itself is intentionally **not** closed at the end of the flash route — residual WSOL profit stays there. That is correct for keeping rent and profit together, but means the “100% rent reclaim” comments (`:2972`, `:3208`) are inaccurate: canonical accounts persist; rent reclaim only recycles accounts created this tx.

### 3.3 `pump_fun_raydium.rs` closes
`crates/arb-tx/src/pump_fun_raydium.rs:256/261` (buy→sell direction) and `:322/327` (reverse direction) close `user_wsol` and optionally `user_meme` with `unwrap_wsol` / `close_meme_ata` flags. No existence/balance pre-checks; same canonical-ATA destruction issue for the WSOL ATA (here `user_wsol` is wrapped fresh via `build_system_transfer` + `sync_native` at `:266-271`, so in the reverse direction the WSOL ATA is guaranteed created by this tx — closing it is safe). In the buy→sell direction (`:225-261`) `user_wsol` is assumed to already hold the buy proceeds — closing it after a swap leaves zero balance, so `CloseAccount` succeeds, but it is again a canonical ATA being destroyed; optional `close_meme_ata` may close a canonical meme ATA with zero balance.

---

## 4. Intermediate-balance isolation

### 4.1 What the flash/native path actually binds
- `derive_swap_legs` (`arb-tx/src/lib.rs:3002-3057`) takes `opportunity.hop_inputs` (`[borrow, out_1, …, out_n]`) and uses **previous hop’s expected output as the next hop’s `amount_in`**, discounting each hop’s `min_out` by slippage once, and raising the final `min_out` to `borrow + flash_fee + tip + min_profit` (`:3018-3055`). Isolation property: hop 2 never spends more than the expected hop-1 output; the on-chain swap will require the ATA to hold that much, so **pre-existing inventory can never be silently consumed beyond failure** — but the design does **not** tie hop `i+1`’s `amount_in` to hop `i`’s *minimum* output (unlike the Meteora builder), so any adverse fill < expected ⇒ insufficient balance ⇒ revert. Fail-closed, never “profit” stolen from user wallet.
- Nothing in `assemble_flash_arbitrage` measures pre-existing ATA balances; profit figures never include them, and no instruction can debit more than `amount_in` of the intermediate.
- Pre-existing WSOL: the borrow is added *on top of* any pre-existing WSOL in the same canonical ATA. Repay takes `borrow + fee` from the shared account — if the route is profitable but less than the tip owed, the shortfall would come from the user’s pre-existing WSOL balance **only up to ATA solvency**; there is no separate isolation account. The `intermediate_balance_isolation_tests.rs` prove isolation only for the Meteora two-swap builder (model-based), not for the flash path; no test covers `assemble_flash_arbitrage` for this property.

### 4.2 What the tests prove (and don’t)
`crates/arb-tx/tests/intermediate_balance_isolation_tests.rs` — model-based (`WalletBalanceModel`), not an on-chain simulation:
- Case 2 (`:187-229`): pre-existing intermediate inventory (500M units) is never depleted by hop 2 because hop 2 input is bound to `hop1_min_out`.
- Case 1/3 (`:146-266`): residual = actual − min; exact-minimum ⇒ zero residual.
- Case 4 (`:268-302`): hop-1 under-delivery reverts; balances roll back.
- `test_hop2_instruction_data_strictly_bounded_by_hop1_min` (`:305-359`): decodes compiled hop-2 instruction bytes, asserts `amount_in == hop1_min_out`. This is a real wire-format check.
- Token-2022 transfer-fee case (`:401-440`): fee-aware min-out reasoning, model only.
- **Not tested**: flash-loan route (`assemble_flash_arbitrage`), pre-existing **WSOL** isolation for the repay source, pre-existing canonical ATA close behaviour, SPL vs Token-2022 ATA-program mismatch.

`bundle_tests.rs` asserts borrow/repay wire layout (discriminators, 17-byte repay with `borrow_instruction_index = 2`): e.g. `:218-277`. `p0_golden_vectors.rs` asserts opcode/account layouts but nothing about balances or profit. Neither suite exercises `derive_swap_legs`’ profit-floor units mix.

---

## 5. Flash-loan provider integration

### 5.1 Protocol and program id
- Kamino Lend (`klend`), program **`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`** (`arb-constants/src/lib.rs:322`, used throughout `kamino.rs`).
- Main Market `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF` (`lib.rs:324`), authority PDA `9DrvZvy…` (`lib.rs:326`, verified PDA derivation test with bump 248, `kamino.rs:272-283`).
- WSOL reserve `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q` (`lib.rs:328`), supply vault `GafNuU…` (`lib.rs:330`), fee/destination vault `3JNof8…` (`lib.rs:332`), USDC reserve also catalogued (`lib.rs:334`) but unused (flash restricted to WSOL, `NonWsolFlashLoan`, `arb-tx/src/lib.rs:3131-3133`).

### 5.2 Pinned SDK / IDL?
- **No Anchor SDK and no IDL is consumed at build time.** Instructions are hand-packed byte buffers with hard-coded discriminators:
  - `FLASH_BORROW_DISCRIMINATOR = [135, 231, 52, 167, 7, 52, 212, 193]` = sha256("global:flash_borrow_reserve_liquidity")[..8] (`arb-constants/src/lib.rs:337`),
  - `FLASH_REPAY_DISCRIMINATOR = [185, 117, 0, 203, 96, 245, 180, 186]` (`arb-constants/src/lib.rs:339`),
  - data layouts documented at `kamino.rs:82-128` (16- and 17-byte payloads),
  - account offsets of `ReserveConfig.fees.flash_loan_fee_sf` etc. are **hard-coded byte offsets of the deployed account layout** (`KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET = 8 + 4896`, `STATUS_OFFSET = 8 + 4848`, `MINT_OFFSET = 8 + 120`, `DECIMALS_OFFSET = 8 + 264`; `kamino.rs:197-200`), with the explicit caveat that the deployed program predates the master-branch layout and offsets must be re-verified on program upgrade (`kamino.rs:190-196`).
- Fee is **read live from the on-chain reserve account** via Yellowstone snapshots and decoded fail-closed (`kamino.rs:204-250`, `decode_reserve_flash_fee_sf` verifies discriminator, length, mint, active status; any mismatch is an error, never a default). Fee math: `ceil(borrow * sf / 2^64)` (`kamino.rs:170-179`, `arb-math/src/economic.rs:460-476`). Verified main-market sf ≈ 11_529_215_046_068 (≈ 0.006 bps) on 2026-11-04 (`kamino.rs:185-187`, `arb-constants/src/lib.rs:342-347`).
- Repayment semantics: `repay_amount = borrow_amount + flash_fee` (`arb-tx/src/lib.rs:3186`), `borrow_instruction_index` set to the borrow’s actual instruction index (`borrow_idx as u8`) — **not** hard-coded 2 on the flash path (`:3121`, `:3205`, and split path `:3301`, `:3316`; hard-coded `2` only in `build_kamino_flash_repay`’s legacy default wrapper, `:422-428`). The bundled tests assert index 2 for `assemble_atomic_bundle` layouts (`bundle_tests.rs:258-268`).
- Fee receiver vault is passed as account 7 on both borrow and repay; the protocol takes the fee there. Repay does not re-specify the fee — amount is authoritative.

### 5.3 Real or stub?
- The builder is **real instruction construction** (deterministic bytes, wire-level tests in `bundle_tests.rs:189-300`, `p0_golden_vectors.rs`), not a mock — there is no trait abstraction or feature-flagged stub for the flash loan.
- **But four integration defects make it unsafe to treat as production-verified:**
  1. `build_kamino_bundle`/`KaminoBundleParams` path calls `assemble_atomic_bundle` with **`flash_loan_fee_sf = 0`** (`arb-tx/src/lib.rs:2831-2833`), producing `repay = borrow` and a floor computed with fee 0 — guaranteed on-chain `InsufficientRepayAmount` against the live (non-zero) fee. The code comments admit “callers must re-price” (`:2810-2812`), but the wrapper itself is a footgun and is covered by a test only for the 0-fee fixture (`:3959`, `:4017`).
  2. `build_kamino_bundle` also sets `min_profit_raw = TokenAmount(opp.net_profit.as_u64())` — lamports used as base-raw floor (`:2824`).
  3. Offsets are empirical, unbounded by the IDL; a klend program upgrade silently invalidates `decode_reserve_flash_fee_sf` (it fails closed — by design, `kamino.rs:204-216` — so the tick is skipped rather than mispriced; good).
  4. No end-to-end liveness test of borrow+repay exists in-repo; `mainnet_non_broadcast_simulation.rs` exists but is not asserted as a passing integration check here (it is ignored/optional in CI; not run per this audit’s constraint).
- Jupiter path (`jupiter.rs`) is a real HTTP client (quote/swap-instructions, key rotation, 3 s timeout). It is **disabled at the call site** (`route_via_jupiter = false`, `run.rs:1107-1110`), so the only live execution path is native builders. The Jupiter flash wrapper (`assemble_jupiter_flash_arbitrage`, `arb-tx/src/lib.rs:3228-3279`) creates **no ATAs at all** — it would fail on any wallet without a pre-existing WSOL ATA; dead code today.
- Flash-loan fee is structurally **not** zero on-chain; any path quoting with `sf = 0` (all test fixtures; `build_kamino_bundle`) understates repay.

### 5.4 Token-2022 / withheld fees
- The live verified-input path accepts a Token-2022-owned base mint (`verified_config.rs:201-210`) and Token-2022 reserve mint (WSOL reserve is SPL-classic in practice).
- Intermediate Token-2022 meme mints are **broken end-to-end** on the native path: ATAs derived and created against SPL-Token (`arb-tx/src/lib.rs:443`, `:3109`, `:3116-3118`), swap account metas pass Token-2022 program ids (`:2220-2223` via token_program_in/out in the Meteora/Whirlpool branches), and intermediate close instructions target the SPL program (`:3209-3211` → `build_close_token_account` `:534-550`). In no configuration does a Token-2022 intermediate route produce a valid transaction. (PumpSwap hops derive ATAs program-aware, `:1828-1857`, so Pump.fun-only routes with 2022 mints could self-consistently pass swaps — but the intermediate create/close still targets SPL.)
- Withheld transfer fees are never withdrawn nor checked in the hot paths; the meme compiler explicitly guards them (§3.2). Transfer fees **are** priced into `transfer_fee_bps` on the PoolState for quoting (`arb-engine/src/lib.rs:221`), but `build_economic_quote` is invoked with `transfer_fees_raw = 0` in both call sites.

---

## 6. Instruction ordering and atomicity

Order enforced in `assemble_flash_arbitrage` (`arb-tx/src/lib.rs:3059-3223`), `assemble_atomic_bundle` (`:2739-2817`), `assemble_jupiter_flash_arbitrage` (`:3225-3279`), `assemble_split_flash_arbitrage` (`:3284-3329` — split routing is disabled at runtime, `run.rs:1007-1009`):
```
[0] SetComputeUnitLimit → [1] SetComputeUnitPrice → (optional ATA creates) →
[n] Kamino FlashBorrow → swaps (in route order; optional WSOL unwrap/wrap around Pump.fun legs) →
[m] Kamino FlashRepay (same tx, same message) → (optional intermediate-ATA closes) → [last] Jito tip transfer
```
- Borrow and repay are in **the same transaction**, and the repay references the borrow by `borrow_instruction_index` resolved at build time (`borrow_idx as u8`, `:3205`). Kamino’s on-chain sysvar-instructions check enforces CPI-locality of the pair; repaying a borrow from a different transaction is impossible.
- **Same-transaction repayment is therefore enforced by construction.** If the final hops cannot satisfy `repay_amount`, the on-chain `flash_repay_reserve_liquidity` fails and the whole transaction reverts — nothing is consumed except the (unlanded ⇒ unpaid) fees; no partial fill, no stranded loan.
- Final-output floor: `derive_swap_legs` raises the **last hop’s `min_out`** to `borrow + flash_fee + tip + min_profit_raw` (`:3041-3052`); intermediate hops get only slippage-discounted `min_out` (`:3035-3039`, `DEFAULT_HOP_SLIPPAGE_BPS = 10`, `arb-constants/src/lib.rs:352`). Any route whose expected final output can’t cover the floor fails closed with `ProfitFloorNotMet` at build time (`:3043-3048`).
- Jupiter routes carry **no profit floor at all** — only 50 bps slippage from the Jupiter quote API (`execution.rs:77`); the code comments admit this (`run.rs:1390-1395`). Route is dead code anyway (`route_via_jupiter = false`).
- Failure modes:
  - Final ATA balance < `repay_amount` (adverse fill but above slippage min): on-chain revert; nothing lands; tip not paid (transfer is in the same tx, so it reverts too).
  - `min_out` of any hop not met: revert.
  - **The hot path never calls `simulateTransaction`** (`run.rs:1389-1395`), so a guaranteed-revert transaction (e.g. the Pump.fun-first flash route, §3.2) is constructed, signed, and dispatched to Jito — landing cost is zero (bundle fails), but the opportunity logic never learns from the failure except via "unlanded" accounting (`dispatch.rs:640-650`, `715`).
  - `assemble_flash_arbitrage` CU math: `calculate_dynamic_cu_limit(cu_limit)` applies a 15% buffer (`:3075`); estimated route CU seeded from protocol-specific costs + 20,000 CU per intermediate ATA (`arb-tx/src/lib.rs:142-158`, `:113-158`). Under-provisioning CU ⇒ revert; over-provisioning wastes nothing.
- Priority fee vs Jito tip: base/priority/tip all paid by payer; if bundle fails, **no tip and no base fee land** (whole tx reverts). The run.rs comment "Unaccepted bundle / transport error does not spend tip or base tx fee" (`dispatch.rs:715`) reflects this.
- Insurance check before dispatch: `final_health_check`-style helper at `run.rs:1670-1730` asserts closed cycle (input mint == output mint, `:1681`), state age within limits (`:1689`), `net_profit > 0` (`:1697`), and `expected_out ≥ size_in + flash_fee` (`:1717-1730`). This is redundant with `derive_swap_legs` but harmless.

---

## 7. Profit reporting: SOL/USD, conversions, mislabeling

### 7.1 What is reported and where
- `ExecutionOpportunity.gross_profit / net_profit` are **`Lamports`** (`arb-types/src/lib.rs:1108-1110`) — for WSOL base this is correct; the values were converted from base-raw via `base_raw_to_lamports(rate)` at the opportunity site (`arb-engine/src/lib.rs:391-402`) with the verified rational rate (`verified_config.rs:91-112`; `decimals == 9 ⇒ (1,1)`, `:95-97`; 6-decimal ⇒ `(round(sol_price·10^d), 1e9)`, `:102-111`).
- `run.rs:1410-1505`: `gross_sol = gross/1e9` (`:665`), `profit_lamports` re-derived via `arb_types::dimensional::base_raw_to_lamports_checked` (`:1439-1444`, `dimensional.rs:310`), `profit_sol = /1e9` (`:1445`), `live_sol_price` (`:1446`), USD = `profit_sol * live_sol_price` (`:1503`, `dispatch.rs:96`, `116`, `475`, `603`, `625`).
- Dispatch records `real_net_profit: Lamports(real_net_base_raw)` — **base-raw passed through as lamports without conversion** (`run.rs:1629`; used at `dispatch.rs:21`, `110`, `474`, `582`). For WSOL base identical; for any non-WSOL base mint this is a unit corruption flowing into the metrics store and outcome records.
- `arb_risk::evaluate_flash_loan(Lamports(real_net_base_raw), …)` same cast at `:1048`, `:1059` — the flash-loan risk gate consumes base-raw values as lamports.
- `deductions_base = (opp.gross_profit − real_net_base_raw)/base_scale` (`:1456-1457`) — cross-unit subtraction (lamports − base raw) before scaling.
- Telegram/metrics report `gross_sol`, `profit_sol`, `profit_usd = profit_sol·sol_price` — correct **only because base == WSOL**. Log line 1466-1467 hard-codes "Gas Fee (Priority Fees + Cu Burned): ~0.000010 SOL" regardless of actual CU cost.

### 7.2 Is there a valid SOL/USD conversion?
Yes, one: `derive_rational_rate(decimals, sol_price_usd)` (`verified_config.rs:91-112`) used to build `lamports_per_base_raw/denom` (stored as verified inputs with freshness stamp, `:233-244`). It is used by the economics module for cost conversion (professional) but partially bypassed in reporting:
- USD display multiplies `profit_sol * live_sol_price` — circular but self-consistent for WSOL (price cancels), and equally consistent for USDC base (conversion used the same price), so numerically it is fine; semantically it labels a USDC-denominated profit as "SOL-denominated" in the telegram messages (`dispatch.rs:565-582` prints "Net Profit: +{:.6} SOL" with `profit_sol_val` — for USDC base that number is a lamports-equivalent, not SOL).
- Mint mislabeling: `pools_str` prints protocol + pool id, not mints; the informational dump prints `base_mint` as `MintId` Display (`run.rs:1474`, `1479`, `1482`) — full base58 mint, no symbol. Messages that hard-code "SOL" in `initial_loan/gross/net` labels (`:1474`, `:1497`, `:1501`) are wrong for any non-WSOL base.
- `base_scale = 10^base_decimals` display path (`:1431-1434`, `1454-1457`) is decimals-correct.
- The profit metrics (`arb_metrics::add_net_profit(profit_sol)`, `dispatch.rs:596`) are in SOL-equivalents; store outcomes persist `real_net_profit` (Lamports cast, §7.1).
- Three-leg meme path reports `gross_profit_lamports` as "lamports" but never crashes only because WSOL is the quote asset; it ignores all costs (§2.4) so any USD figure derived from it overstates profitability.

### 7.3 Verdicts
- For the **live** path (WSOL base): SOL reporting is sound; USD reporting is self-consistent though derived from the same SOL price it displays.
- For any **non-WSOL base**: SOL, lamport, raw-unit, and USD comparisons are all crossed in at least one of the gates/logs/stores; the correct conversion exists but is not applied uniformly (`run.rs:1439` converts properly; `run.rs:1629`, `dispatch.rs:110`, `arb-risk/src/lib.rs:592` do not).
- No unit tests assert cross-mint profit accounting; all fixtures use WSOL/USDC 9/6-decimal-agnostic raw integers.

---

## 8. Summary of findings (severity-ordered)

**Critical**
1. Flash route with first hop Pump.fun closes the canonical WSOL ATA **while it holds the flash-borrowed WSOL**, before the swaps (`arb-tx/src/lib.rs:3165-3167`; same bug in `assemble_arbitrage:2968-2970`). SPL `CloseAccount` requires zero balance ⇒ 100 % on-chain failure for this route shape. `pump_fun_raydium.rs:322` closes the same canonical account in the reverse direction.
2. `build_kamino_bundle` / `KaminoBundleParams` path assembles atomic bundles with `flash_loan_fee_sf = 0` (`arb-tx/src/lib.rs:2831-2833`); repay omits the on-chain fee ⇒ guaranteed `InsufficientRepayAmount` whenever anyone uses this builder. Test fixtures also all pass `0` (`:3959`, `:4017`).
3. Flash path closes **pre-existing canonical intermediate ATAs** with no provenance/balance check (`arb-tx/src/lib.rs:3208-3211`) — zero-balance user ATAs are destroyed (rent to payer); non-zero balances brick the route. Compare the correct, implemented-but-unwired policy `should_close_intermediate_ata` (`compiler.rs:600-632`).
4. Token-2022 end-to-end breakage: SPL-only ATA derivation/creation/closing (`lib.rs:443`, `:464-486`, `:534-550`, `:3109-3118`, `:3209-3211`) combined with Token-2022 program ids in swap account metas; withheld fees never withdrawn or checked in the hot paths.

**High**
5. Unit mismatch minefield on `flash_fee_raw` (base-raw) subtracted from `gross` (lamports) without conversion (`run.rs:676-679`, chronos `:962`, profit-floor `derive_swap_legs` `:3020-3024`); masked today only because live base == WSOL.
6. `Lamports(real_net_base_raw)` cast without conversion (`run.rs:1048`, `:1059`, `:1629`; `dispatch.rs:21`, `:110`, `:474`; `arb-risk/src/lib.rs:592-596`) — base-raw flows into lamport-typed risk gates, metrics, and outcome recording.
7. `EconomicConfig.min_profit_raw = TokenAmount(min_net_profit_abs)` stuffs a lamport constant into a base-raw field (`run.rs:432`; floor semantics differ for 6-decimal mints); `dispatch.rs:110` and `run.rs:1700` compare unlike units against the same constant.
8. `build_kamino_bundle` sets `min_profit_raw = TokenAmount(opp.net_profit.as_u64())` (lamports as base-raw) (`lib.rs:2824`).
9. No profit floor on the Jupiter path (50 bps slippage only, `execution.rs:77`; acknowledged at `run.rs:1392-1395`); path disabled today (`route_via_jupiter = false`, `run.rs:1110`) but `resolve_jupiter_swap_route` is still reachable code.
10. Hot path dispatches guaranteed-revert transactions without `simulateTransaction`; CU under-provisioning and ATA-owner mismatches are only discovered on-chain (`run.rs:1389-1395`).

**Medium**
11. Opportunity-level net profit ignores base/priority/network fees (`arb-engine/src/lib.rs:372-378` pass 0); only the dispatch-time recalc (`run.rs:805`) deducts them — two inconsistent net-profit definitions coexist.
12. Three-leg meme path’s "profitability gate" checks **gross** profit only; fields named `*net_profit*` carry gross values (`three_leg_meme_pipeline.rs:123-146`, `:615-629`); USD/report derivations from it overstate profit. Pipeline is not wired into the live loop.
13. Meteora two-swap builder (`meteora_atomic_builder.rs`) binds hop 2 to `hop1_min_out` (isolation done right) but uses **pre-existing canonical ATAs** for in/mid/out (`:177-182`) — pre-existing tokens sit in the same debited accounts (protected by the amount binding, not by isolation); intermediates are created idempotently and never closed, leaving rent locked.
14. `intermediate_balance_isolation_tests.rs` are model-based; nothing tests `assemble_flash_arbitrage`, the WSOL repay-source pre-existing-balance interplay, or canonical-ATA close rules in the hot path.
15. Flash fee integration is hand-packed (no IDL/SDK pin) with hard-coded deployed-layout byte offsets (`kamino.rs:197-200`) — a klend upgrade silently invalidates the fee reader (fails closed, by design) and every compiled borrow/repay.

**Low / informational**
16. `assemble_split_flash_arbitrage` (`lib.rs:3284-3329`) does no ATA management and enforces no profit floor; split routing disabled (`run.rs:1009`) — dead but compiled.
17. `Gross/net` ranking uses lamports while `hop_inputs` are base raw — consistent for WSOL only (`arb-engine/src/lib.rs:963`, `998`).
18. `build_atomic_bundle` vs `assemble_flash_arbitrage` vs `assemble_jupiter_flash_arbitrage` are three near-identical assemblers with divergent safety behaviour; `bundle_tests.rs` covers only the first.
19. USD reports label any base mint’s profit as "SOL" (`run.rs:1474`, `1497`, `1501`; `dispatch.rs:565-582`); cosmetic "~0.000010 SOL" gas line hard-coded (`run.rs:1478`).
20. Rent reclaimed from intermediate accounts (≈0.00203928 SOL per ATA) is not counted anywhere in profit; comments claim "100% rent reclaim" while WSOL ATA is never closed.

**Controlling positives**: flash fee is read live from the verified reserve account fail-closed; borrow/repay are same-transaction with a build-time `borrow_instruction_index`; non-closing routes and zero borrow are rejected; CLMM/DLMM pools never fall back to constant-product quotes (fail-closed `arb-engine/src/lib.rs:127-128`, `184-185`); SOL/USD rate for 6-decimal bases fails closed when price is unavailable; slot-based staleness gating exists for inputs and hops.

**Bottom line**: the design correctly keeps the flash borrow and repay atomic in one transaction and never lets a pre-existing intermediate subsidize hop 2 *in the Meteora builder* — but the **live** flash path is (a) guaranteed-reverting for Pump.fun-first routes, (b) destroys zero-balance pre-existing canonical ATAs without checking them, (c) cannot execute any Token-2022 intermediate, and (d) carries systematic lamports↔base-raw casts that are invisible only because the live base mint is WSOL. No file was modified by this audit.

---

## 9. Which transaction path actually signs and sends

### 9.1 Sign
The only signer is `self.keypair` (the bot wallet); there is no delegated signer and no second wallet anywhere in the flow.
- Keypair presence gates the live send: `run.rs:1276` (`if let Some(ref signer) = self.keypair`).
- `payer = self.keypair.map(|k| k.pubkey()).unwrap_or([1u8; 32])` (`run.rs:294-298`) — when no keypair is configured, dry paths proceed with a placeholder `[1;32]` payer and never broadcast (falls into the serialize-only branch at `run.rs:1384-1386`).
- Signing happens via `PreWarmedTxTemplate::patch_and_sign_with_swaps` (RAM template fast path, `run.rs:1302-1316`), Redis-backed `PreWarmedTxTemplate::from_precompiled_bytes` + sign (Tier-2, `run.rs:1321-1336`), or a full rebuild fallback. Legacy (v0-less) transactions with a real blockhash only — the fabricated ALT (ALT111…) is explicitly disavowed (`run.rs:1258-1260`).
- Zero blockhash ⇒ fail closed, no send (`run.rs:1280-1301`).
- Jito bundle dispatch: the signed bytes are submitted as a bundle whose tip transfer must be the last instruction — enforced by every assemble builder (`lib.rs:3213-3220`, `:3269-3276`, `:3319-3326`), and the tip carries the `jitodontfront` marker (`build_marked_jito_tip_instruction`, `lib.rs:288-292`, called from `:2809`, `:2979`, `:3215`, `:3271`, `:3321`).

### 9.2 Send
- Mode gates: `BotMode::Canary` never broadcasts (`dispatch.rs:78-104`); `Shadow`/`Simulate` never broadcast (`dispatch.rs:105-125`); `Live` proceeds to dispatch (`dispatch.rs:126+`).
- Live path selects Jito bundle vs PlainRpc vs SwqosDirect via `jito_service.plan_with_context(...)` (`run.rs:723-731`); PlainRpc and non-Bid decisions are rejected pre-dispatch (`run.rs:737-795`); SwqosDirect sends a single transaction with tip = 0 (`run.rs:732`).
- Settlement tracking: a background task polls landing status (`dispatch.rs:498-707`); on `Landed` it:
  - records outcome `"landed"` with `Some(net_profit_val)` where `net_profit_val = opp.net_profit` (**lamports**) — `dispatch.rs:473-475`, `582`,
  - stores `loan_size_sol_val = opp.size_in.as_u64() as f64 / 1e9` (`dispatch.rs:479` — again assumes 9 decimals / "SOL"),
  - mints USD as `profit_sol_val * sol_price` (`dispatch.rs:475`, `603`, `625`),
  - bumps `arb_metrics::add_net_profit(profit_sol_val)` (`dispatch.rs:596`).
- On `Dropped` / `Expired`: no outcome PnL is recorded, no tip loss charged to the breaker (`dispatch.rs:634-664`; comment at `dispatch.rs:640`).
- On `Failed` (reverted on-chain): `risk_gate.record_loss(5_000 / 1e9 * sol_price)` — models only the 5,000-lamport base fee as lost; records outcome `"failed"`; trips breaker accounting (`dispatch.rs:665-680`). This is the honest accounting of an atomic revert — tip and swaps never landed.
- The 5-minute performance digest and Telegram alerts report `profit_sol` / `profit_usd` / `loan_sol` — all SOL-assuming (`dispatch.rs:600-632`, `565-582`).

### 9.3 Profit destination — final, unambiguous statement
The residual WSOL profit stays **in the payer's canonical SPL WSOL ATA** (or native SOL in the payer wallet when the final hop is Pump.fun and the excess stays unwrapped after `repay_amount` is re-wrapped, `lib.rs:3188-3198`). The only key that can spend it is the bot keypair. There is no operator fee account, no fee recipient, no referral account anywhere in the flow; the only transfers out of the route are (i) the Jito tip to a protocol tip account and (ii) the Kamino flash fee routed to the reserve fee vault. The 3-leg meme compiler, when used, likewise leaves residual WSOL in the bot-derived WSOL ATA (`compiler.rs:347-353`, `bot_wsol_ata`).

---

## 10. Failure-mode matrix for the live flash path

| # | Condition | On-chain result | Off-chain visibility |
|---|---|---|---|
| F1 | Hop fills adversely but `min_out` met | Tx lands; profit < expected; repay still enforced | Landed; PnL recorded as expected-not-actual |
| F2 | Any hop `min_out` not met | Revert; nothing lands; tip unpaid | `Failed` path; 5000-lamport loss modelled (`dispatch.rs:672`) |
| F3 | Final ATA < repay_amount | Revert at `flash_repay` | Same as F2; silent economics leak (no simulation) |
| F4 | First hop Pump.fun buy | **Always** revert: canonical WSOL ATA `CloseAccount` with non-zero balance (§3.2) | `Failed` every time; route never lands |
| F5 | Intermediate meme mint with pre-existing user ATA, balance > 0 | Revert at final `CloseAccount` (`lib.rs:3209-3211`) | `Failed` |
| F6 | Intermediate meme mint with pre-existing user ATA, balance = 0 | ATA destroyed; rent → payer | `Landed` but user account gone |
| F7 | Token-2022 intermediate mint | SPL ATA create/swap mismatch; revert | `Failed` |
| F8 | Kamino reserve decode/fee sf stale or mismatched | Tick skipped pre-quote (fail closed) | Only debug log (`run.rs:423-428`) |
| F9 | `t_max < MIN_JITO_TIP_LAMPORTS` | Never dispatched | Rejection alert, Arabic reason string (`run.rs:691-707`) |
| F10 | Verified inputs missing/stale | Tick skipped | debug log (`run.rs:421-429`) |
| F11 | `ProfitFloorNotMet` at build (expected_out < borrow+fee+tip+min_profit) | Never signed | Build-time error; silent `continue` via `?` in assembly callers (`execution.rs:25-50`) |
| F12 | Bundle not accepted by Jito | Dropped/expired, 0 loss | `Dropped` path; no PnL; no tip charged (`dispatch.rs:634-664`) |

Contrast with the 3-leg meme compiler path (currently unwired into `run.rs` — no reference to it exists there): it validates ATAs exists/balance/withheld-fees **before** building, refuses unsafe closes, and signs nothing when validation fails (`compiler.rs:281-339`, `should_close_intermediate_ata:600-632`; unit tests `three_leg_meme_compiler_tests.rs:1293-1314`). It is the reference implementation for what the hot path should do.

---

## 11. Answers to the commissioned questions (compact)

**Q1 — Where the borrowed asset arrives / who pays / where intermediates go / where final output lands / who repays / where residual profit lives / whose wallet can spend it?**
Borrowed WSOL → payer's canonical SPL WSOL ATA (`lib.rs:3085`, `kamino.rs:110`). Swap fees in-pool; base/priority/tip/rent paid by payer in native SOL; Kamino fee inside the repay amount from the same WSOL ATA. Intermediates → payer's ATAs of those mints (SPL-derived in the hot path; token-program-aware in the Meteora standalone builder and in PumpSwap hops). Final output → payer's WSOL ATA (or native SOL when the last hop is Pump.fun; the excess stays native SOL after re-wrapping exactly `repay_amount`, `lib.rs:3188-3198`). Repay → payer's WSOL ATA via `flash_repay_reserve_liquidity` with `borrow_instruction_index` = build-time borrow index. Residual WSOL profit → payer's WSOL ATA; spendable only by the bot keypair. No external profit destination exists anywhere in the code.

**Q2 — Net-profit formula; deductions; double counting; cross-mint comparisons; stale prices?**
Authoritative: `build_economic_quote` (`economic.rs:274-388`): `net = (expected_final − borrow) − (flash_fee + Σ sol_costs→raw)`; `is_profitable` requires `minimum_net ≥ min_profit_raw ∧ minimum_net > 0` (`economic.rs:190-194`, `359`). Dispatch re-check: `calculate_net_profit_raw` with actual tip/base/priority (`run.rs:805-818`). The opportunity-level call deducts only flash fee + configured tip ceiling (network/priority passed as 0). The gate stack deducts flash fee + base 5,000 + priority + actual tip (`net_before_tip`, `run.rs:676-679`; recalc, `:805-818`). No in-formula double counting, but **two divergent net formulas** coexist and the floor passed into `derive_swap_legs` re-adds `tip_raw` (lamports) to base-raw borrow/fee (`lib.rs:3020-3024`) — triple-counted-or-misdenominated depending on base mint. Cross-mint comparisons: `deductions_base` (`run.rs:1456-1457`) mixes lamports and base raw before dividing; `dispatch.rs:110`, `arb-risk/src/lib.rs:592` compare lamport constants against base-raw-cast-as-Lamports. Stale prices: inputs and hops are slot-age gated fail-closed; SOL/USD for USD display is derived per tick; USD becomes $0.00 when SOL/USDC staleness trips. `quote_timestamp_ms` is wall-clock and unused for gating (`arb-engine/src/lib.rs:360-363`).

**Q3 — Creates/closes ephemeral ATAs? Ever closes a pre-existing canonical ATA? Token-2022 withheld fees?**
It does **not** use dedicated ephemeral accounts — only idempotent creation of the payer's canonical ATAs, then optional closes of the same (`lib.rs:3115-3118`, `3208-3211`, `3094-3099`). Yes, it **does** close pre-existing canonical ATAs (WSOL ATA in Pump.fun-first routes, `:2968-2970`, `:3165-3167`; intermediate meme ATAs unconditionally, `:3208-3211`) — with no provenance or balance pre-check; close builder is SPL-only (`:534-550`). SPL CloseAccount's zero-amount rule is the only (accidental) balance protection. Token-2022 withheld fees are never inspected or withdrawn in the hot paths; the meme compiler refuses to close 2022 ATAs with withheld fees (`compiler.rs:600-632`) but is unwired.

**Q4 — Pre-existing wallet balance counted as profit?**
In the Meteora two-swap builder, no: hop 2's input is hard-bound to `hop1_min_out` (`meteora_atomic_builder.rs:246-253`; proved at wire-format level by `intermediate_balance_isolation_tests.rs:305-359`). In the flash/native path, hop i+1's input is the previous hop's **expected** output, never a discounted min-out and never a wallet top-up (`derive_swap_legs`, `lib.rs:3002-3057`), but isolation is enforced only via min_out revert, and the WSOL repay draws from an account that may contain pre-existing user WSOL — the user's own WSOL can therefore fund the repay above the borrowed amount; no instruction can take more than `repay_amount`, so the loss is bounded, never silent. Profit figures never incorporate ATA balances; no code path measures them. Rent reclaimed from intermediate accounts is not counted as profit either.

**Q5 — Provider integration: protocol, program, SDK/IDL, pairing, fee, real or stub?**
Kamino Lend `klend` `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`, main market `7u3HeHx…`, WSOL reserve `d4A2prbA…`, byte-packed hand-rolled Anchor instructions (no IDL/SDK dependency), hard-coded deployed-layout offsets for `flash_loan_fee_sf` (`kamino.rs:197-200`, `decode_reserve_flash_fee_sf` `:221-250` with discriminator/mint/status/length checks), discriminators pinned in `arb-constants/src/lib.rs:337-339`, PDA-derivation asserted by tests (`kamino.rs:272-283`). Repay = borrow + ceil(borrow·sf/2^64) (`kamino.rs:170-179`), same-tx and same-builder pairing with index binding (`lib.rs:3121`, `:3143`, `:3186`, `:3201-3206`). It is real construction — with three stub-quality traps: `flash_loan_fee_sf = 0` in `build_kamino_bundle` (`lib.rs:2831-2833`), all test fixtures use sf=0 (`:3959`, `:4017`), and zero end-to-end borrow/repay tests exist. A klend upgrade invalidates the hard-coded offsets; the reader fails closed (`kamino.rs:204-216`).

**Q6 — Ordering/atomicity; insufficient final balance?**
Same-transaction borrow → swaps → repay → tip, enforced by construction; repay references the borrow by `borrow_instruction_index` (build-time) and Klend checks cross-instruction pairing via the sysvar instructions account (account 10, `kamino.rs:114`, `:154`). Insufficient final balance ⇒ on-chain revert of the whole message; swap `min_out`s and the final profit floor (`derive_swap_legs`, `lib.rs:3041-3052`) bound the blast radius. Nothing is simulated pre-flight; guaranteed-revert bundles can still be dispatched (Pump.fun-first route) at zero tip cost (whole tx reverts in-bundle).

**Q7 — Profit in SOL/USD; valid conversion; mint mislabeling?**
Reported in "SOL" (`profit_sol = lamports/1e9`) and USD (`profit_sol * live_sol_price`); for WSOL base both are numerically right. A correct rational conversion exists (`verified_config.rs:91-112`, `dimensional.rs:275-330`) but is applied only on some paths; `run.rs:1629` (`Lamports(real_net_base_raw)`), `dispatch.rs:479` (`loan_size_sol_val = size/1e9`), `dispatch.rs:110`, `run.rs:1048`, `:1059`, `arb-risk/src/lib.rs:592-596` cast base-raw as lamports/SOL; telegram/log/summary texts hard-label all bases as SOL (`run.rs:1454-1505`, `dispatch.rs:565-582`); USD becomes $0.00 when SOL/USDC staleness trips. No mint mislabeling in what is *stored* (raw units are persisted in outcome records), but consistent mislabeling in what is *displayed*, plus raw-vs-lamport unit drift into the risk gate and metrics.

---

## 12. File:line evidence index

- Flash borrow/repay builders: `crates/arb-tx/src/kamino.rs:82-119` (borrow, 12 accounts, 16-byte payload), `:121-159` (repay, 17-byte payload), `crates/arb-tx/src/lib.rs:412-439` (wrappers), `:421-428` (repay default idx 2), `:430-439` (indexed repay).
- Flash fee: `kamino.rs:161-179`, `arb-math/src/economic.rs:455-476`, `arb-constants/src/lib.rs:342-347`.
- On-chain fee decode: `kamino.rs:185-250`; reserve freshness/decode wiring: `crates/arb-bot/src/supervisor/execution.rs:155-283`, `crates/arb-bot/src/verified_config.rs:171-283` (owner check `:185-190`, mint decode `:214`, rate derivation `:91-112`).
- Economic formula: `arb-math/src/economic.rs:196-265` (conversions), `:274-388` (`build_economic_quote`), `:390-452` (`calculate_net_profit_raw`).
- Opportunity call site: `crates/arb-engine/src/lib.rs:345-416`; re-check: `crates/arb-bot/src/supervisor/run.rs:805-820`; ranking sorts: `crates/arb-engine/src/lib.rs:963`, `:998`; `MIN_NET_PROFIT_LAMPORTS` `:61`, `EPSILON_CYCLE_SPREAD_BPS` `:59`.
- Lamports↔base-raw: `crates/arb-types/src/dimensional.rs:275-330`.
- ExecutionOpportunity: `crates/arb-types/src/lib.rs:1096-1115`.
- Correct conversion site: `run.rs:1439-1446`; incorrect casts: `run.rs:1629`, `:1048`, `:1059`, `:1456-1457`, `dispatch.rs:21`, `:110`, `:473-475`, `:479`, `:582`, `:596`, `:602-604`, `crates/arb-risk/src/lib.rs:592-596`.
- Verified-input staleness: `crates/arb-bot/src/verified_config.rs:51-81` (`is_fresh`), `run.rs:279-281` (1500-slot cap), `execution.rs:288-307` (`max_state_age_slots`, fail-closed `u64::MAX`).
- ATA create/close builders: `crates/arb-tx/src/lib.rs:462-512` (create idempotent SPL, and 2022-aware variant), `:514-531` (close, program-aware), `:533-550` (close, SPL-only).
- Live create/close sites: `lib.rs:3094-3118` (create WSOL + intermediates), `:3165-3167` and `:2968-2970` (close canonical WSOL ATA on Pump.fun-first hop), `:3173-3180` (re-create WSOL ATA), `:3188-3198` (wrap to WSOL for repay), `:3208-3211` (close intermediates), `:2917-2923` / `:2973-2975` (same in `assemble_arbitrage`), `pump_fun_raydium.rs:254-262`, `:320-328`.
- Meme compiler policy: `compiler.rs:126-131` (budget), `:247-263` (exists/balance/2022/withheld tracking), `:281-339` (validation), `:390-432` (create/close emission), `:600-632` (`should_close_intermediate_ata`), `:347-353` (`bot_wsol_ata`).
- Meteora builder isolation: `meteora_atomic_builder.rs:1-27`, `:166-187` (program-aware ATAs), `:225-257` (hop-2 bound to `hop1_min_out`), `:270-287` (idempotent creates; intermediates never closed), `:290-312` (MTU check).
- Tests: `intermediate_balance_isolation_tests.rs:145-229` (pre-existing inventory), `:305-359` (wire-level hop-2 bound), `:361-398` (delta evidence), `:401-440` (Token-2022 fee model), `:442-507` (rollback model); `bundle_tests.rs:172-300` (layout assertions), `:442-500` (template repay patch); `p0_golden_vectors.rs` (opcode vectors); `three_leg_meme_compiler_tests.rs:1293-1314` (close-safety).
- Live pipeline: `run.rs:421-436` (verified inputs → EconomicConfig), `:564-610` (cycle DFS), `:615-679` (per-tx cost basis; flash fee via `calculate_flash_fee_raw` `:624-632`; CU cap `:646-663`), `:681-707` (`t_max` gate), `:709-795` (Jito plan selection), `:797-909` (authoritative recalc + floor compare), `:1004-1101` (state-age + risk gates), `:1107-1111` (Jupiter disabled), `:1114-1254` (assembly), `:1258-1386` (blockhash/templates/sign), `:1389-1395` (no simulateTransaction), `:1402-1434`, `:1464-1520` (reporting), `:1600-1635` (dispatch context).
- Dispatch: `dispatch.rs:44-125` (Canary/Shadow/Simulate), `:126-150` (Live safety floor), `:460-475` (landed bookkeeping), `:479` (loan size / 1e9), `:564-632` (landed metrics/telegram), `:634-714` (dropped/expired/failed).
- Constants: `arb-engine/src/lib.rs:59-61`; `arb-constants/src/lib.rs:69-81`, `:352` (`DEFAULT_HOP_SLIPPAGE_BPS`), `:57`, `:85` (tip accounts), `crates/arb-bot/src/supervisor.rs:43` (`BASE_FEE_LAMPORTS = 5_000`).
- Constants for Kamino: `arb-constants/src/lib.rs:322-339`; `jitodontfront` tip builder: `arb-tx/src/lib.rs:238-260`, `:288-300`.

*End of audit 09. No code was modified; `cargo` was not run.*

---

## 13. Discriminator and pairing cross-check (offline verification)

Verified locally with SHA-256 (no chain access, no `cargo`):
- `sha256("global:flash_borrow_reserve_liquidity")[:8]` = `[135, 231, 52, 167, 7, 52, 212, 193]` = `FLASH_BORROW_DISCRIMINATOR` (`arb-constants/src/lib.rs:337`). Match.
- `sha256("global:flash_repay_reserve_liquidity")[:8]` = `[185, 117, 0, 203, 96, 245, 180, 186]` = `FLASH_REPAY_DISCRIMINATOR` (`arb-constants/src/lib.rs:339`). Match.

On-chain klend semantics the builder relies on (as encoded in the account metas, not independently verified against the program binary in this audit):
- `flash_repay_reserve_liquidity` requires the sysvar-instructions account (account 10, `SYSVAR_INSTRUCTIONS_ID`, `kamino.rs:46`, `:114`, `:154`) to hold the borrow at index `borrow_instruction_index`; klend rejects repays without a matching borrow in the same transaction. The builder passes the *actual build-time index* (`borrow_idx as u8`, `lib.rs:3121`, `:3205`), so pairing survives the dynamic ATA-creation prefix (borrow is not necessarily at index 2 — it is at `2 + (#create ixs)` on the native path; the `build_kamino_flash_repay` default of 2 is only used by the legacy wrapper).
- Borrow credited to `user_liquidity_ata` (account 6, writable); repay debits the same account (account 6, writable) — same account both ways, `kamino.rs:110`, `:150`, resolved once in `to_flash_accounts` (`lib.rs:379-409`).
- Fee vault (account 7) is debited on borrow (protocol collects fee on repay into the same vault, `kamino.rs:111`, `:151`), using `KAMINO_WSOL_FEE_VAULT` (`arb-constants/src/lib.rs:332`). No referrer accounts are set (`referrer_token_state/account = None` → program id as placeholder, `kamino.rs:93-99`), meaning no referral split is paid.
- Amounts are raw u64 atomic WSOL units; no decimals handling in the flash path (WSOL = 9, `decode_mint_decimals` used only for the base mint, `verified_config.rs:134-149`).

Bundle tests confirm this pairing at the wire level for the 2- and 3-hop fixtures (`bundle_tests.rs:218-277`: borrow at index 2, repay at index 5/6 with `borrow_instruction_index = 2`). The native flash path’s dynamic borrow index is **not** covered by those golden tests — `bundle_tests.rs` uses pre-warmed templates where instruction order is fixed; a template-based repay patch exists (`prewarmed_template_tests.rs`, `bundle_tests.rs:442-500`) but asserts the repay amount bytes, not the borrow-index byte.

---

## 14. Remediation pointers mapped to existing (unused) code

The codebase already contains a correct policy/builder pair that the hot path ignores; the natural remediation is wiring, not invention:
1. `should_close_intermediate_ata` semantics (`compiler.rs:600-632`) should gate every intermediate-ATA close in `assemble_flash_arbitrage` — pass the pre-trade existence/balance/2022/withheld snapshot through instead of unconditional close (`lib.rs:3209-3211`).
2. `build_close_token_account_for_program` / `build_create_ata_idempotent_for_program` (`lib.rs:516-531`, `:488-512`) should replace the SPL-only `build_close_token_account`/`build_create_ata_idempotent` (`lib.rs:534-550`, `:464-486`) in all paths, with the token program threaded from `hop.token_programs` (already plumbed for swaps, `lib.rs:1988-2003`).
3. Never close `user_wsol_ata` (canonical) — the Pump.fun-first unwrap must instead transfer borrowed WSOL → new ephemeral WSOL account, `syncNative`, swap native SOL, then re-wrap into the repay-bound ATA, mirroring `compiler.rs`'s "only close accounts created by this route" rule. The existing close at `lib.rs:3166` is the single instruction standing between this codebase and a working Pump.fun-first flash route.
4. `min_profit_raw` should flow into `EconomicConfig` as a *converted* base-raw value (`lamports_to_base_raw_checked`, `dimensional.rs:275`) — the correct conversion helper already exists and is already called elsewhere (`run.rs:853-861`).
5. `Lamports(real_net_base_raw)` casts should use `base_raw_to_lamports_checked` (`run.rs:1439` is the only correct example) before touching `dispatch.rs`, `arb-risk`, and `arb_metrics`.
6. Wire `simulateTransaction` (or enable the existing non-broadcast simulation path, `run.rs:1507-1525`) before live Jito submission until the dead-on-arrival route shapes are eliminated by construction.
