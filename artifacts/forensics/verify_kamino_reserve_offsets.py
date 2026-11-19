"""Read-only live verification of klend Reserve offsets used by arb-tx.

Loads SOLANA_RPC_PRIMARY from .env without printing it, fetches the WSOL
main-market reserve and its supply vault, and prints only public on-chain
fields. No transaction is signed or sent.
"""

from __future__ import annotations

import base64
import json
import os
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

RESERVE = "d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q"
WSOL_MINT = "So11111111111111111111111111111111111111112"
EXPECTED_SUPPLY = "GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U"
EXPECTED_FEE = "3JNof8s453bwG5UqiXBLJc77NRQXezYYEBbk3fqnoKph"


def load_rpc() -> str:
    with open(os.path.join(ROOT, ".env"), encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, _, value = line.partition("=")
            if key.strip() == "SOLANA_RPC_PRIMARY":
                return value.strip().strip('"').strip("'")
    raise SystemExit("SOLANA_RPC_PRIMARY not found in .env")


def rpc(url: str, method: str, params):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    ).encode()
    req = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json", "User-Agent": "read-only-audit"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read())


def b58decode(s: str) -> bytes:
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    n = 0
    for ch in s:
        n = n * 58 + alphabet.index(ch)
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    pad = len(s) - len(s.lstrip("1"))
    return b"\x00" * pad + raw


def b58encode(b: bytes) -> str:
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    n = int.from_bytes(b, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = alphabet[r] + out
    pad = len(b) - len(b.lstrip(b"\x00"))
    return "1" * pad + (out or "")


def main() -> int:
    url = load_rpc()
    acct = rpc(
        url,
        "getAccountInfo",
        [RESERVE, {"encoding": "base64", "commitment": "finalized"}],
    )
    value = acct["result"]["value"]
    data = base64.b64decode(value["data"][0])
    print(f"reserve owner        : {value['owner']}")
    print(f"reserve data length  : {len(data)}")
    print(f"reserve lamports     : {value['lamports']}")

    disc = data[0:8]
    print(f"discriminator        : {list(disc)}")
    print(f"version (8..16)      : {int.from_bytes(data[8:16], 'little')}")

    mint = b58encode(data[128:160])
    supply = b58encode(data[160:192])
    fee = b58encode(data[192:224])
    print(f"liquidity.mint @128  : {mint}  (wsol={mint == WSOL_MINT})")
    print(f"supply_vault  @160   : {supply}  (expected={supply == EXPECTED_SUPPLY})")
    print(f"fee_vault     @192   : {fee}  (expected={fee == EXPECTED_FEE})")

    decimals = data[272]
    status = data[4856]
    fee_sf = int.from_bytes(data[4904:4912], "little")
    print(f"mint_decimals @272   : {decimals}")
    print(f"config.status @4856  : {status}  (active={status == 0})")
    print(f"flash_loan_fee_sf    : {fee_sf}  (u64::MAX={fee_sf == 2**64 - 1})")
    print(f"fee as fraction      : {fee_sf / 2**60:.12f}")
    print(f"fee per 1 SOL (lamps): {round(fee_sf * 1_000_000_000 / 2**60)}")

    vault = rpc(
        url,
        "getAccountInfo",
        [supply, {"encoding": "base64", "commitment": "finalized"}],
    )
    vdata = base64.b64decode(vault["result"]["value"]["data"][0])
    v_mint = b58encode(vdata[0:32])
    v_owner = b58encode(vdata[32:64])
    v_amount = int.from_bytes(vdata[64:72], "little")
    print(f"supply vault owner   : {vault['result']['value']['owner']}")
    print(f"supply vault mint    : {v_mint}  (wsol={v_mint == WSOL_MINT})")
    print(f"supply vault owner pk: {v_owner}")
    print(f"supply vault amount  : {v_amount} lamports = {v_amount / 1e9:.9f} SOL")
    return 0


if __name__ == "__main__":
    sys.exit(main())
