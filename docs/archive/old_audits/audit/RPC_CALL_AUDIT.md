> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# RPC Call Audit — duplicate-call verification (checklist 654)

Date: 2026-11-17 · Branch `vibe/ws-provider-audit-0fcd5e`

Method: exhaustive inventory of every RPC call site reachable from the live
runtime path (`grep` over `arb-bot`, `arb-ingest`, `arb-land` source, excluding
tests/binaries), then classification of each by trigger condition and cache.

## Inventory (live runtime path)

| Call site | RPC method | Trigger | Duplicate risk |
|---|---|---|---|
| `supervisor/run.rs:712` | `getMultipleAccounts` | Startup bootstrap only (Kamino vault + payer settlement ATA) | None — runs once at startup |
| `supervisor/run.rs:830` | `getMultipleAccounts` | Kamino WSOL supply-vault hydration, **only when store snapshot is >100 slots stale** | None — freshness-gated |
| `supervisor/execution.rs:88` | `getMultipleAccounts` | `preexisting_intermediate_atas` — one batched call per dispatch attempt, mints deduped in-call (`!probes.iter().any(...)`) | None — batched + deduped |
| `supervisor/execution.rs:117` | `getLatestBlockhash` | Only when the 3-second-validity blockhash cache misses | None — cache removes per-send duplicates |
| `supervisor/execution.rs:206` | `getMultipleAccountsWithSlot` | Kamino raw fallback fetch — **only when both the stream store and the fallback store are stale** (fail-closed recovery with explicit provenance) | None — conditional recovery, not periodic |
| `supervisor/run.rs:131` | `getMultipleAccounts` | `fetch_payer_token_balances` — one call per 3-leg capture tick | None — one batched call |
| `arb-ingest/rpc_reconcile.rs` (4 sites) | various | Reconciliation paths — isolated from the live market-state path (see HTTP-isolation audit, Domain 5) | None on the opportunity path |
| `arb-ingest/helius_ws.rs` (3 sites) | auth/slot bootstraps | WS connection lifecycle only | None per tick |
| `arb-ingest/rpc_pool.rs` (6 sites) | pool internals | `RpcPool` itself — the single unified client; all callers go through it | Architecture eliminates independent HTTP clients |

## Single-source-of-truth guarantees already in code

1. `RpcPool` is the only HTTP JSON-RPC client construction point for runtime
   calls; `shared_dispatch_client` is process-wide for submissions (batch-4
   audit, `PERFORMANCE_HOT_PATH_REVIEW.md`).
2. Long-lived store reads (`kamino_available_flash_liquidity`, ATA existence in
   the dependency guard) read the in-memory `AccountSnapshotStore` — no RPC at all.
3. Every raw-RPC recovery write into a snapshot store stamps the **real** context
   slot, explicit `provider_id` (`ProviderId(0)` = HTTP), and `is_startup: false`.
   Known documented ordering token: RPC transports do not provide write versions,
   so recovery snapshots carry a fixed `write_version: 1` ordering token (real
   stream-write versions grow monotonically and order above it only at equal
   slots, which is the intended precedence). This is documented, not fabricated
   provenance — the slot and owner always come from the RPC response.

## Verdict

**No duplicate RPC call patterns remain on the live per-tick path.** All
remaining call sites are either startup-only, freshness-gated, batched, or
cached. Item 654 → `[x]` (VERIFIED via code-path audit; runtime duplication
counters live under `rpc_pool` per-provider OK/error stats observable on the
dashboard).
