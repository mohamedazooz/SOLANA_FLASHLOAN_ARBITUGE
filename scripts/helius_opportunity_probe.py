#!/usr/bin/env python3
"""Helius WS + Pump.fun/Raydium opportunity probe.

This script does two things:
1. Verifies that the Helius RPC/WebSocket endpoints are reachable with the key in env.
2. Runs the project's Pump.fun <-> Raydium closed-form opportunity model against
   sample pool states and prints whether a profitable trade is detected.

It is read-only and intentionally not an execution path.
"""

from __future__ import annotations

import asyncio
import json
import math
import os
import ssl
import subprocess
import sys
from urllib import request


def load_dotenv(path: str | None = None) -> None:
    candidates = [path] if path else [".env", os.path.join(os.path.dirname(__file__), ".env")]
    seen: set[str] = set()
    for candidate in candidates:
        try:
            with open(candidate, "r", encoding="utf-8") as handle:
                for raw in handle:
                    line = raw.strip()
                    if not line or line.startswith("#") or "=" not in line:
                        continue
                    key, value = line.split("=", 1)
                    key = key.strip()
                    value = value.strip()
                    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
                        value = value[1:-1]
                    if key and key not in seen:
                        os.environ.setdefault(key, value)
                        seen.add(key)
        except OSError:
            continue


def load_key() -> str:
    load_dotenv()
    for name in (
        "HELIUS_API_KEY",
        "helius_api_key",
        "HELIUS_API",
        "helius_api",
    ):
        value = os.getenv(name, "").strip()
        if value:
            if "api-key=" in value:
                return value.split("api-key=", 1)[1].split("&", 1)[0]
            return value
    return ""


def rpc_http(url: str) -> str:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "getHealth", "params": []}).encode()
    req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    with request.urlopen(req, timeout=20) as resp:
        return resp.read().decode("utf-8")


async def ws_probe(ws_url: str) -> str:
    try:
        import websockets
    except Exception:
        subprocess.check_call([sys.executable, "-m", "pip", "install", "websockets", "--quiet"])
        import websockets

    payload = {
        "jsonrpc": "2.0",
        "id": "1",
        "method": "accountSubscribe",
        "params": [
            "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
            {"encoding": "base64", "commitment": "processed"},
        ],
    }

    async with websockets.connect(ws_url, open_timeout=15, ping_interval=None) as ws:
        await ws.send(json.dumps(payload))
        try:
            msg = await asyncio.wait_for(ws.recv(), timeout=15)
            return str(msg)
        except asyncio.TimeoutError:
            return "WS_TIMEOUT"


DEFAULT_FLASH_LOAN_FEE_BPS = 30
DEFAULT_RAYDIUM_FEE_BPS = 25
MIN_NET_PROFIT_LAMPORTS = 30_000


def pump_fun_buy_cost(reserve: float, supply: float, delta_t: float) -> float:
    if delta_t <= 0 or reserve <= 0 or supply <= 0:
        return 0.0
    return (reserve * delta_t) / (supply + delta_t)


def raydium_sell_output(sol_reserve: float, token_reserve: float, delta_t: float, fee_bps: float) -> float:
    if delta_t <= 0 or sol_reserve <= 0 or token_reserve <= 0:
        return 0.0
    fee_factor = 10000.0 - fee_bps
    return (sol_reserve * delta_t * fee_factor) / ((token_reserve + delta_t) * 10000.0)


def calculate_optimal_trade(pump_reserve: float, pump_supply: float, ray_sol_reserve: float, ray_token_reserve: float, flash_fee_bps: float = DEFAULT_FLASH_LOAN_FEE_BPS, ray_fee_bps: float = DEFAULT_RAYDIUM_FEE_BPS, max_delta_t: float | None = None) -> dict:
    if pump_reserve <= 0 or pump_supply <= 0 or ray_sol_reserve <= 0 or ray_token_reserve <= 0:
        return {"ok": False, "reason": "zero reserves"}

    max_delta_t = max_delta_t if max_delta_t is not None else min(pump_supply, ray_token_reserve)
    max_delta_t = max(1.0, min(max_delta_t, pump_supply, ray_token_reserve))

    ray_fee = 10000.0 - min(ray_fee_bps, 10000.0)
    loan_factor = 10000.0 + min(flash_fee_bps, 10000.0)

    a = math.sqrt(ray_fee * ray_sol_reserve * ray_token_reserve)
    b = math.sqrt(loan_factor * pump_reserve * pump_supply)
    if not math.isfinite(a) or not math.isfinite(b) or a >= b:
        return {"ok": False, "reason": "no positive optimum"}

    numerator = (pump_supply * a) - (ray_token_reserve * b)
    denominator = b - a
    if denominator <= 0:
        return {"ok": False, "reason": "denominator <= 0"}

    delta_t = max(0.0, numerator / denominator)
    delta_t = min(delta_t, max_delta_t)
    if delta_t <= 0:
        delta_t = max_delta_t

    buy_cost = pump_fun_buy_cost(pump_reserve, pump_supply, delta_t)
    sell_output = raydium_sell_output(ray_sol_reserve, ray_token_reserve, delta_t, ray_fee_bps)
    flash_cost = buy_cost * (flash_fee_bps / 10000.0)
    total_cost = buy_cost + flash_cost + MIN_NET_PROFIT_LAMPORTS
    net_profit = sell_output - total_cost

    return {
        "ok": True,
        "delta_t": delta_t,
        "buy_cost": buy_cost,
        "sell_output": sell_output,
        "flash_cost": flash_cost,
        "total_cost": total_cost,
        "net_profit": net_profit,
        "profitable": net_profit > 0,
    }


def brute_force_sample_search() -> list[dict]:
    results: list[dict] = []
    for pump_reserve in [1_500_000_000.0, 5_000_000_000.0, 12_000_000_000.0, 30_000_000_000.0]:
        for pump_supply in [500_000_000.0, 1_000_000_000.0, 2_000_000_000.0, 4_000_000_000.0]:
            for ray_sol in [5_000_000_000.0, 15_000_000_000.0, 30_000_000_000.0]:
                for ray_token in [200_000_000_000.0, 500_000_000_000.0, 1_000_000_000_000.0]:
                    trade = calculate_optimal_trade(pump_reserve, pump_supply, ray_sol, ray_token)
                    if trade.get("ok") and trade["profitable"]:
                        results.append({
                            "pump_reserve": pump_reserve,
                            "pump_supply": pump_supply,
                            "ray_sol": ray_sol,
                            "ray_token": ray_token,
                            "net_profit": trade["net_profit"],
                            "delta_t": trade["delta_t"],
                        })
    results.sort(key=lambda x: x["net_profit"], reverse=True)
    return results[:10]


async def main() -> None:
    key = load_key()
    print("[helius-probe] key-present:", "yes" if key else "no")

    if not key:
        print("[helius-probe] No HELIUS_API_KEY / HELIUS_API env found. Skipping live-proof section.")
    else:
        rpc_url = f"https://mainnet.helius-rpc.com/?api-key={key}"
        ws_url = f"wss://mainnet.helius-rpc.com/?api-key={key}"
        try:
            rpc_body = rpc_http(rpc_url)
            print("[helius-rpc]", rpc_body[:400])
        except Exception as exc:
            print("[helius-rpc] error:", exc)

        try:
            ws_body = await ws_probe(ws_url)
            print("[helius-ws]", ws_body[:400])
        except Exception as exc:
            print("[helius-ws] error:", exc)

    print("\n[opportunity-model] running Pump.fun <-> Raydium closed-form search...")
    hits = brute_force_sample_search()
    if hits:
        for hit in hits[:3]:
            print(hit)
    else:
        print("No profitable sample found in the brute-force range used for this probe.")

    print("\n[opportunity-model] single demonstration from project-style reserves:")
    demo = calculate_optimal_trade(
        pump_reserve=30_000_000_000.0,
        pump_supply=1_073_000_000_000_000.0,
        ray_sol_reserve=15_000_000_000.0,
        ray_token_reserve=500_000_000_000.0,
    )
    print(demo)


if __name__ == "__main__":
    asyncio.run(main())
