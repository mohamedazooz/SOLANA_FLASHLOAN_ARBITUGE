# AGENTS.md — Engineering Guide for AI Agents and Future Maintainers

Read this file before changing anything. It records the conventions, the
invariants that must never be broken, and the traps previous agents fell into.

## Ground rules

1. **One source of truth per operation.** There is exactly one discovery
   pipeline, one economic calculator (`arb-math`), one Kamino fee
   implementation (`arb-math/src/economic.rs`, re-exported by `arb-tx`), one
   execution entry point (`Supervisor`), one control service (Telegram and web
   both call `ControlService`). Do not add parallel implementations; extend the
   existing one.
2. **Never touch `main` directly.** Work on a feature branch, open a PR.
3. **Safety defaults are load-bearing**: `ARB_LIVE=0`, `DRY_RUN=true`,
   shadow mode. Any change that weakens these requires explicit operator
   approval and is out of bounds for autonomous agents.
4. **Secrets live only in the git-ignored `.env`** (Helius RPC/WSS keys,
   Sentinel, Privy). Never commit, echo, or log them. `.env.example` documents
   variable names with placeholders only.
5. **Verify before claiming.** Never report a test as passing unless you ran
   it. Never report a transaction as landed without an on-chain signature and
   status. Never report profit without realized balance deltas.

## Build / test / lint commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # 0 warnings policy
set -a; . ./.env; set +a                                 # for live-network tests
cargo test --workspace --all-targets
```

Toolchain is pinned (1.99.1, edition 2024, let-chains in use). Linux-only
build (yellowstone gRPC dep). Reference script: `scripts/verify-linux.sh`.

## Architecture map

See `docs/architecture/CODE_TREE.md` for the annotated file tree and
`docs/architecture/BUSINESS_LOGIC.md` for the business logic. The short
version:

- `arb-types` / `arb-constants` — leaf crates, no workspace deps.
- `arb-math` — all quote math + canonical economics (leaf crate).
- `arb-dex` — protocol adapters (decode + quote).
- `arb-ingest` — market data, provider supervision, freshness.
- `arb-engine` — route graph, discovery, sizing optimizer.
- `arb-risk` — risk gate, kill switch.
- `arb-tx` — instruction builders, V0/V1 compiler, flash-loan assemblers.
  Organized into `primitives/`, `protocols/`, `flash/`, `transport/`.
- `arb-sim` / `arb-land` / `arb-store` / `arb-metrics` — simulation, landing,
  persistence, telemetry.
- `arb-bot` — the binary: `main.rs` wires everything; `supervisor/` holds the
  run loop (`run.rs` → per-tick `strategy.rs` → `opportunity.rs` →
  `dispatch.rs`, plus `recovery.rs` for dispatch-intent reconciliation).

Dependency direction is strictly downward in the list above. `arb-tx` may
depend on `arb-math`; never the reverse.

## Invariants that must never break

- **Atomicity**: borrow + all swaps + repay in ONE transaction. No bundle
  splitting of a flash loan. `atomic_integrity::verify_atomic_integrity` is the
  checker — keep it wired.
- **Repayment math**: `repayment = principal + fee`, fee from the canonical
  `arb-math` klend formula (`fee_bits = max(amount*sf, 2^60)`, round
  ties-away). The `BorrowTooSmall` guard compares RAW bits pre-rounding —
  do not "simplify" it to compare the rounded fee.
- **Borrow sizing**: bounded by actual supply-vault liquidity decoded from the
  live reserve account, never by a config ceiling alone. "الاقتراض على حسب عمق
  البول" — if liquidity cannot be verified, skip the opportunity (fail closed).
- **Net profit**: enforced after ALL costs, integer math, mint-aware, no f64
  in authoritative decisions, no silent saturation that turns a loss into a
  profit. `MIN_NET_PROFIT_USD` is configurable (small targets allowed).
- **V1 transactions**: 0x81 discriminator, 4,096-byte wire ceiling, 64 inline
  accounts, no ALT, explicit CU limit / loaded-data limit / total-lamports
  priority fee in the message, `maxSupportedTransactionVersion: 1` on reads,
  base64 transport. Never silently downgrade V1→V0 or strip instructions to fit.
- **No stale fallbacks**: if the authoritative stream fails, fail closed —
  never serve cached market data as fresh.
- **Ambiguous dispatch ≠ failure**: on send timeout, check the signature
  status first (`supervisor/recovery.rs`); mark expired only when slot
  progression proves it (blockhash validity ≈ 150 slots, margin 160). Never
  blindly resubmit; the same signed tx is resent, never re-signed.
- **Fail-closed web auth**: the web API refuses to start without
  `WEB_ADMIN_PASSWORD`; binds localhost; reverse proxy terminates TLS.

## Known traps (learned the hard way)

- **Env vars are process-global.** Tests that touch `WEB_ADMIN_PASSWORD` must
  hold the `ENV_LOCK` (tokio Mutex in `web_api.rs` tests) or they flake under
  cargo's parallel test threads. `std::env::set_var` is `unsafe` in edition 2024.
- **Glob re-exports collide silently.** `arb-tx` groups re-export modules via
  `pub use module::*;`. `kamino` uses a CURATED explicit re-export list in
  `flash/mod.rs` because `kamino.rs` and `kamino_flash.rs` both define
  `build_kamino_flash_borrow/repay` (wrapper vs real builder). If you add a pub
  item to a re-exported module and hit E0659 ambiguity, extend the curated list
  or rename.
- **`use super::*` vs `use crate::*`**: files directly under `arb-tx/src/<group>/`
  use `use crate::*;`; original modules with their own test mods keep
  `use super::*;` in tests (test mods need the parent module's own items, which
  are not all re-exported at the crate root).
- **Evidence files are test-written.** `reports/phase15_2/*` JSONs are
  regenerated by tests (timestamps change). `git checkout -- <file>` instead of
  committing timestamp churn, unless the substance changed.
- **Live-network tests skip when the RPC returns 0 pools** — that is an
  environmental/rate-limit outcome, not a code bug. Source `.env` first.
- **`Supervisor.ingest.current_slot()` returns `u64`**, wrap in `Slot(..)`
  where the typed value is needed; store APIs take `i64`.
- **axum 0.8**: `middleware::from_fn` is `axum::middleware`,
  `DefaultBodyLimit` is `axum::extract`.
- **The `gh` token in CI cannot merge PRs or mutate the API** (pushes work).
  Merging is a human step in the GitHub UI.

## File organization policy

- Cohesion over line counts: split a file when two responsibilities live in it,
  not to hit a number. Current largest cohesive files (`compiler.rs` 1,490,
  `master_alt.rs` 1,417, `opportunity.rs` ~1,170) are single-responsibility.
- Rust convention: leaf modules as `name.rs` with a `mod.rs` only when the
  directory has submodules; keep items private by default; re-export curated
  public APIs.
- Docs follow a light Diátaxis split: `architecture/` (explanation + reference),
  `operations/` (how-to), `economics/` (domain explanation), `quality/`
  (evidence), `adr/` (decisions), `archive/` (frozen history — do not update,
  do not cite as current).
- When behavior changes, update `docs/PROJECT_STATE.md` and the relevant
  architecture doc in the same PR.

## Commit style

Conventional commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`),
focused units, no unrelated churn. PRs: draft by default, HEREDOC body files,
report the PR URL, ask before monitoring CI.
