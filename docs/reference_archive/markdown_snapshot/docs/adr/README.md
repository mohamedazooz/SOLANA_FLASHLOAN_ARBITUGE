# Architecture Decision Records

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-remove-arb-live-gate.md) | Remove ARB_LIVE env gate and default shadow mode | proposed | 2026-11-09 |
| [0002](0002-kill-switch-persistence.md) | Modify KillSwitch to not auto-block on marker file | proposed | 2026-11-09 |
| [0003](0003-protocol-guards-retention.md) | Retain MTU/blockhash/min-out protocol guards | accepted | 2026-11-09 |
| [0004](0004-risk-gate-bypass-config.md) | Allow risk gate bypass via env (already implemented) | accepted | 2026-11-09 |
| [0005](0005-simulation-policy.md) | Keep at least one simulation (LiteSVM) before send | accepted | 2026-11-09 |
| [0006](0006-stream-readiness-guard.md) | Weaken final stream-readiness guard to warning-only | proposed | 2026-11-09 |
| [0007](0007-enable-pumpswap-adapter.md) | Build PumpSwap adapter with IDL parity | proposed | 2026-11-09 |
| [0008](0008-enable-jupiter-live.md) | Enable Jupiter live path with quote parity | proposed | 2026-11-09 |
| [0009](0009-split-routing-implementation.md) | Implement split routing with per-leg re-quote | proposed | 2026-11-09 |
| [0010](0010-guard01-quote-guarded-wiring.md) | Wire guarded quote as sole builder/dispatch path | proposed | 2026-11-09 |
| [0011](0011-durable-execution-state-machine.md) | Build durable execution state machine with reconciliation | proposed | 2026-11-09 |
| [0012](0012-fork-simulation-integration.md) | Integrate fork simulation for complete route | proposed | 2026-11-09 |
| [0013](0013-dex-account-resolution-and-builders.md) | Dynamic On-Chain Account Resolution & DEX Instruction Builders | accepted | 2026-11-09 |