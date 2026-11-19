import urllib.request
import json

TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJjcmVhdGVkQXQiOjE3OTUwOTA3MTE4OTgsImVtYWlsIjoibWVkb19zb2xkcjE1QHlhaG9vLmNvbSIsImFjdGlvbiI6InRva2VuLWFwaSIsImFwaVZlcnNpb24iOiJ2MiIsImlhdCI6MTc5NTA5MDcxMX0.qAcxzbutCvO6J5atOYTUajJkB3tgvSRa9kPWAd-a8q4"

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
