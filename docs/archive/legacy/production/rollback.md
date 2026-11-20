> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Production Rollback & Recovery Procedures

## 1. Emergency Halt Protocol
To instantly halt all trading activities:
1. **Durable Marker Touch**:
   ```bash
   touch /var/run/arb-bot/paused
   ```
2. **Web API Emergency Pause**:
   ```bash
   curl -X POST http://127.0.0.1:8080/api/pause -H "Authorization: Bearer <ADMIN_TOKEN>"
   ```
3. **Process Termination**:
   ```bash
   systemctl stop arb-bot
   ```

## 2. Binary Rollback Steps
If a newly deployed release exhibits unexpected latency or simulation failures:
1. Re-link current binary to previous known-good binary:
   ```bash
   ln -sfn /opt/solana-arb-bot/releases/v-previous /opt/solana-arb-bot/current
   ```
2. Re-verify configuration schema:
   ```bash
   /opt/solana-arb-bot/current/bin/validate --config /opt/solana-arb-bot/config/production.toml
   ```
3. Start service under supervised systemd unit:
   ```bash
   systemctl start arb-bot
   ```
4. Verify initial logs for clean startup without exceptions.
