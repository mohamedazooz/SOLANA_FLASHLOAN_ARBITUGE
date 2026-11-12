Comprehensive Audit Report: crates/arb-ingest, crates/arb-types, and crates/arb-constants
1. Token and Pool Discovery Architecture
1.1 Discovery Workflow & Entry Points (crates/arb-ingest/src/discovery.rs)
Primary Orchestrator: PoolDiscovery (crates/arb-ingest/src/discovery.rs:164-177), executed via PoolDiscovery::discover(&self, cfg: &DiscoveryConfig) -> Vec<DiscoveredPoolState> (lines 194–390).
Design Philosophy: As stated in lines 3–27, exhaustive getProgramAccounts sweeps are rejected because they timeout or get blocked by RPC providers (e.g. Orca has >15,000 pools). Instead, bounded external catalogs and targeted multi-venue searches are fetched.
Venues Discovered:
Orca Whirlpool (discovery.rs:548–625):
Fetches ORCA_CATALOG_URL (https://api.orca.so/v1/whirlpool/list).
Parses the whirlpools array, filters by tvl >= min_tvl_usd.
Rejects Token-2022 mints (tokenA.token2022 == true || tokenB.token2022 == true).
Extracts address, tokenA.mint, tokenB.mint, decimals, and lpFeeRate (multiplied by 10,000 to get bps).
Raydium AMM v4 (discovery.rs:439–544):
Queries Raydium API v3: RAYDIUM_POOL_LIST_URL (https://api-v3.raydium.io/pools/info/list?poolType=standard&poolSortField=liquidity&sortType=desc&pageSize={limit}&page=1).
Explicitly filters for programId == arb_constants::RAYDIUM_V4_PROGRAM_ID (675kPX9M...).
Rejects Token-2022 tokens (programId != TOKEN_PROGRAM_ID).
Vault Resolution: Because Raydium's API does not output vault addresses, discover_raydium batches RPC calls to fetch pool accounts via fetch_accounts_chunked (lines 513–515) and decodes coin_vault and pc_vault using decode_raydium_vaults (registry.rs:557–572, offsets 336..368 and 368..400).
Meteora DLMM (discovery.rs:630–772):
Hub Pairs: Iterates over cfg.hub_mints (WSOL, USDC, USDT, plus 25+ top tokens: POPCAT, WIF, JUP, JTO, BONK, BOME, RENDER, RAY, ME, TRUMP, FARTCOIN, AI16Z, PYTH, DRIFT, PENGU, mSOL, bSOL, JitoSOL, GRASS, KMNO, ORCA, FWOG, MOODENG; lines 93–148). Queries DEXSCREENER_TOKEN_PAIRS (https://api.dexscreener.com/token-pairs/v1/solana/{hub}).
Filters dexId == "meteora" and requires labels array containing "DLMM" (lines 647–651) to avoid confusing Meteora Dynamic AMM (DYN) with LbPair DLMM.
Search Endpoint: Calls discover_meteora_search querying DEXSCREENER_SEARCH_URL (https://api.dexscreener.com/latest/dex/search?q={query}) with queries "meteora", "SOL%20meteora", "USDC%20meteora" (lines 703–772).
Pump.fun Bonding Curves (discovery.rs:253–317):
Direct API: Invokes PumpFunDiscoveryClient::new().fetch_top_coins(limit) (pumpfun_discovery.rs:80–141), taking top N active curves.
Cross-Venue Pairing: For any discovered pool whose base or quote mint ends with "pump" (lines 278–284), derives the bonding curve PDA using seeds [b"bonding-curve", meme_mint] under PUMP_FUN_PROGRAM_ID (6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P) and pairs it with WSOL.
Mint Decimals & Token Safety Filter (discovery.rs:327–370 and 777–841):
Calls resolve_mint_decimals(&missing) over RPC in batches of 100 (RPC_BATCH).
Reads byte offset 44 of the mint account.
Strict Safety Gate (lines 801–836):
Skips non-SPL mints (acc.owner != TOKEN_PROGRAM_ID).
For non-hub tokens, unpacks mint authority tag (offset 0..4) and freeze authority tag (offset 46..50). If either != 0, the meme token is dropped to prevent honeypots/freezes.
Drops all pools with unresolved or Token-2022 decimals (line 363).
Watch-Set Selection (discovery.rs:851–953):
select_watch_set_per_venue(found, max_per_venue, max_pools): Selects up to max_per_venue (default 33) pools per DEX venue, ranked by TVL. This prevents deep venues (like Orca) from crowding out cross-venue pairs.
select_dual_30_watch_set(found): Selects exactly up to 30 Pump.fun curves + 30 multi-DEX pools (lines 925–953).
register_discovered(registry, pools): Registers DiscoveredPoolState records into MultiDexRegistry (lines 956–973).
1.2 Dedicated Discovery Clients
Client / Module	Primary File & Lines	Endpoints Used	Logic & Invariants	Wired in Bot?
PumpFunDiscoveryClient	crates/arb-ingest/src/pumpfun_discovery.rs:44–228	https://frontend-api.pump.fun/coins
https://pumpdev.io/api	Concurrent queries for recently traded + top mcap coins via tokio::join!. Derives PDA [b"bonding-curve", mint]. Scores TVL; prioritizes uncompleted curves between 20% and 98% graduation progress.	Yes, called in discovery.rs:255 and arb-mcp/src/tools.rs:64.
DexScreenerClient	crates/arb-ingest/src/dexscreener.rs:70–218	https://api.dexscreener.com/latest/dex/tokens/{mint}	Filters out single-pool tokens (pairs.len() < 2). Maps dexId (raydium, raydium-clmm, orca, meteora, pumpfun, pumpswap). Requires liquidity_usd >= min_liq_usd ($25k).	No (GAP). Standalone only. discovery.rs makes its own direct reqwest calls to DexScreener URLs instead of using DexScreenerClient.
GmgnClient	crates/arb-ingest/src/gmgn.rs:30–299	https://gmgn.ai/defi/quotation/v1/rank/sol/swaps/1h
https://api.dexscreener.com/token-boosts/top/v1	Scrapes 1h swaps rank with browser header spoofing. Employs an in-memory 60s TTL cache (snapshot: Arc<RwLock<Option<(Instant, Vec<TrendingToken>)>>>) to prevent HTTP 429/403. Fallback to DexScreener top boosts.	No (GAP). Only called in standalone CLI crates/arb-bot/src/bin/live_scanner.rs:377. Never run by supervisor.rs.
BitqueryClient & BitqueryWsClient	crates/arb-ingest/src/bitquery.rs:46–363, 378–684	https://oauth2.bitquery.io/oauth2/token
https://streaming.bitquery.io/graphql
wss://streaming.bitquery.io/graphql	OAuth2 Client Credentials token exchange with auto-refresh on 401. GraphQL polling for DEXTrades. WebSocket client using graphql-transport-ws protocol streaming parsed BitqueryTradeEntry records.	No (GAP). Completely unwired from the live trading bot; only used in integration test tests/bitquery_ws_tests.rs.
2. Live Streaming, Subscriptions, and Byte Decoding
2.1 Yellowstone Geyser gRPC (crates/arb-ingest/src/yellowstone.rs)
Subscription Constructor: build_request (lines 23–72):
accounts.dex_program_accounts: Subscribes to accounts owned by DEX program IDs (lines 93–106):
Raydium V4 (675kPX9M...)
Raydium CPMM (CPMMoo8L...)
Raydium CLMM (CAMMCzo5...)
Orca Whirlpool (whirLbMi...)
Meteora DLMM (LBUZKhRx...)
Pump.fun (6EF8rrec...)
PumpSwap (pAMMBay6...)
Pump Fees (pfeeUxB6...)
Kamino klend (KLend2g3...)
accounts.tracked_accounts: Explicit pubkey filter updated dynamically via self.tracked_accounts_tx watch channel (lines 40–51, 170–176). Capped at 4,096 accounts.
slots.slot_watermark: Commitment Processed, interslot_updates: true (lines 63–69).
entry.entry_parent_reconciliation: Optional via env YELLOWSTONE_ENTRY_PARENT_UPDATES (lines 53–62).
Stream Processing Loop (yellowstone.rs:168–226):
UpdateOneof::Slot(slot_update): Updates slot tracking with record_slot_update_with_bank(slot, parent, status, bank_id). If status == SLOT_DEAD (6), invalidates the dead slot and all its descendants from AccountSnapshotStore, MarketStateCache, and BoundedDedupeBuffer (lib.rs:573–613).
UpdateOneof::EntryUpdateParent(parent_update): Clears snapshots and pool states matching (slot, cleared_bank_id) (lib.rs:625–639).
UpdateOneof::Account(account_update):
Extracts pubkey: [u8; 32] and owner: [u8; 32].
Updates slot watermark and checks slot_is_dead(slot) / bank_is_dead(slot, bank_id).
Appends raw AccountSnapshot into self.account_store (lib.rs:118–160).
CRITICAL ARCHITECTURAL GAP:
yellowstone.rs never decodes incoming AMM account bytes (Raydium, Orca, Meteora) into PoolState!
apply_raw_account exists in crates/arb-ingest/src/lib.rs:663–712, but is dead code (0 call sites across the workspace).
Yellowstone account events only update account_store for PumpSwap (via refresh_pump_swap_states) and for economic inputs (WSOL mint, Kamino reserve). Raydium, Orca, and Meteora pool states are NOT updated from Yellowstone!
2.2 Helius & Multi-Provider WebSocket (crates/arb-ingest/src/helius_ws.rs)
Connection & Endpoints: Collects endpoints from SOLANA_WS_1..10_URL, FAST_WEBSOCKET, HELIUS_WS_URL, SOLAMI_WS_URL, HELIUS_API_KEY (helius_ws.rs:52–95).
Initial Account Seeding (helius_ws.rs:130–212):
Before connecting WebSocket, sends HTTP getMultipleAccounts for all tracked accounts (WSOL mint, Kamino WSOL reserve, etc.) and pre-populates AccountSnapshotStore with is_startup: true, provider_id: ProviderId(2).
WebSocket Subscriptions (helius_ws.rs:251–284):
Sends slotSubscribe (ID 1, commitment: "processed").
Sends accountSubscribe for each tracked account (IDs 100 + i, base64 encoding, commitment: "processed"). Maintains a mapping of request_id -> pubkey and subscription_id -> pubkey (req_to_account, sub_to_account).
Incoming Frame Handling (helius_ws.rs:386–466):
slotNotification: Extracts slot & parent; updates YellowstoneClient::record_slot_update and marks stream connected.
accountNotification: Decodes base64 payload from params.result.value.data[0], builds AccountSnapshot with provider_id: ProviderId(2), bank_id: None, and calls yellowstone.account_store().apply(snap).
Failover Watchdog: 15-second frame timeout (helius_ws.rs:293–300). If stalled or if slotSubscribe errors out, automatically rotates to the next WebSocket endpoint.
2.3 Solami WebSocket Client (crates/arb-ingest/src/solami_stream.rs)
Subscribes to wss://ws.solami.dev/data/subscribe?chain=solana&api_key=... (solami_stream.rs:149–155).
Dispatches SolamiFilterMessage (solami_stream.rs:183–197):
action: "subscribe"
event_types: ["swap", "liquidity", "token_create", "pool_create", "graduating", "graduated"]
commitment: "processed" (shred latency)
Decodes incoming JSON text into SolamiStreamEvent (solami_stream.rs:79–96, parsing lines 207–216).
CRITICAL GAP: COMPLETELY UNWIRED. Never instantiated in supervisor.rs or arb-bot.
2.4 Pumpdev WebSocket Client (crates/arb-ingest/src/pumpdev_stream.rs)
Connects to wss://pumpdev.io/ws (pumpdev_stream.rs:17).
Sends {"method": "subscribeNewToken"} (pumpdev_stream.rs:81–85).
Decodes PumpDevEvent (pumpdev_stream.rs:21–34) containing mint, sol_amount, token_amount, is_buy, v_sol_in_bonding_curve, v_tokens_in_bonding_curve, market_cap_sol.
Delivers events over a broadcast channel (broadcast::Sender<PumpDevEvent>, line 51).
CRITICAL GAP: COMPLETELY UNWIRED. Never called anywhere in arb-bot.
2.5 Provider Supervisor (crates/arb-ingest/src/provider_supervisor.rs)
State Machine: Tracks ProviderHealth (Healthy, Degraded, OpenCircuit, Disabled) with priority sorting (provider_supervisor.rs:18–42).
Policy Invariants (SupervisorPolicy, lines 44–61):
failure_threshold: 3 consecutive failures trips circuit to OpenCircuit.
cooldown: 2 seconds before half-open probe.
max_slot_lag: 8 slots from independent cluster slot.
stale_after: 3 seconds without success marks provider stale.
Dependency Stamp Validation (validate_dependency_group, lines 164–198):
Validates DependencyStamp slice (provider, slot, bank_id, write_version, is_startup).
Strict invariants:
Rejects startup snapshots (CoherenceError::StartupSnapshot).
Rejects mixed providers (CoherenceError::MixedProviders).
Rejects future slots (s.slot > target_slot).
Rejects conflicting bank IDs for the same slot (CoherenceError::MixedBanks).
Rejects slot skew > max_slot_skew (CoherenceError::SlotSkew).
CRITICAL GAP: The ProviderSupervisor struct is never instantiated in the live runtime. YellowstoneClient hardcodes provider_id = ProviderId(1) (lib.rs:393) and HeliusWsClient hardcodes provider_id = ProviderId(2) (helius_ws.rs:193, 451).
3. State Storage, Tracking, and Reconciliation
3.1 MultiDexRegistry & registry.rs
State Storage: MultiDexRegistry (registry.rs:50–54) stores Vec<RegisteredPool>.
Pre-loaded Static Pools (registry.rs:58–154):
Raydium V4 SOL-USDC (58oQChx4...)
Orca Whirlpool SOL-USDC (4 fee tiers: 5 bps Czfq3xZZ..., 1 bps 83v8iPyZ..., 2 bps FpCMFDFG..., 8 bps 7qbRF6Ys...)
Meteora DLMM SOL-USDC (bin step 4 5rCf1DM8...)
Raydium CPMM SOL-USDC (1 bps tier 79RbHpxo...)
Live State Fetching (MultiDexRegistry::fetch_live_pool_states, registry.rs:179–554):
Batches vault balance keys and pool account keys concurrently in chunks of 100 via fetch_accounts_chunked_with_slot (lines 202–205).
Meteora DLMM Resolution (lines 217–225, 249–263, 453–510):
Decodes LbPair header using decode_dlmm_state (offset 76..80 active_bin_id, offset 80..82 bin_step_bps, static/variable fee params).
Derives 3 BinArray PDAs (index - 1, index, index + 1) using floor divisor 70 (METEORA_BINS_PER_BIN_ARRAY = 70, lines 1134–1155).
Fetches BinArray accounts, decodes bins (decode_bin_array, header 56 bytes, 70 bins of 144 bytes each with amount_x @0, amount_y @8).
Derives exact active bin price in Q64.64 via arb_math::dlmm::get_price_from_id(active_bin_id, bin_step_bps).
Linearizes active-bin capacity: maps base/quote mints against LbPair.token_x_mint (@88..120) to compute (reserve_in, reserve_out).
Orca Whirlpool CLMM Resolution (lines 226–236, 265–301, 440–451):
Decodes sqrt_price (Q64.64 @65..81), liquidity (@49..65), tick_current (@81..85), tick_spacing (@41..43), fee_rate (@45..47) via decode_clmm_state.
Derives 3 tick array PDAs covering tick_current (current, previous, next; 88 ticks/array; lines 827–847).
Computes instantaneous virtual reserves: $x = L/\sqrt{P}$, $y = L \cdot \sqrt{P}$ via ruint::aliases::U256 (lines 442–450).
Raydium V4 & CPMM Resolution (lines 312–326):
Reads base vault and quote vault balances from SPL Token account data offset 64..72 via decode_token_amount (lines 1237–1255).
Decodes fee: Raydium V4 via decode_raydium_fee_bps (numerator @176..184, denominator @184..192); Raydium CPMM via decode_raydium_cpmm_fee_bps (@290..298).
Live Polling in the Bot:
In crates/arb-bot/src/supervisor/run.rs:117–142, a dedicated worker runs every 750ms calling reg.fetch_live_pool_states(Slot(s)) over HTTP JSON-RPC and applying them to MarketStateCache. This HTTP polling loop is what actually updates pool reserves in the bot.
3.2 PumpSwap Coherent Joiner (crates/arb-ingest/src/pump_state.rs)
join_pump_swap_state(store, pool_key, fee_config_key) (pump_state.rs:360–510):
Atomically joins 7 on-chain accounts from AccountSnapshotStore:
Pool Account: Decodes PumpSwapPoolHeader. Verifies PDA seeds [b"pool", index, creator, base_mint, quote_mint].
GlobalConfig: Verifies PDA [b"global_config"]. Decodes disable_flags (bit 3 = buy disabled, bit 4 = sell disabled; creator_fee_configurable @940).
FeeConfig: Verifies PDA [b"fee_config", pump_swap_program] under PUMP_FEES_PROGRAM_ID. Verifies Anchor discriminator [143, 52, 146, 187, 219, 123, 76, 155].
Base & Quote Vaults: Must have slot == pool.slot (strict zero-slot skew, lines 433–438). Verifies canonical ATAs [pool, token_program, mint]. Verifies account state initialized (1) and not frozen. Decodes balance @64..72.
Base & Quote Mints: Snapshot slot <= pool.slot. Decodes decimals (@44) and supply (@36..44). Rejects any Token-2022 risky extensions.
select_fee: Decodes PumpFeesFeeConfig tier ladder (flat fees, standard tiers, stable tiers for USDC, exotic flat fees). Selects fee based on market cap and converts to PPM (pump_state.rs:142–168, 301–346).
refresh_pump_swap_states: Scans store.snapshots_by_owner(&pump_swap_program). Subscribes all vault/mint/fee accounts in Yellowstone. Runs join_pump_swap_state. Converts fee to conservative integer BPS and pushes PoolState into MarketStateCache via apply_pool_update (pump_state.rs:525–689).
Executed every tick in supervisor/run.rs:267–278.
3.3 Dependency Manifest & Quote Readiness (dependency_manifest.rs & quote_readiness.rs)
dependency_manifest.rs:
AccountRole enum (lines 10–31): PoolState, VaultA, VaultB, MintA, MintB, FeeConfig, TickArray, BinArray, BitmapExtension, GlobalConfig, BondingCurve, CreatorVault, etc.
resolve_protocol_dependencies(pool_id, protocol, input) (lines 246–369): Validates that all structural and dynamic accounts needed for a protocol are registered before quoting.
route_requirements: Merges multi-hop route dependencies, ensures owner consistency, and deduplicates accounts (lines 156–189).
rpc_snapshot_requirements: Enforces the 100-account RPC limit (MAX_RPC_SNAPSHOT_ACCOUNTS = 100, lines 193–205).
quote_readiness.rs:
build_quote_dependency_set: Fail-closed gate assembling an executable dependency set from AccountSnapshotStore (lines 41–108).
Fails if:
Stream disconnected (StreamDisconnected)
Required account missing or owner mismatch (MissingAccount, OwnerMismatch)
Account is from startup/bootstrap (StartupSnapshot)
Provider ID or generation mismatch (WrongProvider, WrongGeneration)
Missing bank ID (MissingBankIdentity)
Slot or bank is dead (DeadSlot)
Account slot > target slot or older than max_age_slots (FutureSlot, StaleSlot)
Group coherence validation fails (Incoherent(CoherenceError): mixed providers, mixed banks, slot skew > max_slot_skew).
validate_execution_readiness: Lightweight fire-time check called in crates/arb-bot/src/supervisor/dispatch.rs:381–388 immediately before submitting transaction bundle (checks stream connected, generation unchanged, observed slot not lagging quote slot, quote not expired).
CRITICAL GAP: While validate_execution_readiness is called before dispatch, build_quote_dependency_set and quote_route_guarded (arb-engine/src/guarded.rs:54) are never called in the main trading loop! The bot prices routes directly off MarketStateCache without assembling the verified snapshot dependency set.
3.4 RPC Reconciliation & Slot Integrity (rpc_reconcile.rs & slot_integrity.rs)
rpc_reconcile.rs:
RpcSnapshotClient::fetch_and_publish: HTTP fallback to bootstrap/reconcile up to 100 accounts atomically (rpc_reconcile.rs:86–212).
Uses minContextSlot in getMultipleAccounts.
Verifies all expected owners and base64 payloads.
Publishes batch atomically into AccountSnapshotStore::apply_batch with is_startup: true, provider_id: ProviderId(0).
slot_integrity.rs:
SlotIntegrity (slot_integrity.rs:22–157): Tracks slot graph up to 8,192 slots.
Reconstructs parent-child slot tree (parents: BTreeMap<u64, u64>).
On SLOT_DEAD (6), cascades down the fork tree, identifying every child slot built on the dead fork and returning SlotUpdateOutcome::Dead { invalidated_slots }.
In YellowstoneClient::record_slot_update_with_bank (lib.rs:573–613), dead slots purge:
Accounts from account_store (account_store.invalidate_slots(&invalidated)).
Accounts from cleared banks (account_store.invalidate_bank(dead_slot, dead_bank)).
Pools from cache.pools matching dead slots (pools.retain(|_, pool| !invalidated.contains(&pool.slot.0))).
Dedupe ring buffer entries for dead slots.
SLOT_CREATED_BANK: When a re-created bank arrives for a previously dead slot with a new candidate bank_id, SlotIntegrity and AccountSnapshotStore reopen the slot for the new bank while keeping the old bank ID tombstoned (slot_integrity.rs:83–90, lib.rs:249–254).
4. DEX Protocols: Wired vs. Ignored / Unimplemented / Gaps

The DexProtocol enum (crates/arb-types/src/lib.rs:219–234) contains 7 variants:

rust
pub enum DexProtocol {
    RaydiumV4,
    RaydiumCpmm,
    RaydiumClmm,
    OrcaWhirlpool,
    MeteoraDlmm,
    PumpFun,
    PumpSwap,
}

Detailed audit of each protocol across arb-ingest and supervisor:

DEX Protocol	Constants / Program ID	Ingest / Discovery	Registry & Live Decoding	Live Status	Gaps & Broken Wiring
RaydiumV4	arb_constants::RAYDIUM_V4_PROGRAM_ID (675kPX9M...)	discovery.rs:439–544 (discover_raydium)	registry.rs:65–76, decode_raydium_vaults, decode_raydium_fee_bps, fetch_live_pool_states	FULLY WIRED	None. Static pool seeded in mainnet_default; dynamic pools discovered via Raydium API; live reserves polled every 750ms.
RaydiumCpmm	arb_constants::RAYDIUM_CPMM_PROGRAM_ID (CPMMoo8L...)	Missing in discovery	registry.rs:139–151, decode_raydium_cpmm_vaults, decode_raydium_cpmm_fee_bps, fetch_live_pool_states	PARTIALLY WIRED	1. Discovery blind: discover_raydium() hardcodes programId == RAYDIUM_V4_PROGRAM_ID and poolType=standard. It never queries CPMM pools.
2. dexscreener.rs:168–176 drops "raydium-cpmm" / "cpmm".
3. Only the single static SOL-USDC pool (79RbHpxo...) in mainnet_default receives live updates.
RaydiumClmm	arb_constants::RAYDIUM_CLMM_PROGRAM_ID (CAMMCzo5...)	Subscribed in Yellowstone owner_programs (yellowstone.rs:96)	Stubbed / Ignored in registry.rs	UNIMPLEMENTED	1. Fee decoding returns None: In registry.rs:374–398 and 1227, RaydiumClmm has no fee decoder branch and returns None. Lines 393–398 skip any pool whose fee is None.
2. decode_clmm_state (registry.rs:353) is only called for OrcaWhirlpool, not RaydiumClmm.
3. clmm_tick_array_pdas is not derived for RaydiumClmm.
4. decode_pool_info (registry.rs:617–750) falls through to _ => None. Any RaydiumClmm pool is immediately dropped.
OrcaWhirlpool	arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID (whirLbMi...)	discovery.rs:548–625 (discover_orca)	registry.rs:77–126, decode_orca_fee_bps, decode_clmm_state, clmm_tick_array_pdas, fetch_live_pool_states	FULLY WIRED	None. 4 static fee tiers seeded; full Orca catalog discovered; 3 tick arrays fetched per tick; virtual reserves ($L/\sqrt{P}, L\sqrt{P}$) computed.
MeteoraDlmm	arb_constants::METEORA_DLMM_PROGRAM_ID (LBUZKhRx...)	discovery.rs:630–772 (DexScreener hubs & search)	registry.rs:127–138, decode_dlmm_state, dlmm_bin_array_pdas, decode_bin_array, fetch_live_pool_states	FULLY WIRED	None. Static pool seeded; DexScreener DLMM pairs discovered; 3 BinArrays derived (divisor 70) and fetched; active-bin capacity linearized.
PumpFun	arb_constants::PUMP_FUN_PROGRAM_ID (6EF8rrec...)	discovery.rs:253–317, pumpfun_discovery.rs:44–228	Unconditionally skipped in registry.rs	BROKEN / DEAD END	1. Explicit skip in registry.rs:375: (DexProtocol::PumpFun, _) => None for onchain_fee_bps. Lines 393–398 log skipping (fail closed) and drop the pool.
2. resolve_fee_bps (registry.rs:1229) returns None.
3. decode_pool_info (registry.rs:749) returns None.
4. No decoder in pump_state.rs exists for PumpFun (only PumpSwap).
Result: Even though Pump.fun bonding curves are discovered and registered, fetch_live_pool_states drops 100% of them. They never reach MarketStateCache.
PumpSwap	arb_constants::PUMP_SWAP_PROGRAM_ID (pAMMBay6...)	Missing in discovery	Handled in pump_state.rs:525–689 (refresh_pump_swap_states)	STREAM-WIRED ONLY	1. Zero discovery: discovery.rs has no catalog query or search for PumpSwap pools.
2. MultiDexRegistry::mainnet_default has 0 PumpSwap pools.
3. registry.rs:343, 376, 426 explicitly stubs PumpSwap to None / (0, 0).
4. Working Path: pump_state.rs:525 scans Yellowstone account_store by owner, joins 7 accounts, decodes fee schedule, and calls apply_pool_update every tick (supervisor/run.rs:267). Only functions if Yellowstone streams PUMP_SWAP_PROGRAM_ID accounts.
5. Critical Systemic Gaps & Architectural Disconnects
Yellowstone gRPC Stream vs. 750ms HTTP Polling Disconnect:
YellowstoneClient::start_stream (yellowstone.rs:194–221) journals incoming accounts into account_store: AccountSnapshotStore. It never decodes them into PoolState (the method apply_raw_account in lib.rs:663 is dead code).
In reality, the bot's live market reserves for Raydium, Orca, and Meteora are updated by an HTTP JSON-RPC polling task running every 750ms (supervisor/run.rs:117–142) using MultiDexRegistry::fetch_live_pool_states.
Impact: The bot is operating on a 750ms HTTP poll cadence rather than a sub-millisecond Yellowstone Geyser push stream.
Pump.fun Bonding Curves Cannot Enter MarketStateCache:
discovery.rs actively discovers Pump.fun curves via PumpFunDiscoveryClient and pairs them into MultiDexRegistry.
However, registry.rs:375 hardcodes (DexProtocol::PumpFun, _) => None for onchain_fee_bps. Lines 393–398 reject any pool with undecodable fee.
Impact: Pump.fun bonding curves are completely discarded each tick; 0 Pump.fun pools ever reach MarketStateCache or the quote graph.
Four Independent Ingestion Streams Are Completely Unused:
SolamiStreamClient (crates/arb-ingest/src/solami_stream.rs)
PumpDevWebSocketClient (crates/arb-ingest/src/pumpdev_stream.rs)
BitqueryClient & BitqueryWsClient (crates/arb-ingest/src/bitquery.rs)
DexScreenerClient (crates/arb-ingest/src/dexscreener.rs)
Impact: These modules are fully implemented with unit tests, but are never instantiated or wired into the bot supervisor.
Guarded Route Quoting (quote_route_guarded) Is Bypassed:
arb-engine/src/guarded.rs defines quote_route_guarded, requiring quote_dependencies_for_route, zero-slot-skew validation, and live generation matching.
The bot's main loop (supervisor/run.rs:312–405) instead reads raw snapshots from MarketStateCache::snapshot_pools() and runs pure Dijkstra/Bellman-Ford without validating the dependency snapshot set.
Furthermore, quote_dependencies strictly rejects accounts with is_startup: true (QuoteReadinessError::StartupSnapshot). Because fetch_live_pool_states and seed_initial_accounts populate accounts with is_startup: true, invoking quote_route_guarded would currently fail closed on all HTTP-seeded pools.
Raydium CLMM is Completely Stubbed:
While RAYDIUM_CLMM_PROGRAM_ID is defined and subscribed in Yellowstone, registry.rs lacks pool account decoding, tick array fetching, and fee decoding for RaydiumClmm. Any Raydium CLMM pool is dropped.