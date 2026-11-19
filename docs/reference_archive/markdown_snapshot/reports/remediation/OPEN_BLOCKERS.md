# Open Blockers & Production Operational Prerequisites

## 1. Code & Architectural Status
All architectural, financial, statistical, and code defects identified in the mandate have been resolved:
- Critical USD / lamport conversion mismatch: **RESOLVED**.
- Jito post-selection tip clipping: **RESOLVED**.
- $P_{50} = 50\%$ statistical assumption: **RESOLVED**.
- Configuration & default drift: **RESOLVED**.
- Priority fee calculation & V1 absolute fee semantics: **RESOLVED**.
- All workspace compiler errors, warnings, and tests: **RESOLVED (0 errors, 0 warnings, 624/624 tests passing)**.

---

## 2. Operational Prerequisites Before Live Mainnet Broadcast

The following items are intentional operational boundaries requiring human operator action before live mainnet trading:

| # | Item | Severity | Impact | Required Operator Action |
|---|---|---|---|---|
| 1 | Mainnet RPC / gRPC Secrets | High | Bot cannot stream live mainnet pools without authorized credentials | Operator must provision valid `HELIUS_API_KEY` and `SHYFT_API_KEY` in git-ignored `.env` file |
| 2 | Keypair Provisioning | High | Bot cannot sign or submit transactions in live mode without funded keypair | Operator must provision funded local keypair at `solana_keypair.json` with >= 0.05 SOL reserve |
| 3 | Operator Live Flag Authorization | High | Live trading blocked by fail-closed gate | Operator must explicitly set `[bot] mode = "live"`, `live_enabled = true` in `config/production.toml` and export `ARB_LIVE=1` |
| 4 | GitHub Pull Request Merge | Medium | Remediation branch needs operator merge to `main` | Operator reviews PR in GitHub UI and merges `fix/production-economic-engine-hardening` |
