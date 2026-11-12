# Source decomposition status — 2026-11-07

## Completed in this increment: Supervisor

The former 2,114-line `crates/arb-bot/src/supervisor.rs` was decomposed into four focused modules. The 1,042-line Pump state joiner was also reduced below the limit by moving its test module into `pump_state_tests.rs`. No `include!` fragments, generated wrappers, or duplicate execution paths were introduced.

| File | Lines | Responsibility |
|---|---:|---|
| `crates/arb-bot/src/supervisor.rs` | 519 | State, constructor, market snapshot reporting |
| `crates/arb-bot/src/supervisor/run.rs` | 938 | Slot lifecycle, ingestion, discovery, economics and risk prechecks |
| `crates/arb-bot/src/supervisor/dispatch.rs` | 534 | Mode dispatch, preflight, Jito/SWQoS send and settlement tracking |
| `crates/arb-bot/src/supervisor/execution.rs` | 195 | Instruction assembly, route resolution and verified inputs |

The extraction preserves the `Supervisor` type and its public `run()` entry point. Private helper methods are exposed only to the parent module via `pub(super)`.

## Remaining Rust source files above 1,000 lines

These files remain above the requested threshold and are explicitly not claimed as refactored:

| File | Lines | Proposed responsibility split |
|---|---:|---|
| `crates/arb-tx/src/lib.rs` | 3,372 | instruction types, Pump builders, DEX builders, Kamino flash, Jupiter, transaction serialization, templates |
| `crates/arb-ingest/src/registry.rs` | 1,419 | Raydium decoders, Orca decoders, Meteora decoders, generic registry and promotion |
| `crates/arb-engine/src/lib.rs` | 1,342 | graph storage, pool edge construction, base-cycle search, multi-hop search, Bellman-Ford |
| `crates/arb-types/src/lib.rs` | 1,264 | amounts/units, IDs, pool state, route/opportunity, risk/execution state, errors |
| `crates/arb-math/src/dlmm.rs` | 1,115 | bin-array decoding, price/bin math, exact-in/out quote, traversal and tests |
| `crates/arb-math/src/clmm.rs` | 1,095 | tick math, sqrt-price math, swap-step, tick-array traversal and tests |

## Refactor rules for the remaining files

1. Extract one responsibility at a time; do not split by arbitrary line ranges.
2. Preserve public re-exports at the original crate/module boundary while migrating call sites.
3. Do not use `include!` to disguise file length or duplicate source inclusion.
4. Use typed context structs for cross-module state rather than long positional argument lists.
5. Prove dead code with call-graph and feature-gated reference searches before removal.
6. Run workspace check/tests/clippy after each crate-level extraction. No API-moving extraction should be marked complete without a successful Rust build.

## Validation boundary

Supervisor decomposition passed static delimiter checks, but was not compiler-verified. Cargo/rustc/rustfmt and Docker are not installed in the current execution environment. The next agent with a Rust toolchain must run the documented acceptance commands before merging or deploying.
