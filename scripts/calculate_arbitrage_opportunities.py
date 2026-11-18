import json
import urllib.request
import urllib.error
import time

# ==============================================================================
# 1. CANONICAL PUMP.FUN BOOST MIGRATION ARBITRAGE MODEL
# Reconstructs exact economics from crates/arb-math/src/pump_boost_arb.rs
# ==============================================================================

PUMP_GRADUATION_TARGET_LAMPORTS = 85_000_000_000       # 85 SOL
PUMP_BOOST_ALLOCATION_LAMPORTS  = 17_580_000_000       # ~17.58 SOL TWAP buyback
POOL_OPENING_QUOTE_LAMPORTS     = 84_990_000_000       # ~85 SOL opening pool quote reserve
POOL_OPENING_BASE_TOKENS        = 206_900_000_000_000  # 206.9M tokens (6 decimals)
VIRTUAL_SOL_INIT_LAMPORTS       = 30_000_000_000       # 30 SOL initial virtual reserve
VIRTUAL_TOK_INIT_ATOMIC         = 1_073_000_000_000_000 # 1073M tokens initial virtual reserve
CURVE_K = VIRTUAL_SOL_INIT_LAMPORTS * VIRTUAL_TOK_INIT_ATOMIC

# Kamino Reserve Fee Scaling Factor (U68F60): sf = 11_529_215_046_068 = 0.1 bps (0.001%)
KAMINO_FLASH_LOAN_FEE_SF = 11_529_215_046_068

def calculate_flash_fee_raw(amount_lamports, fee_sf):
    """Matches crates/arb-math/src/economic.rs::calculate_flash_fee_raw"""
    if fee_sf == 0:
        return 1
    fee = (amount_lamports * fee_sf) >> 60
    return max(1, fee)

def tokens_acquired_on_curve(vsol, vtok, cost_lamports):
    """Invariant: (vsol + cost) * (vtok - delta_tok) = k = vsol * vtok"""
    if cost_lamports == 0 or vsol == 0 or vtok == 0:
        return 0
    k = vsol * vtok
    new_vsol = vsol + cost_lamports
    new_vtok = k // new_vsol
    return vtok - new_vtok

def evaluate_boost_amm_dump(acquired_tokens, capture_efficiency_bps, include_boost_buyback=True):
    """
    Evaluates gross return from selling acquired tokens into newly formed pool.
    - include_boost_buyback=True: pool quote reserve absorbs the protocol BOOST buyback (~17.58 SOL).
    - include_boost_buyback=False: raw CPMM pool without boost realization (downside floor).
    """
    if acquired_tokens == 0:
        return 0, 0
    
    if include_boost_buyback:
        eff_quote = POOL_OPENING_QUOTE_LAMPORTS + int(PUMP_BOOST_ALLOCATION_LAMPORTS * (capture_efficiency_bps / 10000.0))
    else:
        eff_quote = POOL_OPENING_QUOTE_LAMPORTS

    denom = POOL_OPENING_BASE_TOKENS + acquired_tokens
    raw_dump = (eff_quote * acquired_tokens) // denom if denom > 0 else 0
    boost_absorbed = min(PUMP_BOOST_ALLOCATION_LAMPORTS, raw_dump)
    realized_gross = (raw_dump * capture_efficiency_bps) // 10000 if not include_boost_buyback else raw_dump
    return realized_gross, boost_absorbed

def simulate_pump_boost(
    real_sol_reserves_sol,
    include_boost_buyback=True,
    capture_efficiency_bps=7000,
    is_flash_loan=True,
    execution_tip_lamports=15_000_000,
    sol_price_usd=73.14
):
    real_sol_lamports = int(real_sol_reserves_sol * 1e9)
    cost_lamports = max(0, PUMP_GRADUATION_TARGET_LAMPORTS - real_sol_lamports)
    if cost_lamports == 0:
        return None
        
    vsol = VIRTUAL_SOL_INIT_LAMPORTS + real_sol_lamports
    vtok = CURVE_K // vsol
    
    acquired_tokens = tokens_acquired_on_curve(vsol, vtok, cost_lamports)
    gross_sol_out, boost_absorbed = evaluate_boost_amm_dump(acquired_tokens, capture_efficiency_bps, include_boost_buyback)
    
    fee_sf = KAMINO_FLASH_LOAN_FEE_SF if is_flash_loan else 0
    flash_loan_fee = calculate_flash_fee_raw(cost_lamports, fee_sf)
    
    total_expenses = cost_lamports + flash_loan_fee + execution_tip_lamports
    net_profit_lamports = gross_sol_out - total_expenses
    
    net_profit_sol = net_profit_lamports / 1e9
    net_profit_usd = net_profit_sol * sol_price_usd
    cost_sol = cost_lamports / 1e9
    roi_pct = (net_profit_sol / cost_sol) * 100.0 if cost_sol > 0 else 0.0
    
    return {
        "real_sol_reserves": real_sol_reserves_sol,
        "completion_cost_sol": cost_sol,
        "acquired_tokens_m": round(acquired_tokens / 1e6, 2),
        "gross_sol_out": round(gross_sol_out / 1e9, 4),
        "boost_absorbed_sol": round(boost_absorbed / 1e9, 4),
        "flash_fee_sol": round(flash_loan_fee / 1e9, 6),
        "execution_tip_sol": round(execution_tip_lamports / 1e9, 4),
        "net_profit_sol": round(net_profit_sol, 4),
        "net_profit_usd": round(net_profit_usd, 2),
        "roi_pct": round(roi_pct, 2),
        "is_executable": net_profit_lamports > 20_000
    }

# ==============================================================================
# 2. LIVE CROSS-POOL ARBITRAGE SCANNER (SOLANA DEXes)
# ==============================================================================

def fetch_json(url, timeout=8):
    headers = {"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"}
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode('utf-8'))

def scan_live_cross_pool_arbitrage():
    tokens = {
        "WIF": "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm",
        "BONK": "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB268",
        "POPCAT": "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr",
        "JUP": "JUPyiwrYJFngUPXSiqZFkKHMXkGs3WEXKaJE1agWK",
        "RAY": "4k3Dyjzvzp8eMZWUXbBCjEvwSkkU59Y5j3uYBqZ3e5pV",
        "SOL": "So11111111111111111111111111111111111111112"
    }
    
    opportunities = []
    
    for symbol, mint in tokens.items():
        try:
            url = f"https://api.dexscreener.com/latest/dex/tokens/{mint}"
            data = fetch_json(url)
            pairs = data.get("pairs", [])
            sol_pairs = [p for p in pairs if p.get("chainId") == "solana" and p.get("priceUsd")]
            
            by_quote = {}
            for p in sol_pairs:
                quote = p.get("quoteToken", {}).get("symbol", "").upper()
                if quote in ["SOL", "WSOL", "USDC", "USDT"]:
                    by_quote.setdefault(quote, []).append(p)
                    
            for quote, pool_list in by_quote.items():
                if len(pool_list) < 2:
                    continue
                for i in range(len(pool_list)):
                    for j in range(i + 1, len(pool_list)):
                        p1 = pool_list[i]
                        p2 = pool_list[j]
                        addr1 = p1.get("pairAddress")
                        addr2 = p2.get("pairAddress")
                        if addr1 == addr2:
                            continue
                            
                        price1 = float(p1.get("priceUsd", 0))
                        price2 = float(p2.get("priceUsd", 0))
                        liq1 = float(p1.get("liquidity", {}).get("usd", 0) or 0)
                        liq2 = float(p2.get("liquidity", {}).get("usd", 0) or 0)
                        
                        if price1 <= 0 or price2 <= 0 or liq1 < 1000 or liq2 < 1000:
                            continue
                            
                        diff_pct = abs(price1 - price2) / min(price1, price2) * 100.0
                        spread_bps = diff_pct * 100.0
                        
                        if spread_bps >= 20.0:
                            buy_p = p1 if price1 < price2 else p2
                            sell_p = p2 if price1 < price2 else p1
                            buy_dex = buy_p.get("dexId", "unknown")
                            sell_dex = sell_p.get("dexId", "unknown")
                            buy_price = min(price1, price2)
                            sell_price = max(price1, price2)
                            
                            safe_size_usd = min(1000.0, min(liq1, liq2) * 0.01)
                            if safe_size_usd < 25.0:
                                continue
                                
                            est_fees_usd = safe_size_usd * (0.0025 + 0.0025 + 0.00001) + 1.10
                            gross_profit_usd = safe_size_usd * (diff_pct / 100.0)
                            net_profit_usd = gross_profit_usd - est_fees_usd
                            
                            opportunities.append({
                                "token": symbol,
                                "quote": quote,
                                "buy_venue": buy_dex,
                                "sell_venue": sell_dex,
                                "buy_pool": buy_p.get("pairAddress"),
                                "sell_pool": sell_p.get("pairAddress"),
                                "buy_price": buy_price,
                                "sell_price": sell_price,
                                "spread_bps": round(spread_bps, 1),
                                "spread_pct": round(diff_pct, 3),
                                "safe_size_usd": round(safe_size_usd, 2),
                                "gross_profit_usd": round(gross_profit_usd, 2),
                                "est_fees_usd": round(est_fees_usd, 2),
                                "net_profit_usd": round(net_profit_usd, 2),
                                "is_executable": net_profit_usd > 0.05
                            })
        except Exception:
            continue
            
    return opportunities

if __name__ == "__main__":
    print("=" * 90)
    print("1. PUMP.FUN BOOST GRADUATION ARBITRAGE SIMULATION (/boost /goal)")
    print("=" * 90)
    
    print("\n--- MODEL 1: Protocol BOOST Absorbed (70% Capture of ~17.58 SOL TWAP Buyback) ---")
    boost_captured = []
    for s in [50.0, 55.0, 60.0, 65.0, 66.0, 70.0, 75.0, 80.0]:
        res = simulate_pump_boost(s, include_boost_buyback=True, capture_efficiency_bps=7000, is_flash_loan=True)
        if res:
            boost_captured.append(res)
            print(f"Reserves: {res['real_sol_reserves']:4.1f} SOL | Cost: {res['completion_cost_sol']:4.1f} SOL | Gross: {res['gross_sol_out']:5.2f} SOL | Net Profit: {res['net_profit_sol']:+7.4f} SOL (${res['net_profit_usd']:+6.2f}) | ROI: {res['roi_pct']:+5.2f}%")

    print("\n--- MODEL 2: Conservative Downside Risk Floor (0% Boost Realized, Immediate Dump) ---")
    boost_unrealized = []
    for s in [50.0, 55.0, 60.0, 65.0, 66.0, 70.0, 75.0, 80.0]:
        res = simulate_pump_boost(s, include_boost_buyback=False, capture_efficiency_bps=6500, is_flash_loan=True)
        if res:
            boost_unrealized.append(res)
            print(f"Reserves: {res['real_sol_reserves']:4.1f} SOL | Cost: {res['completion_cost_sol']:4.1f} SOL | Gross: {res['gross_sol_out']:5.2f} SOL | Downside Net: {res['net_profit_sol']:+7.4f} SOL (${res['net_profit_usd']:+6.2f})")

    print("\n" + "=" * 90)
    print("2. LIVE CROSS-POOL ARBITRAGE OPPORTUNITIES (REAL-TIME DEX QUOTES)")
    print("=" * 90)
    cross_ops = scan_live_cross_pool_arbitrage()
    if cross_ops:
        cross_ops.sort(key=lambda x: x["net_profit_usd"], reverse=True)
        print(f"Total cross-pool pairs analyzed: {len(cross_ops)}")
        print("\nTop Profitable Opportunities:")
        for op in cross_ops[:10]:
            print(f"[{op['token']}/{op['quote']}] Buy on {op['buy_venue']} (${op['buy_price']:.6f}) -> Sell on {op['sell_venue']} (${op['sell_price']:.6f})")
            print(f"   Spread: {op['spread_bps']} bps ({op['spread_pct']}%) | Sizing: ${op['safe_size_usd']} | Net Profit: +${op['net_profit_usd']:.2f}")
    else:
        print("No live cross-pool opportunities found above minimum spread threshold.")

    report = {
        "timestamp": time.time(),
        "pump_boost_captured": boost_captured,
        "pump_boost_downside_floor": boost_unrealized,
        "live_cross_pool_top": cross_ops[:15] if cross_ops else []
    }
    with open("arbitrage_report.json", "w") as f:
        json.dump(report, f, indent=2)
    print("\nFull report successfully written to arbitrage_report.json")
