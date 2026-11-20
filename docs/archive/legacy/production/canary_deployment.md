> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Controlled Canary Deployment Runbook

## 1. Canary Constraints & Micro-Limits
Canary deployment represents the first stage of real on-chain execution, strictly gated by explicit operator authorization.

| Constraint | Canary Value | Production Value | Enforcement Mechanism |
|---|---|---|---|
| **Mode** | `canary` | `live` | `config/production.toml` |
| **Max Trade Size** | **$5.00 - $10.00** | $50.00 - $250.00 | `arb-risk/policy.rs` Check #5 |
| **Max Daily Loss** | **$15.00** | $50.00 | `arb-risk/policy.rs` Check #12 |
| **Max Hourly Loss** | **$5.00** | $20.00 | `arb-risk/policy.rs` Check #13 |
| **Max Single Tip** | **0.01 SOL ($2.00)** | 0.05 SOL | `arb-land/bidder.rs` |
| **Max Tip Profit Share**| **30%** | 40% | `arb-land/bidder.rs` |
| **Max Inflight Bundles** | **1** | 3 | `arb-risk/policy.rs` Check #14 |
| **Allowed Pools** | Meteora DLMM SOL/USDC only | All Whitelisted Pools | `config/protocols.toml` |

## 2. Canary Activation Checklist (Operator Only)
- [ ] 1. Verify hot wallet balance does not exceed 0.1 SOL (~$15.00).
- [ ] 2. Ensure `KILL_SWITCH_DISABLED=0` and marker directory is writable.
- [ ] 3. Verify Yellowstone gRPC stream is connected with 0 dropped slots.
- [ ] 4. Verify Jito Tip Floor WebSocket is streaming fresh bids.
- [ ] 5. Confirm Web Admin password is set to a secure, non-default string.
- [ ] 6. Operator executes explicit `/resume` or launches with `--mode canary`.
- [ ] 7. Monitor first 5 executions in Telegram alert channel.
- [ ] 8. Automatic fail-closed halt if any execution produces unexpected balance delta.
