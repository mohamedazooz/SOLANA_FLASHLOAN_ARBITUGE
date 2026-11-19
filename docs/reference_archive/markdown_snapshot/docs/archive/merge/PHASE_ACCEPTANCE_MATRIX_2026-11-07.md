# Phase acceptance matrix — 2026-11-07

| Stage | Scope | Status | Evidence still required |
|---|---|---|---|
| Baseline merge | Full workspace + Phase-09 remediation + remediation-v1 reference | Integrated archive prepared | Rust build/test on toolchain; compare with any unavailable prior Telegram/control-panel artifact |
| 09A.1 | CLMM/DLMM math fail-closed corrections | Code changes applied | Rust build/tests and differential fixtures |
| 09A.2 | Native CLMM/DLMM account adapters and dynamic array coverage | Not complete | Orca/Raydium/Meteora owner/PDA/layout fixtures; multi-array traversal |
| 09A.3 | Quote parity | Not complete | Official SDK/program differential test suite |
| 09A.4 | Instruction parity | Not complete | Account metas, tick/bin arrays, Token-2022 and simulation parity |
| 09B | Risk, sizing, split routing, cost ledger, settlement | Partial / staged | Unit contracts, replay, transaction-meta reconciliation |
| 09C | RPC + three WS slots + Kafka | Partial / staged | WebSocket transport adapters, failover tests, Kafka schemas and replay |
| Live gate | Real transaction execution | Disabled | All above accepted + isolated signer + independent security review |
