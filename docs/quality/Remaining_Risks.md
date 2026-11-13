# Remaining Risks

Honest, severity-ranked. Nothing here is hidden behind a green CI badge.

## HIGH

### R-A — No ≥24 h shadow soak on a persistent host
- **Risk:** predicted-vs-observed drift (fees, landing rates, CU usage) is
  unproven over a full trading day; memory/CPU growth unproven long-term.
- **Mitigation:** runbook ready (`production/SHADOW_MODE_RUNBOOK.md`);
  shadow-only builds cannot dispatch (feature-gated signer removal).
- **Owner:** operator (VPS/docker deployment).

### R-B — Live V1/SWQoS transport unproven end-to-end
- **Risk:** V1 acceptance by the full production transport path (validator
  + RPC provider) is verified offline and by probe, not by a real broadcast.
- **Mitigation:** fail-closed `ARB_LIVE=0`; canary plan in
  `operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md` requires explicit approval
  tied to the exact transaction.

## MEDIUM

### Postgres `NoTls`
- TLS-requiring PostgreSQL hosts fail closed at connect. Supported topology is
  trusted-network (docker-compose / private VPC). Extending to rustls is a
  documented, isolated change in `postgres_store.rs`.

### Live PostgreSQL in CI
- Store SQL is exercised via SQLite with identical schema semantics; a live
  PostgreSQL service in CI would close the last integration gap.

## LOW

### R-F — Unmaintained transitive dependencies
- ansi_term, bincode 1.x, derivative, libsecp256k1, paste. No upstream fixes;
  no runtime exposure identified. Tracked via `cargo audit` (allowed).

### Rate-limit exposure (single provider)
- With one Helius endpoint configured, 429s pause RPC-dependent paths (observed
  honestly in health sweeps). Adding a second provider restores full rotation.

## INFO

### R-H — Fork validation is snapshot-based
- `arb-sim/fork_sim.rs` + `mainnet_non_broadcast_simulation` exercise real
  mainnet state without broadcast; a persistent forked validator would
  strengthen borrow/repay evidence but needs infrastructure.
