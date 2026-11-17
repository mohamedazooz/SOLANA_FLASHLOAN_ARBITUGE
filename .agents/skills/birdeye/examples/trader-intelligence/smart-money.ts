/**
 * Smart Money Signals — Birdeye GET /smart-money/v1/token/list
 *
 * Identifies tokens being accumulated or distributed by Birdeye-classified
 * "smart money" wallets (historically profitable, high-capital traders).
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/trader-intelligence/smart-money.ts
 *
 * ⚠️ Requires PRO tier or higher.
 * ⚠️ Solana only.
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

// Live-verified response shape. NOTE: /smart-money/v1/token/list returns
// {data: [...]} directly — NO {success:true} wrapper and NO {items:[]} nesting.
interface SmartMoneyToken {
  token: string;              // token address (NOT `address`)
  symbol: string;
  name: string;
  price: number;
  liquidity: number;
  market_cap: number;
  net_flow: number;           // + = accumulation, - = distribution
  smart_traders_no: number;   // count of smart-money wallets
  trader_style: string;       // e.g. "trenchers", "risk_balancers"
  volume_usd: number;
  volume_buy_usd: number;
  volume_sell_usd: number;
  price_change_percent: number;
  logo_uri?: string;
}

async function getSmartMoneyList(
  apiKey: string,
  sortType: 'desc' | 'asc',
  limit: number,
): Promise<SmartMoneyToken[]> {
  const params = new URLSearchParams({
    sort_by: 'net_flow',
    sort_type: sortType,
    limit: String(limit),
    offset: '0',
  });
  const url = `${BASE}/smart-money/v1/token/list?${params}`;
  const res = await fetch(url, { headers: headers(apiKey) });
  if (!res.ok) throw new Error(`Smart money API ${res.status}: ${res.statusText}`);
  const json = await res.json() as any;
  return Array.isArray(json.data) ? json.data : [];
}

function formatUSD(n: number): string {
  if (Math.abs(n) >= 1_000_000) return `$${(n / 1_000_000).toFixed(2)}M`;
  if (Math.abs(n) >= 1_000) return `$${(n / 1_000).toFixed(1)}k`;
  return `$${n.toFixed(2)}`;
}

async function run() {
  const apiKey = process.env.BIRDEYE_API_KEY;
  if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

  console.log('=== Smart Money Accumulation (top buys by net_flow) ===\n');
  const accumulating = await getSmartMoneyList(apiKey, 'desc', 10);
  for (const t of accumulating) {
    console.log(
      `🟢 ${(t.symbol ?? t.token.slice(0, 8)).padEnd(12)} ` +
      `| Net flow: ${formatUSD(t.net_flow)} ` +
      `| Style: ${t.trader_style} (${t.smart_traders_no} wallets) ` +
      `| Buy vol: ${formatUSD(t.volume_buy_usd)}`
    );
  }

  console.log('\n=== Smart Money Distribution (top sells by net_flow) ===\n');
  const distributing = await getSmartMoneyList(apiKey, 'asc', 10);
  for (const t of distributing) {
    console.log(
      `🔴 ${(t.symbol ?? t.token.slice(0, 8)).padEnd(12)} ` +
      `| Net flow: ${formatUSD(t.net_flow)} ` +
      `| Style: ${t.trader_style} (${t.smart_traders_no} wallets) ` +
      `| Sell vol: ${formatUSD(t.volume_sell_usd)}`
    );
  }
}

run().catch(console.error);
