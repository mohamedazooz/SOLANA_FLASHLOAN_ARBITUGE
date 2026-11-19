"""Dump raw Orca TickArray bytes for layout confirmation (read-only)."""

import base64
import json
import os
import urllib.request

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def rpc_url():
    for line in open(os.path.join(ROOT, ".env"), encoding="utf-8"):
        line = line.strip()
        if line.startswith("SOLANA_RPC_PRIMARY="):
            return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise RuntimeError("missing rpc")


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(rpc_url(), data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read())
    if "error" in payload:
        raise RuntimeError(payload["error"])
    return payload["result"]


def account(pubkey):
    res = rpc("getAccountInfo", [pubkey, {"encoding": "base64", "commitment": "finalized"}])
    value = res.get("value") if res else None
    if not value:
        return None
    return base64.b64decode(value["data"][0])


d = account("HME5BJtcqSa2Y1sLPXrftqZ9HRwKV4bGhnw41qw5Leka")
vals = [d[12 + i * 113] for i in range(88)]
print("len", len(d))
print("start@8", int.from_bytes(d[8:12], "little", signed=True))
print("b40", int.from_bytes(d[40:44], "little", signed=True))
print("init_byte_hist", {v: vals.count(v) for v in sorted(set(vals))})
print("tick0_hex", d[12:12 + 113].hex())
print("net0", int.from_bytes(d[13:29], "little", signed=True))
print("gross0", int.from_bytes(d[29:45], "little"))
print("net1", int.from_bytes(d[126:142], "little", signed=True))
print("gross1", int.from_bytes(d[142:158], "little"))
print("last_init_byte", d[12 + 87 * 113])
print("trailing", d[9956:9988].hex())
