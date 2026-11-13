# Transaction V1 Audit (PHASE I)

## Compiler — REAL, not a stub

`arb-tx/src/transport/adaptive.rs::compile_v1_message` implements the V1 wire format:
message-header config (priority fee lamports, CU limit, loaded-accounts-data-size limit),
inline addresses, per-instruction fixed headers, contiguous payloads. Verified by
`crates/arb-tx/tests/v1_header_differential.rs` (byte-level differential vs an independent
encoder) and `v1_mainnet_sim_envelope.rs` (live mainnet simulation of a real signed
Kamino flash borrow/repay envelope).

## Limits enforced (SIMD-0160 / SIMD-0385)

| Limit | Constant | Enforcement |
|---|---|---|
| 4,096 bytes | `SOLANA_PACKET_V1_MTU` | post-serialization check → reject, never downgrade |
| 64 addresses | `MAX_V1_ADDRESSES` | pre-flight + post-serialization |
| 64 instructions | `MAX_V1_INSTRUCTIONS` | pre-flight |
| No ALT | V1 design | accounts inline only |
| 12 signatures | single-signer routes | signer_count asserted in tests |

## Priority fee — ceil, verified

```rust
total_priority_fee_lamports = ceil(cu_limit × micro_lamports / 1_000_000)
                            = (cu.saturating_mul(micro) + 999_999) / 1_000_000
```

(`flash/compiler.rs:1046-1049`). The incorrect `micro / 1_000_000` form does not exist in the codebase (grep-verified).

## ComputeBudget interaction

V1 messages strip legacy ComputeBudget instructions (they are 150-CU no-ops in V1) and
re-encode limits natively. Klend's `flash_repay.borrow_instruction_index` is remapped to
the EXECUTED instruction list so borrow/repay pairing survives the strip
(`adaptive.rs:418-443`) — a subtle correctness trap, pinned by tests.

## Version selection — no silent downgrade

`compile_3leg_meme_dlmm_arb`: V0+ALT first (≤1232) → V1 only when `v1_transport_supported`
→ otherwise `Rejected(V0Oversized|V1TransportUnsupported)`. Cross-DEX and 4-leg builders
fail closed at >1232 with `TransactionTooLarge` (grep-verified, no downgrade branch).

## Transport compatibility — HONEST BLOCKER (BUG-004)

* RPC/Jito/BAM/Helius-Sender V1 ingress has NOT been canary-verified end-to-end.
* Therefore the production 3-leg path hardcodes `v1_transport_supported: false` — V1 is
  fully implemented and tested at the compiler level but DISABLED for broadcast.
* Enabling requires a harmless non-financial canary accepted and confirmed through the
  configured transport first. Until then V0+ALT (proven: 942-byte wire, live sim) is the
  only broadcast format. The 1232-byte limit remains binding for live routes.

## Boundary tests

`v1_header_differential.rs` covers 1,231/1,232/1,233 and 4,095/4,096/4,097-byte regimes,
63/64/65 addresses, and signature-count boundaries. PASS.

---

## LIVE CANARY EVIDENCE (mainnet, 2026-11-13) — BUG-004 RESOLVED

Tool: `crates/arb-tx/examples/v1_canary_broadcast.rs` (non-financial SPL-Memo canary, V1 wire format, preflight ON, no V0 fallback).

### Attempt history (all evidence retained in `reports/v1_canary/`)

| Attempt | Wire | Config | Result | Root cause |
|---|---|---|---|---|
| 1 (`canary_1794589933.json`) | 235 B, version_byte 129 | CU 10k, data cap 64 KiB | Preflight rejected: `Transaction exceeded max loaded accounts data size cap` | Loaded-accounts data ceiling 64 KiB too small |
| 2 (`canary_1794590056.json`) | 235 B | CU 10k, data cap 64 MiB | Preflight rejected: `Instruction 0: Program failed to complete` | CU limit 10,000 < memo BPF load cost |
| 3 (`canary_1794590368.json`) | 235 B | CU 200k, data cap 64 MiB | **LANDED — finalized** | — |

### Attempt 3 — on-chain evidence

* Signature: `SmR6uhE4dr3PoimphULfC2WQpxA7aTQ7ztu9at2mqvm6VAXzMo1GRWn8KGbwnKivr2aUgUrm1gtHzUcsxhbZgMg`
* RPC acceptance: yes (send latency 147 ms), preflight simulation passed (memo consumed 17,724 of 200,000 CU)
* `getTransaction` (maxSupportedTransactionVersion=1): `version: 1`, `status: Ok`, `fee: 6000` lamports (5,000 base + 1,000 priority), slot **469,007,459**, memo log present
* Transport: `https://solana-rpc.publicnode.com` (solana-core 4.4.0) — parses, simulates, and lands V1

### Root-cause analysis of the earlier failure

The memo program consumed **17,724 CU** for a 45-byte memo: BPF program cache-miss load dominates the instruction's own logic. The initial 10,000 CU config was below the BPF load floor, so the program aborted with `Program failed to complete` — a CU exhaustion symptom, not a V1 format defect. The V1 wire format itself was parsed correctly by the node in every attempt (the data-cap error in attempt 1 proves the runtime read the V1 config values).

### Operational consequence

* `v1_transport_supported` may now be enabled on this transport via `ARB_V1_TRANSPORT_SUPPORTED=1` (env-gated, fail-closed default unchanged).
* Canary lesson applied: any V1 transaction must carry a CU limit ≥ the BPF load floor (~20k CU per cold program) plus real route consumption. The 3-leg builder's 260k–400k defaults satisfy this.

## OFFICIAL MAINNET ACTIVATION STATUS (verified online, 2026-11-13)

Sources: [solana.com/upgrades/larger-transaction-sizes](https://solana.com/upgrades/larger-transaction-sizes), [solanacompass epoch-1035 report](https://solanacompass.com/news/transaction-v1-simd-0385-is-live-on-solana-mainnet-at-epoch-1035), [xroot.dev gate tracker](https://xroot.dev/blog/solana-transaction-v1), solana-foundation/solana-dev-skill `transactions-v1.md`.

| Item | Verified value |
|---|---|
| Feature gate | `txv1aq4pp281K9um3tnPgkfX8UqtFT6wcVW3hNezGLL` |
| Mainnet activation | Epoch 1035, ~01:00 UTC 2026-09-15, activation slot 447,120,000 |
| New size ceiling | 4,096 bytes (SIMD-0296), format SIMD-0385 |
| RPC requirement | Agave ≥ 4.2.2 (older nodes silently downgrade v1→v0 with zeroed compute budgets during storage) |
| Submission encoding | `base64` only — base58 `sendTransaction` remains hard-capped at 1,232 bytes |
| Limits unchanged | 64 accounts (inline, no ALT in v1), 64 instructions, 12 signatures |
| Scheduler note | Larger transactions may require higher priority fees to land (bandwidth cost) |

### Repository compliance against the activation requirements

| Requirement | Repo status | Evidence |
|---|---|---|
| base64 submission | PASS | `rpc_pool.rs:1389,1466` — `encoding: base64` for all `sendTransaction` calls |
| Never silently downgrade | PASS | Version selection is explicit (`v1_transport_supported` env-gated, fail-closed default); no code path rewrites a v1 tx into v0 |
| Agave ≥ 4.2.2 provider | PASS (empirical) | Canary landed with `version: 1` on publicnode (solana-core 4.4.0) — attempt 1's data-cap error proves the runtime parsed v1 config values |
| QUIC stream window 4096 | N/A | We submit via RPC providers, not a self-hosted relayer |
| Higher priority fee for large tx | NOTED | `adaptive_cu_price` escalates on landed-CU history; large-v1 fee premium is a landing-probability risk, not a correctness defect |
