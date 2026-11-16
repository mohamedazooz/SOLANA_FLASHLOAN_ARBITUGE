# PHASE 7 — Flash Loan & Routing Audit

## Flash loan (Kamino) lifecycle — VERIFIED structurally

1. **Borrow**: `flash_borrow_reserve_liquidity` — first top-level instruction; verified
   discriminator + account order against official IDL (arb-tx/src/flash/kamino.rs).
2. **Swaps**: 2–3 hop groups between borrow and repay; per-hop min-out guards from
   chained quotes; direction from canonical orientation (on-chain mints).
3. **Repay**: `flash_repay_reserve_liquidity` — last instruction; principal + fee;
   fee read from reserve config bytes (NOT hardcoded).
4. **Atomicity**: single transaction; any failure reverts all state (Solana runtime
   semantics) — no partial-execution path exists in the builder.
5. **Instruction introspection**: `Sysvar1nstructions…` used to enforce ordering
   (borrow at index 0, repay at last) matching Kamino's requirements.
6. **Profit guard**: on-chain guarded path aborts if
   `final < principal + flash_fee + min_net_profit` (guarded.rs; zero `fee_paid` reads —
   gating uses chained `amount_out`, immune to the fee-denomination bug fixed in `152ad88`).

## Fee accounting per platform (verified this cycle)

| Platform | Fee source | Verified |
|---|---|---|
| Meteora DLMM | dynamic base + variable from LbPair bytes; `collect_fee_mode` from byte @36 forwarded end-to-end (`152ad88`) | oracle 6/6 + regression tests |
| Orca Whirlpool | `decode_orca_fee_bps` from fee_rate @45 (bps) | probe + decoder tests |
| Raydium CLMM | fee from pool bytes (bps→ppm documented) | decoder tests |
| Raydium V4/CPMM | on-chain fee decoders | decoder tests |
| Kamino | reserve config bytes | IDL cross-check |

## Dynamic loan sizing — VERIFIED structurally

- No fixed borrow amount in the live path (grep-verified; sizes derive from pool depth,
  bin distribution, fees, and hurdle math).
- Candidate-size selection exists in the guarded engine; **live optimum calibration
  NOT PROVEN** (no live opportunity has reached dispatch — nothing to calibrate against).

## Route engine

- Base cycles (2-hop) + cross-pool DLMM + multi-hop (3-leg DLMM live path =
  primitives/builder.rs). Route validity re-checked at execution time (fresh quotes,
  no stale pricing — verified: quotes built from cache written this tick).
- Route labels propagate into rejection WARNs (soak11+ provenance).

## Route test matrix (results)

| Scenario | Result |
|---|---|
| 2-pool DLMM routes | PASS (soak15/16: pairs=15–21, directions=30) |
| 3-pool DLMM routes | PASS structurally (first real 3-hop cycles appeared soak15, correctly gate-rejected) |
| Insufficient liquidity | PASS (zero-depth fail-closed tests) |
| Negative expected profit | PASS (EDGE_BELOW_HURDLE rejections; honest gate) |
| Fee changes mid-flight | PASS (fees re-decoded every fetch; dynamic DLMM fee per tick) |
| Insufficient repayment | PASS (atomic revert guard + profit guard) |
| CU budget exhaustion | PARTIAL (estimator exists; sim-fed CU update verified in code, live measurement N/A) |
| Transaction-size exhaustion | PASS (V0 measured; V1 blocked — see EXECUTION_AUDIT) |

## NOT PROVEN

- Any live flash-loan execution (shadow-only so far — by design).
- Optimal-size calibration against real fills.
