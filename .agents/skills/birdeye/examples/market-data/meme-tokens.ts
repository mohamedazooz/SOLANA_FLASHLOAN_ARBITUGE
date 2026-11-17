/**
 * Meme Token Discovery — Birdeye /defi/v3/token/meme/list
 *
 * Fetches the live meme token leaderboard (pump.fun, bonding curves, etc.)
 * and enriches with security data to surface safe high-momentum memes.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/market-data/meme-tokens.ts
 *
 * NOTE: /defi/v3/token/meme/list — official docs mark both `sort_by` and
 * `sort_type` as required; pass them together. Common sort_by values:
 * 'liquidity', 'volume_24h_usd', 'market_cap', 'fdv', 'recent_listing_time',
 * 'volume_24h_change_percent', 'progress_percent', 'holder',
 * 'price_change_24h_percent', 'trade_24h_count'. See the official docs
 * (https://docs.birdeye.so/reference/get-defi-v3-token-meme-list) for the
 * full enum. Supported chains: solana, bsc, monad.
 */

const BASE = 'https://public-api.birdeye.so';

function headers(apiKey: string) {
  return {
    'X-API-KEY': apiKey,
    'x-chain': 'solana',
    'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)',
    'Accept': 'application/json',
  };
}

interface MemeToken {
  address: string;
  symbol: string;
  name: string;
  price: number;
  volume_24h_usd: number;
  market_cap: number;
  price_change_24h_percent: number;
  liquidity: number;
  last_trade_unix_time: number;
  logo_uri?: string;
}

async function getMemeList(apiKey: string, limit = 20): Promise<MemeToken[]> {
  // sort_by + sort_type are both marked required by the official docs.
  const params = new URLSearchParams({
    sort_by: 'liquidity',
    sort_type: 'desc',
    limit: String(limit),
  });
  const url = `${BASE}/defi/v3/token/meme/list?${params}`;
  const res = await fetch(url, { headers: headers(apiKey) });
  const json = await res.json() as any;
  if (!json.success) throw new Error(`Meme list failed: ${json.message}`);
  return json.data?.items ?? [];
}

async function getMemeDetail(apiKey: string, address: string): Promise<any> {
  const url = `${BASE}/defi/v3/token/meme/detail/single?address=${address}`;
  const res = await fetch(url, { headers: headers(apiKey) });
  const json = await res.json() as any;
  if (!json.success) throw new Error(`Meme detail failed: ${json.message}`);
  return json.data;
}

async function getTokenSecurity(apiKey: string, address: string): Promise<any> {
  const url = `${BASE}/defi/token_security?address=${address}`;
  const res = await fetch(url, { headers: headers(apiKey) });
  const json = await res.json() as any;
  return json.data;
}

function riskLevel(security: any): string {
  // /defi/token_security response fields: isMintable, freezeable, freezeAuthority,
  // mutableMetadata, nonTransferable, transferFeeEnable, creatorPercentage,
  // top10HolderPercent, ownerPercentage. (Field is `isMintable`, not `mintable`.)
  if (!security) return 'unknown';
  if (security.isMintable || security.freezeable || security.freezeAuthority) return '🔴 HIGH';
  if (security.transferFeeEnable) return '🟡 MEDIUM';
  if ((security.creatorPercentage ?? 0) > 0.2) return '🟡 MEDIUM';
  if ((security.top10HolderPercent ?? 0) > 0.5) return '🟡 MEDIUM';
  return '🟢 LOW';
}

async function run() {
  const apiKey = process.env.BIRDEYE_API_KEY;
  if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

  console.log('Fetching meme token leaderboard...\n');
  const tokens = await getMemeList(apiKey, 20);

  console.log(`Top ${tokens.length} meme tokens:\n`);
  console.log('─'.repeat(90));

  for (const token of tokens.slice(0, 10)) {
    // Get security data for top 10
    const security = await getTokenSecurity(apiKey, token.address).catch(() => null);
    const risk = riskLevel(security);

    const price = token.price?.toFixed(8) ?? 'N/A';
    const vol = token.volume_24h_usd
      ? `$${(token.volume_24h_usd / 1000).toFixed(1)}k`
      : 'N/A';
    const change = token.price_change_24h_percent
      ? `${token.price_change_24h_percent > 0 ? '+' : ''}${token.price_change_24h_percent.toFixed(1)}%`
      : 'N/A';

    console.log(`${token.symbol.padEnd(12)} | Price: $${price} | 24h Vol: ${vol} | Change: ${change} | Risk: ${risk}`);
    console.log(`  ${token.address}`);
  }

  console.log('\nFor real-time meme token discovery, use WebSocket SUBSCRIBE_MEME channel.');
}

run().catch(console.error);
