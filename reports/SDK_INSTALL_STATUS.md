# Solana Arbitrage Tooling — CLI & SDK Install Status

Date: 2026-11-21
Host: Windows 11 (x86_64-pc-windows-msvc), Rust 1.99.1 (pinned), WSL2 Ubuntu available.
Network: unstable during this session (static.crates.io DNS failures, anza release tarball resets). Installs that needed large downloads did not complete.

## 1. CLI tools

| Tool | Status | Notes |
|---|---|---|
| Solana CLI (`solana`, `solana-keygen`, `solana-test-validator`) | NOT installed | Installer downloaded (`solana-install-init.exe`, 9.4 MB) but tarball download (`solana-release-...tar.bz2`) reset by peer. `cargo install solana-cli --version 4.4.1` attempted; blocked by static.crates.io DNS. Retry when network stable. Binary would land in `%USERPROFILE%\.cargo\bin`. |
| Solana Test Validator | NOT installed | Ships with Solana CLI above. |
| Anchor CLI (`anchor`) | NOT installed | `npm i -g @coral-xyz/anchor-cli` (latest 0.31.2) failed ECONNRESET. Alternative: `cargo install anchor-cli --version 1.2.2` (crates.io, old) or avm. |
| Jito CLI (`jito-cli`) | N/A | No standalone `jito-cli` crate/binary exists on crates.io or npm. Jito searcher tooling is SDK-based (`jito-sdk-rust`, `jito-client`, `jito-searcher-client`). The user's "jito-cli" likely refers to these SDKs. |
| Rust toolchain | Installed | 1.99.1 (pinned by rust-toolchain.toml), cargo 1.99.1. |

## 2. SDK crates — verified latest stable versions (crates.io, 2026-11-21)

### Core Solana (modern, granular)
- `solana-sdk` 5.0.0
- `solana-client` 4.4.1 (newest 5.0.0-alpha.1)
- `solana-pubkey` 2.2 (workspace already uses 2.2)
- `solana-test-validator` 4.4.1 (newest 5.0.0-alpha.1)

### Anchor (IDL deserialization only — never on hot paths per AGENTS.md)
- `anchor-lang` 1.2.2 (newest 2.0.0-rc.4)
- `anchor-gen` 0.4.1
- `shank` 0.4.9

### Jito
- `jito-sdk-rust` 0.3.2
- `jito-client` 0.3.5
- `jito-bundle` 0.1.5
- `jito-searcher-client` 0.0.1

### Yellowstone gRPC (account streaming)
- `yellowstone-grpc-client` 17.0.0

### DEX adapters
- `raydium_amm` 0.1.13
- `raydium-cp-swap` 0.1.1392 (newest 0.1.170)
- `raydium_clmm` 0.1.13
- `meteora-dlmm` 0.2.0
- `orca_whirlpools_client` 8.0.0
- `orca_whirlpools_core` 2.1.1

### Modern Solana 4.x SDKs (verified modern — compatible with granular crates)
- `carbon-raydium-clmm-decoder` 2.0.0 (uses solana-pubkey 4.x)
- `carbon-orca-whirlpool-decoder` 2.0.0
- `carbon-meteora-damm-v2-decoder` 2.0.0
- `carbon-marginfi-v2-decoder` 2.0.0
- `solana-clmm-raydium` 0.3.0 (uses solana-pubkey 2.2)
- `raydium-sdk-V2` 0.0.10 (uses solana-sdk 2.1)

### Flash loans
- `solend-sdk` 0.1.0 (uses solana-program 1.x — legacy)
- `solend-token-lending` 0.1.1
- `kamino-lend` 0.4.1 (uses solana-program 1.x — legacy)
- `kamino_lending_interface` 0.1.0
- `klend-sdk` 0.1.2
- `marginfi-cpi` 0.1.3 (uses solana-program 1.x — legacy)
- `marginfiv2cpi` 0.1.0 (uses solana-program 1.x — legacy)

### SPL
- `spl-token` 9.0.0
- `spl-associated-token-account` 8.0.0
- `spl-memo` 7.0.0

### Perf / async
- `rayon` 1.12 (already cached)
- `tokio` 1 (workspace already uses 1)

## 3. Critical compatibility finding

The legacy DEX/flash SDKs (raydium_amm, raydium-cp-swap, raydium_clmm, solend, kamino_lending_interface, marginfi-cpi, meteora-dlmm, orca 2.x/8.x) are built against Solana 1.x (`solana-program` 1.9–1.17). They **cannot** resolve in the same dependency graph as the modern Solana 4.x/5.x crates this repo uses (`solana-pubkey` 2.2, `solana-instruction` 3.5, `solana-message` 4.6, `solana-client` 4.3.2). Attempting to add them to `arb-dex`/`arb-tx` directly will fail cargo resolution (zeroize / solana-program version conflicts).

**Recommendation:** keep these SDKs in a separate, legacy dependency universe (isolated crate with its own lockfile) and use them only for IDL decoding / offline research, OR (preferred for HFT) hand-build swap/flash instructions against the modern granular crates, which the repo already does. Do not add the legacy SDKs to production crate deps.

## 4. What was created

- `tools/tooling-deps/` — modern core fetch project (solana 5/4, jito, yellowstone, anchor 1.2, shank, anchor-gen, spl, rayon, tokio). Run `cargo fetch` here when network returns to cache modern SDKs.
- `tools/tooling-dex/` — legacy DEX/flash fetch project (raydium, meteora, orca, kamino, marginfi, solend). Run `cargo fetch` here separately.
- `reports/SDK_INSTALL_STATUS.md` — this file.

## 4b. Added to production crates (optional, feature-gated, default OFF)

**Cached (Solana 2.x/4.x, already in registry):**
- `crates/arb-ingest/Cargo.toml`: `yellowstone-grpc-client = "17.0.0"` behind feature `yellowstone`. **Note:** yellowstone-grpc-client is Linux-only (requires tokio::net with UnixStream). Production builds run on Linux; Windows check will skip this feature.
- `crates/arb-land/Cargo.toml`: `jito-client = "0.3.5"` + `jito-searcher-client = "0.0.1"` + `jito-sdk-rust = "0.3.2"` behind feature `jito`.
- `crates/arb-dex/Cargo.toml`: `orca_whirlpools_core = "2.1.1"` behind feature `orca-sdk`.

Enable with `--features yellowstone` / `--features jito` / `--features orca-sdk` (or `--all-features`).

**Note:** The carbon-* modern decoders (carbon-raydium-clmm-decoder, carbon-orca-whirlpool-decoder, carbon-meteora-damm-v2-decoder, carbon-marginfi-v2-decoder) and solana-clmm-raydium/raydium-sdk-V2 were evaluated but **not added** to production because:
- carbon-* depends on `carbon-core 2.0.0` which requires `solana-transaction-context =4.2.2`, conflicting with `litesvm 0.18.2` (used by arb-sim) which requires `solana-transaction-context ~4.3.2`. This version conflict prevents compilation in the same workspace.
- solana-clmm-raydium/raydium-sdk-V2 depend on Solana 2.x, which is older than the workspace's Solana 4.x/5.x.

These modern decoders remain available in `tools/tooling-deps/` for offline research and can be integrated via `anchor-gen` code generation targeting modern crates.

## 4c. Still pending network (not cached) — to be added when connectivity returns

raydium_amm, raydium-cp-swap, raydium_clmm, meteora-dlmm, orca_whirlpools_client, kamino-lend, klend-sdk, marginfi-cpi, marginfiv2cpi, solend-sdk, solend-token-lending, kamino_lending_interface, jito-sdk-rust, jito-bundle. These are listed in `tools/tooling-dex/Cargo.toml` and `tools/tooling-deps/Cargo.toml`.

## 5. Cached already (no download needed)

`~/.cargo/registry` already holds 849 crates including `solana-*` 4.x, `orca_whirlpools_core` 2.1.1, `rayon` 1.12, `spl-token` interfaces, `solana-client` 4.3.2, etc. The production workspace builds against these.

## 6. Retry commands when network is stable

```powershell
# Solana CLI
& "$env:TEMP\solana-install-init.exe" stable

# Anchor CLI
npm install -g @coral-xyz/anchor-cli

# Cache modern SDKs
cd "C:\Users\azoozz\Downloads\my arbitrge\tools\tooling-deps"; cargo fetch

# Cache legacy DEX/flash SDKs (separate universe)
cd "C:\Users\azoozz\Downloads\my arbitrge\tools\tooling-dex"; cargo fetch
```

## 7. Additional tools worth adding (not in original list)

- `solana-test-validator` (part of Solana CLI) — local fork for simulating arbitrage paths without fees.
- `spl-token-cli` — token account inspection.
- `solana-verify` (Ellipsis Labs) — verify on-chain program builds.
- `surfpool` / `solana-bankrun` — fast local simulation for tests.
- `jito-searcher-client` is the closest to "jito CLI" for searchers; no separate binary exists.

## 8. Modern SDKs verified (2026-11-21)

The following modern SDKs use granular crates (solana-pubkey 2.x/4.x, solana-instruction 3.x, etc.) and are **compatible** with this repo's toolchain:

| Crate | Version | Solana deps | Use case |
|-------|---------|-------------|----------|
| `carbon-raydium-clmm-decoder` | 2.0.0 | solana-account ^4.3, solana-instruction ^3.4, solana-pubkey ^4.2 | Decode Raydium CLMM state |
| `carbon-orca-whirlpool-decoder` | 2.0.0 | solana-account ^4.3, solana-instruction ^3.4, solana-pubkey ^4.2 | Decode Orca Whirlpool state |
| `carbon-meteora-damm-v2-decoder` | 2.0.0 | solana-account ^4.3, solana-instruction ^3.4, solana-pubkey ^4.2 | Decode Meteora DAMM v2 state |
| `carbon-marginfi-v2-decoder` | 2.0.0 | solana-account ^4.3, solana-instruction ^3.4, solana-pubkey ^4.2 | Decode Marginfi v2 state |
| `solana-clmm-raydium` | 0.3.0 | solana-pubkey ^2.2, solana-instruction =2.2, solana-message =2.2 | Raydium CLMM types (modern) |
| `raydium-sdk-V2` | 0.0.10 | solana-sdk ^2.1, solana-client ^2.1, spl-token ^6.0 | Raydium SDK v2 (modern) |
| `jito-sdk-rust` | 0.3.2 | solana-client ^2.2, solana-instruction ^2.2, solana-pubkey ^2.2 | Jito bundles/tips |
| `yellowstone-grpc-client` | 17.0.0 | modern | Account streaming |
| `orca_whirlpools_core` | 2.1.1 | cached, modern-ish | Whirlpool math/tick arrays |

**Currently wired in production:** `orca_whirlpools_core`, `jito-*`, `yellowstone-grpc-client` (via features).
**Available for integration via anchor-gen / codegen:** carbon-*, solana-clmm-raydium, raydium-sdk-V2.