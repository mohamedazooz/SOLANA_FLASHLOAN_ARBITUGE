# Operator Runbook

## 1. Daily Operations & Routine Checks
1. **Health Endpoint Inspection**:
   ```bash
   curl -s http://127.0.0.1:8080/health | jq .
   ```
2. **Prometheus Metrics Scrape**:
   ```bash
   curl -s http://127.0.0.1:8080/metrics | grep -E "arb_opportunities|arb_executions|arb_net_profit"
   ```
3. **Telegram Status Query**:
   Send `/status` in the private operator Telegram channel to view current mode, slot lag, and active breaker state.

## 2. Safe Parameter Tuning
To update risk limits without restarting the process:
- Use authenticated REST endpoint `/api/config`:
  ```bash
  curl -X POST http://127.0.0.1:8080/api/config \
    -H "Authorization: Bearer <TOKEN>" \
    -H "Content-Type: application/json" \
    -d '{"max_trade_size_usd": 10.0, "min_net_profit_usd": 0.50}'
  ```
- All mutating updates are logged to the persistent control audit log.
