import urllib.request
import os
import json

TOKEN = os.environ.get("MCP_TEST_TOKEN", "").strip()
if not TOKEN:
    raise SystemExit("Set MCP_TEST_TOKEN to a fresh, scoped test bearer token.")
def test_mcp(url):
    print(f"\n--- Testing MCP endpoint: {url} ---")
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }).encode("utf-8")

    req = urllib.request.Request(url, data=body, headers={
        "Authorization": f"Bearer {TOKEN}",
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream"
    })

    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            print("Status:", resp.status)
            res = json.loads(resp.read().decode("utf-8"))
            tools = res.get("result", {}).get("tools", [])
            print(f"Total tools returned: {len(tools)}")
            for t in tools:
                print(f"  * {t.get('name')}: {t.get('description', '')[:70]}")
            return tools
    except urllib.error.HTTPError as e:
        print(f"HTTPError {e.code}: {e.read().decode('utf-8')}")
    except Exception as e:
        print("Error:", e)
    return []

if __name__ == "__main__":
    test_mcp("https://public-mcp.solscan.io/mcp")
    test_mcp("https://mcp.solscan.io/mcp")
