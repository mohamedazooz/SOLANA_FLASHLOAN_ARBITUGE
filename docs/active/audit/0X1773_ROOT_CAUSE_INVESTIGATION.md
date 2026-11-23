# ISSUE-P0-04 — `0x1773` Root-Cause Investigation

Status: **UNRESOLVED — FAIL-CLOSED** (error mapping VERIFIED; historical root cause NOT proven).
Last reviewed: 2026-11-22 (HEAD `26ccba1`, branch `fix/solana-arbitrage-production-remediation`).
Mandate §12 explicitly forbids closing this issue by assumption or by documentation edits.

---

## 1. What is now VERIFIED (evidence-backed, at HEAD)

### E1 — Error identity and emitting program (from the captured incident itself)

The only captured failure is the Phase 17 non-broadcast mainnet simulation
(`reports/phase17_cross_dex/INDEPENDENT_AUDIT.md` L120–210, context slot **471552883**).
Its log text names BOTH the failing program AND the error:

```
[25] Program log: AnchorError thrown in programs/lb_clmm/src/instructions/v2/swap2.rs:262.
     Error Code: ExceededAmountSlippageTolerance. Error Number: 6003.
     Error Message: Exceeded amount slippage tolerance.
[27] Program LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo failed: custom program error: 0x1773
```

Because the log prints the Anchor error **name** and the failing **program id**
(`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` = Meteora DLMM `lb_clmm`), the previously
recorded numeric ambiguity in `crates/arb-sim/src/forensic.rs` — `Custom(6003)` without program
context could also be **arb-executor `InvalidMemoProgramId = 6003`**
(`programs/arb-executor/src/error.rs:19`) — is **resolved for THIS incident**: it is the
Meteora DLMM `ExceededAmountSlippageTolerance`. The generic `Custom(6003)` mapping remains
ambiguous for logs that do NOT name the program; `forensic.rs` keeps the emitting-program hint
exactly for that reason.

### E2 — IDL corroboration (pinned in this repo)

`fixtures/dlmm.json` (program address `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`):

- error `6003` = `ExceededAmountSlippageTolerance` — "Exceeded amount slippage tolerance";
- `swap2` args: `amount_in: u64`, `min_amount_out: u64`, `remaining_accounts_info`;
  sibling errors on the same instruction: `6001 InvalidBinId`, `6002 InvalidInput`,
  `6010 InvalidTokenMint`, `6012 PairInsufficientLiquidity`, `6018 MathOverflow`,
  `6027 InvalidBinArray`, `6028 NonContinuousBinArrays`, `6039 InsufficientOutAmount`,
  `6042 PoolDisabled`.

### E3 — Official documentation corroboration

Meteora docs, "DLMM Program Errors"
(https://docs.meteora.ag/developer-guides/dlmm/program/errors):

- `6003` = `ExceededAmountSlippageTolerance`, raised from `swap`, `swap2`,
  `rebalance_position`, `add_liquidity_single_side_precise2`.

### E4 — Independent third-party occurrence of the same failure mode

`MeteoraAg/dlmm-sdk` issue #255 (open) records a trader's Swap2 failing with
`custom program error: 0x1773` at **`programs/lb_clmm/src/instructions/v2/swap2.rs:244`** —
a different line inside the same instruction (our incident fired at `swap2.rs:262`). The FR
drops 2 bin-level transfers before aborting on the min-out check; the community-acknowledged
failure mode is quote staleness between quoting and execution ("requote immediately before
send and rebuild min_out_amount; a 5% slippage setting still fails if the bin array moved
between quote and execution"). This is corroboration of the *failure class*, NOT a proof for
our incident.

### E5 — Upstream source availability

The `lb_clmm` program source is **closed** on GitHub (GitHub code search finds `6003` only in
IDLs; there is no `MeteoraAg/dlmm` program source repo; `MeteoraAg/dlmm` returns 404). The
`swap2.rs:262` line number comes from the Anchor panic string in the deployed binary and
cannot be mapped to a source line we can read. The repo therefore re-implements the quote
math in `crates/arb-math/src/dlmm/` and has correct `ExceededAmountSlippageTolerance`
semantics only as an *expected* on-chain behavior, not as vendored source.

### E6 — Failure classification

Per mandate §12(7): this is a **program failure** (the `lb_clmm` Swap2 min-amount-out
enforcement reverted the instruction), **not** a transport failure. The 1331-byte V1
transaction (32 inline accounts, 0 ALTs, version discriminator `0x81`) assembled and
dispatched to simulation correctly; the SVM rolled back atomically: Kamino principal was
protected, the Jito tip was never paid (bundle aborted), and only base + priority fees burned
(34,635 CU consumed at failure).

### E7 — Which of our gates should have bounded this

The encoded `min_amount_out` for the failing leg was produced by the quote chain under the
then-active slippage authority. Two of the seven remediation findings directly bound the
recurrence probability, and both are FIXED on the remediation branch (PR #151):

- **F-02** (commit `08e6657`): canonical Bin Array span semantics. Before the fix, a leg
  quoted across an array boundary could under-provision the Bin Array account set by one
  array. Such a mismatch would normally surface as account-not-found / `InvalidBinArray`
  / `NonContinuousBinArrays` — **not** as `0x1773` — so F-02 did not cause this incident,
  but the quote↔execution consistency now guarantees the account set the quote assumed.
- **F-04** (commit `8d58487`): single authoritative per-opportunity slippage
  (`hop_slippage_bps` 10 bps vs `dlmm_three_leg.slippage_bps` 15 bps drift removed; stamped
  as one value at quote time, validated at config load, never overwritten after quoting).

Additionally the final-hop min-out in `TxBuilder::derive_swap_legs`
(`crates/arb-tx/src/primitives/builder.rs:385-392`) is
`min_out_slippage.max(floor)` where the floor is `borrow + flash_fee + tip + min_profit`
— i.e. the encoded minimum is deliberately **tighter than pure slippage tolerance** so a
marginally-drifting quote reverts fail-closed instead of losing capital. A `0x1773`
therefore ALWAYS means the on-chain fill could not even cover the repayment+fee+floor:
the transaction protected the principal exactly as designed.

---

## 2. Root-cause candidates and their evidence status

| # | Candidate | Evidence at HEAD | Verdict |
|---|-----------|------------------|---------|
| a | Quote staleness / slot drift between quote and simulation (meme pools move fast; 15 bps margin) | Consistent with E4's accepted failure mode; context slot 471552883 is recorded but the quote slot and the simulated bank slot are NOT in the capture | **Plausible, unprovable without replay inputs** |
| b | Variable-fee model drift between `arb-math`'s re-implementation and on-chain | Differential suites exist (`docs/active/audit/QUOTE_PARITY_REPORT.md`), but no captured per-hop fee values for the incident | OPEN — unconfirmed |
| c | min-out floor composition (last hop floor = repay + fee + tip + profit) — designed tightness causes the revert instead of a loss | By construction (`builder.rs:385-392`). This is intended fail-closed economics, not a defect | CONFIRMED behavior; not a "fixable" cause |
| d | Empty-bin decoding (F-03) distorting the quote | F-03 domain; would more likely produce `BinNotFound` locally / `InvalidBinArray` on-chain, not `0x1773` | Unconfirmed, indirect |
| e | `bins_traversed` off-by-one under-provisioning (F-02 root cause found this session) | Would surface as missing/extra Bin Array accounts, upstream `InvalidBinArray`/`NonContinuousBinArrays` — **not** `0x1773` | Ruled out as the 0x1773 mechanism (see E7) |

Per mandate §12(9): **no remediation beyond what is already proven is implemented.** We do
NOT widen slippage, do NOT clairvoyantly re-quote further, and do NOT close the issue.

---

## 3. Replay requirements (blocking the issue closure)

Per `reports/phase17_cross_dex/PHASE17_CORRECTION_2026-11-20.md` — the historical event lacks
ALL of:

1. Exact signed wire bytes of the attempted transaction (1331-byte V1);
2. Account snapshot of the pool/bin arrays at the simulation bank (slot 471552883);
3. The quote slot the opportunity was derived from;
4. Per-hop quote outputs and the encoded per-hop `min_amount_out`s;
5. Slot-skew proof between (3) and the simulation bank.

The forensic capture plumbing (`crates/arb-sim/src/forensic.rs`, `ForensicBundle`,
sha256-pinned wire + per-hop provenance + bounded logs + emitting-program hint + context
slot; wired into supervisor pre-flight failure paths writing `reports/forensics/`, and
replayed via the fail-closed LiteSVM harness) records every FUTURE failure as a replayable
bundle. The first live-captured failure with a matching bundle is the dependency for
re-closing or re-defining ISSUE-P0-04.

---

## 4. Verdict for this investigation

- **Error mapping**: VERIFIED (E1–E3, for this incident; generic `Custom(6003)` without
  program context stays ambiguous by design).
- **Failure class**: VERIFIED program-level min-out enforcement failure with correct atomic
  rollback (E6).
- **Root cause of the historical event**: NOT PROVEN — replay inputs do not exist (§3).
- **Issue status**: UNRESOLVED — FAIL-CLOSED. Live execution remains blocked. Slippage
  limits remain unchanged.
- **Recurrence bounding already shipped (PR #151)**: F-02 quote↔execution account-set
  consistency, F-04 single slippage authority, F-05 checked min-out arithmetic, F-07 signed
  shortfall observability on every pre-flight rejection, and forensic capture for every
  future simulation failure.
