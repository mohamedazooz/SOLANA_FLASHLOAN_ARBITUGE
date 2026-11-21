# Phase 1B — Durable Kill Switch and Panic Alert Hardening

## Scope

This work package addresses the runtime kill-switch persistence contract and Telegram panic-alert credential handling. It is isolated from the Phase 1A dashboard/login transport branch.

## Implemented

- Startup restores a blocked state when the configured marker path already exists.
- Activation engages the in-memory block before filesystem work. It reports durable success (`Ok(true)`), memory-only operation (`Ok(false)`), or persistence failure (`Err`); failure never clears the in-memory block.
- Marker writes use `write_all` and `sync_all` before reporting durable success.
- Deactivation removes the marker before clearing the in-memory block. A removal error preserves the block and returns an error.
- ControlService disables execution before activating the kill switch; `start` enables execution only after successful marker removal.
- Emergency-stop latch and execution disable are engaged before marker persistence is attempted.
- Status API exposes marker presence and the last persistence error. The dashboard displays these states.
- Admin pause/resume endpoints return service-unavailable on persistence failures instead of reporting success.
- Panic alert handling no longer invokes `curl`, does not place Telegram credentials in process arguments, redacts known token/chat values from the panic message, and does not chain the default panic hook (which would print the unredacted payload).

## Operational semantics

- A missing persistence path is explicitly memory-only. `stop`/`emergency_stop` returns an error to avoid implying restart-safe protection; the in-memory execution block remains engaged.
- A persistence failure is fail-closed for the current process, but cannot guarantee protection across a process restart. The error is surfaced in status and the control response.
- Panic notification is best-effort and is scheduled only when a Tokio runtime is available during unwinding. No notification is guaranteed outside a runtime.

## Verification

- `cargo test -p arb-risk`: 17 passed, 0 failed after the marker fsync refinement.
- `cargo check -p arb-bot`: passed before the final marker fsync refinement.
- `cargo test -p arb-bot --lib`: 83 passed, 0 failed before the final marker fsync refinement.
- Final `cargo test -p arb-risk`: 17 passed, 0 failed. Final `cargo test -p arb-bot --lib`: 84 passed, 0 failed. Final `cargo clippy -p arb-bot --all-targets`: passed. Frontend `npm run build` (`tsc -b && vite build`): passed. `git diff --check`: passed.
- No live transaction, signing, broadcast, deployment, or asset movement was performed.

## Remaining gate

This phase has passed the listed local gates and is ready for a final diff review and remote PR creation. No PR has been opened yet.
