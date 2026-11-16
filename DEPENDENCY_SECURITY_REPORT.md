# DEPENDENCY_SECURITY_REPORT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Manifest

- Workspace resolver 2, edition 2024, rust-version 1.99.
- No git-sourced deps in Cargo.toml; deny.toml sets `unknown-git = "deny"`.
- Key deps: solami 0.1.58 (full), solana-pubkey 2.2, axum 0.8.9, tokio 1, reqwest 0.12 (rustls), serde, toml, thiserror, anyhow, tracing, ruint, proptest, criterion.

## Audits run this session

- `cargo audit`: NOT RUN (cargo unavailable).
- `cargo deny check`: NOT RUN.
- `cargo geiger`: NOT RUN.
- `cargo tree`: NOT RUN.

## Static observations

- No build.rs (no build-script network egress).
- .gitignore excludes target/, .env, keys, logs.
- deny.toml configured for unknown git.

## Recommendations

- Run `cargo audit` + `cargo deny check` on Linux; resolve advisories.
- Pin solami to a verified commit if supply-chain assurance required.
- Re-run cargo geiger to quantify unsafe usage (workspace allows unsafe_code).

## Status

Configuration sound; live audit pending.
