# Remediation Plan — PR #8 to Production round (2026-11-13)

Status legend: **Applied** = fix merged into this branch with passing tests.
**Verified-correct** = audited, no change required. **Open** = remaining,
with severity and exact blocker. **Blocked** = cannot be executed in this
environment; documented honestly.

## 1. Applied this round

| ID | Severity | Fix | Location | Test |
|---|---|---|---|---|
| F1 | HIGH | Dispatch-intent persistence made fail-closed: signature must be extractable and the intent durably persisted BEFORE send; otherwise abort with `dispatch_aborted_unreconcilable` / `dispatch_aborted_unpersisted` + Telegram alert | `supervisor/dispatch.rs:503-551` | `cargo test -p arb-bot` 51/51 |
| F2 | HIGH | Fire-time kill-switch/emergency gate immediately before any network submission (stop issued during simulation/slot-wait now wins) | `supervisor/dispatch.rs:451-463`; `ControlService::is_emergency_stopped` (control_service.rs:387-389) | arb-bot suite green |
| F3 | MEDIUM | Fail-closed flash-liquidity verification: when `use_flash_loan` and Kamino supply-vault liquidity is unverifiable (`None`), 3-leg capture is skipped entirely instead of proceeding unsized | `supervisor/strategy.rs` (capture skip) | workspace suite green |
| F4 | MEDIUM | `ip_of` trusts the RIGHT-MOST `X-Forwarded-For` entry (proxy-added), not the client-controllable first entry | `web_api.rs:214-218` | web matrix tests |
| F5 | MEDIUM | Single-use WS tickets (60 s TTL, sha256-stored, cap 64, remove-on-use) replace long-lived tokens in WS URLs; `POST /api/ws-ticket`; frontend fetches a ticket per connection | `web_api.rs:41-141,519`; `web/src/api.ts`; `App.tsx` | `ws_ticket_is_single_use_and_unknown_tickets_fail`; tsc + build pass |
| F6 | LOW | Property-test invariant corrected (`out <= y` + dust-condition proof) instead of false `out > 0`; regression seed committed | `arb-math/tests/differential.rs` + `.proptest-regressions` | 50/50 proptest pass |

Recovery hardening (same round): reconciliation decision extracted into the
pure, unit-tested `classify_reconciliation` (`supervisor/recovery.rs:29`),
with deterministic tests for landed/failed resolution, processed ≠ landed,
slot-proof expiry, and restart-surviving pending intents. Reconciliation
still **never resubmits**.

## 2. Verified-correct (audited, unchanged)

- Kamino fee = `arb-math::economic::calculate_flash_fee_raw` (RAW bits,
  2^60 floor, round-half-up, `u64::MAX` disabled sentinel) — single source;
  repayment gate `expected_out >= size_in + flash_fee`; borrow candidates
  above verified supply-vault liquidity dropped, not clamped.
  → [FINANCIAL_LOGIC_AUDIT.md](FINANCIAL_LOGIC_AUDIT.md)
- V1 pipeline: wire-size gate on signed bytes vs 4096, no downgrade,
  transport flag required, `MissingLoadedAccountsDataLimit` enforced,
  `maxSupportedTransactionVersion` plumbed. →
  [TRANSACTION_FORMAT_AUDIT.md](TRANSACTION_FORMAT_AUDIT.md)
- Emergency-latch semantics: clear ≠ resume; `start()` explicit and
  latch-blocked. → [SECURITY_AUDIT.md](SECURITY_AUDIT.md)
- Route graph: log-transformed fee-encoded edge weights, multigraph
  best-edge, bounded hops — matches published best practice; covered by the
  cross-DEX connectivity + per-pool fee ranking tests (commit `a6ad38d`).

## 3. Research note (directive §3)

No protocol-sensitive change was made this round: F1–F6 are
execution-safety, web-security, and test-correctness fixes verified against
the repo's existing canonical implementations (arb-math fee formula, arb-tx
V1 model, Kamino liquidity reader). Fresh online research was therefore not
required before these changes; any future protocol-format change must do it
first per directive.

## 4. Open register (severity-ranked)

| ID | Severity | Item | Blocker / next step |
|---|---|---|---|
| R-A | HIGH | ≥24 h shadow soak with predicted-vs-observed comparison | **Blocked**: sandbox cannot run a multi-day soak. Required before any readiness claim beyond Shadow Ready. |
| R-B | HIGH | Live V1 transport verification (SWQoS direct send) | **Blocked by policy** (`ARB_LIVE=0`) and by directive; gates + flag in place, awaiting authorized canary. |
| R-C | MEDIUM | `rusqlite`/`libsqlite3-sys` RUSTSEC-2026-0459 (medium, transitive via sqlx) | No semver-compatible fix exists (`cargo update --dry-run`); requires upstream breaking upgrade. Monitored via cargo-audit. |
| R-D | MEDIUM | `protobuf` RUSTSEC-2024-0437 (medium, transitive via prometheus) | Same: needs breaking major upgrade of the transitive path. |
| R-E | LOW | Legacy `?token=` WS fallback retained | Remove after frontend ticket rollout is confirmed everywhere. |
| R-F | LOW | 5 unmaintained transitive deps (ansi_term, bincode 1.x, derivative, libsecp256k1, paste) | No runtime exposure identified; track for upstream replacement. |
| R-G | LOW | Audit log is append-only, not fsync'd | Acceptable for single-operator localhost model; revisit if multi-instance. |
| R-H | INFO | Mainnet-fork full simulation run | **Blocked**: fork harness requires infrastructure beyond this sandbox; builder/sim units cover the transaction shape. |

## 5. Verification gates satisfied this round

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
- Full workspace suite green (see TEST_EVIDENCE.md run 2026-11-13 and the
  final campaign run in this round's commit).
- `tsc` + `npm run build` for the web app — clean.
- CI on PR #8 — green (auto-updates on push to `feat/remediation-r2-r12`).
