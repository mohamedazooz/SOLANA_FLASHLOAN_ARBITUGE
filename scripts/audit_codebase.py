import os
import re

CRATES_DIR = "crates"

url_pattern = re.compile(r'https?://[^\s"\'\)\>]+')
ws_pattern = re.compile(r'wss?://[^\s"\'\)\>]+')
ip_port_pattern = re.compile(r'\b(?:127\.0\.0\.1|0\.0\.0\.0|localhost):\d+\b')
base58_pattern = re.compile(r'"[1-9A-HJ-NP-Za-km-z]{32,44}"')

hardcoded_urls = []
hardcoded_ws = []
hardcoded_pubkeys = []
hardcoded_ports = []

for root, dirs, files in os.walk(CRATES_DIR):
    for f in files:
        if f.endswith(".rs"):
            p = os.path.join(root, f)
            rel = os.path.relpath(p, CRATES_DIR)
            with open(p, "r", encoding="utf-8", errors="ignore") as fp:
                for idx, line in enumerate(fp, 1):
                    # ignore test modules for noise reduction
                    if "/tests/" in p.replace("\\", "/") or "\\tests\\" in p:
                        continue
                    
                    for m in url_pattern.finditer(line):
                        val = m.group(0)
                        if not val.startswith("http://127.0.0.1") and not val.startswith("http://localhost"):
                            hardcoded_urls.append((rel, idx, val, line.strip()))
                            
                    for m in ws_pattern.finditer(line):
                        val = m.group(0)
                        hardcoded_ws.append((rel, idx, val, line.strip()))
                        
                    for m in ip_port_pattern.finditer(line):
                        val = m.group(0)
                        hardcoded_ports.append((rel, idx, val, line.strip()))
                        
                    for m in base58_pattern.finditer(line):
                        val = m.group(0)
                        # don't flag if it's already in arb-constants
                        if "arb-constants" not in rel:
                            hardcoded_pubkeys.append((rel, idx, val, line.strip()))

print(f"=== HARDCODED WS ENDPOINTS ({len(hardcoded_ws)}) ===")
for r, i, v, l in hardcoded_ws:
    print(f"  {r}:{i} -> {v}")

print(f"\n=== HARDCODED HTTP URLS ({len(hardcoded_urls)}) ===")
for r, i, v, l in hardcoded_urls[:25]:
    print(f"  {r}:{i} -> {v}")

print(f"\n=== HARDCODED LOCALHOST/PORTS ({len(hardcoded_ports)}) ===")
for r, v in sorted(set((r, v) for r, i, v, l in hardcoded_ports)):
    print(f"  {r} -> {v}")

print(f"\n=== HARDCODED PUBKEYS OUTSIDE ARB-CONSTANTS ({len(hardcoded_pubkeys)}) ===")
unique_pks = set(v for r, i, v, l in hardcoded_pubkeys)
print(f"Unique pubkey strings found outside arb-constants: {len(unique_pks)}")
for pk in sorted(unique_pks)[:20]:
    # find where it occurs
    locs = [f"{r}:{i}" for r, i, v, l in hardcoded_pubkeys if v == pk]
    print(f"  {pk} in {locs[:3]}")
