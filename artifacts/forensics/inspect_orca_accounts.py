"""Read-only Orca account layout probe.

Derives the Whirlpool tick-array PDA locally (ed25519 off-curve check),
fetches the pool + tick array from the configured mainnet RPC, and prints
the byte offsets used by the Rust decoder. No secrets are printed.
"""

import base64
import hashlib
import json
import os
import sys
import urllib.request

ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
INDEX = {c: i for i, c in enumerate(ALPHABET)}

P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def b58decode(s: str) -> bytes:
    n = 0
    for c in s:
        n = n * 58 + INDEX[c]
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    pad = len(s) - len(s.lstrip("1"))
    return b"\x00" * pad + body


def b58encode(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    out = []
    while n:
        n, r = divmod(n, 58)
        out.append(ALPHABET[r])
    pad = len(b) - len(b.lstrip(b"\x00"))
    return "1" * pad + "".join(reversed(out))


def is_on_curve(compressed: bytes) -> bool:
    if len(compressed) != 32:
        return False
    y = int.from_bytes(compressed, "little") & ((1 << 255) - 1)
    if y >= P:
        return False
    y2 = (y * y) % P
    u = (y2 - 1) % P
    v = (D * y2 + 1) % P
    # x^2 = u / v must be a quadratic residue (or zero).
    x2 = (u * pow(v, P - 2, P)) % P
    if x2 == 0:
        return True
    return pow(x2, (P - 1) // 2, P) == 1


def find_pda(seeds, program_id: bytes):
    for bump in range(255, -1, -1):
        h = hashlib.sha256()
        for s in seeds:
            h.update(s)
        h.update(bytes([bump]))
        h.update(program_id)
        h.update(b"ProgramDerivedAddress")
        digest = h.digest()
        if not is_on_curve(digest):
            return b58encode(digest), bump
    raise RuntimeError("no PDA")


def rpc_url() -> str:
    env = os.path.join(os.path.dirname(__file__), "..", "..", ".env")
    with open(env, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line.startswith("SOLANA_RPC_PRIMARY="):
                return line.split("=", 1)[1].strip().strip('"').strip("'")
    raise RuntimeError("SOLANA_RPC_PRIMARY missing")


def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(
        rpc_url(), data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read())
    if "error" in payload:
        raise RuntimeError(payload["error"])
    return payload["result"]


def get_account(pubkey: str):
    res = rpc(
        "getAccountInfo",
        [pubkey, {"encoding": "base64", "commitment": "finalized"}],
    )
    if not res or not res.get("value"):
        return None
    value = res["value"]
    return {
        "owner": value["owner"],
        "lamports": value["lamports"],
        "data": base64.b64decode(value["data"][0]),
    }


def main():
    program_id = b58decode("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc")
    pool = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
    spacing = 4
    tick_current = -23621
    ticks_in_array = 88 * spacing
    base = (tick_current // ticks_in_array) * ticks_in_array
    print("base_start_tick", base)
    pda, bump = find_pda([b"tick_array", b58decode(pool), str(base).encode()], program_id)
    print("tick_array_pda", pda, "bump", bump)

    pool_acc = get_account(pool)
    print("pool_len", len(pool_acc["data"]), "owner", pool_acc["owner"])
    data = pool_acc["data"]
    print("pool[8..12]", data[8:12].hex())
    print("tick_spacing@41", int.from_bytes(data[41:43], "little"))
    print("fee_rate@45", int.from_bytes(data[45:47], "little"))
    print("liquidity@49", int.from_bytes(data[49:65], "little"))
    print("sqrt_price@65", int.from_bytes(data[65:81], "little"))
    print("tick_current@81", int.from_bytes(data[81:85], "little", signed=True))
    print("mint_a@101", b58encode(data[101:133]))
    print("vault_a@133", b58encode(data[133:165]))
    print("mint_b@181", b58encode(data[181:213]))
    print("vault_b@213", b58encode(data[213:245]))

    ta = get_account(pda)
    if ta is None:
        print("tick array account not found")
        return
    td = ta["data"]
    print("ta_len", len(td), "owner", ta["owner"])
    print("ta[0..8]", td[0:8].hex())
    print("start_tick@8", int.from_bytes(td[8:12], "little", signed=True))
    print("start_tick@40", int.from_bytes(td[40:44], "little", signed=True))
    print("tick0@12.initialized", td[12])
    print("tick0@12.net", int.from_bytes(td[13:29], "little", signed=True))
    print("tick0@12.gross", int.from_bytes(td[29:45], "little"))
    print("tick0@44.initialized", td[44])
    print("tick0@44.net", int.from_bytes(td[45:61], "little", signed=True))
    print("tick0@44.gross", int.from_bytes(td[61:77], "little"))
    print("whirlpool@9956", b58encode(td[9956:9988]))
    print("whirlpool@9952", b58encode(td[9952:9984]))


if __name__ == "__main__":
    main()
