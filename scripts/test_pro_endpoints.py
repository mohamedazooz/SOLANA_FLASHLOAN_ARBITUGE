import urllib.request
import os
import json

TOKEN = os.environ.get("MCP_TEST_TOKEN", "").strip()
if not TOKEN:
    raise SystemExit("Set MCP_TEST_TOKEN to a fresh, scoped test bearer token.")
endpoints = [
    "https://pro-api.solscan.io/v2.0/chaininfo",
    "https://pro-api.solscan.io/v2.0/block/last",
    "https://pro-api.solscan.io/v2.0/token/trending",
    "https://pro-api.solscan.io/v2.0/monitor/usage",
]

for url in endpoints:
    print(f"Testing {url}...")
    req = urllib.request.Request(url, headers={"token": TOKEN, "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            print("OK:", resp.status, resp.read().decode("utf-8")[:100])
    except urllib.error.HTTPError as e:
        print(f"HTTPError {e.code}: {e.read().decode('utf-8')[:120]}")
    except Exception as e:
        print(f"Err: {e}")
