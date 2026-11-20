> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# OPERATIONS — INCIDENT HANDLING & EMERGENCY PROCEDURES

**Author:** QA / Security / DevOps Engineer  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. Emergency Kill Switch Protocols

When anomalous market conditions, RPC desynchronization, or unexplained losses occur, the operator must trigger the Emergency Kill Switch immediately:

### Option A: Via Web Dashboard (One-Click)
1. Open `http://127.0.0.1:8080/dashboard`.
2. Click the red **🚨 زر الطوارئ (EMERGENCY STOP)** button.
3. Confirm the prompt. The bot transitions to `PAUSED` mode instantly.

### Option B: Via Command Line / File Trigger
```powershell
# Create the kill switch file on VPS or local host
New-Item -ItemType File -Force -Path "./data/kill_switch"
```
Or on Linux:
```bash
touch ./data/kill_switch
```
The supervisor detects the marker within 100ms and suspends all transaction dispatches.

### Option C: Via Telegram Control
Send `/stop` or `/pause` from an authorized operator chat ID.

---

## 2. Common Incident Runbooks

### Incident 1: High Slot Lag / Provider Desync
- **Symptom:** `arb_slot_current` lags behind public cluster slot by $> 10$ slots.
- **Action:**
  1. Trigger provider rotation via `/api/control/rotate_provider`.
  2. If all providers are lagging, pause the bot.
  3. Verify provider status on Helius / Solami status pages.

### Incident 2: Flash Loan Transaction Reverting On-Chain
- **Symptom:** Transaction lands with error status (e.g. slippage exceeded or Kamino error).
- **Impact:** Only priority fee and Jito tip are lost; flashloan capital is 100% protected by atomicity.
- **Action:**
  1. Inspect transaction error logs via Solscan or Helius explorer.
  2. If caused by slippage, verify `MAX_SLIPPAGE_BPS` configuration and market volatility.
  3. Check whether competing searcher sandwiched the intermediate hop.

### Incident 3: Persistent HTTP 429 Rate Limits
- **Symptom:** `rpc_pool_concurrent_429` alerts firing.
- **Action:**
  1. Verify `RPC_MAX_CONCURRENT_REQUESTS` is $\le 8$ and `RPC_MIN_INTERVAL_MS` is $\ge 50$.
  2. Rotate API keys or add fallback provider slots in `.env`.
