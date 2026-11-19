# Business Logic and Analysis

What this system is as a business: an automated, capital-light arbitrage
desk for Solana DEXs. It rents its trading capital per-opportunity (Kamino
flash loans), so the operator's own money is never at trading risk — only
network fees, priority fees, and tips are spent from the operator wallet.

## 1. Revenue model

- **Input**: zero trading capital. Each trade borrows 100% of its principal
  from a Kamino reserve inside the same transaction that trades and repays.
- **Output**: the price difference across a cyclic route of pools, minus:
  - Kamino flash-loan fee (live reserve config, never hardcoded),
  - DEX trading fees (protocol fee + market-maker fee, DLMM dynamic fees),
  - Token-2022 transfer fees where applicable,
  - Solana priority fee (V1 total-lamports semantics),
  - Jito tip when the Jito path is used,
  - base network fee (~5,000 lamports/signature class),
  - account-creation rent where genuinely applicable,
  - conservative execution risk reserve.
- **Edge source**: momentary price dislocations between pools sharing the
  same token pair (same-DEX) or across DEXs (cross-DEX), detected from
  real-time pool state — never from external price APIs.

## 2. Strategy

WSOL-anchored cyclic routes (matching the Kamino WSOL reserve funding the
loan):

```
Kamino flash-borrow N WSOL
  → Pool A: WSOL → Token X
  → Pool B: Token X → Token Y
  → Pool C: Token Y → WSOL
Kamino flash-repay N + fee
```

- 2-pool, 3-pool, and 4-pool variants exist; 3-pool is the primary target.
- Cross-DEX variants mix Meteora DLMM, Raydium (v4/CPMM/CLMM), Orca
  Whirlpool, Pump.fun, PumpSwap per the compatibility matrix.
- Direction per pool is derived from the on-chain LbPair mint ordering —
  never assumed.
- Cycle enumeration is dynamic (route graph + Bellman-Ford negative-cycle
  signal), not a hardcoded pool list.

## 3. Decision pipeline (where money decisions are made)

1. **Discovery** (`arb-engine::route_graph` + `bellman_ford`): build the
   pool→token graph from fresh pool states; enumerate WSOL-anchored cycles;
   rank candidates by estimated net profit.
2. **Sizing** (`flash_loan_sizing`, `meteora_size_optimizer`): bounded
   multi-stage search (coarse log → linear → local-max → fine integer) over
   borrow amount B. For each B: fee = canon fee(B); quote each hop passing the
   conservative actual output forward; net = final_out − repay − all costs.
   Hard constraint: `B ≤ actual supply-vault liquidity` decoded from the live
   reserve. Non-monotonic profit is expected; the search assumes nothing.
3. **Gate ladder** (`supervisor/opportunity.rs`): per candidate — state
   freshness, account completeness, mint continuity, net profit ≥
   `MIN_NET_PROFIT_USD` (configurable, micro-arb friendly), tx size ≤ 4,096
   (V1) / 1,232 (V0), CU budget, atomic structure (borrow+swaps+repay),
   duplicate-dispatch protection, kill switch, execution mode gate.
4. **Compile** (`arb-tx::adaptive`): V0 if it fits 1,232 bytes and is
   supported; otherwise V1 (0x81, inline accounts, explicit CU/loaded-data/
   priority-fee in the message); reject > 4,096 bytes; never downgrade
   silently; never strip instructions to fit.
5. **Preflight** (`arb-sim`): simulation before dispatch when enabled;
   simulation success is a gate, not a profit claim.
6. **Dispatch** (`supervisor/dispatch.rs`): one signed transaction, sent once
   via SWQoS or Jito; the intent is persisted BEFORE send; the landing tracker
   polls to a terminal state; ambiguous timeouts are reconciled by signature
   status and slot progression, never blind-resubmitted.
7. **Settlement** (`arb-land`, `arb-store`): realized landing recorded with
   actual fees and balance deltas; PnL is reported only from realized data.

## 4. Risk policy (proportionate, not enterprise)

Borrowing amounts are intentionally small. Controls are sized to that:

- **Mandatory financial correctness**: repayment coverage, net-profit floor,
  liquidity-bounded sizing, integer checked math, atomicity verification.
- **Mandatory protocol correctness**: account matrices, PDAs, discriminators,
  token-program handling, Token-2022 transfer-fee exactness (unsupported
  extensions are rejected, never approximated).
- **Operational**: kill switch file, execution-mode gate (shadow/paper/live),
  duplicate-send protection, dispatch-intent reconciliation, fail-closed
  provider policy.
- **Removed as waste**: redundant re-validation of unchanged state, repeated
  RPC verification of data already held fresh from the authoritative stream,
  enterprise-scale liquidity forecasting, stale-cache fallbacks.

## 5. Configuration surface (business knobs)

| Knob | Meaning |
|---|---|
| `MIN_NET_PROFIT_USD` | minimum net profit to execute (small values allowed) |
| min/max borrow amount | bounding box for the sizing search |
| max slippage / hop slippage | conservative minimum-output propagation |
| max route length | route-length cap (account-budget safety) |
| allowed protocols / tokens | venue and token whitelist |
| priority fee / tip limits | cost ceilings |
| execution mode | shadow / paper / live (live requires explicit authorization) |
| `MIN_NET_PROFIT_ABS` | absolute lamport floor used by the graph config |

## 6. Competitive advantages by design

- **Capital-free trading**: flash-loan funding removes inventory risk.
- **Atomic execution**: no legging risk; any failure reverts everything.
- **V1 capability**: transactions beyond the 1,232-byte legacy limit are
  compiled natively (inline accounts, no ALT), enabling richer routes than
  V0-only competitors.
- **Freshness discipline**: every quote is derived from validated on-chain
  state at a known slot; stale data fails closed instead of fabricating edges.
- **Honest accounting**: net profit is measured in the flash-loan mint after
  every cost; nothing is reported as profit until balances prove it.

## 7. Unit economics worked example (shape, not a claim)

For borrow B (raw WSOL units, 9 decimals):

```
fee        = max(B * fee_sf, 2^60) rounded ties-away to u64   # canonical klend
out_A      = quote_A(B)          # conservative actual output
out_B      = quote_B(out_A)
out_C      = quote_C(out_B)      # final WSOL
net        = out_C − B − fee − priority_fee − tip − base_fee − reserve
executable ⇔ net ≥ MIN_NET_PROFIT and B ≤ vault_liquidity and tx fits
```

If any step is uncomputable or unverifiable, the opportunity is rejected —
the system never invents a number to make a trade look profitable.
