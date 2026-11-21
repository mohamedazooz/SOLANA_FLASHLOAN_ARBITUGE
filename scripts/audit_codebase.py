#!/usr/bin/env python3
"""
Production Hardcoded & Security Audit Scanner for Solana Flashloan Arbitrage Engine.

Distinguishes between real production technical debt and test/mock fixtures.
Enforces the Zero Hardcoded Secrets & Endpoints Policy.
"""

import os
import re
import sys

CRATES_DIR = "crates"

url_pattern = re.compile(r'https?://[^\s"\'\)\>]+')
ws_pattern = re.compile(r'wss?://[^\s"\'\)\>]+')
ip_port_pattern = re.compile(r'\b(?:127\.0\.0\.1|0\.0\.0\.0|localhost):\d+\b')
base58_pattern = re.compile(r'"[1-9A-HJ-NP-Za-km-z]{32,44}"')

prod_ws = []
prod_urls = []
prod_ports = []
prod_pubkeys = []

test_ws = []
test_urls = []
test_ports = []
test_pubkeys = []

for root, dirs, files in os.walk(CRATES_DIR):
    for f in files:
        if not f.endswith(".rs"):
            continue
        p = os.path.join(root, f)
        rel = os.path.relpath(p, CRATES_DIR)
        
        is_test_file = (
            "/tests/" in p.replace("\\", "/") or
            "\\tests\\" in p or
            "/examples/" in p.replace("\\", "/") or
            "\\examples\\" in p or
            "/benches/" in p.replace("\\", "/") or
            "\\benches\\" in p or
            "arb-idl-gen" in p
        )
        
        in_test_block = False
        with open(p, "r", encoding="utf-8", errors="ignore") as fp:
            for idx, line in enumerate(fp, 1):
                stripped = line.strip()
                if "#[cfg(test)]" in stripped or "mod tests " in stripped or "mod test " in stripped:
                    in_test_block = True
                    
                target_ws = test_ws if (is_test_file or in_test_block) else prod_ws
                target_urls = test_urls if (is_test_file or in_test_block) else prod_urls
                target_ports = test_ports if (is_test_file or in_test_block) else prod_ports
                target_pks = test_pubkeys if (is_test_file or in_test_block) else prod_pubkeys
                
                for m in ws_pattern.finditer(line):
                    val = m.group(0)
                    target_ws.append((rel, idx, val, stripped))
                    
                for m in url_pattern.finditer(line):
                    val = m.group(0)
                    if not val.startswith("http://127.0.0.1") and not val.startswith("http://localhost"):
                        target_urls.append((rel, idx, val, stripped))
                        
                for m in ip_port_pattern.finditer(line):
                    val = m.group(0)
                    target_ports.append((rel, idx, val, stripped))
                    
                for m in base58_pattern.finditer(line):
                    val = m.group(0)
                    if "arb-constants" not in rel:
                        target_pks.append((rel, idx, val, stripped))

print("=====================================================================")
print("     SOLANA FLASHLOAN ARBITRAGE — CODEBASE HARDCODED AUDIT")
print("=====================================================================")

print(f"\n[+] REAL PRODUCTION TECHNICAL DEBT (Hot Paths):")
print(f"    - Hardcoded Production WS Endpoints: {len(prod_ws)}")
for r, i, v, _ in prod_ws:
    print(f"      * {r}:{i} -> {v}")

print(f"    - Hardcoded Production Localhost/Ports: {len(prod_ports)}")
for r, v in sorted(set((r, v) for r, i, v, _ in prod_ports)):
    print(f"      * {r} -> {v}")

unique_prod_pks = sorted(set(v for _, _, v, _ in prod_pubkeys))
print(f"    - Unique Production Pubkeys outside arb-constants: {len(unique_prod_pks)}")
for pk in unique_prod_pks[:20]:
    locs = [f"{r}:{i}" for r, i, v, _ in prod_pubkeys if v == pk]
    print(f"      * {pk} in {locs[:2]}")
if len(unique_prod_pks) > 20:
    print(f"      ... and {len(unique_prod_pks) - 20} more.")

print(f"\n[i] TEST & FIXTURE NOISE (Cold Paths / Isolated):")
print(f"    - Test Mocks WS Endpoints: {len(test_ws)}")
print(f"    - Test Mocks Localhost/Ports: {len(test_ports)}")
unique_test_pks = set(v for _, _, v, _ in test_pubkeys)
print(f"    - Unique Test/Mock Pubkeys: {len(unique_test_pks)}")

print("\n=====================================================================")
print(f"AUDIT SUMMARY: {len(prod_ws)} Prod WS | {len(prod_ports)} Prod Ports | {len(unique_prod_pks)} Prod Pubkeys")
print("=====================================================================")
