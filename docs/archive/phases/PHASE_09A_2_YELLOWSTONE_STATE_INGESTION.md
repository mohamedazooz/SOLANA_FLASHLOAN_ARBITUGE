# Phase 09A.2 — Solami Yellowstone gRPC + Update-Only Account State

**Status:** implementation started; not accepted for live use.  
**Execution mode:** Shadow-only.  
**Market-data transport policy:** no HTTP JSON-RPC. Solami Yellowstone gRPC is the authoritative account/slot stream. Three provider-neutral Solana PubSub WebSocket slots are reserved for auxiliary stream failover and are not yet wired to the runtime.

## Decisions applied

1. Removed HTTP JSON-RPC URLs from `.env.example`.
2. `RpcPool::from_env()` now returns an empty provider pool. Runtime code cannot silently activate HTTP JSON-RPC by finding a URL in the environment.
3. Supervisor no longer performs HTTP-catalog/GMGN/DexScreener discovery, HTTP-RPC registry polling, or RPC slot fallback.
4. A new Yellowstone gRPC client uses `yellowstone-grpc-client 16.0.1` and `yellowstone-grpc-proto 14.0.1`.
5. The gRPC subscription includes DEX program-owner account updates and slot updates. Explicit tracked-account changes resend the complete subscription request because Yellowstone subscription requests replace the active filter set.
6. Account payloads are stored individually with pubkey, owner, slot, write version, startup marker and raw bytes. Older `(slot, write_version)` updates are rejected.
7. Market pool updates mutate one map entry under a short write lock. They no longer clone the entire `MarketStateCache` for each pool update.
8. Strategy snapshots copy pool values once per strategy cycle, not on each incoming account update.
9. The graph builder now receives only fresh pools. If fewer than two fresh pools exist, it does not reintroduce stale pools.
10. Verified Kamino inputs fail closed until Kamino reserve/mint account decoders are promoted from the Yellowstone account store. No HTTP fallback is retained.

## Important limitations

- The gRPC stream currently journals raw account state; it does not yet promote every DEX pool, vault, TickArray, BinArray, Token-2022 mint or Kamino reserve into quote-ready state.
- The raw account store is ordered by slot/write-version but is not yet fork-aware. A bank-aware rollback/replacement model is required before treating processed updates as canonical.
- Dynamic address subscriptions are supported through `set_tracked_accounts`, but the pool/tick/bin dependency planner does not yet populate that set automatically.
- The three WebSocket slots exist in `.env.example` only. A provider-neutral Solana PubSub adapter with ordered failover, per-provider health, resubscription and gap handling remains a separate implementation item.
- Yellowstone gRPC is the authoritative pool/account stream. Solami Blur or other decoded trade-event WebSockets may corroborate discovery, but must never overwrite pool reserves or CLMM/DLMM state.
- `RpcPool::new()` and the HTTP JSON-RPC implementation remain in the source for isolated tests and not-yet-migrated legacy call sites. Runtime `from_env()` is empty. This is a staged disablement, not a claim that every HTTP client in the entire workspace has been deleted. Telegram, admin, webhook and Jito control/dispatch transports are separate concerns.

## Next work inside 09A.2

1. Fork-aware slot watermark and rollback/replacement handling.
2. Program-owner/discriminator/PDA validation for every account family.
3. Dynamic dependency planner: pool -> vaults -> Orca TickArrays / Raydium TickArrays + bitmap extension / Meteora BinArrays / Token-2022 mints.
4. Snapshot completeness barrier: quote state is promoted only when every required account is present and mutually coherent.
5. Per-protocol update decoder that updates only the affected pool and its dependent arrays.
6. Differential fixtures and replay tests before quote-engine promotion.

## Safety gates

- No stale cache fallback.
- No HTTP JSON-RPC runtime fallback.
- No quote from incomplete account coverage.
- No Live/Canary mode.
- No transaction broadcast until account-state parity, quote parity and transaction reconciliation are independently verified.
