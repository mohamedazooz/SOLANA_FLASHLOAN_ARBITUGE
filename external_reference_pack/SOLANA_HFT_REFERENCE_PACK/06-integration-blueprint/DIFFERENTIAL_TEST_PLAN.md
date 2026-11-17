# Differential Test Plan

## 1. CPMM
For each fixture, compare project quote against an independent integer reference:
- both directions.
- tiny input, zero input, near-reserve input.
- fee = 0, typical fee, max valid fee, invalid fee.
- reserve zero/near zero.
- overflow boundaries.
- exact-output inverse: verify forward(amount_in) >= desired_out and amount_in-1 is insufficient where mathematically applicable.

## 2. CLMM / Whirlpool / Raydium CLMM
- current tick and sqrt_price consistency.
- cross one tick and multiple ticks.
- missing tick arrays and bitmap extension.
- correct direction and token ordering.
- fee tier and rounding.
- account ownership/discriminator validation.

## 3. Meteora DLMM
- bin step and active bin.
- zero liquidity bin, one-bin, multi-bin traversal.
- bitmap extension and out-of-range bin arrays.
- dynamic fee and protocol fee.
- limit-order fill paths if enabled.
- Token-2022 transfer fee and hook behavior.
- exact-input and exact-output comparison with pinned official SDK.

## 4. Route graph
- direct two-hop and triangles.
- cycles with repeated pools/tokens.
- disconnected graph.
- negative/zero edge.
- duplicate pools.
- route budget exhausted.
- deterministic tie-break.
- ensure every edge has a quote tied to the same snapshot and amount.

## 5. Yellowstone / State
- duplicate (pubkey, slot, write_version).
- lower write_version in same slot.
- older slot arriving after newer slot.
- account tombstone.
- reconnect and resubscribe.
- slot dead / fork rollback.
- missing bootstrap account.
- provider disagreement.
- queue overflow/backpressure.
- stale Clock/blockhash.
- mixed commitment.

## 6. Transaction execution
- exact instruction ordering.
- account meta flags and ordering.
- missing optional/required accounts.
- packet/account/CU limits.
- simulation error categories.
- relay accepted but not landed.
- timeout then late landing.
- duplicate submission.
- blockhash expiry.
- actual fee and tip reconciliation.

## Evidence format
Each differential case emits JSON:
`case_id, upstream_repo, upstream_commit, protocol, input_fixture_hash, expected, actual, delta, pass, test_command`.
