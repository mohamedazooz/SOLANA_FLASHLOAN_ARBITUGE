"""Validate the Orca TickArray decoder offsets against live mainnet accounts.

Checks:
- start_tick_index is at byte 8 (struct starts after the 8-byte Anchor discriminator)
- ticks start at byte 12, 113 bytes each
- the trailing whirlpool pubkey equals the pool id
- uninitialized ticks are fully zeroed (strict-parser assumption)
"""

import base64
import hashlib
import json
import os
import urllib.request

ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
INDEX = {c: i for i, c in enumerate(ALPHABET)}
P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def b58decode(s):
    n = 0
    for c in s:
        n = n * 58 + INDEX[c]
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    return b"\x00" * (len(s) - len(s.lstrip("1"))) + body


def b58encode(b):
    n = int.from_bytes(b, "big")
    out = []
    while n:
        n, r = divmod(n, 58)
        out.append(ALPHABET[r])
    return "1" * (len(b) - len(b.lstrip(b"\x00"))) + "".join(reversed(out))


def on_curve(c):
    y = int.from_bytes(c, "little") & ((1 << 255) - 1)
    if y >= P:
        return False
    y2 = y * y % P
    x2 = (y2 - 1) * pow(D * y2 + 1, P - 2, P) % P
    return x2 == 0 or pow(x2, (P - 1) // 2, P) == 1


def find_pda(seeds, program):
    for bump in range(255, -1, -1):
        h = hashlib.sha256()
        for s in seeds:
            h.update(s)
        h.update(bytes([bump]) + program + b"ProgramDerivedAddress")
        if not on_curve(h.digest()):
            return b58encode(h.digest())
    raise RuntimeError("no pda")


def rpc_url():
    env = os.path.join(os.path.dirname(__file__), "..", "..", ".env")
    for line in open(env, encoding="utf-8"):
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


def main():
    program = b58decode("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc")
    pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
    spacing = 4
    pool_data = account(pool)
    tick_current = int.from_bytes(pool_data[81:85], "little", signed=True)
    ticks_in_array = 88 * spacing
    base = (tick_current // ticks_in_array) * ticks_in_array
    print("live_tick_current", tick_current, "current_array_base", base)

    for start in [base, base - ticks_in_array, base + ticks_in_array]:
        pda = find_pda([b"tick_array", b58decode(pool), str(start).encode()], program)
        data = account(pda)
        if data is None:
            print(start, pda, "MISSING")
            continue
        start8 = int.from_bytes(data[8:12], "little", signed=True)
        start40 = int.from_bytes(data[40:44], "little", signed=True)
        trailing = b58encode(data[9956:9988])
        initialized = 0
        nonzero_uninit = 0
        for i in range(88):
            off = 12 + i * 113
            if data[off] == 1:
                initialized += 1
            elif data[off] != 0 or any(data[off + 1 : off + 33]):
                nonzero_uninit += 1
        print(
            start,
            pda,
            "len",
            len(data),
            "start@8",
            start8,
            "start@40",
            start40,
            "trailing_pool_ok",
            trailing == pool,
            "initialized",
            initialized,
            "nonzero_uninit",
            nonzero_uninit,
        )


if __name__ == "__main__":
    main()
