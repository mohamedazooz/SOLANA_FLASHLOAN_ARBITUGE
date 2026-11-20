> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Forensic Baseline — 2026-11-16

All findings below were independently verified against the actual repository on
branch `main`, HEAD `45031bb2bce6aeb9a0de3dff03a2f1daa76309a0`. No third-party
audit report was trusted without re-execution.

## Repository facts

| Item | Value |
|---|---|
| URL | https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE |
| Default branch | `main` (public repo, `private: false`) |
| HEAD at baseline | `45031bb` — "Merge pull request #35 from mohamedazooz/feature/dehardcode-config-merge" |
| Working tree | clean, no uncommitted or untracked work |
| Open PRs | none |
| CI | "Rust CI" green on `main` (run 43790364529) |
| Workspace | `crates/*` (14 crates) + standalone `webhook-service` |
| Rust LOC (crates/) | ~103,285 lines |
| Toolchain | pinned `1.99.1` via `rust-toolchain.toml` (rustfmt + clippy components) |
| Edition | 2024, `rust-version = "1.99"` |

## Verification gates executed at baseline

| Gate | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | PASS (exit 0) |
| Compile | `cargo check --workspace --all-targets` | PASS (exit 0, 1m28s) |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0, zero warnings) |
| Tests | `cargo test --workspace` | PASS (exit 0) — **1017 passed, 0 failed, 2 ignored** |

The 2 ignored tests are `crates/arb-tx/tests/mainnet_alt_integration_tests.rs`
(`test_live_alt_compiles_v0_and_measures_bytes`,
`test_live_alt_validates_and_warmup`), annotated
`#[ignore = "live RPC; run with --ignored"]` — intentional live-network tests,
not suppressed failures.

## Prior-remediation state

The repository has already absorbed multiple full remediation campaigns
(`docs/archive/audit_reports/` 00–18, `docs/audit/`, `reports/phase15_*`,
`reports/phase16`, `reports/production`). Key verified-in-place properties:

- **Live execution is double-gated**: `mode live` requires
  `execution.live_enabled == true` **and** truthy `ARB_LIVE=1`
  (`crates/arb-config/src/validation.rs:348`,
  `crates/arb-bot/src/control_service.rs:534-536`). Defaults are
  `default_safe()` with `live_enabled = false`.
- **Net-profit gate (not gross)**: `crates/arb-math/src/chronos.rs:516`
  enforces `net_profit_raw >= min_profit_raw` after all costs.
- **Layered validated config**: `config/base.toml` + env overlays with
  `config.schema.json`, `config.example.toml`, `.env.example` present.
- **No committed secrets**: pattern scan over `*.rs|*.toml|*.json|*.ts|*.tsx`
  for private-key/mnemonic literals returned zero hits outside
  `docs/archive/`.

## Baseline conclusion

This is not a broken prototype. The correct engineering posture for this pass
is **verification-first**: re-run every gate independently, probe the critical
financial paths for real defects, and only change code where a defect is
demonstrated — not reorganize a working 100k-LOC workspace.
