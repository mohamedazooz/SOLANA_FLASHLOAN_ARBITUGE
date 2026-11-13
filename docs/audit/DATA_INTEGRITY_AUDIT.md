# PHASE 6 — Data Pipeline & State Integrity Audit

## Ingestion surfaces (all verified in code)

| Surface | Implementation | Write path |
|---|---|---|
| WebSocket (JSON-RPC) | helius_ws.rs, subscription budget `WS_MAX_ACCOUNT_SUBS` (default 48, ≤1024) | 3 fail-closed arms (756/807/842) |
| HTTP polling | registry.fetch_live_pool_states @750ms | run.rs:385/451 |
| Discovery refresh | 60–90s, Orca catalog + Raydium v3 + DexScreener (DLMM only) | registry.register |
| Yellowstone gRPC | IngestClient plumbing present | provider exposure NOT VERIFIED in current env |

## State integrity rules (verified)

- **Write-version ordering**: `next_pool_write_version()` monotonic; `apply_pool_update`
  rejects older writes (the `let _ =` sites are intentional older-write rejections).
- **Slot freshness**: `state.slot`, `last_update_slot`, `last_update_ms` updated on every
  accepted write; quote freshness consumed downstream by gates.
- **Fail-closed on incomplete hydration**:
  - WS DLMM arm: drops update until active BinArray present (helius_ws.rs:794 region).
  - WS Orca arm: drops update until tick arrays cover `tick_current` (helius_ws.rs:779).
  - `compute_dlmm_reserves`: None on missing active bin, zero depth, or price-band
    divergence > `DLMM_RESERVE_PRICE_TOLERANCE` (4x) — rewritten fail-closed in `228c330`.
- **No stale-cache fallback**: no code path re-serves stale reserves after provider
  failure; provider failure → pool dropped from graph → no opportunities built.
  Verified by exhaustive cache-writer enumeration (ARCHITECTURE_AUDIT.md).

## Provider health & rotation

- RpcPool: primary + fallbacks from `SOLANA_RPC_PRIMARY`, `SOLANA_RPC_FALLBACK_1/2`,
  `ALCHEMY_RPC_URL`; 429 → 60s cooldown + rotation (soak16: 3 rotations, zero impact).
- Batch-100 `getMultipleAccounts` (official limit) — replaced ~430 sequential
  fetches/cycle; 429s: 39,519 (soak4) → 83 (soak10) → 6–9 (soak15/16).
- WS reconnect: present; seed_initial_accounts batched with 3-attempt retry.

## Duplicate / out-of-order updates

- WS: write-version guard drops stale rewrites.
- HTTP: full-state refresh overwrites atomically per pool.

## Findings

| # | Finding | Severity | Status |
|---|---|---|---|
| D-1 | `pairs` degrades on some rediscovery cycles (163→91) | P3 | OPEN (decimals cache added; needs long soak) |
| D-2 | Yellowstone gRPC path unexercised in current env | P3 | DOCUMENTED |
| D-3 | `SLOT_DEAD` / processed-fork invalidation: handled via slot+write-version guards; no dedicated SLOT_DEAD state machine | P3 | DOCUMENTED (works for current single-ws topology) |
