import json, urllib.request, base64, os

env = {}
for line in open('.env', encoding='utf-8', errors='ignore'):
    line = line.strip()
    if line and not line.startswith('#') and '=' in line:
        k, _, v = line.partition('=')
        env[k.strip()] = v.strip().strip('"').strip(chr(39))

url = env['SOLANA_RPC_PRIMARY']
# Kamino USDC reserve (from prior probe)
reserve = 'D6q6wuQSrifJKZY4xuanECmZ1jzM8N6f0PzPe1SjwJ9o'
body = json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'getAccountInfo',
                   'params': [reserve, {'encoding': 'base64', 'commitment': 'confirmed'}]}).encode()
req = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'})
r = json.load(urllib.request.urlopen(req, timeout=30))
print('resp keys:', list(r.keys()))
if 'error' in r:
    print('error:', r['error'])
    raise SystemExit
d = base64.b64decode(r['result']['value']['data'][0])
print('reserve len', len(d))
print('decimals @264 (wrong):', d[264])
print('decimals @272 (correct):', d[272])
print('status @4856:', d[4856])
print('flash_fee_sf @4904:', int.from_bytes(d[4904:4912], 'little'))