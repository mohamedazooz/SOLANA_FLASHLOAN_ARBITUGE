import json
import urllib.request
import time

TOKENS = {
    "SOL": "So11111111111111111111111111111111111111112",
    "BONK": "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB268",
    "WIF": "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm",
    "JUP": "JUPyiwrYJFngUPXSiqZFkKHMXkGs3WEXKaJE1agWK",
    "RAY": "4k3Dyjzvzp8eMZWUXbBCjEvwSkkU59Y5j3uYBqZ3e5pV",
    "POPCAT": "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr",
    "FARTCOIN": "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump",
    "TRUMP": "6p6xgHyF7AeQHydaKVtRNVUjCAWoTeZwXHnkTXQKpump",
    "AI16Z": "HeLp6NuQkmYB4pYWo2zYs22mESHXPQYzXbB8n4V98jwC"
}

def fetch_token_pools(token_address):
    url = f"https://api.dexscreener.com/latest/dex/tokens/{token_address}"
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode('utf-8'))
            return data.get("pairs", [])
    except Exception as e:
        print(f"Error fetching pools for {token_address}: {e}")
        return []

all_opportunities = []

for symbol, mint in TOKENS.items():
    print(f"Scanning pools for {symbol} ({mint[:8]}...)...")
    pairs = fetch_token_pools(mint) or []
    # Filter for Solana pairs only
    sol_pairs = [p for p in pairs if isinstance(p, dict) and p.get("chainId") == "solana" and p.get("priceUsd")]
    
    # Group pairs by quote token (e.g. SOL or USDC)
    by_quote = {}
    for p in sol_pairs:
        quote = p.get("quoteToken", {}).get("symbol", "").upper()
        if quote in ["SOL", "WSOL", "USDC", "USDT"]:
            by_quote.setdefault(quote, []).append(p)
            
    for quote, pool_list in by_quote.items():
        if len(pool_list) < 2:
            continue
        # Compare every pair of pools for this token
        for i in range(len(pool_list)):
            for j in range(i + 1, len(pool_list)):
                p1 = pool_list[i]
                p2 = pool_list[j]
                
                dex1 = p1.get("dexId", "unknown")
                dex2 = p2.get("dexId", "unknown")
                
                # We want cross-DEX arbitrage (or different pools)
                p1_addr = p1.get("pairAddress")
                p2_addr = p2.get("pairAddress")
                if p1_addr == p2_addr:
                    continue
                
                price1 = float(p1.get("priceUsd", 0))
                price2 = float(p2.get("priceUsd", 0))
                
                liq1 = float(p1.get("liquidity", {}).get("usd", 0) or 0)
                liq2 = float(p2.get("liquidity", {}).get("usd", 0) or 0)
                
                # Must have reasonable liquidity (at least $2,000)
                if price1 <= 0 or price2 <= 0 or liq1 < 2000 or liq2 < 2000:
                    continue
                
                # Check price difference
                diff_pct = abs(price1 - price2) / min(price1, price2) * 100.0
                spread_bps = diff_pct * 100.0
                
                # If there's an exploitable spread (> 15 bps to cover 10-30 bps fees)
                if spread_bps >= 15.0:
                    buy_pool = p1 if price1 < price2 else p2
                    sell_pool = p2 if price1 < price2 else p1
                    buy_dex = dex1 if price1 < price2 else dex2
                    sell_dex = dex2 if price1 < price2 else dex1
                    buy_price = min(price1, price2)
                    sell_price = max(price1, price2)
                    
                    # Available capacity based on liquidity depth (safe trade size: 0.5% - 1% of pool liquidity)
                    max_safe_size_usd = min(1500.0, min(liq1, liq2) * 0.01)
                    if max_safe_size_usd < 50.0:
                        continue
                    
                    # Fees: DEX A fee (~0.25%) + DEX B fee (~0.25%) + Kamino Flash Loan fee (0.01%) + Jito tip ($1.10)
                    est_fees_usd = max_safe_size_usd * (0.0025 + 0.0025 + 0.0001) + 1.10
                    gross_profit_usd = max_safe_size_usd * (diff_pct / 100.0)
                    net_profit_usd = gross_profit_usd - est_fees_usd
                    
                    all_opportunities.append({
                        "token": symbol,
                        "quote": quote,
                        "buy_dex": buy_dex,
                        "buy_pool": buy_pool.get("pairAddress"),
                        "buy_price": buy_price,
                        "sell_dex": sell_dex,
                        "sell_pool": sell_pool.get("pairAddress"),
                        "sell_price": sell_price,
                        "spread_bps": round(spread_bps, 1),
                        "spread_pct": round(diff_pct, 3),
                        "safe_trade_size_usd": round(max_safe_size_usd, 2),
                        "gross_profit_usd": round(gross_profit_usd, 2),
                        "est_fees_usd": round(est_fees_usd, 2),
                        "net_profit_usd": round(net_profit_usd, 2),
                        "executable": net_profit_usd > 0.05
                    })

all_opportunities.sort(key=lambda x: x["net_profit_usd"], reverse=True)

print("\n" + "="*90)
print(f" LIVE SOLANA CROSS-POOL ARBITRAGE OPPORTUNITIES ({len(all_opportunities)} FOUND)")
print("="*90)

for op in all_opportunities[:15]:
    status = "EXECUTABLE" if op['executable'] else "MARGINAL"
    print(f"[{status}] {op['token']}/{op['quote']}: Buy on {op['buy_dex'].upper()} @ ${op['buy_price']:.6f} -> Sell on {op['sell_dex'].upper()} @ ${op['sell_price']:.6f}")
    print(f"       Spread: {op['spread_bps']} bps ({op['spread_pct']}%) | Sizing: ${op['safe_trade_size_usd']:.2f}")
    print(f"       Gross: ${op['gross_profit_usd']:.2f} | Fees: ${op['est_fees_usd']:.2f} | Net Profit: ${op['net_profit_usd']:+.2f}")
    print(f"       Buy Pool:  {op['buy_pool']}")
    print(f"       Sell Pool: {op['sell_pool']}")
    print("-" * 90)

with open("c:/Users/azoozz/Downloads/my arbitrge/live_pool_opportunities.json", "w") as f:
    json.dump(all_opportunities, f, indent=2)
print(f"\nSaved {len(all_opportunities)} opportunities to live_pool_opportunities.json")
