/**
 * Top Traders for a Token — Birdeye /defi/v2/tokens/top_traders
 * 
 * Identifies the highest-volume wallets trading a specific token over a time window.
 * Useful for smart money tracking and whale monitoring.
 */

const TOKEN_ADDRESS = process.env.TOKEN_ADDRESS || "So11111111111111111111111111111111111111112";
const BIRDEYE_API_KEY = process.env.BIRDEYE_API_KEY!;

async function getTopTraders(
  address: string,
  timeFrame: string = "24h",     // '30m'|'1h'|'2h'|'4h'|'6h'|'8h'|'12h'|'24h' (+ '2d'|'3d'|'7d'|'14d'|'30d'|'60d'|'90d' on Solana)
  sortBy: string = "volume",      // 'volume' | 'trade' | 'total_pnl' | 'unrealized_pnl' | 'realized_pnl' | 'volume_usd' (PnL values Solana-only)
  sortType: "asc" | "desc" = "desc",  // REQUIRED by /defi/v2/tokens/top_traders
  limit: number = 10
): Promise<void> {
  const url = new URL("https://public-api.birdeye.so/defi/v2/tokens/top_traders");
  url.searchParams.set("address", address);
  url.searchParams.set("time_frame", timeFrame);
  url.searchParams.set("sort_by", sortBy);
  url.searchParams.set("sort_type", sortType);
  url.searchParams.set("limit", String(limit));

  const res = await fetch(url.toString(), {
    headers: {
      "X-API-KEY": BIRDEYE_API_KEY,
      "x-chain": "solana",
      "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
      "Accept": "application/json",
    },
  });

  if (!res.ok) throw new Error(`API Error ${res.status}: ${res.statusText}`);
  const json = await res.json() as any;
  const traders = json.data?.items ?? [];

  // Response fields (live-verified): owner, tokenAddress, trade, tradeBuy, tradeSell,
  // volume, volumeBuy, volumeSell, volumeUsd, realizedPnl, unrealizedPnl, totalPnl, tags, type.
  console.log(`=== TOP ${limit} TRADERS (${timeFrame}) ===`);
  traders.forEach((trader: any, i: number) => {
    console.log(`${i + 1}. ${trader.owner}`);
    console.log(`   Volume:    ${trader.volume?.toFixed(2)} (USD: $${trader.volumeUsd?.toLocaleString(undefined, { maximumFractionDigits: 0 })})`);
    console.log(`   Trades:    ${trader.trade} (buy: ${trader.tradeBuy}, sell: ${trader.tradeSell})`);
    console.log(`   Realized:  $${trader.realizedPnl?.toFixed(2)}`);
    console.log(`   Tags:      ${trader.tags?.join(", ") || "none"}`);
  });
}

getTopTraders(TOKEN_ADDRESS, "24h", "volume", "desc", 10).catch(console.error);
