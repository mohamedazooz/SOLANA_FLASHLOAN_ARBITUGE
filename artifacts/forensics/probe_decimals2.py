import base64, json, os, urllib.request

def load_rpc():
    with open('.env', encoding='utf-8') as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith('#') or '=' not in line:
                continue
            k, _, v = line.partition('=')
            if k.strip() == 'SOLANA_RPC_PRIMARY':
                return v.strip().strip('"').strip("'")
    raise SystemExit('no rpc')

url = load_rpc()
body = json.dumps({'jsonrpc':'2.0','id':1,'method':'getAccountInfo','params':['d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q',{'encoding':'base64','commitment':'finalized'}]}).encode()
req = urllib.request.Request(url, data=body, headers={'Content-Type':'application/json'})
r = json.load(urllib.request.urlopen(req, timeout=60))
d = base64.b64decode(r['result']['value']['data'][0])
print('len', len(d))
print('byte @264:', d[264])
print('byte @272:', d[272])
print('u64 @264:', int.from_bytes(d[264:272], 'little'))
print('u64 @272:', int.from_bytes(d[272:280], 'little'))