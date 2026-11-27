#!/usr/bin/env python3
"""
Solscan Pro v2 API CLI & Python SDK Utility for Solana Flashloan Arbitrage.

Usage:
    python scripts/solscan_tool.py check-key
    python scripts/solscan_tool.py token-meta So11111111111111111111111111111111111111112
    python scripts/solscan_tool.py token-markets EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm
    python scripts/solscan_tool.py portfolio 2Cby3xFT48ifATWhqDVmpnMCe4tiXvY8MtGFWYDfec7n
    python scripts/solscan_tool.py tx <SIGNATURE>
"""

import sys
import os
import json
import time
import argparse
import urllib.request
import urllib.error

DEFAULT_API_URL = "https://pro-api.solscan.io/v2.0"
DEFAULT_PUBLIC_MCP = "https://public-mcp.solscan.io/mcp"
DEFAULT_PRO_MCP = "https://mcp.solscan.io/mcp"


def load_api_key():
    # 1. Environment variable
    key = os.environ.get("SOLSCAN_API_KEY")
    if key and key.strip():
        return key.strip()

    # 2. .env files
    for p in [".env", "SOLANA_FLASHLOAN_ARBITUGE/.env", os.path.expanduser("~/.config/solscan-cli/config.json")]:
        if os.path.exists(p):
            try:
                with open(p, "r", encoding="utf-8") as f:
                    content = f.read()
                    if p.endswith(".json"):
                        data = json.loads(content)
                        if "api_key" in data:
                            return data["api_key"]
                    else:
                        for line in content.splitlines():
                            if line.strip().startswith("SOLSCAN_API_KEY="):
                                return line.split("=", 1)[1].strip().strip('"').strip("'")
            except Exception:
                pass
    return None
class SolscanSDK:
    def __init__(self, api_key=None, base_url=DEFAULT_API_URL):
        self.api_key = api_key or load_api_key()
        if not self.api_key:
            raise RuntimeError("Solscan API key missing; configure SOLSCAN_API_KEY or a local key file.")
        self.base_url = base_url.rstrip("/")
        self.last_req_time = 0.0

    def _request(self, endpoint, params=None):
        # Enforce 1 request per second
        now = time.time()
        elapsed = now - self.last_req_time
        if elapsed < 1.05:
            time.sleep(1.05 - elapsed)
        self.last_req_time = time.time()

        url = f"{self.base_url}/{endpoint.lstrip('/')}"
        if params:
            qs = "&".join(f"{k}={v}" for k, v in params.items())
            url += f"{'?' if '?' not in url else '&'}{qs}"

        req = urllib.request.Request(
            url,
            headers={
                "accept": "application/json",
                "token": self.api_key,
                "User-Agent": "Sentinel-HFT-SDK/1.0"
            }
        )
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                data = json.loads(resp.read().decode("utf-8"))
                return {"success": True, "status": resp.status, "data": data}
        except urllib.error.HTTPError as e:
            body = e.read().decode("utf-8", errors="ignore")
            try:
                parsed = json.loads(body)
            except Exception:
                parsed = {"raw": body}
            return {"success": False, "status": e.code, "error": parsed}
        except Exception as e:
            return {"success": False, "status": 0, "error": str(e)}

    def check_key(self):
        return self._request("token/meta", {"address": "So11111111111111111111111111111111111111112"})

    def token_meta(self, mint):
        return self._request("token/meta", {"address": mint})

    def token_markets(self, mint, page=1, page_size=10):
        return self._request("token/markets", {"address": mint, "page": page, "page_size": page_size})

    def portfolio(self, wallet):
        return self._request("account/portfolio", {"address": wallet})

    def transaction(self, tx_hash):
        return self._request("transaction/detail", {"tx": tx_hash})


def main():
    parser = argparse.ArgumentParser(description="Solscan Pro v2 API Toolkit")
    subparsers = parser.add_subparsers(dest="command")

    subparsers.add_parser("check-key", help="Check Solscan API key status")
    
    p_meta = subparsers.add_parser("token-meta", help="Get token metadata")
    p_meta.add_argument("mint", help="Token mint address")

    p_markets = subparsers.add_parser("token-markets", help="Get DEX pools for a token")
    p_markets.add_argument("mint", help="Token mint address")

    p_port = subparsers.add_parser("portfolio", help="Get wallet portfolio")
    p_port.add_argument("wallet", help="Solana wallet address")

    p_tx = subparsers.add_parser("tx", help="Get transaction detail")
    p_tx.add_argument("signature", help="Transaction signature")

    args = parser.parse_args()
    if not args.command:
        parser.print_help()
        sys.exit(0)

    sdk = SolscanSDK()
    if args.command == "check-key":
        res = sdk.check_key()
        print(json.dumps(res, indent=2))
    elif args.command == "token-meta":
        res = sdk.token_meta(args.mint)
        print(json.dumps(res, indent=2))
    elif args.command == "token-markets":
        res = sdk.token_markets(args.mint)
        print(json.dumps(res, indent=2))
    elif args.command == "portfolio":
        res = sdk.portfolio(args.wallet)
        print(json.dumps(res, indent=2))
    elif args.command == "tx":
        res = sdk.transaction(args.signature)
        print(json.dumps(res, indent=2))


if __name__ == "__main__":
    main()
