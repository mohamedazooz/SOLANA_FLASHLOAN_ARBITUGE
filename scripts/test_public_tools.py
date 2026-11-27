import urllib.request
import os
import json

TOKEN = os.environ.get("MCP_TEST_TOKEN", "").strip()
if not TOKEN:
    raise SystemExit("Set MCP_TEST_TOKEN to a fresh, scoped test bearer token.")
URL = "https://public-mcp.solscan.io/mcp"

tools_to_test = [
    ("chainInfoApi", {}),
    ("analyticsDexActivityApi", {}),
    ("analyticsFeesApi", {}),
    ("analyticsComputeUnitsApi", {}),
]

for name, args in tools_to_test:
    print(f"\nTesting {name}...")
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": args}
    }).encode("utf-8")
    req = urllib.request.Request(URL, data=body, headers={
        "Authorization": f"Bearer {TOKEN}",
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream"
    })
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            print(f"Success! Result:")
            print(json.dumps(data, indent=2)[:500])
    except Exception as e:
        print(f"Failed: {e}")
