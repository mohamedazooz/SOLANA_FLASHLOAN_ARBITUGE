/**
 * Birdeye API Client Template
 *
 * A production-ready client for the Birdeye multi-chain DeFi data API.
 * Copy this file to your project and customize as needed.
 *
 * Features:
 * - Multi-chain support (Solana, Ethereum, BSC, Base, Arbitrum, and more)
 * - Built-in rate limiting and exponential backoff for 429/5xx errors
 * - Typed response interfaces for all major endpoints
 * - Custom error class with status code
 * - Sub-clients for organized domain access
 *
 * Usage:
 * 1. Copy this file to your project
 * 2. Set the BIRDEYE_API_KEY environment variable
 * 3. Import and use the client:
 *
 * ```typescript
 * import { BirdeyeClient } from './birdeye-client';
 *
 * const client = BirdeyeClient.create();                    // reads BIRDEYE_API_KEY
 * const price  = await client.price.getPrice('So111...');
 * const tokens = await client.market.getTrending();
 * const wallet = await client.wallet.getNetWorth('ABC...');
 * ```
 */

// ============================================================================
// NODE PROCESS SHIM  (template has no package.json — avoids @types/node dep)
// ============================================================================

declare const process: { env: Record<string, string | undefined> };

// ============================================================================
// TYPES
// ============================================================================

export interface BirdeyeConfig {
    apiKey: string;
    chain?: string;
    /**
     * Account rate limit in requests/minute. Match your Birdeye plan:
     *   Standard: 60 (1 rps)  •  Lite/Starter: 900 (15 rps)
     *   Premium:  1000         •  Business:      1500
     * Env fallback: BIRDEYE_RATE_LIMIT_RPM. Default: 60 (Standard).
     */
    rateLimitRpm?: number;
}

export interface TokenPrice {
    value: number;
    updateUnixTime: number;
    updateHumanTime: string;
    priceChange24h: number;
}

/**
 * Response from GET /defi/token_overview.
 * Field names mirror the actual API (camelCase v1-style).
 * 24h volume is `v24hUSD` — NOT `volume24h`.
 */
export interface TokenOverview {
    address: string;
    name: string;
    symbol: string;
    decimals: number;
    price: number;
    marketCap: number;
    fdv: number;
    liquidity: number;
    /** 24h volume in USD */
    v24hUSD: number;
    /** 24h volume in base token units */
    v24h: number;
    v24hChangePercent: number;
    holder: number;
    priceChange1mPercent: number;
    priceChange5mPercent: number;
    priceChange1hPercent: number;
    priceChange4hPercent: number;
    priceChange24hPercent: number;
    trade24h: number;
    uniqueWallet24h: number;
}

export interface TokenMetadata {
    address: string;
    name: string;
    symbol: string;
    decimals: number;
    logoURI: string;
    extensions: Record<string, string>;
}

/**
 * One OHLCV candle.
 * ⚠️ Field name for the timestamp differs by endpoint:
 *   /defi/v3/ohlcv  → `unix_time` (snake_case)
 *   /defi/ohlcv     → `unixTime`  (camelCase — V1 legacy)
 * Both are included as optional for compatibility. Use the one for your endpoint.
 */
export interface OHLCVCandle {
    o: number;  // open
    h: number;  // high
    l: number;  // low
    c: number;  // close
    v: number;  // volume (base token units)
    /** V3 `/defi/v3/ohlcv` returns `unix_time` */
    unix_time?: number;
    /** V1 `/defi/ohlcv` returns `unixTime` */
    unixTime?: number;
    type: string;
    address?: string;
    currency?: string;
    /** V3 only — volume denominated in USD */
    v_usd?: number;
}

/** Response item from GET /defi/token_trending. */
export interface TrendingToken {
    address: string;
    name: string;
    symbol: string;
    decimals: number;
    logoURI: string;
    price: number;
    liquidity: number;
    /** 24h volume in USD (note: `volume24hUSD`, not `volumeUSD`) */
    volume24hUSD: number;
    volume24hChangePercent: number;
    rank: number;
}

/**
 * One trade from GET /defi/v3/token/txs (token trade feed).
 * ⚠️ The v3 endpoint returns SNAKE_CASE fields — verified against Birdeye docs.
 * Note: this differs from wallet tx history (/v1/wallet/tx_list).
 */
export interface TradeTransaction {
    tx_hash: string;
    /** Unix seconds — use: new Date(tx.block_unix_time * 1000) */
    block_unix_time: number;
    block_number: number;
    /** "buy" | "sell" — same as `side` for swaps */
    tx_type: 'buy' | 'sell' | string;
    /** "buy" | "sell" — trade direction from token-address perspective */
    side: 'buy' | 'sell' | string;
    source: string;
    owner: string;
    /** Trade size in USD (snake_case: `volume_usd`, not `volumeUSD`) */
    volume_usd: number;
    /** Trade size in token units */
    volume: number;
    price_pair: number;
    /** Token being spent (object with symbol/address/decimals/amount/ui_amount) */
    from: { symbol: string; address: string; decimals: number; price: number; amount: string; ui_amount: number; ui_change_amount: number };
    /** Token being received */
    to:   { symbol: string; address: string; decimals: number; price: number; amount: string; ui_amount: number; ui_change_amount: number };
    pool_id?: string;
}

/**
 * One transaction from GET /v1/wallet/tx_list (wallet history).
 *
 * ⚠️ This endpoint has DIFFERENT field shapes than /defi/v3/token/txs:
 *   - blockTime  → ISO string (e.g. "2026-04-13T06:10:38+00:00"), NOT unix number
 *   - from / to  → plain wallet address strings, NOT objects
 *   - token info → inside balanceChange[] array
 *
 * Parse time: new Date(tx.blockTime).getTime()  (not tx.blockTime * 1000)
 */
export interface WalletTransaction {
    txHash: string;
    /** ISO 8601 string — e.g. "2026-04-13T06:10:38+00:00". Use new Date(tx.blockTime) */
    blockTime: string;
    blockNumber: number;
    /** Plain sender wallet address string (NOT an object with .symbol) */
    from: string;
    /** Plain receiver wallet address string (NOT an object with .symbol) */
    to: string;
    mainAction: string;
    contractLabel?: any;
    /** Token movements in this tx — symbol and amount are here, not in from/to */
    balanceChange: Array<{
        address: string;
        symbol: string;
        name?: string;
        decimals?: number;
        amount: number;       // already parsed, positive = received
        logoURI?: string;
    }>;
    tokenTransfers?: any[];
    fee?: number;
    status?: string;
}

/**
 * Response wrapper from GET /v1/wallet/tx_list.
 * ⚠️ Results are keyed by chain slug, NOT `items`. For Solana calls, read `data.solana`.
 */
export interface WalletTxListResponse {
    solana?: WalletTransaction[];
    ethereum?: WalletTransaction[];
    [chain: string]: WalletTransaction[] | undefined;
}

/**
 * One entry in `/defi/v2/tokens/top_traders`. camelCase fields.
 * (Verified against live API — `owner` is the wallet, `trade` is the count.)
 */
export interface TopTrader {
    owner: string;
    tokenAddress: string;
    tags: string[];
    type: string;
    /** Total trade count in the time window */
    trade: number;
    tradeBuy: number;
    tradeSell: number;
    volume: number;
    volumeBuy: number;
    volumeSell: number;
    volumeUsd: number;
    volumeBuyUSD: number;
    volumeSellUSD: number;
    realizedPnl: number;
    unrealizedPnl: number;
    totalPnl: number;
}

/**
 * One entry from `/defi/v3/token/holder`. snake_case fields.
 * `mint` is the token mint address; `owner` is the holder wallet address.
 */
export interface TokenHolder {
    mint: string;
    owner: string;
    token_account: string;
    amount: string;
    ui_amount: number;
    decimals: number;
    multiplier?: number;
    is_scaled_ui_token?: boolean;
}

export interface HolderDistribution {
    token_address: string;
    summary: {
        wallet_count: number;
        total_holding: string;
        percent_of_supply: number;
    };
    holders: Array<{
        wallet: string;
        holding: string;
        percent_of_supply: number;
    }>;
}

/**
 * One token position inside a wallet net-worth response.
 *
 * ⚠️ Field names are snake_case as returned by the real API.
 *    amount  → already a parsed number (safe to call .toFixed directly)
 *    value   → string   → coerce: Number(item.value).toFixed(2)
 *    price   → number
 */
export interface WalletNetWorthItem {
    address: string;
    symbol: string;
    name?: string;
    /** Token balance — already a parsed number from the API */
    amount: number;
    /** USD value of this position — returned as a STRING by the API. Coerce: Number(item.value) */
    value: string;
    /** Token price in USD */
    price: number;
    /** Raw on-chain balance as a string (divide by 10**decimals for UI) */
    balance?: string;
    decimals?: number;
    /** Logo URL — returned as snake_case `logo_uri` by /wallet/v2/current-net-worth */
    logo_uri?: string;
    network?: string;
}

/**
 * Response from GET /wallet/v2/current-net-worth
 *
 * ⚠️ total_value is snake_case and a STRING — coerce: Number(data.total_value)
 */
export interface WalletNetWorth {
    wallet: string;
    /** Total portfolio USD value — returned as a STRING. Coerce: Number(data.total_value) */
    total_value: string;
    items: WalletNetWorthItem[];
}

export interface WalletPnL {
    summary: {
        counts: { total_buy: number; total_sell: number; total_trade: number; win_rate: number };
        cashflow_usd: { total_invested: number; total_sold: number };
        pnl: { realized_profit_usd: number; realized_profit_percent: number };
    };
}

/**
 * OHLCV interval types. 1s/15s/30s are V3-only.
 * Values are case-sensitive: '1H' not '1h', '1D' not '1d'.
 */
export type OHLCVType =
    | '1s' | '15s' | '30s'
    | '1m' | '3m' | '5m' | '15m' | '30m'
    | '1H' | '2H' | '4H' | '6H' | '8H' | '12H'
    | '1D' | '3D' | '1W' | '1M';

/**
 * Supported chains per GET /defi/networks (authoritative list).
 * Not every endpoint supports every chain — see resources/supported-networks.md.
 */
export type SupportedChain =
    | 'solana' | 'ethereum' | 'bsc' | 'base'
    | 'arbitrum' | 'optimism' | 'polygon'
    | 'zksync' | 'avalanche' | 'sui'
    | 'monad' | 'megaeth' | 'fogo' | 'aptos'
    | 'hyperevm' | 'mantle';

/** sort_by for /defi/token_trending */
export type TrendingSortBy = 'rank' | 'volumeUSD' | 'liquidity';

/** sort_by for /defi/v2/markets */
export type MarketsSortBy = 'liquidity' | 'volume24h';

/** time_frame for /defi/v2/markets */
export type MarketsTimeFrame = '30m' | '1h' | '2h' | '4h' | '6h' | '8h' | '12h' | '24h';

/** time_frame for /defi/v2/tokens/top_traders */
export type TopTradersTimeFrame =
    | '30m' | '1h' | '2h' | '4h' | '6h' | '8h' | '12h' | '24h'
    // Solana-only per official docs:
    | '2d' | '3d' | '7d' | '14d' | '30d' | '60d' | '90d';

/** type for /trader/gainers-losers */
export type GainersLosersType = 'today' | 'yesterday' | '1W';

// ============================================================================
// ERROR
// ============================================================================

export class BirdeyeError extends Error {
    constructor(
        message: string,
        public statusCode: number,
        public endpoint: string
    ) {
        super(message);
        this.name = 'BirdeyeError';
    }
}

// ============================================================================
// RATE LIMITER (token-bucket, per-minute)
// ============================================================================

class RateLimiter {
    private timestamps: number[] = [];

    constructor(private maxPerMinute: number) {}

    async acquire(): Promise<void> {
        const now = Date.now();
        this.timestamps = this.timestamps.filter(t => now - t < 60_000);
        if (this.timestamps.length >= this.maxPerMinute) {
            const wait = 60_000 - (now - this.timestamps[0]) + 100;
            await new Promise(r => setTimeout(r, wait));
            return this.acquire();
        }
        this.timestamps.push(Date.now());
    }
}

// ============================================================================
// BIRDEYE CLIENT
// ============================================================================

export class BirdeyeClient {
    readonly chain: string;  // exposed for sub-clients
    private apiKey: string;
    private baseUrl = 'https://public-api.birdeye.so';
    private rateLimiter: RateLimiter;

    // Sub-clients for organized domain access
    public price:  PriceClient;
    public token:  TokenClient;
    public market: MarketClient;
    public trades: TradesClient;
    public trader: TraderClient;
    public holder: HolderClient;
    public wallet: WalletClient;

    private constructor(config: BirdeyeConfig) {
        if (!config.apiKey) throw new Error('BirdeyeClient requires an API key.');
        this.apiKey = config.apiKey;
        this.chain  = config.chain || 'solana';

        // Per-account rate limit — override to match your plan. Official tiers:
        //   Standard: 60 rpm (1 rps)   Lite/Starter: 900 rpm (15 rps)
        //   Premium:  1000 rpm         Business:     1500 rpm (WebSocket included)
        // Wallet V1 group: documented 30 rpm cap (enforcement may vary by plan).
        const envRpm = Number(process.env.BIRDEYE_RATE_LIMIT_RPM);
        const rpm = config.rateLimitRpm ?? (Number.isFinite(envRpm) && envRpm > 0 ? envRpm : 60);
        this.rateLimiter = new RateLimiter(rpm);

        this.price  = new PriceClient(this);
        this.token  = new TokenClient(this);
        this.market = new MarketClient(this);
        this.trades = new TradesClient(this);
        this.trader = new TraderClient(this);
        this.holder = new HolderClient(this);
        this.wallet = new WalletClient(this);
    }

    /** Create client from BIRDEYE_API_KEY env var */
    static create(chain: SupportedChain = 'solana'): BirdeyeClient {
        const apiKey = process.env.BIRDEYE_API_KEY;
        if (!apiKey) throw new Error('BIRDEYE_API_KEY environment variable is required');
        return new BirdeyeClient({ apiKey, chain });
    }

    /** Create client with explicit config */
    static createWithConfig(config: BirdeyeConfig): BirdeyeClient {
        return new BirdeyeClient(config);
    }

    /** Internal GET with rate limiting and retry on 429/5xx. */
    async get<T>(endpoint: string, params: Record<string, any> = {}): Promise<T> {
        await this.rateLimiter.acquire();

        const url = new URL(`${this.baseUrl}${endpoint}`);
        for (const [k, v] of Object.entries(params)) {
            if (v !== undefined && v !== null) url.searchParams.append(k, String(v));
        }

        return this._fetchWithRetry<T>(url.toString(), {
            method: 'GET',
            headers: this._headers(),
        }, endpoint);
    }

    /** Internal POST */
    async post<T>(endpoint: string, body: Record<string, any> = {}): Promise<T> {
        await this.rateLimiter.acquire();
        const url = `${this.baseUrl}${endpoint}`;
        return this._fetchWithRetry<T>(url, {
            method: 'POST',
            headers: { ...this._headers(), 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        }, endpoint);
    }

    private _headers() {
        // User-Agent is not required by Birdeye today, but some runtimes (old Node,
        // certain proxies) historically caused 403s when omitted. Included defensively.
        return {
            'X-API-KEY': this.apiKey,
            'x-chain':   this.chain,
            'User-Agent': 'birdeye-client-ts',
            'Accept':    'application/json',
        };
    }

    private async _fetchWithRetry<T>(url: string, init: RequestInit, endpoint: string, maxRetries = 4): Promise<T> {
        for (let attempt = 0; attempt <= maxRetries; attempt++) {
            try {
                const res = await fetch(url, init);

                if (res.ok) {
                    const json = await res.json() as any;
                    if (json.success === false) {
                        // Error envelope varies: some endpoints use `message`,
                        // others nest details under `error: { code, message, details }`.
                        const msg = json.error?.message || json.message || JSON.stringify(json);
                        throw new BirdeyeError(msg, 200, endpoint);
                    }
                    return json.data as T;
                }

                if (res.status === 429 || res.status >= 500) {
                    if (attempt === maxRetries) {
                        throw new BirdeyeError(`Failed after ${maxRetries} retries`, res.status, endpoint);
                    }
                    const delay = Math.min(1000 * Math.pow(2, attempt), 16_000);
                    await new Promise(r => setTimeout(r, delay));
                    continue;
                }

                const body = await res.json().catch(() => ({})) as any;
                const msg = body.error?.message || body.message || res.statusText;
                throw new BirdeyeError(msg, res.status, endpoint);
            } catch (err) {
                // BirdeyeError (thrown above) is not retryable — rethrow as-is.
                if (err instanceof BirdeyeError) throw err;

                // Network-level failure (DNS, TCP reset, timeout). Retry with backoff.
                const msg = err instanceof Error ? err.message : String(err);
                if (attempt === maxRetries) {
                    throw new BirdeyeError(`Network error after ${maxRetries} retries: ${msg}`, 0, endpoint);
                }
                const delay = Math.min(1000 * Math.pow(2, attempt), 16_000);
                await new Promise(r => setTimeout(r, delay));
            }
        }
        throw new BirdeyeError('Unreachable', 0, endpoint);
    }

    /** Check remaining API credits */
    getCredits() {
        return this.get<any>('/utils/v1/credits');
    }
}

// ============================================================================
// PRICE CLIENT
// ============================================================================

class PriceClient {
    constructor(private c: BirdeyeClient) {}

    /** Latest price for a single token */
    getPrice(address: string): Promise<TokenPrice> {
        return this.c.get<TokenPrice>('/defi/price', { address });
    }

    /**
     * Prices for multiple tokens in one call (Standard+ tier).
     * Pass up to 100 addresses.
     */
    getMultiPrice(addresses: string[]): Promise<Record<string, TokenPrice>> {
        return this.c.get<Record<string, TokenPrice>>('/defi/multi_price', {
            list_address: addresses.join(','),
        });
    }

    /**
     * OHLCV candles for a token.
     * type values are case-sensitive: '1H' not '1h', '1D' not '1d'.
     * time_from / time_to are required — both must be provided.
     * Max 1000 records per call.
     */
    getOHLCV(address: string, type: OHLCVType, time_from: number, time_to: number): Promise<{ items: OHLCVCandle[] }> {
        return this.c.get('/defi/v3/ohlcv', { address, type, time_from, time_to });
    }

    /**
     * OHLCV for a specific pair address.
     * time_from / time_to required.
     */
    getOHLCVPair(pairAddress: string, type: OHLCVType, time_from: number, time_to: number): Promise<{ items: OHLCVCandle[] }> {
        return this.c.get('/defi/v3/ohlcv/pair', { address: pairAddress, type, time_from, time_to });
    }

    /**
     * Historical price at a specific unix timestamp.
     * Param is `unixtime` (singular) — not time_from/time_to.
     */
    getHistoricalPrice(address: string, unixtime: number): Promise<any> {
        return this.c.get('/defi/historical_price_unix', { address, unixtime });
    }
}

// ============================================================================
// TOKEN CLIENT
// ============================================================================

class TokenClient {
    constructor(private c: BirdeyeClient) {}

    /**
     * Full overview: price, marketCap, fdv, liquidity, volume,
     * holder count, priceChange1m/5m/1h/4h/24h, trade24h, uniqueWallet24h.
     */
    getOverview(address: string): Promise<TokenOverview> {
        return this.c.get<TokenOverview>('/defi/token_overview', { address });
    }

    /** Metadata: name, symbol, decimals, logoURI, extensions */
    getMetadata(address: string): Promise<TokenMetadata> {
        return this.c.get<TokenMetadata>('/defi/v3/token/meta-data/single', { address });
    }

    /** Market data: price, mc, FDV */
    getMarketData(address: string): Promise<any> {
        return this.c.get('/defi/v3/token/market-data', { address });
    }

    /**
     * Security check: mint authority, creator concentration, honeypot flags.
     * Key fields: isMintable (not `mintable`), freezeable, freezeAuthority,
     * transferFeeEnable, isTrueToken, top10HolderPercent, creatorPercentage.
     */
    getSecurity(address: string): Promise<any> {
        return this.c.get('/defi/token_security', { address });
    }

    /**
     * Creation info: creator address, creation timestamp (Solana only).
     */
    getCreationInfo(address: string): Promise<any> {
        return this.c.get('/defi/token_creation_info', { address });
    }

    /**
     * Paginated token list.
     * sort_by valid values (live-tested): 'liquidity' | 'fdv' | 'market_cap' | 'holder'
     * NOTE: 'v24hUSD', 'volume24h', 'price' all return 400 "invalid sort_by parameter".
     */
    getList(sort_by: 'liquidity' | 'fdv' | 'market_cap' | 'holder' = 'liquidity', sort_type: 'asc' | 'desc' = 'desc', offset = 0, limit = 50): Promise<any> {
        return this.c.get('/defi/v3/token/list', { sort_by, sort_type, offset, limit });
    }
}

// ============================================================================
// MARKET CLIENT  (discovery & pairs)
// ============================================================================

class MarketClient {
    constructor(private c: BirdeyeClient) {}

    /**
     * Trending tokens.
     * sort_by: 'rank' | 'volumeUSD' | 'liquidity'  (NOT 'volume24hUSD')
     * sort_by and sort_type are required.
     */
    async getTrending(
        sort_by: TrendingSortBy = 'rank',
        sort_type: 'asc' | 'desc' = 'asc',
        limit = 20
    ): Promise<TrendingToken[]> {
        const data = await this.c.get<any>('/defi/token_trending', { sort_by, sort_type, offset: 0, limit });
        return data.tokens ?? data;
    }

    /** Recently listed tokens */
    getNewListings(limit = 20): Promise<any> {
        return this.c.get('/defi/v2/tokens/new_listing', { limit });
    }

    /**
     * Meme token leaderboard. Supported chains: solana, bsc, monad.
     * Official docs mark `sort_by` and `sort_type` as required — pass them together
     * for predictable results.
     *
     * Full `sort_by` enum (per official docs):
     *   progress_percent · graduated_time · creation_time · liquidity · market_cap ·
     *   fdv · recent_listing_time · last_trade_unix_time · holder ·
     *   volume_{1m,5m,30m,1h,2h,4h,8h,24h,7d,30d}_usd ·
     *   volume_{1m,5m,30m,1h,2h,4h,8h,24h,7d,30d}_change_percent ·
     *   price_change_{1m,5m,30m,1h,2h,4h,8h,24h,7d,30d}_percent ·
     *   trade_{1m,5m,30m,1h,2h,4h,8h,24h,7d,30d}_count
     * Typed loosely as string — the full enum is large and may expand; caller is
     * responsible for passing a valid value.
     */
    getMemeTokens(
        sort_by: string = 'liquidity',
        sort_type: 'asc' | 'desc' = 'desc',
        limit = 20,
    ): Promise<any> {
        return this.c.get('/defi/v3/token/meme/list', { sort_by, sort_type, limit });
    }

    /**
     * Search tokens or markets by keyword/address. Multi-chain — the `chain`
     * query param accepts `all` or any supported network value.
     * sort_by and sort_type are required per official docs.
     * target: 'all' | 'token' | 'market'
     * search_by: 'combination' | 'address' | 'name' | 'symbol'
     * Note: `verify_token` and `markets` filters are Solana-only.
     */
    search(
        keyword: string,
        sort_by = 'liquidity',
        sort_type: 'asc' | 'desc' = 'desc',
        target: 'all' | 'token' | 'market' = 'token',
        search_by: 'combination' | 'address' | 'name' | 'symbol' = 'combination',
        limit = 20
    ): Promise<any> {
        return this.c.get('/defi/v3/search', {
            keyword,
            chain: this.c.chain,  // also sent as x-chain header
            target,
            search_by,
            sort_by,
            sort_type,
            limit,
        });
    }

    /**
     * All liquidity markets (pairs) for a token.
     * time_frame, sort_by, sort_type are all REQUIRED — omitting any causes 400.
     * sort_by: 'liquidity' | 'volume24h'
     * time_frame: '30m'|'1h'|'2h'|'4h'|'6h'|'8h'|'12h'|'24h'
     */
    getMarkets(
        address: string,
        time_frame: MarketsTimeFrame = '24h',
        sort_by: MarketsSortBy = 'liquidity',
        sort_type: 'asc' | 'desc' = 'desc'
    ): Promise<any> {
        return this.c.get('/defi/v2/markets', { address, time_frame, sort_by, sort_type });
    }

    /**
     * Stats for a specific pair address. Supported chains (per official docs):
     * `solana`, `fogo`.
     */
    getPairOverview(pairAddress: string): Promise<any> {
        return this.c.get('/defi/v3/pair/overview/single', { address: pairAddress });
    }
}

// ============================================================================
// TRADES CLIENT
// ============================================================================

class TradesClient {
    constructor(private c: BirdeyeClient) {}

    /**
     * Recent trades for a token.
     * Response fields are snake_case: `tx_type`, `side`, `volume_usd`, `block_unix_time`.
     * tx_type: 'swap' | 'buy' | 'sell' | 'add' | 'remove' | 'all'
     * `side` is "buy"/"sell" from the token-address perspective.
     * Filter by `volume_usd > 0` if you need to exclude zero-value entries.
     */
    getTokenTrades(
        address: string,
        tx_type: 'swap' | 'buy' | 'sell' | 'add' | 'remove' | 'all' = 'swap',
        limit = 50
    ): Promise<{ items: TradeTransaction[] }> {
        return this.c.get('/defi/v3/token/txs', { address, tx_type, limit });
    }
}

// ============================================================================
// TRADER CLIENT
// ============================================================================

class TraderClient {
    constructor(private c: BirdeyeClient) {}

    /**
     * Top traders for a token.
     * time_frame: '30m'|'1h'|'2h'|'4h'|'6h'|'8h'|'12h'|'24h' (plus '2d'|'3d'|'7d'|'14d'|'30d'|'60d'|'90d' on Solana)
     * sort_by: 'volume' | 'trade' | 'total_pnl' | 'unrealized_pnl' | 'realized_pnl' | 'volume_usd'
     *   PnL sort values are Solana-only per official docs.
     * sort_type is required by the official docs.
     */
    getTopTraders(
        address: string,
        time_frame: TopTradersTimeFrame = '24h',
        sort_by: 'volume' | 'trade' | 'total_pnl' | 'unrealized_pnl' | 'realized_pnl' | 'volume_usd' = 'volume',
        sort_type: 'asc' | 'desc' = 'desc',
        limit = 10
    ): Promise<{ items: TopTrader[] }> {
        return this.c.get('/defi/v2/tokens/top_traders', { address, time_frame, sort_by, sort_type, limit });
    }

    /**
     * Best performing traders on-chain.
     * type: 'today' | 'yesterday' | '1W'
     * WARNING: 'gainers' / 'losers' are INVALID — they cause 400 "type invalid format".
     * sort_by: 'PnL' (only valid value per canonical dict)
     * sort_type is REQUIRED.
     */
    getGainers(
        type: GainersLosersType = 'today',
        sort_by: 'PnL' = 'PnL',
        sort_type: 'asc' | 'desc' = 'desc',
        limit = 10
    ): Promise<any> {
        return this.c.get('/trader/gainers-losers', { type, sort_by, sort_type, limit });
    }
}

// ============================================================================
// HOLDER CLIENT
// ============================================================================

class HolderClient {
    constructor(private c: BirdeyeClient) {}

    /** Paginated list of top token holders (Solana only) */
    getHolders(address: string, limit = 100, offset = 0): Promise<{ items: TokenHolder[] }> {
        return this.c.get('/defi/v3/token/holder', { address, limit, offset });
    }

    /**
     * Holder concentration distribution (Solana only).
     * CRITICAL: param is `token_address` — NOT `address`. Using `address` returns 400.
     */
    getDistribution(tokenAddress: string): Promise<HolderDistribution> {
        return this.c.get<HolderDistribution>('/holder/v1/distribution', {
            token_address: tokenAddress,
        });
    }
}

// ============================================================================
// WALLET CLIENT  (Solana only)
// ============================================================================

class WalletClient {
    constructor(private c: BirdeyeClient) {}

    /**
     * Current net worth + token list.
     * sort_type is REQUIRED — omitting it causes 400.
     *
     * ⚠️ ACTUAL API FIELD NAMES (snake_case, not camelCase):
     *   data.total_value  → string  (NOT totalUsd)
     *   item.amount       → number  (NOT balance)
     *   item.value        → string  (NOT valueUsd) — coerce: Number(item.value)
     *   item.price        → number  (NOT priceUsd)
     *
     * @example
     * const data = await client.wallet.getNetWorth(wallet);
     * const total = Number(data.total_value);   // ← total_value, not totalUsd
     * for (const item of data.items) {
     *   const bal = item.amount;                 // ← amount (already a number)
     *   const val = Number(item.value);          // ← value is a string, coerce it
     *   console.log(`${item.symbol}: ${bal.toFixed(4)} = $${val.toFixed(2)}`);
     * }
     */
    getNetWorth(wallet: string, sort_type: 'asc' | 'desc' = 'desc'): Promise<WalletNetWorth> {
        return this.c.get<WalletNetWorth>('/wallet/v2/current-net-worth', { wallet, sort_type });
    }

    /** All token balances in a wallet */
    getTokenList(wallet: string): Promise<any> {
        return this.c.get('/v1/wallet/token_list', { wallet });
    }

    /**
     * P&L summary — PRO tier only (403 on Standard/Lite).
     * Optional duration: 'all' | '90d' | '30d' | '7d' | '24h'
     */
    getPnL(wallet: string, duration = 'all'): Promise<WalletPnL> {
        return this.c.get<WalletPnL>('/wallet/v2/pnl/summary', { wallet, duration });
    }

    /**
     * Transaction history from /v1/wallet/tx_list.
     *
     * ⚠️ RESPONSE WRAPPER: `{ solana: [...] }` (keyed by chain), NOT `{ items: [...] }`.
     *    For Solana: const txs = data.solana ?? [];
     *
     * ⚠️ FIELD SHAPES (differ from /defi/v3/token/txs):
     *   tx.blockTime     → ISO string e.g. "2026-04-13T06:10:38+00:00"  (NOT unix number)
     *   tx.from / tx.to  → plain wallet address string                   (NOT objects)
     *   token info       → tx.balanceChange[].symbol / .amount
     *
     * Parse time correctly:
     *   new Date(tx.blockTime).getTime()  ✅
     *   tx.blockTime * 1000               ❌ (NaN — blockTime is a string)
     *
     * Many Raydium/Pump.fun swaps return mainAction="unknown".
     * Filter by balanceChange[].amount > 0 to find received tokens.
     */
    getTxHistory(wallet: string, limit = 50): Promise<WalletTxListResponse> {
        return this.c.get('/v1/wallet/tx_list', { wallet, limit });
    }

    /**
     * First funded transaction — wallet age / origin tracing.
     * Uses POST. Body is {"wallets": [address]} — array, NOT {"wallet": "..."}.
     */
    getFirstFunded(wallet: string): Promise<any> {
        return this.c.post('/wallet/v2/tx/first-funded', { wallets: [wallet] });
    }
}

// ============================================================================
// EXPORTS
// ============================================================================

export default BirdeyeClient;
