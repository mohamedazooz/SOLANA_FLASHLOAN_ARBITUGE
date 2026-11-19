import urllib.request
import json

TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJjcmVhdGVkQXQiOjE3OTUwOTA3MTE4OTgsImVtYWlsIjoibWVkb19zb2xkcjE1QHlhaG9vLmNvbSIsImFjdGlvbiI6InRva2VuLWFwaSIsImFwaVZlcnNpb24iOiJ2MiIsImlhdCI6MTc5NTA5MDcxMX0.qAcxzbutCvO6J5atOYTUajJkB3tgvSRa9kPWAd-a8q4"
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
