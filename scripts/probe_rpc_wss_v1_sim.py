#!/usr/bin/env python3
"""Read-only RPC / WSS / V1-transport probe for the three-pool arbitrage engine.

This script never signs or broadcasts a transaction. It performs:
  1. RPC connectivity: getVersion / getSlot / getLatestBlockhash.
  2. On-chain state validation against the repo constants:
       - Kamino WSOL reserve discriminator + live flash_loan_fee_sf (U68F60)
         + liquidity mint at the deployed layout offsets used by arb-tx.
       - Kamino WSOL supply-vault SPL token balance = available flash liquidity.
       - Meteora DLMM SOL/USDC LbPair owner program.
  3. WSS connectivity: slotSubscribe and slot-latency sampling.
  4. V1 transport compatibility: submits unsigned V1 (SIMD-0385) simulation
     envelopes (produced by
     `cargo run -p arb-tx --example v1_mainnet_sim_envelope`) to
     `simulateTransaction` with maxSupportedTransactionVersion = 1,
     sigVerify = false, replaceRecentBlockhash = true.

Usage:
  cargo run -q -p arb-tx --example v1_mainnet_sim_envelope > /tmp/v1_envelopes.json
  python3 scripts/probe_rpc_wss_v1_sim.py /tmp/v1_envelopes.json
"""

from __future__ import annotations

import asyncio
import base64
import json
import os
import sys
import time
from urllib import request

DEFAULT_RPC_HTTP = "https://api.mainnet-beta.solana.com"
DEFAULT_RPC_WSS = "wss://api.mainnet-beta.solana.com"

WSOL_MINT = "So11111111111111111111111111111111111111112"
METEORA_DLMM_PROGRAM_ID = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
KAMINO_WSOL_RESERVE = "d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q"
KAMINO_WSOL_SUPPLY_VAULT = "GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U"
METEORA_SOL_USDC_POOL = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6"

KAMINO_RESERVE_DISCRIMINATOR = bytes([0x2B, 0xF2, 0xCC, 0xCA, 0x1A, 0xF7, 0x3B, 0x7F])
KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET = 8 + 120
KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET = 8 + 4896
EXPECTED_WSOL_FEE_SF = 11_529_215_046_068
SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET = 64


B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def base58_decode(raw: bytes) -> str:
    num = 0
    for byte in raw:
        num = num * 256 + byte
    encoded = ""
    while num > 0:
        num, rem = divmod(num, 58)
        encoded = B58_ALPHABET[rem] + encoded
    pad = len(raw) - len(raw.lstrip(b"\x00"))
    return B58_ALPHABET[0] * pad + encoded


def rpc_call(url: str, method: str, params: list) -> dict:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = request.Request(url, data=payload, headers={"Content-Type": "application/json"})
    with request.urlopen(req, timeout=30) as resp:
        body = json.loads(resp.read().decode())
    if "error" in body:
        raise RuntimeError(f"{method}: {body['error']}")
    return body["result"]


def get_account(url: str, address: str) -> tuple[str, bytes]:
    result = rpc_call(
        url,
        "getAccountInfo",
        [address, {"encoding": "base64", "commitment": "confirmed"}],
    )
    value = result["value"]
    if value is None:
        return "", b""
    data = base64.b64decode(value["data"][0])
    return value["owner"], data


def probe_rpc(url: str) -> dict:
    report: dict = {}
    report["rpc_version"] = rpc_call(url, "getVersion", [])
    report["rpc_slot_processed"] = rpc_call(url, "getSlot", [{"commitment": "processed"}])
    blockhash = rpc_call(url, "getLatestBlockhash", [{"commitment": "confirmed"}])
    report["blockhash_context_slot"] = blockhash["context"]["slot"]

    owner, reserve = get_account(url, KAMINO_WSOL_RESERVE)
    report["kamino_reserve_owner"] = owner
    report["kamino_reserve_discriminator_ok"] = reserve[:8] == KAMINO_RESERVE_DISCRIMINATOR
    fee_sf = int.from_bytes(
        reserve[KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET : KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET + 8],
        "little",
    )
    report["kamino_flash_loan_fee_sf_live"] = fee_sf
    report["kamino_flash_loan_fee_sf_matches_constant"] = fee_sf == EXPECTED_WSOL_FEE_SF
    liquidity_mint = base58_decode(
        reserve[
            KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET : KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET + 32
        ]
    )
    report["kamino_reserve_liquidity_mint"] = liquidity_mint
    report["kamino_reserve_liquidity_mint_is_wsol"] = liquidity_mint == WSOL_MINT

    _, vault = get_account(url, KAMINO_WSOL_SUPPLY_VAULT)
    available = int.from_bytes(
        vault[SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET : SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET + 8], "little"
    )
    report["kamino_wsol_available_flash_liquidity_lamports"] = available
    report["kamino_wsol_available_flash_liquidity_sol"] = available / 1e9

    owner, pool = get_account(url, METEORA_SOL_USDC_POOL)
    report["meteora_pool_owner_is_dlmm_program"] = owner == METEORA_DLMM_PROGRAM_ID
    report["meteora_pool_account_bytes"] = len(pool)
    return report


async def probe_wss(url: str, samples: int = 5) -> dict:
    import websockets

    slots: list[int] = []
    async with websockets.connect(url, max_size=2**22, open_timeout=20) as ws:
        sub_id = 1
        await ws.send(
            json.dumps({"jsonrpc": "2.0", "id": sub_id, "method": "slotSubscribe", "params": []})
        )
        deadline = time.monotonic() + 30
        while len(slots) < samples and time.monotonic() < deadline:
            msg = json.loads(await asyncio.wait_for(ws.recv(), timeout=15))
            if msg.get("id") == sub_id:
                if "error" in msg:
                    raise RuntimeError(f"slotSubscribe: {msg['error']}")
                continue
            result = msg.get("params", {}).get("result")
            slot = result.get("slot") if isinstance(result, dict) else result
            if isinstance(slot, int):
                slots.append(slot)
    return {
        "wss_url": url,
        "wss_connected": True,
        "slots_received": slots,
        "slot_advance_count": max(0, len(slots) - 1),
    }


def probe_v1_simulation(url: str, envelopes_path: str) -> dict:
    with open(envelopes_path, "r", encoding="utf-8") as handle:
        envelopes = json.load(handle)

    report: dict = {
        "baseline_wire_bytes": envelopes["baseline_wire_bytes"],
        "roundtrip_wire_bytes": envelopes["roundtrip_wire_bytes"],
        "flash_fee_lamports": envelopes["flash_fee_lamports"],
    }

    for name in ("baseline_base64", "roundtrip_base64"):
        params = [
            envelopes[name],
            {
                "encoding": "base64",
                "sigVerify": False,
                "replaceRecentBlockhash": True,
                "commitment": "processed",
                "maxSupportedTransactionVersion": 1,
            },
        ]
        try:
            result = rpc_call(url, "simulateTransaction", params)
            value = result.get("value", {})
            report[name] = {
                "accepted": True,
                "err": value.get("err"),
                "units_consumed": value.get("unitsConsumed"),
                "logs_head": (value.get("logs") or [])[:4],
            }
        except RuntimeError as exc:
            report[name] = {"accepted": False, "error": str(exc)}
    return report


def main() -> int:
    envelopes_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/v1_envelopes.json"
    http_url = os.environ.get("SOLANA_RPC_URL", DEFAULT_RPC_HTTP)
    wss_url = os.environ.get("SOLANA_RPC_WSS", DEFAULT_RPC_WSS)

    report: dict = {"generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}

    report["rpc"] = probe_rpc(http_url)
    try:
        report["wss"] = asyncio.run(probe_wss(wss_url))
    except Exception as exc:  # noqa: BLE001 - the probe must report, not crash
        report["wss"] = {
            "wss_url": wss_url,
            "wss_connected": False,
            "error": f"{type(exc).__name__}: {exc}",
        }
    report["v1_simulation"] = probe_v1_simulation(http_url, envelopes_path)

    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
