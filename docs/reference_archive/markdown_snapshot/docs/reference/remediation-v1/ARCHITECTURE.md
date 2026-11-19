# Proposed architecture: Proof-Carrying Opportunity Pipeline

```text
On-chain feeds
  └─> Versioned State Store (slot, owner, account hash, decode version)
       └─> Pool Decoders (one canonical decoder per protocol)
            └─> Quote Engine (exact integer math per protocol)
                 └─> Route Search (candidate paths only)
                      └─> Split Re-Quote (per-pool amount allocation)
                           └─> Economic Ledger (single base mint, explicit costs)
                                └─> Risk Gate (freshness + limits + net floor)
                                     └─> Simulation Gate (real simulator only)
                                          └─> Transaction Planner
                                               └─> Execution Adapter (still gated)
                                                    └─> Confirmation + Balance Reconciliation
```

## The unique design element: proof-carrying opportunity

A route is not represented only by “expected profit”. It carries a compact evidence object:

- `snapshot_id`: slot, account hashes, decoder versions, observation age.
- `leg_quotes[]`: pool ID, protocol, input/output mints, raw input/output, fee, min-out, state slot.
- `economic_ledger`: costs with units, source IDs, observation slots, and uniqueness keys.
- `simulation_receipt`: simulator identity/version, state slot, compute result, success/failure.
- `risk_decision`: thresholds and exact quote/ledger hashes used for approval.

The planner must reject an opportunity if any evidence is missing, stale, duplicated, mismatched, or from a different snapshot. A route quote and the transaction builder must consume the same leg records and economic ledger; neither may independently reconstruct fees or costs.

## Split routing contract

For each split leg, obtain a fresh protocol-native quote from that pool. Never reuse the first leg's mint pair, fee, active tick/bin, or decoder state. Sum raw intermediate outputs only when they share the same mint and decimals. Then quote the return leg against the actual aggregate amount. Aggregate impact is calculated from the aggregate re-quote and a common reference price; it is not `old_impact / number_of_legs`.

## Economic contract

All trading amounts and all converted costs are represented in raw units of one declared base mint. Lamports are a separate type until converted using a fresh, verified rate. `min_profit` has one unit contract at the API boundary. Costs are itemized once: flash-loan fee, DEX fees not already embedded in quote outputs, transfer fees, account creation, base network fee, priority fee, Jito tip, and other verified costs. Avoid double-counting DEX fees if the protocol quote already returns net output.

## Simulation contract

`Passed` is the only state that can pass. `NotRun`, `Disabled`, `TimedOut`, and `Failed` all reject. RPC `simulateTransaction` and LiteSVM are not interchangeable proofs: record which simulator ran, against which bank/state slot, and what accounts/program versions it used.

## Performance contract

Correctness first, then optimize the hot path:

- decode once per account update; publish immutable snapshots;
- avoid HTTP calls in the candidate loop;
- cache only with slot/TTL provenance and explicit invalidation;
- precompute route topology, not executable quotes;
- use bounded queues and backpressure for feeds;
- instrument age, decode lag, quote latency, simulation latency, rejection reasons, send-to-land latency, and realized PnL;
- benchmark p50/p95/p99 and CPU/allocation counts before adding Chronos or other ranking features.
