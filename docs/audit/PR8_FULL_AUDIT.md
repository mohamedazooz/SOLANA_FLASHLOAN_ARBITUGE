# PR #8 Full Audit — Findings Register (2026-11-13)

Scope: every commit of `feat/remediation-r2-r12` (PR #8) audited against the
actual source, not against prior reports. Verdict terms are used strictly:
**Verified** = inspected in code AND exercised by a test that ran in this
campaign. **Partially verified** = inspected, partially exercised. **Not
verified** = inspected only, or blocked by environment.

## Findings fixed in this audit round

| # | Severity | File | Root cause | Fix | Test evidence |
|---|---|---|---|---|---|
| F1 | HIGH | `crates/arb-bot/src/supervisor/dispatch.rs` (~line 481) | Dispatch-intent persistence was best-effort: on store error the send proceeded, producing an unrecoverable transaction after a crash/timeout (violates "no dispatch without durable intent") | Fail-closed: signature extraction failure or persistence failure aborts dispatch, records `dispatch_aborted_unreconcilable` / `dispatch_aborted_unpersisted` outcome, alerts Telegram | `cargo test -p arb-bot` 51/51 (dispatch path compiles + all existing dispatch tests green); recovery store lifecycle test |
| F2 | HIGH | `crates/arb-bot/src/supervisor/dispatch.rs` (fire-time gate) | Kill switch / emergency latch were checked at capture time only; a stop issued during simulation/slot-wait did not block the in-flight submission | New fire-time safety gate immediately before any network submission; aborts with `aborted_kill_switch` outcome | `cargo test -p arb-bot` green; `control_service::is_emergency_stopped` getter added and consumed |
| F3 | MEDIUM | `crates/arb-bot/src/supervisor/strategy.rs` (~line 251) | With flash funding enabled and Kamino supply-vault liquidity **unverifiable** (`None`), the 3-leg capture proceeded with all candidate sizes (fail-open) | Capture is skipped when `use_flash_loan` and liquidity is unverified — the opportunity is dropped, not sized against an invented value | `cargo check` + full workspace suite green |
| F4 | MEDIUM | `crates/arb-bot/src/web_api.rs` `ip_of` | Took the FIRST `X-Forwarded-For` entry — client-controlled and spoofable through the trusted proxy | Takes the RIGHT-MOST entry (added by the trusted reverse proxy) | Inspected; covered by existing web matrix tests |
| F5 | MEDIUM | `web_api.rs` + `web/src` | Long-lived session token accepted in WS URLs (`?token=`) — retained in proxy/access logs | Single-use 60 s tickets: `POST /api/ws-ticket` (authed) → `?ticket=` consumed at upgrade; legacy `?token=` fallback retained | `web_api::tests::ws_ticket_is_single_use_and_unknown_tickets_fail` passes; `tsc` + `npm run build` pass |
| F6 | LOW | `crates/arb-math/tests/differential.rs` | `prop_cpmm_output_bounds` asserted `out > 0`, false for floor-rounded dust (`dx=1,x=4,y=1,gamma=9000`) | Invariant corrected to `out <= y` + dust-condition proof; proptest regression seed committed | `cargo test -p arb-math --test differential` 50/50 |

## Verified correct (inspected, no change required)

- **V1 transaction pipeline**: wire size checked on the SIGNED bytes
  (`tx_bytes.len()` vs `SOLANA_PACKET_V1_MTU` = 4096, hard reject, no
  downgrade); live dispatch refuses without the explicit V1 transport flag;
  loaded-accounts-data limit is a required builder parameter
  (`MissingLoadedAccountsDataLimit`); `maxSupportedTransactionVersion` carried
  in `arb-ingest` and `arb-sim` RPC configs. →
  [TRANSACTION_FORMAT_AUDIT.md](TRANSACTION_FORMAT_AUDIT.md)
- **Kamino economics**: canonical fee in `arb-math` (RAW bits, round-half-up,
  disabled sentinel `u64::MAX`); repayment funding gate
  `expected_out >= size_in + flash_fee`; borrow candidates ABOVE verified
  supply-vault liquidity are dropped (not clamped). →
  [FINANCIAL_LOGIC_AUDIT.md](FINANCIAL_LOGIC_AUDIT.md)
- **RiskGate consumed by execution**: `evaluate_flash_loan`/`evaluate_full`
  gate every opportunity; kill switch checked even under `BYPASS_RISK_GATE`;
  emergency latch blocks `set_mode`; `CONFIRM-LIVE` + `ARB_LIVE=1` required for
  live mode. → [SECURITY_AUDIT.md](SECURITY_AUDIT.md)
- **Recovery**: reconciliation never resubmits; `expired_unconfirmed` requires
  slot-proof of the blockhash window; `unresolved_after_poll` is honest. Now
  decision-extracted and deterministically unit-tested. →
  [REMEDIATION_PLAN.md](REMEDIATION_PLAN.md)
- **Telegram**: allowlist enforced, listener disabled without it, bounded
  retries, same `execute_command` as web (one control path).

## Not verified / blocked (honest register)

| Item | Status | Blocker |
|---|---|---|
| ≥24 h shadow soak | Not verified | Sandbox cannot run a 24 h soak; requires VPS deployment |
| Live transport (V1 direct SWQoS) | Not verified by design | `ARB_LIVE=0`; requires operator authorization + endpoint capability proof |
| Fork simulation of full 5-instruction tx | Partially verified | `arb-sim` simulation path unit-tested; full mainnet-fork run needs live RPC quota |
| `rsa`/`protobuf`/`libsqlite3-sys` medium advisories | Open | All need breaking dependency upgrades; documented in TEST_EVIDENCE |

## Campaign evidence (this round)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
- `cargo test -q --workspace --all-targets` (`.env` sourced) — **946 passed /
  0 failed** (941 prior + 5 cross-DEX route tests; recovery/web additions
  included in the 946).
- CI on PR #8: Rust + Web jobs **pass** (run 43346416198).
