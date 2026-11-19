# End-to-End System Data Flow & State Transitions

## 1. Stage-by-Stage Transition Matrix

| Stage | Input | Transformation / Action | Output | Failure Policy |
|---|---|---|---|---|
| **1. Ingestion** | Yellowstone gRPC account updates | Decode raw account state; extract reserves, active bin, tick index | `PoolState` / `ReserveState` | Stale or lagged stream fails closed |
| **2. Supervised State** | Multi-provider streams | Filter slots, track lag, prevent split-brain updates across banks | Authoritative In-Memory Registry | Drop updates if slot lag > threshold |
| **3. Route Discovery** | Updated pool ID | Query graph for triangular cycles (2-hop / 3-hop) with mint continuity | `Vec<RouteCandidate>` | Drop cycle on duplicate pool or broken mint chain |
| **4. Quote & Sizing** | Route candidate + live reserve depth | Sizing optimizer bounds borrow amount by actual reserve vault liquidity; computes DEX quotes | `EconomicQuote` | Abort if borrow exceeds vault depth or net profit < min |
| **5. Risk Gate** | `EconomicQuote` + system health | 20-point risk evaluation (kill switch, circuit breaker, loss caps, tip floor) | `RiskDecision::Authorize` or `Reject` | Any check failure rejects transaction |
| **6. Assembly** | Authorized opportunity | Compile atomic instruction sequence (Borrow -> Hop 1 -> Hop 2 -> Hop 3 -> Repay -> Tip) | Unsigned `VersionedMessage` | Abort on CU or account overflow (>64 accounts) |
| **7. Signing & Sim** | Message + Zeroized Keypair | Sign message; execute local preflight simulation (LiteSVM) | Signed Transaction Envelope | Abort if simulation fails or balance delta <= 0 |
| **8. Jito Landing** | Signed bundle | Query live Jito Tip Floor; attach tip; submit to multi-region Block Engines | Bundle UUID & signature | Ambiguous timeout treated as pending recovery |
| **9. Recovery & PnL** | Transaction signature + slot | Poll signature status; verify slot expiration (> last_valid + 160 slots) | Realized PnL & state update | Re-sync balance; notify Telegram |

## 2. Invariants Across Transitions
- **No Bundle Splitting**: The borrow, swap hops, and repayment must exist within one single atomic transaction.
- **Fail-Closed Stale Check**: If market data is older than `MAX_STATE_AGE_SLOTS` (default 2 slots), drop immediately.
- **Zero Hallucination**: No fallback to cached prices when authoritative streams disconnect.
