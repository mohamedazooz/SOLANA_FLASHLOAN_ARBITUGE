# OPERATIONS — OBSERVABILITY, METRICS & DASHBOARD

**Author:** QA / Security / DevOps Engineer  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. Observability Architecture

The Sentinel bot exposes operational observability through three synchronized interfaces:

1. **Prometheus Metrics Endpoint:** Accessible at `/metrics` (served by `arb-metrics` and `arb-bot`).
2. **Authenticated Web Dashboard:** React + TypeScript web app and lightweight SSR dashboard at `http://127.0.0.1:8080/dashboard`.
3. **Telegram Operational Alerts:** Real-time push notifications for trade executions, rejections, and health warnings.

---

## 2. Core Operational Metrics

| Metric Name | Type | Description | Alert Threshold |
|---|---|---|---|
| `arb_slot_current` | Gauge | Current cluster high-water slot observed | Lag $> 8$ slots |
| `arb_slot_gaps_total` | Counter | Count of skipped slots observed across feeds | $> 5$ per minute |
| `arb_opportunities_discovered_total` | Counter | Total candidate cycles discovered | 0 for $> 30$ mins |
| `arb_opportunities_rejected_total` | Counter (by reason) | Opportunities rejected by risk/economic gates | Informational |
| `arb_transactions_submitted_total` | Counter | Total transactions dispatched to Jito/RPC | Informational |
| `arb_transactions_landed_total` | Counter | Transactions confirmed landed on-chain | Landing rate $< 40\%$ |
| `arb_realized_profit_lamports_total` | Counter | Cumulative realized net profit in lamports | Decrement indicates loss |
| `arb_latency_opportunity_to_submit_us` | Histogram | End-to-end latency from WS event to wire submit | P95 $> 25\text{ ms}$ |

---

## 3. Web Dashboard Interfaces

- **Lightweight Standalone Dashboard:** Served directly from `crates/arb-bot/src/web_api.rs` at `/dashboard`. Requires zero npm/Node dependencies. Localized in both Arabic and English with full visual guide.
- **Full React Dashboard:** Built with Vite + Tailwind CSS under `web/` with interactive charts, real-time WebSocket telemetry, and configuration management.
- **Access Control:** Authenticated via `WEB_ADMIN_PASSWORD` (session token via `/api/login`).
