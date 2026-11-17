# SECURITY_FORENSIC_REPORT.md

**Repository:** https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE
**Branch:** newrepair (up to date with origin/newrepair)
**Audit date:** 2026-11-16
**Auditor:** OpenCode agent (static + repository forensics; live cargo execution blocked — see blockers)

## 1. Repository Forensics

- Remote: `origin https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git`
- Current branch: `newrepair`, tracking `origin/newrepair`, up to date.
- Modified (unstaged) files at audit start:
  - `crates/arb-bot/src/supervisor/opportunity.rs`
  - `crates/arb-bot/src/supervisor/run.rs`
  - `crates/arb-tx/src/lib.rs`
  - `crates/arb-tx/src/primitives/builder.rs`
  - `crates/arb-tx/src/primitives/cu.rs`
  - `crates/arb-tx/tests/address_regression_guards.rs`
  - `reports/phase15_2/optimizer_search_statistics.json`
  - `reports/phase15_2/sdk_differential_manifest.json`
- Untracked (preserved, not modified): multiple `*.md` notes, `SOLANA_FLASHLOAN_ARBITUGE/` (a separate clone on `main`), `programs/`, `verification_script.sh`, `verify_fix.sh`, `phase1210.rar`.
- A nested clone `SOLANA_FLASHLOAN_ARBITUGE/` (on `main`, clean aside from unstaged) was found and treated as a reference snapshot only; it was not modified.
- Git submodules: none configured (`.gitmodules` absent).
- Toolchain: Rust edition 2024, pinned 1.99.1 (per `rust-toolchain.toml`); Linux-only build (yellowstone gRPC).

## 2. Static Security Audit — SlowMist Threat Indicators

Searched the entire working tree (Rust, TOML, env, scripts) for the SlowMist-documented indicators and backdoor fingerprints:

| Indicator | Result |
|---|---|
| `create_coingecko_proxy` | NOT FOUND |
| `HELIUS_PROXY` | NOT FOUND |
| `import_wallet` / `import_env_var` | NOT FOUND (as exfiltration patterns) |
| `to_base58_string` | NOT FOUND |
| `storebackend-qpq3.onrender.com` | NOT FOUND |
| `103.35.189.28` | NOT FOUND |
| `/api/wallets` | NOT FOUND |
| Base58-encoded `HELIUS_PROXY` value (`2HeX3Zi2vTf…`) | NOT FOUND |
| Hardcoded external IP/domain POST endpoints | NOT FOUND |
| Build scripts (`build.rs`) | NONE present in workspace |
| Git dependencies in `Cargo.toml` | NONE (deny.toml sets `unknown-git = "deny"`) |

SHA-256 of `src/common/config.rs` and the archive were not computed because the repository layout does not contain a top-level `src/common/config.rs` (the project is a Cargo workspace under `crates/`); no single file matches the SlowMist `config.rs` fingerprint path. This is recorded as a structural difference, not a safety claim.

Conclusion: **No SlowMist indicators or documented backdoor fingerprints were found in the audited tree.** This is a negative static result only; it does not prove absence of all malicious behavior. No network egress beyond configured RPC/Jito/DEX endpoints was identified in source review.

## 3. Secrets & Wallet Handling

- `.env` is git-ignored (`.gitignore` lines 3–9); `.env.example` / `.env.shadow.example` present.
- No private-key material, seed phrases, or secret bytes found in tracked source or reports; long base58 strings present are transaction signatures and serialized account data, not keys.
- Wallet signing is separated from arbitrage logic via `Signer` interface (per AGENTS.md invariants). No key printing/serialization-to-network paths identified in the audited modules.
- Recommendation: rotate any key ever used in this repo's history and treat the nested `SOLANA_FLASHLOAN_ARBITUGE/` clone's `.env` (if present) with the same caution.

## 4. Dependency & Supply Chain

- Workspace uses `resolver = "2"`, edition 2024, rust-version 1.99.
- No git-sourced dependencies in `Cargo.toml`; `deny.toml` denies unknown git.
- `solami = "0.1.58"` (full features) is the primary Solana SDK; `solana-pubkey = "2.2"`.
- `cargo audit` / `cargo deny check` / `cargo geiger` were **not executed** (shell/cargo unavailable in this environment). This is a verification gap, not a pass.

## 5. Network Egress (declared)

From source review, egress destinations are limited to:
- Solana RPC / WebSocket / Yellowstone gRPC (provider-configured)
- Jito Block Engine (bundle submission)
- DEX program accounts (on-chain, via RPC)
- Price oracle endpoints (config-gated)

No telemetry/analytics endpoints or undocumented HTTP POST of wallet material identified. A full egress capture test (dummy keys + local sink) was **not executed** in this session.

## 6. Blockers Encountered

- The shell tool could not reliably spawn `pwsh.exe` during this session (intermittent "Access is denied"), preventing execution of `cargo fmt`, `cargo clippy`, `cargo test`, `cargo audit`, and `git diff`. All cargo-based verification claims in prior reports could not be re-run here.
- Consequently, no test results, build status, or lint status are re-verified in this report. Existing evidence artifacts under `reports/` are treated as historical, not freshly reproduced.

## 7. Security Status

- SlowMist IoCs: **none found** (negative static result).
- Data exfiltration paths: **none identified** in reviewed source.
- Wallet security posture: **structurally sound** (env-based, git-ignored, signer-separated) pending live key-rotation hygiene.
- **Security Gate 1: PASS (static)** — no unremediated malicious behavior identified; proceed to engineering audit with the caveat that dynamic egress capture was not performed.

## 8. Recommended Next Steps (not yet executed)

1. Re-run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets` on Linux (1.99.1) and record results.
2. Run `cargo audit` and `cargo deny check` and resolve advisories.
3. Perform dynamic egress capture with dummy keys against a local sink.
4. Re-verify the Kamino fee formula (`fee_bits = max(amount*sf, 2^60)`, ties-away) and `BorrowTooSmall` raw-bits comparison against the on-chain program.
5. Validate V1 transaction size ladder (4,096-byte ceiling) with real compiled bytes.
