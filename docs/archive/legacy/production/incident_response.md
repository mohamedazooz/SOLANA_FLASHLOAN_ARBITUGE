> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Incident Response & On-Call Playbook

## 1. Severity Levels

| Severity | Definition | Response Time | Action |
|---|---|---|---|
| **SEV-0** | Negative realized PnL or balance drain | IMMEDIATE (<2 min) | Activate durable kill switch; drain hot wallet to cold storage |
| **SEV-1** | Circuit breaker tripped; repeated simulation drops | <15 min | Inspect RPC latency, Yellowstone connection, and Jito tip floor |
| **SEV-2** | Provider slot lag >3 slots; failover active | <1 hour | Check primary RPC quota and network jitter |
| **SEV-3** | Non-critical metric alert (e.g. disk warning) | Next business day | Clean up old SQLite snapshots and rotated logs |

## 2. SEV-0 Immediate Action Checklist
1. Execute `touch /var/run/arb-bot/paused`.
2. Inspect `target/debug/deps/` or systemd logs: `journalctl -u arb-bot -n 200 -f`.
3. Check wallet balances on Solscan for the hot wallet address.
4. Transfer remaining hot wallet balance to operator cold storage address.
5. Do NOT restart the bot until root cause analysis (RCA) is completed and documented.
