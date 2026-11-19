"""Read-only probe: Raydium AMM v4 pool `status` + fee fields from live mainnet.

Prints no secrets: the RPC URL and key are never echoed.
"""

import base64
import json
import os
import sys
import urllib.request

POOLS = {
    "SOL/USDC": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
    "RAY/USDC": "6UmmUiYoBjSrhakAobJw8BvkmJtDVxaeBtbt7rxWo1mg",
    "SOL/USDT": "7XawhbbxtsRcQA8KTkHT9f9nc6d69UwqCDh6U5EEbEmX",
}


def load_env():
    here = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    path = os.path.join(here, ".env")
    values = {}
    if os.path.exists(path):
        with open(path, "r", encoding="utf-8", errors="ignore") as handle:
            for line in handle:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, _, value = line.partition("=")
                values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        payload = json.load(response)
    if "error" in payload:
        raise RuntimeError(f"rpc error: {payload['error'].get('message')}")
    return payload["result"]


def u64(data, off):
    return int.from_bytes(data[off : off + 8], "little")


def pubkey(data, off):
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    raw = data[off : off + 32]
    num = int.from_bytes(raw, "big")
    out = ""
    while num:
        num, rem = divmod(num, 58)
        out = alphabet[rem] + out
    for byte in raw:
        if byte:
            break
        out = "1" + out
    return out or "1" * 32


def main():
    env = load_env()
    url = env.get("SOLANA_RPC_PRIMARY")
    if not url:
        print("SOLANA_RPC_PRIMARY missing", file=sys.stderr)
        return 2
    for label, pool in POOLS.items():
        result = rpc(
            url,
            "getAccountInfo",
            [pool, {"encoding": "base64", "commitment": "confirmed"}],
        )
        if not result or not result.get("value"):
            print(f"{label}: account not found")
            continue
        value = result["value"]
        data = base64.b64decode(value["data"][0])
        print(
            f"{label}: len={len(data)} owner={value['owner']} "
            f"status={u64(data, 0)} "
            f"swap_fee={u64(data, 176)}/{u64(data, 184)} "
            f"trade_fee={u64(data, 144)}/{u64(data, 152)} "
            f"need_take_pnl_coin={u64(data, 192)} need_take_pnl_pc={u64(data, 200)}"
        )
        print(
            f"    coin_vault={pubkey(data, 336)} pc_vault={pubkey(data, 368)} "
            f"coin_mint={pubkey(data, 400)} pc_mint={pubkey(data, 432)}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
