# Known Limitations

Honest inventory of what this system does **not** do. Anything not proven is
listed here as NOT VERIFIED or BLOCKED — a green CI badge is not evidence.
Companion: [Remaining_Risks.md](Remaining_Risks.md) (risk-ranked, with owners).

## Structural limitations (by design)

- **No live broadcast ever performed.** `ARB_LIVE=0`; every dispatch path fails
  closed in shadow mode. Live requires explicit operator authorization per
  [../operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md](../operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md).
- **`DRY_RUN` is not a gate.** It is reported in the heartbeat
  (`crates/arb-bot/src/heartbeat.rs:245`) and nothing else. The real live gates
  are `ARB_LIVE` (`main.rs:76-79`), `LIVE`/`EXECUTE_LIVE` (`main.rs:140-142`),
  and `ARB_MODE` (CLI > env > config > `shadow` fallback). Verified 2026-11-13.
- **V1 over Jito is not configurable.** A `SOLANA_TX_V1_JITO_ENABLED` toggle
  was previously documented in `.env.example` but has no code consumer; only
  `SOLANA_TX_V1_DIRECT_ENABLED` is read (`supervisor/dispatch.rs:248`,
  `supervisor/opportunity.rs:766`). Removed from the template 2026-11-13.
  NOT VERIFIED: V1 bundle acceptance by Jito block engines end-to-end.
- **Yellowstone gRPC ingest is reserved, not wired.** `HELIUS_LASERSTREAM_ENDPOINT`
  is documented as a future path; actual ingest is the Helius WebSocket +
  provider-neutral WS pool. `SOLAMI_GRPC_ENDPOINT` has no consumer.
- **PostgreSQL without TLS.** TLS-requiring Postgres hosts fail closed at
  connect; supported topology is trusted-network (docker-compose / private
  VPC). See Remaining_Risks.
- **Files >1,000 lines remain** (`arb-tx/transport/compiler.rs` 1,490,
  `transport/master_alt.rs` 1,417, `arb-bot/supervisor/opportunity.rs` ~1,170).
  Each is single-responsibility; per `AGENTS.md` cohesion outranks line counts.

## NOT VERIFIED / BLOCKED (evidence-based gates)

| Item | Status | What would clear it |
|---|---|---|
| ≥24 h shadow soak on a persistent host | BLOCKED (operator action) | Run [../production/SHADOW_MODE_RUNBOOK.md](../production/SHADOW_MODE_RUNBOOK.md) on the VPS; publish drift report |
| Live V1/SWQoS broadcast end-to-end | BLOCKED (explicit approval required) | Canary per LIVE_PLAN with signed approval tied to the exact transaction |
| Solami WS / Alchemy keys accepted by providers | BLOCKED (provider-side; verified 2026-11-13 rejections) | Valid keys from the providers; see Remaining_Risks |
| V1-over-Jito bundle ingress | NOT VERIFIED | Wire a real toggle + block-engine probe, or drop the claim permanently |
| Full live 3-pool loop with real capital | NOT VERIFIED | All of the above + risk-limit sign-off |

## Environmental caveats

- Live-network tests skip cleanly when the upstream RPC returns 0 pools — an
  environmental/rate-limit outcome, not a code defect (documented in
  [TEST_EVIDENCE.md](TEST_EVIDENCE.md)).
- Linux-only build (yellowstone gRPC dependency); toolchain pinned to 1.99.1.
