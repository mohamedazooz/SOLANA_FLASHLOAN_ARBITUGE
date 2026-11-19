#!/usr/bin/env python3
"""
Archive and index all markdown documentation in the repository.
Preserves relative paths into docs/reference_archive/markdown_snapshot/
Generates MANIFEST.md and DOCUMENTATION_INDEX.md.
"""

import os
import shutil
import hashlib
from datetime import datetime

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
ARCHIVE_ROOT = os.path.join(REPO_ROOT, "docs", "reference_archive")
SNAPSHOT_DIR = os.path.join(ARCHIVE_ROOT, "markdown_snapshot")
MANIFEST_FILE = os.path.join(ARCHIVE_ROOT, "MANIFEST.md")
INDEX_FILE = os.path.join(ARCHIVE_ROOT, "DOCUMENTATION_INDEX.md")

EXCLUDE_DIRS = {
    ".git",
    "target",
    "node_modules",
    "reference_archive",
    ".system_generated",
}

def compute_sha256(filepath):
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()

def classify_document(rel_path, content_sample):
    norm = rel_path.replace("\\", "/").lower()
    sample_lower = content_sample.lower()
    
    # Subject classification
    if any(k in norm for k in ["pump", "bonding", "boost", "migration"]):
        subject = "Pump.fun / PumpSwap"
    elif any(k in norm for k in ["raydium", "orca", "meteora", "kamino", "jito", "protocol", "cpi", "dex"]):
        subject = "Protocol Integrations"
    elif any(k in norm for k in ["math", "formula", "constant", "sizing", "economic", "pnl", "profit"]):
        subject = "AMM Mathematics & Economics"
    elif any(k in norm for k in ["flashloan", "flash_loan", "borrow", "repay"]):
        subject = "Flash Loans"
    elif any(k in norm for k in ["ws", "websocket", "solami", "grpc", "yellowstone", "helius", "drpc", "provider", "ingest"]):
        subject = "Market Data & Ingestion"
    elif any(k in norm for k in ["execution", "pipeline", "route", "landing", "builder", "tx", "transaction"]):
        subject = "Execution & Landing"
    elif any(k in norm for k in ["risk", "safety", "guard", "integrity"]):
        subject = "Risk & State Integrity"
    elif any(k in norm for k in ["security", "forensic", "vulnerability", "audit"]):
        subject = "Security & Audits"
    elif any(k in norm for k in ["test", "fuzz", "regression", "fixture", "mock", "property"]):
        subject = "Testing & Verification"
    elif any(k in norm for k in ["deploy", "ops", "vps", "docker", "service", "systemd", "monitoring", "telegram", "dashboard"]):
        subject = "Deployment & Operations"
    elif any(k in norm for k in ["phase", "phase1", "phase2", "phase3", "phase4", "phase12", "phase13", "phase14", "phase15", "phase16", "checklist"]):
        subject = "Historical Phases & Roadmaps"
    elif any(k in norm for k in ["solana_hft_reference_pack", "atomic", "reference", "external"]):
        subject = "External References"
    elif any(k in norm for k in ["architecture", "overview", "component", "design"]):
        subject = "Architecture"
    else:
        subject = "Solana Infrastructure & General"

    # Status classification: current, historical, duplicate, conflicting, unverified
    if "yellowstone" in sample_lower and ("mandatory" in sample_lower or "required" in sample_lower or "geysergrpcclient" in sample_lower):
        status = "conflicting"  # conflicts with WS-only constraint
    elif "jupiter" in sample_lower and "jupiter route" in sample_lower:
        status = "conflicting"  # conflicts with Direct SDK execution constraint
    elif any(k in norm for k in ["audit_extract", "audit_work", "restore", "remediation", "forensic_baseline", "pr_body", "checklist-g1"]):
        status = "historical"
    elif any(k in norm for k in ["solana_hft_reference_pack"]):
        status = "unverified"
    elif norm in ["readme.md", "claude.md", "agents.md"]:
        status = "current"
    elif any(k in norm for k in ["phase1", "phase2", "phase3", "phase4", "phase5", "phase6", "phase7", "phase8", "phase9", "phase10", "phase11"]):
        status = "historical"
    else:
        status = "historical" if ("archive" in norm or "old" in norm or "v1" in norm) else "current"

    return subject, status

def main():
    print(f"Scanning markdown files in: {REPO_ROOT}")
    os.makedirs(SNAPSHOT_DIR, exist_ok=True)
    
    collected_files = []
    seen_hashes = {}
    today_str = datetime.now().strftime("%Y-%m-%d")

    for root, dirs, files in os.walk(REPO_ROOT):
        # Exclude directories
        dirs[:] = [d for d in dirs if d not in EXCLUDE_DIRS and not d.startswith(".git")]
        
        # Don't walk into docs/reference_archive
        rel_root = os.path.relpath(root, REPO_ROOT)
        if "docs" in rel_root.replace("\\", "/").split("/") and "reference_archive" in rel_root.replace("\\", "/").split("/"):
            continue

        for f in files:
            if f.lower().endswith((".md", ".mdx")):
                full_path = os.path.join(root, f)
                rel_path = os.path.relpath(full_path, REPO_ROOT).replace("\\", "/")
                
                # Check for exclusion
                if rel_path.startswith("docs/reference_archive"):
                    continue

                size = os.path.getsize(full_path)
                sha = compute_sha256(full_path)
                
                try:
                    with open(full_path, "r", encoding="utf-8", errors="ignore") as fh:
                        sample = fh.read(4096)
                except Exception:
                    sample = ""

                subject, status = classify_document(rel_path, sample)
                
                # Check duplicate
                if sha in seen_hashes:
                    status = "duplicate"
                else:
                    seen_hashes[sha] = rel_path

                # Target copy path
                target_path = os.path.join(SNAPSHOT_DIR, os.path.normpath(rel_path))
                os.makedirs(os.path.dirname(target_path), exist_ok=True)
                shutil.copy2(full_path, target_path)

                archived_rel = os.path.relpath(target_path, REPO_ROOT).replace("\\", "/")
                collected_files.append({
                    "orig_path": rel_path,
                    "archived_path": archived_rel,
                    "size": size,
                    "sha256": sha,
                    "date": today_str,
                    "status": status,
                    "subject": subject
                })

    print(f"Archived {len(collected_files)} markdown files into {SNAPSHOT_DIR}")

    # Generate MANIFEST.md
    print(f"Generating MANIFEST: {MANIFEST_FILE}")
    with open(MANIFEST_FILE, "w", encoding="utf-8") as mf:
        mf.write("# DOCUMENTATION REFERENCE ARCHIVE — MANIFEST\n\n")
        mf.write(f"- **Date Collected:** {today_str}\n")
        mf.write(f"- **Total Documents Archived:** {len(collected_files)}\n")
        mf.write(f"- **Archive Root:** `docs/reference_archive/markdown_snapshot/`\n")
        mf.write(f"- **Status Policy:** Documents are preserved without modification. They serve as reference and forensic evidence.\n\n")
        mf.write("| Original Path | Archived Path | Size (Bytes) | SHA-256 | Date | Status | Technical Subject |\n")
        mf.write("|---|---|---|---|---|---|---|\n")
        for item in sorted(collected_files, key=lambda x: x["orig_path"]):
            mf.write(f"| `{item['orig_path']}` | `{item['archived_path']}` | {item['size']} | `{item['sha256'][:12]}...` | {item['date']} | **{item['status'].upper()}** | {item['subject']} |\n")

    # Generate DOCUMENTATION_INDEX.md
    print(f"Generating DOCUMENTATION_INDEX: {INDEX_FILE}")
    categories = [
        "Architecture",
        "Solana Infrastructure & General",
        "Market Data & Ingestion",
        "Protocol Integrations",
        "Pump.fun / PumpSwap",
        "AMM Mathematics & Economics",
        "Flash Loans",
        "Execution & Landing",
        "Risk & State Integrity",
        "Security & Audits",
        "Testing & Verification",
        "Deployment & Operations",
        "Historical Phases & Roadmaps",
        "External References",
    ]
    
    with open(INDEX_FILE, "w", encoding="utf-8") as idx:
        idx.write("# DOCUMENTATION REFERENCE INDEX\n\n")
        idx.write("This index groups all historical, reference, and archived documentation by technical domain.\n")
        idx.write("Archived documents are strictly historical reference snapshots and NOT the authoritative runtime source of truth.\n\n")
        
        # Conflict summary
        conflicts = [item for item in collected_files if item["status"] == "conflicting"]
        if conflicts:
            idx.write("## ⚠️ Conflicting & Deprecated Architectural Documentation\n\n")
            idx.write("The following archived documents conflict with non-negotiable project architectural constraints (e.g., claiming Yellowstone gRPC is mandatory, or relying on Jupiter routing, or conflicting mathematical formulas):\n\n")
            for c in conflicts:
                idx.write(f"- [`{c['orig_path']}`](file:///{c['orig_path']}) (Archived: [`{c['archived_path']}`](file:///{c['archived_path']})) — Subject: {c['subject']}\n")
            idx.write("\n---\n\n")

        for cat in categories:
            items = [item for item in collected_files if item["subject"] == cat]
            idx.write(f"## {cat} ({len(items)} documents)\n\n")
            if not items:
                idx.write("_No documents in this category._\n\n")
                continue
            idx.write("| Original Document | Archived Copy | Status | Size |\n")
            idx.write("|---|---|---|---|\n")
            for item in sorted(items, key=lambda x: x["orig_path"]):
                idx.write(f"| [`{item['orig_path']}`](file:///{item['orig_path']}) | [`{item['archived_path']}`](file:///{item['archived_path']}) | `{item['status']}` | {item['size']} B |\n")
            idx.write("\n")

    print("Documentation consolidation completed successfully.")

if __name__ == "__main__":
    main()
