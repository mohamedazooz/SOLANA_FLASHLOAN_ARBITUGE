import urllib.request
import json

TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJjcmVhdGVkQXQiOjE3OTUwOTA3MTE4OTgsImVtYWlsIjoibWVkb19zb2xkcjE1QHlhaG9vLmNvbSIsImFjdGlvbiI6InRva2VuLWFwaSIsImFwaVZlcnNpb24iOiJ2MiIsImlhdCI6MTc5NTA5MDcxMX0.qAcxzbutCvO6J5atOYTUajJkB3tgvSRa9kPWAd-a8q4"

def call_tool(endpoint, tool_name, arguments):
    print(f"\n--- Calling {tool_name} on {endpoint} ---")
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    }).encode("utf-8")

    req = urllib.request.Request(endpoint, data=body, headers={
        "Authorization": f"Bearer {TOKEN}",
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream"
    })

    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            print("Status:", resp.status)
            res = json.loads(resp.read().decode("utf-8"))
            print("Response:", json.dumps(res, indent=2)[:800])
            return res
    except urllib.error.HTTPError as e:
        print(f"HTTPError {e.code}: {e.read().decode('utf-8')}")
    except Exception as e:
        print("Error:", e)

if __name__ == "__main__":
    # Test monitorUsageApi on Pro MCP
    call_tool("https://mcp.solscan.io/mcp", "monitorUsageApi", {})

    # Test chainInfoApi on Public MCP
    call_tool("https://public-mcp.solscan.io/mcp", "chainInfoApi", {})

    # Test tokenPriceApi on Pro MCP for SOL
    call_tool("https://mcp.solscan.io/mcp", "tokenPriceApi", {
        "address": "So11111111111111111111111111111111111111112"
    })
