import urllib.request
import json

TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJjcmVhdGVkQXQiOjE3OTUwOTA3MTE4OTgsImVtYWlsIjoibWVkb19zb2xkcjE1QHlhaG9vLmNvbSIsImFjdGlvbiI6InRva2VuLWFwaSIsImFwaVZlcnNpb24iOiJ2MiIsImlhdCI6MTc5NTA5MDcxMX0.qAcxzbutCvO6J5atOYTUajJkB3tgvSRa9kPWAd-a8q4"

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
