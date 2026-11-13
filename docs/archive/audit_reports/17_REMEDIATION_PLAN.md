# 17 — Remediation Plan

Ordered by **risk per unit of work**, preserving the audited state (no changes are applied by this
document; implementing them requires a separate patch or branch and a re-read of the diffs).
Every item cites the bug register (`16_COMPLETE_BUG_REGISTER.csv`) and the audit section.

## 0. Immediate containment (do first, same day)

1. **Disarm live firing.** Set `config.toml [bot] mode` to a non-live value, and
   `leader_gate_enabled = true`; treat `.env`'s `ARB_LIVE=1/LIVE=true/EXECUTE_LIVE=true` as hostile
   defaults — rotate every secret in `.env` and `.mcp.json` (Helius key), because they were found
   in plaintext on disk. (ARB-038, ARB-044)
2. **Assert the arming invariant in code.** `if mode == Live { bail unless ARB_LIVE == "1" }` at
   supervisor startup; refuse to run otherwise (fail-closed). (ARB-038)
3. **Stop trusting the 1232 B gate as a backstop.** Until V0 is fixed and wired, deep-route
   opportunities must be rejected *before signing* with a metric — they are unfillable today.
   (ARB-002)

## 1. Correctness stop-the-bleeding (days)

4. **Fix the V0 ALT index bug.** Compilers must receive the *full* ALT table; emit only referenced
   indices. Delete the filtered-subset pattern; make `pos > 255` an `Err`. (ARB-001, ARB-034,
   ARB-046) Regression: sign V0 with a real table and simulated-resolve equals the legacy inline keys.
5. **Fix the bin-array span.** `span = |idx(end_bin) − idx(active_bin)| + 1`, direction-aware,
   from the quote's `end_active_id`; delete the 2-array cap; remove the atomic builder's silent
   under-provisioning; add property tests over (active, crossed, direction). (ARB-006, ARB-007)
6. **Wire the bitmap extension decision.** Compute `exceeds_default_bitmap(|idx|>512)`;
   pass `Some(extension_pda)` iff required; else reject the pool. Parse the extension account or
   fail closed. (ARB-008)
7. **Fix the cross-pool Token-2022 double subtraction** (use the helper's post-fee amount; thread
   per-mint bps) and **hardcoded status/fee-mode** (propagate decoded values). (ARB-003, ARB-009)
   Regression: fixtures with bps>0 and OnlyY mode quote correctly; re-run meteora SDK differentials.
8. **Fix `assemble_flash_arbitrage` instruction order** — never close an ATA pre-swap; apply
   `should_close_intermediate_ata` semantics (created-here, zero balance, no withheld fees,
   token-program aware) for any close. (ARB-004, ARB-018, ARB-019)
9. **Kamino fee everywhere.** `build_kamino_bundle`/`assemble_atomic_bundle` must decode
   `flash_loan_fee_sf` live and pass it through; zero should only appear in explicitly-labeled
   donation-fee tests. (ARB-005)
10. **Mint a 6-decimal base fixture for the whole gate** and fix every unit-mix site by converting
    at the boundary (`lamports_to_base_raw_checked`). (ARB-020, ARB-021, ARB-024, ARB-025)
11. **Ingest honesty.** Stamp pools with the account's context slot only (both WS and HTTP);
    DLMM oracle PDA fix; decode-failure ⇒ fail closed (never catalog fee). (ARB-010, ARB-011,
    ARB-035, ARB-036, ARB-037)

## 2. Accounting and decisioning (days–weeks)

12. **One net-profit definition.** Delete the opportunity-level shortcut or align it with the
    dispatch gate; pass a real slippage-haircut `minimum_final`; delete or use `CostLedger`.
    (ARB-021)
13. **Jito cap chain.** Call `check_tip_lamports`; make `$0.20` the code constant or fix the
    comment; ship `keep_share/max_tip_share_bps/max_tip_spend_ratio` at code defaults, not the
    loose shipped values; expired-after-7s re-poll and neutral ledger mapping; `tip ≥ net_profit`
    stays. (ARB-014, ARB-015, ARB-016, ARB-033, ARB-017)
14. **Size the real problem.** Delete `golden_section_size` (or restrict to CPMM-verified
    unimodal shapes); keep integer multi-basin for DLMM; top-4-by-profit basin truncation;
    saturating conversions → errors. (ARB-026, ARB-027, ARB-028, ARB-053)
15. **ALT manifest surgery.** Evict the 8 tip accounts; verify ALT once per epoch (owner,
    activation, deactivation, authority, presence of required Class A); re-point templates to a
    generation/epoch id. (ARB-022, ARB-042, ARB-023, ARB-046)
16. **Adapter fixes:** LaunchLab discriminators or delete; PumpSwap sell_v2; CLMM bit 4; Whirlpool
    direction; LaunchLab/DammV2 dispatch wiring or deletion; Token-2022 ATAs everywhere or reject
    2022 outright. (ARB-029, ARB-030, ARB-031, ARB-032, ARB-041, ARB-019, ARB-043)

## 3. Evidence gates before any non-live shadow run (weeks)

17. **Simulation-only rehearsal.** For every route class (CPMM 2/3-hop, DLMM X→Y/Y→X both,
    CLMM, Whirlpool non-SOL legacy, Pump.fun, PumpSwap, LaunchLab), produce a signed V0 (after
    gate 4) and pass `simulateTransaction` against current mainnet state **without broadcasting**.
    Record slot, CU consumed, logs, resolved ALT keys.
18. **Flash-loan round trip on a fork.** litesvm or Bankrun: borrow→swap→repay with a captured
    reserve; assert repay == borrow + ceil(B·sf/2^64); assert revert without repay; assert atomic
    revert on any hop failure; assert `borrow_instruction_index` binding.
19. **Coherent-snapshot proof.** Force one stale leg into a 3-pool route; assert rejection via
    `validate_dependency_group` (or its live equivalent) — wire the strong gate first.
20. **Adversarial suite to green.** Every MISSING row in `15_ADVERSARIAL_TEST_RESULTS.md` becomes
    an executable test; the debug-perf flake is quarantined.
21. **Shadow soak.** `mode = shadow` for ≥ 7 days with live ingest; zero sends; report: routes
    found, simulated pass rate, would-be PnL distribution, landing predictor calibration. No SOL
    at risk. A route that would revert on-chain must surface as a sim failure, not a live loss.
22. **Cost ledger reconciliation.** For each simulated route, reconcile
    borrow→hops→repay→fees→tip from trace; residual must equal the ATA delta to the lamport.

## 4. What not to do

- Do **not** create/extend/close an ALT until V0 resolution is fixed and the manifest is amended.
- Do **not** wire V1 — it is unsupported by the dispatch gate, Jito, and (here) every transport.
- Do **not** "fix" this by adding balance-based polling or by switching the base mint; fix the units.
- Do **not** paper over the 1-failed/3-ignored tests by deleting them.

## 5. Sign-off criteria for upgrade from BLOCKED

All of: (a) cargo check/clippy(-D warnings)/fmt/test green (except explicitly quarantined perf
test); (b) every CRITICAL/HIGH bug in the register closed with a regression test; (c) gates 17-22
satisfied; (d) independent re-audit of the diff; (e) config audit: shipped TOML fail-closed end-to-end.
