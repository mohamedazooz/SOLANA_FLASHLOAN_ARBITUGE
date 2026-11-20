# Comprehensive Dependency Audit

## 1. Rust Ecosystem Dependency Audit

### 1.1 Root Cargo Workspace (`Cargo.toml` & `Cargo.lock`)

The workspace uses **Rust edition 2024**, resolver `"2"`, and pins compiler version `1.99.1` (`rust-toolchain.toml`).

#### Workspace Members
* `crates/*` (14 crates)
* `programs/*` (`programs/arb-executor`)
* Explicitly excluded from workspace members: `fuzz`, `webhook-service`

#### Workspace Shared Dependencies Analysis
| Dependency | Version | Purpose | Evaluation / Notes |
| :--- | :--- | :--- | :--- |
| `tokio` | `1` (features: `rt-multi-thread`, `macros`, `sync`, `time`, `signal`, `io-util`) | Asynchronous runtime | Actively maintained, optimized feature set |
| `axum` | `0.8.9` (features: `ws`) | Web application REST & WS server | Latest major version; used for operator dashboard and admin control |
| `reqwest` | `0.12` (features: `rustls-tls`, `json`, `http2`) | HTTP RPC, Jito, and Telegram client | OpenSSL-free TLS via Rustls; HTTP/2 connection pooling |
| `serde` / `serde_json` | `1` | Data serialization | Canonical, zero-copy JSON parsing |
| `toml` | `0.8` | Config file deserialization | Canonical TOML 0.8 |
| `tracing` / `tracing-subscriber` | `0.1` / `0.3` | Structured telemetry & logging | JSON & compact console logging |
| `thiserror` | `2` | Domain error definitions | Upgraded to v2 across all workspace crates |
| `anyhow` | `1` | Application-level error context | Standard Rust error handling |
| `parking_lot` | `0.12` | High-performance sync primitives | Lock-free / spinlock-free fast-path synchronization |
| `ruint` | `1.12` | Wide integer arithmetic (`U256`, `U384`) | Used in `arb-math` for high-precision overflow-free AMM math |
| `solana-pubkey` | `2.2` | Solana 32-byte public key type | Lightweight type without pulling the full `solana-sdk` |
| `solami` | `0.1.58` (features: `full`) | High-speed Solana decoding | Active parsing library |
| `hex` / `hmac` / `sha2` | `0.4.3` / `0.13.0` / `0.11.1` | Webhook and HMAC cryptographic validation | Canonical RustCrypto crates |
| `proptest` / `criterion` | `1` / `0.5` | Property testing and microbenchmarking | Dev-only dependencies |

### 1.2 Isolated Crate Audit: `webhook-service` (`webhook-service/Cargo.toml`)

`webhook-service` is an auxiliary microservice outside the root workspace with its own `Cargo.lock`.

#### Conflicting Dependencies & Version Drift
| Dependency | `webhook-service` Version | Main Workspace Version | Severity | Impact |
| :--- | :--- | :--- | :--- | :--- |
| **`axum`** | `0.7` | `0.8.9` | **Medium** | Incompatible middleware and router signatures (`axum 0.8` changed `middleware::from_fn` and extractor traits). |
| **`thiserror`** | `1.0` | `2.0` | **Low** | Minor API drift in error derive macros. |
| **`tower`** | `0.4` | `0.5` (dev-dep in `arb-bot`) | **Low** | Separate tower version instances compiled into dependencies. |
| **`redis`** | `0.25` | *(Not present in workspace)* | **Informational** | Only used for external deduplication in `webhook-service`. |
| **`dashmap`** | `6.0` | *(Not present in workspace)* | **Informational** | Lock-free hashmap used in `webhook-service` memory cache. |

**Audit Recommendation**: `webhook-service` is not deployed in the primary container image or executed on the VPS by `deploy-vps.yml`. It is either an optional standalone microservice or an orphaned component. If retained, upgrade `axum` to `0.8` and `thiserror` to `2` and add to workspace members or relocate to an `auxiliary/` directory.

---

## 2. Node.js & TypeScript Dependency Audit

The repository contains two distinct Node.js configurations:

### 2.1 Operator Web Dashboard (`web/package.json`)

* **Node Version Target**: Node 22+ (`node:22-slim` in Dockerfile)
* **Build Tooling**: Vite 6.0.3, TypeScript 5.6.3, TailwindCSS 3.4.19, PostCSS 8.5.33

#### Runtime Dependencies Breakdown
| Package | Version | Usage Status | Finding |
| :--- | :--- | :--- | :--- |
| `react` / `react-dom` | `^18.3.1` | Active | Core UI framework for operator dashboard |
| `lucide-react` | `^1.76.0` | Active | Financial HUD & terminal icons |
| `clsx` / `tailwind-merge` | `^2.1.1` / `^3.7.0` | Active | Dynamic CSS class merging for Tailwind |
| **`@pump-fun/pump-sdk`** | `^4.3.1` | **DEAD / UNUSED** | **Bloat Finding**: Zero imports across `web/src`. Pulls `@pump-fun/agent-payments-sdk`. |
| **`@pump-fun/pump-swap-sdk`** | `^2.2.0` | **DEAD / UNUSED** | **Bloat Finding**: Zero imports across `web/src`. Pulls `@solana/web3.js`, `bn.js`, RPC websockets. |

#### Analysis of Dead Packages in `web/package.json`
A comprehensive recursive search across `web/src` reveals zero import statements for `@pump-fun/pump-sdk` or `@pump-fun/pump-swap-sdk`. These packages were apparently added during exploratory frontend work.
* **Impact**: They add over 50 transitive packages, 2,500+ files to `web/node_modules/`, and increase attack surface and `npm install` time in CI without contributing any functionality to the dashboard.
* **Recommendation**: Safely remove `@pump-fun/pump-sdk` and `@pump-fun/pump-swap-sdk` from `web/package.json` and regenerate `web/package-lock.json`.

### 2.2 Differential Test Harness (`scripts/differential-harness/package.json`)

* **Target**: Node.js offline diagnostic script (`run_mainnet_snapshot_differential.js`)

#### Package Manifest Analysis
```json
{
  "dependencies": {
    "@meteora-ag/dlmm": "^1.9.14",
    "bs58": "^6.0.0"
  }
}
```
* **Undeclared Transitive Dependencies Finding**:
  `scripts/differential-harness/run_mainnet_snapshot_differential.js` directly requires:
  * `const { PublicKey } = require('@solana/web3.js');` (line 48)
  * `const { BN } = require('bn.js');` (line 50)
  Neither `@solana/web3.js` nor `bn.js` is declared in `scripts/differential-harness/package.json`. The script works only because `@meteora-ag/dlmm` happens to pull them in transitively.
* **Recommendation**: Explicitly declare `@solana/web3.js` and `bn.js` in `scripts/differential-harness/package.json` to avoid fragile resolution failures under strict package managers.

---

## 3. Python Ecosystem Audit

### 3.1 Package Manifest Inspection
* **`pyproject.toml`**: None present in repository.
* **`requirements.txt`**: None present in repository.
* **`Pipfile` / `poetry.lock`**: None present in repository.

### 3.2 Python Module Import Breakdown
Across all 31 Python files in the repository:
* **Standard Library Only (28 scripts)**:
  * `urllib.request`, `json`, `os`, `sys`, `time`, `hashlib`, `base64`, `subprocess`, `math`, `argparse`, `random`, `re`, `ssl`.
  * The mathematical oracle (`audit/math_oracle/oracle.py`) uses **zero external libraries** (only `json`, `os`). It derives CPMM and DLMM swap formulas using Python's built-in arbitrary-precision integer arithmetic (`int`).
* **External Third-Party Dependency (`websockets`)**:
  * Used in:
    * `scripts/helius_opportunity_probe.py`
    * `scripts/read_only_helius_jito_probe.py`
    * `scripts/probe_rpc_wss_v1_sim.py`

### 3.3 Security Anti-Pattern Finding: Unpinned In-Process `pip install`
In `scripts/helius_opportunity_probe.py` (lines 72–74) and `scripts/read_only_helius_jito_probe.py` (lines 78–80):
```python
try:
    import websockets
except ImportError:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "websockets", "--quiet"])
    import websockets
```
* **Vulnerability**: Invoking `pip install` silently without version pinning or hash checking executes unauthenticated code download at runtime. If executed in a production or shared environment, this creates supply-chain risk and potential environment corruption.
* **Recommendation**: Remove automatic `pip install` subprocess calls. Provide a clean `scripts/requirements-dev.txt` listing pinned `websockets==12.0` with documentation instructing developers to install into a dedicated virtual environment.

---

## 4. Critical Security Findings in Script Files

### 4.1 Production Solscan Pro API JWT Leakage
A live Solscan Pro API JWT token with active entitlement (`action: "token-api"`, `email: "medo_soldr15@yahoo.com"`) is hardcoded in plain text across 5 repository scripts:
1. `scripts/solscan_tool.py` (line 48)
2. `scripts/test_mcp.py` (line 4)
3. `scripts/test_mcp_call.py` (line 4)
4. `scripts/test_pro_endpoints.py` (line 4)
5. `scripts/test_public_tools.py` (line 4)

Additionally, production KiloCode JWT tokens (`kiloUserId: "8304aaa8-2e85-4ced-8812-f4cd3cbc8169"`) are committed in:
* `scripts/decode_jwt.py` (lines 5–6)
* `_backup_pre_unification/decode_jwt.py` (lines 5–6)

**Impact**: Immediate API quota exhaustion, denial of service on market intelligence lookups, and security compromise.
**Action**: Immediately revoke the Solscan Pro API token and KiloCode tokens, rotate credentials in the Solscan developer dashboard, scrub from all scripts, and pass via `.env` (`SOLSCAN_API_KEY`).

### 4.2 Massive API Credential Dump in `test_keys.py`
Both `scripts/test_keys.py` and `_backup_pre_unification/test_keys.py` contain hardcoded plain-text API keys and secrets for over 15 distinct production services:
* **Solana RPCs**: Helius RPC key (`bb94...[REDACTED]`), Solami key (`q88C...[REDACTED]`), Shyft RPC keys (`uhT_...[REDACTED]`), Alchemy Solana key (`alch_...[REDACTED]`).
* **EVM RPCs**: Alchemy Arbitrum key, Arbitrum DRPC key, QuickNode endpoint token.
* **Financial & Market Data**: Birdeye API key (`b0d6...[REDACTED]`), Etherscan API key (`WUJV...[REDACTED]`), TwelveData API key (`f350...[REDACTED]`), FastForex key.
* **AI Model Providers**: OpenRouter API keys (6 separate keys), DeepSeek API key (`sk-...[REDACTED]`), Google AI Studio keys (2 keys), Groq key, Mistral key, Cohere key, HuggingFace tokens (2 tokens).

**Impact**: Severe multi-service secret compromise, financial liability from unmetered usage, and potential account suspension across all cloud and AI vendors.
**Action**: Revoke all compromised keys across all provider consoles immediately. Delete `_backup_pre_unification/test_keys.py` and refactor `scripts/test_keys.py` to read keys exclusively from environment variables.

---

## 5. Docker & CI/CD Dependency Audit

### 5.1 Dockerfile Architecture (`Dockerfile`)
Multi-stage build providing strict environmental isolation:
* **Stage 1 (`builder`)**: `rust:1.99.1-slim-bookworm`
  * Installs `pkg-config`, `libssl-dev`, `ca-certificates`.
  * Compiles `arb-bot` with `--locked --release --features shadow-only`.
* **Stage 2 (`web-builder`)**: `node:22-slim`
  * Installs npm dependencies via `npm ci`.
  * Compiles React dashboard to static assets via `npm run build`.
* **Stage 3 (`runtime`)**: `debian:bookworm-slim`
  * Only runtime packages: `ca-certificates`, `curl` (for healthcheck).
  * Creates unprivileged user `arb (uid=1000)`.
  * Copies `/usr/local/bin/arb-bot` from Stage 1.
  * Copies `/app/web/dist` from Stage 2.
  * Zero Python, zero Node.js, and zero compilers in the runtime image.

### 5.2 CI/CD Workflows (`.github/workflows/`)
* **`rust-ci.yml`**:
  * Pinned `dtolnay/rust-toolchain@1.99.1`.
  * Enforces `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`.
  * Executes security scanning via `gitleaks` and `cargo-deny`.
  * Builds web frontend via `npm ci && npm run build`.
  * *Audit Gap*: Does not currently execute `python audit/math_oracle/oracle.py --verify` to ensure test vectors remain synchronized with the independent math oracle.
