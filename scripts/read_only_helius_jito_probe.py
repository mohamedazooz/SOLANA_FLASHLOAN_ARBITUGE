#!/usr/bin/env python3
"""Read-only Helius + Jito sanity check with unsigned transaction preview.

This script intentionally never signs or broadcasts a real Solana transaction.
It performs:
  - Helius RPC connectivity checks
  - Helius WS sanity check
  - Jito public validator API reachability check
  - Pump.fun <-> Raydium closed-form opportunity math
  - An unsigned transaction template preview for research only
"""

from __future__ import annotations

import asyncio
import base64
import json
import math
import os
import subprocess
import sys
from urllib import error, request


def load_env_file() -> None:
    candidates = [
        ".env",
        os.path.join(os.path.dirname(__file__), ".env"),
    ]
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


def load_helius_key() -> str:
    load_env_file()
    for name in ("HELIUS_API_KEY", "helius_api_key", "HELIUS_API", "helius_api"):
        value = os.getenv(name, "").strip()
        if not value:
            continue
        if "api-key=" in value:
            return value.split("api-key=", 1)[1].split("&", 1)[0]
        return value
    return ""


def http_json(url: str, *, method: str = "GET", headers: dict | None = None, payload: dict | None = None, timeout: int = 20) -> dict:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    req = request.Request(url, data=data, headers=headers or {}, method=method)
    with request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode("utf-8", "ignore")
        if not raw:
            return {}
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {"raw": raw}


async def ws_helius_probe(ws_url: str) -> str:
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
            message = await asyncio.wait_for(ws.recv(), timeout=15)
            return str(message)
        except asyncio.TimeoutError:
            return "WS_TIMEOUT"


def pump_fun_buy_cost(reserve: float, supply: float, delta_t: float) -> float:
    if delta_t <= 0 or reserve <= 0 or supply <= 0:
        return 0.0
    return (reserve * delta_t) / (supply + delta_t)


def raydium_sell_output(sol_reserve: float, token_reserve: float, delta_t: float, fee_bps: float) -> float:
    if delta_t <= 0 or sol_reserve <= 0 or token_reserve <= 0:
        return 0.0
    fee_factor = 10000.0 - fee_bps
    return (sol_reserve * delta_t * fee_factor) / ((token_reserve + delta_t) * 10000.0)


def calculate_optimal_trade(
    pump_reserve: float,
    pump_supply: float,
    ray_sol_reserve: float,
    ray_token_reserve: float,
    flash_fee_bps: float = 30.0,
    ray_fee_bps: float = 25.0,
    min_net_profit_lamports: float = 30_000.0,
    max_delta_t: float | None = None,
) -> dict:
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
    total_cost = buy_cost + flash_cost + min_net_profit_lamports
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


def unsigned_tx_preview() -> dict:
    program_id = "11111111111111111111111111111111"
    blockhash = "<recent_blockhash_from_rpc>"
    ix_data = base64.b64encode(b"dry-run-only-unsigned-preview").decode("ascii")
    return {
        "unsigned": True,
        "simulationOnly": True,
        "note": "No wallet private key used. This is a dry-run template only; not valid for broadcast.",
        "version": "0",
        "feePayer": "<wallet_public_key>",
        "recentBlockhash": blockhash,
        "instructions": [
            {
                "programId": program_id,
                "accounts": [],
                "data": ix_data,
            }
        ],
        "signers": [],
    }


async def main() -> None:
    key = load_helius_key()
    print("[helius-probe] key-present:", "yes" if key else "no")

    if key:
        rpc_url = f"https://mainnet.helius-rpc.com/?api-key={key}"
        ws_url = f"wss://mainnet.helius-rpc.com/?api-key={key}"

        rpc_payload = {"jsonrpc": "2.0", "id": 1, "method": "getHealth", "params": []}
        health = http_json(rpc_url, method="POST", payload=rpc_payload, headers={"Content-Type": "application/json"})
        print("[helius-rpc] getHealth:", health)

        pool_payload = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getAccountInfo",
            "params": [
                "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
                {"encoding": "base64"},
            ],
        }
        pool_info = http_json(rpc_url, method="POST", payload=pool_payload, headers={"Content-Type": "application/json"})
        print("[helius-rpc] getAccountInfo:", pool_info)

        ws_msg = await ws_helius_probe(ws_url)
        print("[helius-ws] sample message:", ws_msg[:500])
    else:
        print("[helius-probe] No HELIUS_API_KEY detected. Skipping live Helius calls.")

    jito_headers = {"User-Agent": "Mozilla/5.0"}
    try:
        jito = http_json("https://kobe.mainnet.jito.network/api/v1/validators", headers=jito_headers, timeout=20)
        validators = jito.get("validators", []) if isinstance(jito, dict) else []
        print("[jito] reachable:", bool(validators), "count=", len(validators))
        if validators:
            sample = validators[0]
            print("[jito] sample validator:", {k: sample.get(k) for k in ("identity_account", "running_jito", "mev_commission_bps")})
    except Exception as exc:
        print("[jito] error:", exc)

    demo = calculate_optimal_trade(
        pump_reserve=30_000_000_000.0,
        pump_supply=1_073_000_000_000_000.0,
        ray_sol_reserve=15_000_000_000.0,
        ray_token_reserve=500_000_000_000.0,
    )
    print("[opportunity] demo:", demo)

    tx_template = unsigned_tx_preview()
    print("[unsigned-tx] preview:")
    print(json.dumps(tx_template, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
