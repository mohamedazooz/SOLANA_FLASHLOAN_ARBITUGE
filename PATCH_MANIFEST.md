# Patch manifest — 2026-11-12

## Applied in this packaging pass

1. Added `arb-bot` Cargo feature `shadow-only`.
2. In shadow-only builds, startup rejects every mode except `shadow`.
3. In shadow-only builds, `.env` loading is disabled.
4. In shadow-only builds, private-key parsing and keypair-file loading are compiled out; the supervisor has no signer.
5. The `BotMode::Live` dispatch implementation is compiled out for shadow-only builds and replaced with a blocked/no-op arm.
6. Docker builds the bot with `--features shadow-only`.
7. Runtime configs now default to `shadow`, clear `wallet.keypair_path`, and use a writable SQLite path under `/app/data`.
8. Added a dedicated hardened Compose deployment: no keypair mount, no general `.env` injection, read-only container filesystem, dropped Linux capabilities, and loopback-only metrics binding.
9. Removed the legacy live launcher/deployment package, operator-specific key material/config artifacts, and two SSH automation scripts that embedded a VPS password from the distributable tree.
10. Added `verify-shadow-package.sh` for static packaging invariants.
11. Set the packaged bot's transaction policy to **V1-only**: the active route compiler calls `compile_v1_only`; V1-capable profiles never downgrade to Legacy/V0; Legacy/V0 requests are rejected under a V1-capable profile.
12. Corrected Solana RPC transaction version mapping: V0 => `maxSupportedTransactionVersion: 0`, V1 => `1` (previous code incorrectly used 1 and 2 respectively).
13. Added an unsigned V1 Shadow transaction envelope with zeroed payer signature slot; RPC simulation explicitly sets `sigVerify: false`. Secondary signers are rejected rather than impersonated.
14. Disabled Legacy/V0 pre-warmed template reuse in the V1-only route path. Current inline account metas are rebuilt for every route.
15. Added `transaction-profile.toml` and `V1_ONLY_RELEASE_POLICY.md`: 4,096-byte V1 limit, no ALTs, 64 inline accounts, 64 top-level instructions, mandatory explicit CU and loaded-account-data limits, and exact Meteora DLMM BinArray PDA inclusion.
16. Updated V1 compiler defaults and template recompilation so V1-capable builds remain V1 even when the transaction is smaller than 1,232 bytes.
17. Converted the secondary `arb-execution` dispatcher to V1-only and routed it through the canonical `arb-tx` V1 compiler, including native priority-fee and loaded-account-data config; it no longer emits V0 or uses the old custom V1 serializer.
18. Rejects single-signer compilation when the message requires more than one signature, rather than serializing an incomplete signature vector.

## Not claimed as fixed

This pass does not close the complete mathematical/protocol audit register. In particular, the package still requires a full Rust build/test/clippy run and evidence for:

- complete DLMM bitmap-driven BinArray selection and exact quote/instruction account-set equality under the V1 inline-account budget;
- end-to-end per-mint Token-2022 epoch fee schedule + transfer-hook handling (unsupported tokens must remain rejected);
- one canonical dimensional `CostLedger` used identically by discovery and dispatch;
- integer/discrete optimizer correctness across DLMM/CLMM discontinuities and checked conversions;
- coherent slot/fork freshness proof across all subscribed accounts;
- official SDK differential tests and non-broadcast RPC simulation for every supported route class (including unsigned V1 Shadow envelopes and RPC provider compatibility);
- Kamino non-zero flash-fee round-trip tests on a fork or equivalent deterministic harness;
- seven-day shadow soak with zero broadcast attempts.

Legacy/V0 compatibility code and historical tests remain in the repository for reference, but the packaged bot's active route path is V1-only and rejects non-V1 transaction bytes at dispatch. The archived audit verdict is preserved under `audit_reports/18_FINAL_AUDIT_VERDICT.md`; it remains the authoritative warning for live-readiness. Do not use a non-shadow build or real funds based on this ZIP.
