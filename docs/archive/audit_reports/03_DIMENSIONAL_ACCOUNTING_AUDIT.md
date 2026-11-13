# 03 — Dimensional Accounting Audit

The codebase *has* a dimensional type system (`arb-types::{Lamports, TokenAmount, Bps, Slot,
ComputeUnits, PoolId, MintId, dimensional::*, currency::*}`) and strong-typed newtypes, but the
**monetary contract is not enforced at the boundaries where it matters**, and the live base
mint is hard-wired so the violations cancel. Evidence: `audit_work/02_MATH_ACCOUNTING.md`,
`audit_work/09_WALLET_FLASHLOAN.md`.

## 1. Amount carriage — what a monetary amount must carry vs what it does

Required: mint · decimals · raw amount · human amount · price source · price slot/timestamp ·
conversion direction.

| Requirement | Status | Evidence |
|---|---|---|
| Mint carried | Partial | `TokenAmount` carries no mint; `MintId` exists but amounts aren't bound to it (arb-types/lib.rs) |
| Decimals carried | Partial | `PoolState.decimals` u8 from verified mint decode; `cycle_profit_at` zeroes them (arb-engine/src/lib.rs:209-210) |
| Raw amount | Yes | u64 base units |
| Human amount | Yes (display only) | UI/Telegram paths format `/1e9` |
| Price source for conversions | Partial | `EconomicQuote` carries lamports↔base rational rate (verified_config.rs:91-112) — the only real conversion; `SOL_PRICE_FALLBACK` env exists; `quote_timestamp_ms` is wall clock, not state slot |
| Price slot/timestamp | Missing | `EconomicQuote` uses `SystemTime::now()` and is unused for gating; slot-based `max_state_age_slots` is used instead |
| Conversion direction | Implicit | ceiling for costs, floor for value — undocumented, two divergent converters (economic.rs:258 floor vs dimensional.rs:323 ceil) |

## 2. Cross-mint arithmetic (never add/subtract across mints)

- **Lamports vs base raw.** The flash path's base mint is hard-wired WSOL
  (`supervisor/execution.rs:159-160`), so lamports and WSOL raw units coincide and every
  mixed-unit bug is invisible today. Any non-SOL base breaks `run.rs:676-679`, `lib.rs:3020-3024`,
  `run.rs:1456-1457`, `run.rs:432`, `dispatch.rs:110`, `arb-risk/src/lib.rs:592`,
  `run.rs:1048/1059/1629`, `dispatch.rs:21/474/582`.
- **Cost constants are lamports; hop inputs are base raw.** `BASE_FEE_LAMPORTS = 5_000`
  (supervisor.rs:43) is subtracted from base-raw gross. `MIN_JITO_TIP_LAMPORTS` caps `t_max`
  (run.rs:690). `PRIORITY_FEE_MAX_SHARE_BPS = 2_000` (20% of gross).
- **Ranking compares `net_profit.0` (lamports) across opportunities** (arb-engine/src/lib.rs:963,998)
  — WSOL-only valid.
- **`deductions_base = (gross_profit − real_net_base_raw)/base_scale`** (run.rs:1456-1457) —
  cross-unit subtraction before scaling.

## 3. Decimals handling

- Decode path reads u8 decimals from the **mint account** for execution (verified_config.rs;
  registry resolve_mints_decimals registry.rs:1394) — good.
- `cycle_profit_at` zeroes decimals for both legs (arb-engine/src/lib.rs:209-210) — snapshot
  economics lose per-hop decimal context.
- 6-decimal bases are supported only insofar as `derive_rational_rate` builds a rational SOL/base
  rate (verified_config.rs:91-112); nothing routes USDC-base flash loans (Kamino flash borrow is
  WSOL-only enforced, NonWsolFlashLoan, lib.rs:3131-3133), so no arbitrage is possible on a USDC
  base today — acceptable but undocumented.
- No assertion anywhere that `mint.decimals` fits the u8 cast or equals the pool's mint-account
  decimals at quote time; mismatched fixtures untested.

## 4. Token-2022 transfer fees

- Decoding is implemented and correct (`decode_transfer_fee_config`, arb-dex/lib.rs:42-241;
  ceil + maximum_fee, mirroring SPL `calculate_fee`).
- **Wiring is absent**: no live quote path prices them (cross-pool engine is the exception and is
  broken — see 02 §2.1). `CGMM/CLMM/PumpSwap` quotes assume zero fee; withheld fees are never
  withdrawn or checked in hot paths; `transfer_fee_bps` is a single per-pool field applied to
  both mints (should be per-mint) and is fed from a field with no freshness bound.
- Token program ownership of ATAs is ignored in the hot path (SPL-derived ATAs for 2022 mints) —
  functionally fatal, fail-closed.

## 5. Profit reconciliation ledger (per mint)

For the canonical flash route (borrow WSOL):

| Stage | Mint | Raw amount | Decimals | Fee | Fee mint | Source |
|---|---|---|---|---|---|---|
| Borrow in | WSOL | B | 9 | — | — | Kamino borrow ix |
| Hop outputs/inputs | base/mid mints | Eᵢ | 9/6 | embedded in pool math | same mint | pool quote |
| Flash fee | WSOL | ceil(B·s/2⁶⁴) | 9 | U68F60 | WSOL | reserve account (fail-closed decode) |
| Swap fees | per leg, in-pool | embedded | — | in-pool | pool | quote |
| Base signature fee | SOL | 5_000 | 9 | fixed | native | network |
| Priority fee | SOL | ceil(cu_price × cu_limit / 1e6) | 9 | per-CU | native | SetComputeUnitPrice ix (inflated uncapped by builder, lib.rs:107-109 — but live path uses capped adaptive price) |
| Jito tip | SOL | min(bid, t_max) | 9 | — | native | last ix, `jitodontfront`-marked |
| ATA create/close rent | SOL | ~0.00203928 SOL/ATA | 9 | rent | native | payer |
| **Residual profit** | **WSOL (or native SOL if last hop is Pump.fun)** | E − B − flash_fee | 9 | — | — | payer's WSOL ATA |

USD figures (`profit_usd = profit_sol × sol_price`) are self-consistent only for WSOL; Telegram
labels hard-code "SOL" for any base (dispatch.rs:565-582). Metrics `add_net_profit(profit_sol)`
same. No profit is ever sent anywhere except the payer — ownership holds.

## 6. What must change to make the accounting honest (design-level, not implemented)

1. Bind every amount to a mint via a `Money<M>`-style newtype or carry `(MintId, u8 decimals,
   u64 raw)` through hop/gate APIs; forbid bare `u64` in profit math.
2. One converter (`dimensional`) with explicit rounding direction annotated at each call site; delete the duplicate in `economic.rs` or unify.
3. Re-encode `flash_fee_raw`, `tip`, `base_fee`, `priority` as base-mint raw at the point they enter base-raw formulas, with an explicit `lamports_to_base_raw_checked` (exists, dim.rs:275, used at run.rs:853 — apply it uniformly at run.rs:676, lib.rs:3020, risk casts).
4. Record the price source + slot alongside `EconomicQuote` and gate on both.
5. Split per-mint `transfer_fee_bps` (x/y) and thread through every quote arm; fail closed when a 2022 mint's bps is unknown.
6. Add unit tests with a 6-decimal base mint fixture to force the masks off.
