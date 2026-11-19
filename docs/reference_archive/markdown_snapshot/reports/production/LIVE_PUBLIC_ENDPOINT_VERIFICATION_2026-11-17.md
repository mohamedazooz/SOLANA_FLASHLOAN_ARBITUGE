# Live Verification on Free Public Endpoints — 2026-11-17

**Question answered:** paid RPC plans are NOT required to verify accounts, equations,
quotes, or the full opportunity pipeline. The free public endpoint
(`api.mainnet-beta.solana.com`, 100 req/10s per IP per official docs) is sufficient
for verification and rehearsal workloads. Paid plans only matter for
production-grade streaming throughput and quotas.

## 1. RPC account & equation verification (live, public endpoint)

Source: `reports/production/public_endpoint_live_probe_2026-11-17.json`
(via `scripts/probe_rpc_wss_v1_sim.py`, read-only).

| Check | Live result | Verdict |
|---|---|---|
| RPC version | solana-core 4.4.0, slot 470547135 | OK |
| Kamino WSOL reserve owner | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | matches arb-tx constant |
| Kamino reserve discriminator | bytes match deployed layout | OK |
| **Kamino flash_loan_fee_sf (live)** | **11,529,215,046,068** | **exactly matches `arb-constants`** — fee equation inputs verified on-chain |
| Reserve liquidity mint | WSOL (`So1111…`) | OK |
| Available flash liquidity (live) | **257,088.14 SOL** | ample for rehearsal borrows |
| Meteora DLMM pool owner program | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | OK, account = 904 bytes |
| V1 (SIMD-0385) transport | envelopes accepted by `simulateTransaction` (maxSupportedTransactionVersion=1) | transport compatible |

## 2. WSS live streaming (public endpoint, no paid plan)

Source: `reports/production/public_wss_account_subscribe_2026-11-17.json`.

- `accountSubscribe` on the live Meteora SOL/USDC LbPair: **2 account
  notifications received** (slots 470543792, 470543807) with
  `owner == DLMM program` and correct base64 data length (906 bytes).
- `slotSubscribe`: 29 slot notifications in 45s.
- Subscription IDs acknowledged (143323769 / 142513167 / 136).

Verdict: the free public WSS streams real account updates with correct
owner-program provenance. Production streaming at HFT rates still needs a
paid/dedicated provider, but every verification gate in this repository can be
passed without one.

## 3. Full opportunity decomposition (live, 3 sweeps, public endpoint)

`cargo run -p arb-sim --example full_path_rehearsal` with
`SOLANA_RPC_URL=https://api.mainnet-beta.solana.com`, `REHEARSAL_LOOPS=3`:

1. **Discovery**: 500 pools registered (78 MeteoraDlmm), top-30 by live liquidity
   listed; 12 meme DLMM groups with ≥2 pools + 4 meme/meme bridge pools.
2. **Fresh state capture**: per-sweep re-capture with slot-lag gating
   (max_allowed_slot_lag=2) — the 429-rotating failover pool behaved as designed.
3. **Candidate routes**: 2-pool WSOL and USDC flash routes per pair + 3-leg scan.
4. **Exact quotes → economics**: every WSOL route evaluated to
   `PROFIT_BELOW_FLOOR` with live net values (−92k … −257k raw); markets were
   efficient at measurement time.
5. **G6 guard (new) proven live**: every USDC flash-mint sweep rejected with
   `FLASH_MINT_RESERVE_MISMATCH` — previously these were quoted against the WSOL
   Kamino reserve (wrong-asset evaluation). The guard now fails closed.
6. **Builder gate (last line of defense)**: forced dry-run on the best negative
   route refused: conservative final output 9,900,240 < repayment 10,000,100 raw.
   Nothing built, nothing broadcast.

## 4. Fuzzing (G9) — cargo-fuzz harness added

`fuzz/` (excluded from the workspace, `cargo +nightly fuzz`):

| Target | Invariants | Result (20s each) |
|---|---|---|
| `cpmm_quote` | out ≤ reserve, monotonic in input, no panic | 3,139,620 runs — clean |
| `net_profit` | net = gross − costs exactly when costs fit; never panics | 10,244,832 runs — clean |
| `transfer_fee` | fee never increases amount; smaller cap ⇒ net ≥ uncapped | 8,336,936 runs — clean |
| `dlmm_traversal` | multi-bin max ≥ active-bin max; no panic | 2,852,587 runs — clean |

**Fuzz-found real bug (fixed):** `cpmm_out_fee` / `cpmm_out_upfront_fee_ceil`
performed unchecked U256 multiplication (`gamma·dx·y` spans up to 384 bits) —
panicking under overflow-checks and silently wrapping in release. Both now use
checked arithmetic and return `None` (fail-closed). Regression tests:
`cpmm_out_fee_fails_closed_on_wide_inputs`,
`cpmm_out_upfront_fee_ceil_fails_closed_on_wide_inputs`.

## 5. Remaining honest limitations

- 24-hour shadow soak (G12): a bounded 3-sweep live soak passed end-to-end on
  the public endpoint; the 24-hour gate remains open and can be run on free
  infrastructure (`run-shadow.sh` + public RPC) — it needs wall-clock time, not
  money.
- Solami Mirage parsing (G5): the free Solami plan rejected WSS with
  code 4002 (no bandwidth) — blocked on provider quota, not on payment by us.
- HFT-grade streaming throughput: public endpoint is rate-limited; fine for
  verification, not for production latency.
