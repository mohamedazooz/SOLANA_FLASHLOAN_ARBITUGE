# Production Deployment & SRE Infrastructure

## 1. Production Deployment Topology
- **Host Target**: Dedicated bare-metal or high-performance VPS (Ubuntu 22.04/24.04 LTS).
- **Network Locality**: Low-latency co-location (e.g. Frankfurt, Amsterdam, or Salt Lake City) adjacent to Solana validators and Jito Block Engines.
- **Process Supervision**: Systemd unit with watchdog timer and automatic log rotation.

## 2. Systemd Service Deployment
Deploy the service unit to `/etc/systemd/system/arb-bot.service`:

```ini
[Unit]
Description=Solana Flashloan Arbitrage Engine
After=network.target

[Service]
Type=simple
User=solana
WorkingDirectory=/opt/solana-arb-bot
ExecStart=/opt/solana-arb-bot/bin/arb-bot --config /opt/solana-arb-bot/config/production.toml
Restart=on-failure
RestartSec=5s
LimitNOFILE=65535
EnvironmentFile=/opt/solana-arb-bot/.env

[Install]
WantedBy=multi-user.target
```

## 3. Log Rotation & Disk Safety
Logs are managed via `logrotate` to prevent disk exhaustion:
```
/var/log/arb-bot/*.log {
    daily
    missingok
    rotate 14
    compress
    delaycompress
    notifempty
    create 0640 solana solana
}
```
