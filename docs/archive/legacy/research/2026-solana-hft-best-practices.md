> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# 2026 Solana HFT Best Practices — Verified External Research (P0)

> **Status:** Research deliverable of Phase P0 per the Master Engineering Directive.
> **Compiled:** 2026-11-18. Sources fetched live on this date. Every claim carries a
> source URL and a classification: **CONFIRMED** (read from official live source),
> **LIKELY** (multiple corroborating secondary sources), **UNKNOWN / NOT VERIFIED**.
> Contract prescribes: source, last-update date, spec/condition, project impact,
> files affected, application method, and test case.

---

## 1. Jito Block Engine & submission paths

### 1.1 Endpoints (CONFIRMED — docs.jito.wtf/lowlatencytxnsend, fetched 2026-11-18)

| Path | Purpose |
|---|---|
| `/api/v1/transactions` | `sendTransaction` (sent as plain tx or `?bundleOnly=true`) |
| `/api/v1/bundles` | `sendBundle` (≤5 signed txs, sequential, all-or-nothing, single slot) |
| `/api/v1/getBundleStatuses` | bundle status (≤5 IDs, confirmationStatus processed/confirmed/finalized, `err`) |
| `/api/v1/getInflightBundleStatuses` | last-5-min in-flight states: `Pending` / `Failed` / `Landed` / `Invalid` |
| `/api/v1/getTipAccounts` | runtime tip-account list |

Regions (mainnet): global `mainnet.block-engine.jito.wtf`; amsterdam, dublin, frankfurt,
london, ny (`ny.`), slc, singapore, tokyo. (Separate shred-receiver/NTP per region listed
in docs — validator-side only.)

**Impact on project:** `arb-land/src/lib.rs` (`send_bundle_multi_region`) uses region
subdomains — matches official layout (CONFIRMED correct). **Files affected:**
`crates/arb-land/src/lib.rs`, `crates/arb-land/src/bidder.rs`, `arb-constants`.
**Test case:** startup integration test asserting each configured region URL returns
`getTipAccounts` and the returned set matches expectations.

### 1.2 Authentication (CONFIRMED)

- Header `x-jito-auth: <uuid>` or query `?uuid=<uuid>`.
- "You no longer need an approved auth key for default sends" — default rate limit
  **1 request/second per IP per region**, HTTP 429 beyond it.
- DBI credentials are issued **separately** from a Block Engine UUID
  (bam.dev/docs/bam/dbi, updated 2026-11-06).

**Impact:** **FINDING (PIPE-04):** `arb-land` never attaches an `Authorization` /
`x-jito-auth` header on `sendBundle`. At production rate limits (bundles sent more than
once/second per region during multi-region broadcast) and for any account-priority
behavior, this fails. Must read `JITO_AUTH` UUID from env and attach it. **Files:**
`arb-land/src/lib.rs:229-262, :383-440`. **Test:** HTTP-layer test asserting the header
present when env is set.

### 1.3 Tip accounts (CONFIRMED — 8 addresses)

`96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5`,
`HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe`,
`Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY`,
`ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49`,
`DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh`,
`ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt`,
`DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL`,
`3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT`.

- Tips = any instruction (top-level or CPI) transferring SOL to one of these. Do **not**
  reference tip accounts via ALTs. Block Engine guidance: pick an account **at random**
  to reduce contention.
- **Minimum tip: 1,000 lamports** (all regions; enforced on bundles).

**Impact:** repo's `JITO_TIP_ACCOUNTS` in `arb-constants/src/lib.rs:59-117` matches the
official 8 (verified char-for-char in P0 audit). But selection is **deterministic by
slot/leader** rather than random among valid accounts — acceptable, yet the set is
**hardcoded with no runtime `getTipAccounts` verification in this tree** (a stale
worktree copy had it; the code was dropped). Fix: fetch + verify at startup, fail-closed
(or warn) on mismatch. **Files:** `arb-constants`, `arb-bot/src/jito_service.rs`,
`arb-land`. **Test:** boot test asserting `getTipAccounts()` comparison logged and
enforced.

### 1.4 Auction mechanics (CONFIRMED — docs.jito.wtf)

- Parallel auctions in ~50 ms ticks; bundles grouped by intersecting write-lock patterns
  `[(w,w),(r,w),(w,r)]`; ranked by **tip / requested-CU efficiency**; highest-paying
  combo submitted up to CU limit.
- sendBundle ack (`bundle_id` = SHA-256 of signatures) is **receipt only**, not
  processing/landing.
- Official dynamic-tip guidance: poll `https://bundles.jito.wtf/api/v1/bundles/tip_floor`;
  p50 normal, p75 typical, p95/p99 competitive arb; adjust for latency to each region.
- Live measured floors (2026-11-18 02:31 UTC): p50 1.37e-6 SOL (~1,371 lamports),
  p75 5.54e-6, p95 1.39e-4, p99 7.53e-4 SOL. **Min tip 1,000 lamports is the absolute
  floor, not a competitive bid.**
- sendTransaction guidance: ~70/30 priority-fee:tip split of total fee budget
  (CONFIRMED docs guidance).

**Impact on project:** `arb-land/src/tip_oracle.rs` already implements a tip-floor
oracle with a 5 s freshness hard gate, median degradation and competition clamp
[1.0, 2.5] (verified in code) — matches official best practice. `MIN_JITO_TIP_LAMPORTS
= 1_000` matches the official floor. The tip must never consume the min-profit floor:
already enforced via `t_max = net_before_tip − min_floor − contingency ≥ MIN_JITO_TIP`
(`supervisor/opportunity.rs:167-220`, verified). **Test:** golden test asserting tip
loss-gate `expected_net_after_tip ≥ MIN_NET_PROFIT` holds across the clamp range.

### 1.5 Bundle lifecycle semantics (CONFIRMED)

Official states: in-flight `Pending | Failed | Landed | Invalid` (5-minute window);
`getBundleStatuses` returns per-bundle `confirmationStatus ∈ {processed, confirmed,
finalized}` + `err`. The vocabulary "ACCEPTED / EXECUTED" is **not** Jito API vocabulary
(third-party shorthand). Submissions must not be upgraded: receipt ≠ landed, landed ≠
finalized.

**Impact:** repo's `TransactionLifecycle` state machine (jito_service.rs) already
distinguishes Processed/Confirmed/Finalized and the recovery engine never resubmits
(verified C12). **FINDING (PIPE-12):** `landed_status_str` labels every landing
"Confirmed / Finalized" regardless of actual lifecycle. Fix: map the real lifecycle into
alerts. **Files:** `dispatch.rs:988-996`, `jito_service.rs:780-810`.

### 1.6 BAM / DBI / leader coverage (2026)

- BAM mainnet rollout Sept 25, 2025; **DBI** (Direct Bundle Ingest) GA Oct 13, 2026 —
  searchers send bundles **directly to a BAM node**: JSON-RPC `:9090/api/v1/bundles`
  (UUID auth) or QUIC `:11228` (ALPN `solana-tpu`, allowlisted Ed25519 keypair). DBI
  enforces **no minimum tip**; BAM ranks by **fee-per-CU** (tips + priority fees), static
  tips. DBI IDs differ from Block Engine IDs — landing must be proven via
  `getSignatureStatuses` on your own RPC. Reference: github.com/jito-labs/bam-dbi.
- `sendBundle` via Block Engine reaches **Jito-Solana validators** (plus forwarded BAM
  leaders); non-Jito leaders never process bundles. Jito stake share ~88-95%+ (LIKELY,
  source-dependent).
- **FIREBAM** for Frankendancer validators live May 2026 (<5% of BAM stake Oct 2026).
- **Preconfirmations** product: BAM leader commits to executing a tx before shreds;
  delivered via Helius/Triton streams; min tip 0.001 SOL via Helius preconf path.

**Impact on project:** current submission layer (Block Engine + SWQoS fallback with
leader-gate, config.toml:13 `jito_regions = frankfurt/amsterdam/ny`) remains correct.
DBI access requires a separate allowlisted credential — **not available in-repo; treat
as roadmap** (add `bam.dbi` transport only after credential + access form), and note BAM
ranks by **fee per CU** which slightly shifts the tip strategy for BAM slots. Test:
transport-matrix unit test asserting DBI disabled unless credential configured.

### 1.7 Submission-path comparison (directive §3.1 required)

| Path | Requirements | Auth | Rate limits | Latency | Cost model | Landing verification | Failure modes |
|---|---|---|---|---|---|---|---|
| A. Standard RPC `sendTransaction` | RPC provider | API key | provider quota | med-high (routed) | base 5,000 lamports + priority fee (charged even on failure) | `getSignatureStatuses`/block height | no revert protection; SWQoS forwarding depends on stake path |
| B. Jito `sendTransaction` | region URL | UUID optional (limits) | 1 rps/IP/region default | low-med | base + priority + tip (70/30 guidance) | `x-bundle-id` + signature tracking | no atomicity; burned fees on failure |
| C. Jito `sendBundle` (current for bundles) | region URL, ≤5 txs, ≤1 slot | `x-jito-auth` UUID (limits) | 1 rps/IP/region default | low | tip ≥1,000 lamports, charged only on landing | `getInflightBundleStatuses` + `getBundleStatuses` | dropped when next leader is non-Jito; blockhash expiry waiting for leader; uncle-bandit tip theft |
| D. DBI (BAM direct) | allowlisted credential (JSON-RPC UUID or QUIC keypair) | separate creds | separate | lowest (no BE hop) | no min tip; ranked fee-per-CU | own RPC `getSignatureStatuses` (DBI IDs invisible in explorer) | paused during BAM maintenance; BAM-leader slots only |
| E. Helius Sender / preconf | Helius plan | API key | plan | low (regional) | preconf min tip 0.001 SOL | provider-reported + signature | provider-coupled |

Already supported in code: A (with Helius Sender regional override —
`arb-ingest/src/rpc_pool.rs:1446-1498`), C with regions + leader gate. D and E-preconf
are roadmap items gated on credentials.

### 1.8 QUIC/TPU outside Jito (SWQoS) — LIKELY (Triton, May 5 2026)

Agave 3.1: unstaked QUIC slots 500→2,000. Agave 4.0: no TPS throttle for staked senders
under normal load (95% safeguard). Agave 4.1: ~10k TPS/connection flow control.
Privileged SWQoS still requires a **staked validator relationship**
(`--staked-nodes-overrides` + `--rpc-send-transaction-tpu-peer`), i.e. not obtainable by
an unstaked RPC client. **Impact:** repo's SWQoS path through public RPC + optional
Helius Sender is the correct realistic ceiling; direct TPU/QUICManyToOne would require
own staked infrastructure (roadmap, not a defect).

### 1.9 Solana failed-tx fee policy & limits (CONFIRMED)

- Base (5,000 lamports/sig, 50% burn / 50% leader) + priority fees are charged and
  retained **even when the tx fails** (SIMD-0191 loading-failure fees and "success fees"
  remain proposals; SIMD-0290 in Agave 4.4.0 — stable Oct 28, 2026, not yet activated).
- Per-tx CU limit 1.4M; legacy/v0 packet budget 1,232 bytes.
- **Transaction v1 (officially SIMD-0296; BAM docs mislabel "SIMD-0385"):** 4,096-byte
  serialized ceiling, 64 inline accounts, 64 instructions, 12 signatures, no ALTs,
  message-header compute config, priority fee as **total lamports**; active mainnet
  epoch 1035 (Sept 15, 2026), `enable_tx_v1` slot 447,120,000.
  Source: solana.com/upgrades/larger-transaction-sizes.
- **classic Block Engine v1 support: NOT VERIFIED**; DBI accepts legacy/v0/v1 mixed.

**Impact:** repo is V1-only end-to-end on dispatch (verified PIPE-09: dispatch.rs
rejects V0; AdaptiveV1 compiler strips budget ixs to `Message.config` with absolute
lamports priority fee and re-maps `borrow_instruction_index`). The shipped `.env`
enables both V1 transport flags. Risk flagged by audit: Simulate-mode override in
transport selection means Live behavior differs per env — close with startup assertion
(PIPE-09).

### 1.10 Jupiter (submission-adjacent, CONFIRMED changelog)

Quote API V6 dead (Aug 2025); Lite API dead (Dec 31, 2025); Swap API V2 (Mar 2026);
tx-v1 opt-in Oct 2026 via `transactionVersion=1` (`/build`) or
`maxSupportedTransactionVersion=1` (`/order`). **Files:** repo's Jupiter wiring in
config (`jupiter_endpoint` in reps) — verify endpoints are `api.jup.ag` V2 lineage.

---

## 2. Kamino (klend) flash loan — verified specification

Canonical project implementation was audited in-code (see
`docs/audit/financial-calculation-audit.md`) and matches: fee =
`round(max(borrow × flash_loan_fee_sf, 2^60) / 2^60)` ties-away from zero over klend's
U68F60 `Fraction` (60 fractional bits), `FeeCalculation::Exclusive`, `sf == u64::MAX`
sentinel = flash loans disabled, `BorrowTooSmall` when raw fee_bits ≥ principal bits,
repay instruction argument must equal the **borrowed principal** (fee debited on top by
the program), start-flash-loan must precede all swaps and end-flash-loan must be last
business instruction. Detailed protocol-side spec verification is cross-attested by the
in-repo empirical verification of the reserve layout (mint@128, decimals@272,
status@4856, flash_loan_fee_sf@4904, supply vault@160, fee vault@192 — verified against
live mainnet accounts) plus the official SDK docs
(docs.rs/kamino_lending, github.com/Kamino-Finance/klend). Program ID
`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (CONFIRMED).

Instruction ordering diagram (as built by `TxBuilder::assemble_flash_arbitrage`,
verified in code by atomic_integrity):

```
[0] SetComputeUnitLimit            (ComputeBudget; stripped to Message.config in V1)
[1] SetComputeUnitPrice            ( ditto )
[2..] Create idempotent ATA(s)     (user WSOL + one per intermediate mint)
[borrow_idx] Klend flashBorrow     (principal = x*)
[swap legs in hop order]           (Pump buy wraps SOL via SystemTransfer+SyncNative;
                                    Pump sell unwraps via CloseAccount)
[repay_idx] Klend flashRepay       (argument == principal; borrow_instruction_index
                                    remapped when budget ixs stripped in V1 — adaptive.rs:436-452)
[..] Close transient ATAs          (rent reclaim)
[last] Jito tip SystemTransfer     (tip ix >0 guarded)
```

Verified by `atomic_integrity::verify_atomic_integrity` pre-signature
(`arb-tx/src/primitives/atomic_integrity.rs:120-260`).

---

## 3. Constraints table (summary, with classification)

| Spec | Value | Class | Source |
|---|---|---|---|
| Bundle max | 5 txs / 1 slot / all-or-nothing | CONFIRMED | docs.jito.wtf |
| Tip min (Block Engine) | 1,000 lamports | CONFIRMED | docs.jito.wtf |
| Tip accounts | 8 addresses above | CONFIRMED | docs.jito.wtf + runtime fetch pending |
| Auction | 50 ms ticks, tip/CU rank, lock-group grouping | CONFIRMED | docs.jito.wtf |
| In-flight states | Pending/Failed/Landed/Invalid (5 min) | CONFIRMED | docs.jito.wtf |
| getBundleStatuses history | ~300 slots (`searchTransactionHistory=false`) | CONFIRMED | docs.jito.wtf |
| Base fee split | 50% burn / 50% leader (SIMD-0096 analog for priority) | CONFIRMED/LIKELY | solana docs, crypto.news 2026-11-17 |
| Fee on failure | retained (no success-fee policy live) | CONFIRMED | 2026 sources |
| Tx v1 | 4,096 B, 64 accounts, 12 sigs, epoch 1035 (2026-09-15) | CONFIRMED | solana.com/upgrades |
| Per-tx CU | 1.4M max; default 200k | CONFIRMED | docs + QuickNode 2026 |
| DBI | :9090 JSON-RPC / :11228 QUIC, allowlist, no min tip | CONFIRMED | bam.dev 2026-11-06 |
| SWQoS 2026 | Agave 4.0/4.1 staked senders ~unthrottled; unstaked 2,000 slots | LIKELY | Triton 2026-05-05 |
| Kamino flash fee | U68F60 formula above | CONFIRMED (code + mainnet reserve reads) | klend SDK/repo |
|Uncled blocks | bundle may land on non-canonical path | CONFIRMED | docs.jito.wtf FAQ |
| jitodontfront | read-only account prefix rejects front-run bundles | CONFIRMED | docs.jito.wtf |

**Open items (UNKNOWN / NOT VERIFIED)** — to close in a later research pass:
1. Classic Block Engine `/api/v1/bundles` acceptance of transaction v1 payloads.
2. Exact UUID-holder rate-limit tiers (officially undocumented; ticket-based).
3. DBI QUIC tip-account targeting for non-zero-tip bundles (in jito-labs/bam-dbi source).
4. Any formal change to "paid during auction" tip behavior (no official announcement).

---

## 4. Procedure for applying this research (application-method column, consolidated)

1. **Jito auth** (PIPE-04): env `JITO_AUTH_UUID` → `x-jito-auth` header in
   `arb-land/src/lib.rs` send paths; startup `getTipAccounts` verification against
   `JITO_TIP_ACCOUNTS`, fail-closed in Live on mismatch.
2. **Realized PnL signed + provenance-gated** (MATH-01/02, PIPE-01/02/03): store schema
   widen; metrics consume on-chain actual; aggregation filter `status='landed'` +
   live-mode only; dashboard separates Expected vs Realized.
3. **Tip strategy floor** (verify-only): existing tip_oracle matches official guidance;
   add regression tests pinning the 1,000-lamport floor and the p75 default.
4. **DBI**: keep disabled; add transport stub gated on credential presence.
5. **V1 transport matrix**: startup fail-fast when Live + both V1 flags off (PIPE-09);
   correct docs.
6. **Config hard-caps** (PIPE-08): validated ceilings on `max_state_age_slots`,
   `max_tip_share_bps`, `max_inflight_bundles` in Live.
7. Each application carries its regression test listed in the corresponding audit doc.
