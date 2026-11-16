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
