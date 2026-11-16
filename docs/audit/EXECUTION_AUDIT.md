# PHASE 8 — Transaction Builder & Executor Audit

## Construction (verified)

- Instruction ordering: ComputeBudget (limit + price) → flash borrow → swap groups →
  repay; enforced by the compiler with instruction-sysvar introspection.
- Versioned transactions: v0 with ALT (`master_alt.rs`); measured serialized sizes in
  arb-tx tests (V0 ≤ 1232 respected).
- V1: header differential test exists; **live V1 submission NOT VERIFIED** — no target
  provider currently exposes V1; no silent downgrade path exists (verified: transport
  rejects rather than downgrades).
- Blockhash: fetched fresh per dispatch; `last_valid_block_height` tracked (variable
  unused-warning under `--all-features` only — containment artifact).
- Duplicate submission: inflight tracking (`InflightStatus`) prevents re-dispatch of the
  same opportunity.

## Simulation gates (verified order)

1. LiteSVM local sim (fast reject) — **known defect P1-2: empty VM, misleading
   "succeeded"**; treated as a filter, NOT as evidence of correctness.
2. RPC `simulateTransaction` on real state — custom errors, CU consumed, logs parsed;
   failure blocks submission.
3. Slippage/min-out baked into swap instructions (not just sim-checked).

## Submission & landing

- Jito bundle path: `jito_lander.send_bundle_multi_region` (dispatch.rs:582) — wired;
  tip part of net-profit math (arb-land TipStrategy). **Live bundle landing NOT PROVEN.**
- SWQoS direct RPC path (dispatch.rs:566 region) — alternative transport.
- Status distinction: RPC acceptance ≠ on-chain execution; landing confirmation logic
  exists (`observe_bundle_rtt` + lander status polling). **Live confirmation UNPROVEN.**

## Failure handling

- Timeout → treated as failure (not evidence); retries bounded; duplicate prevention
  via inflight map.

## Gaps

| # | Gap | Status |
|---|---|---|
| X-1 | V1 live broadcast | BLOCKED (provider support) |
| X-2 | Jito live landing evidence | BLOCKED (requires live mode + funded key) |
| X-3 | LiteSVM empty-VM sim misleading | OPEN (P1-2) |
