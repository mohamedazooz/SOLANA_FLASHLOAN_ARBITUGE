# Solana Flash Loan Arbitrage Engine

High-performance, low-latency Solana Flash Loan Arbitrage Engine in Rust featuring CLMM/DLMM math, Yellowstone gRPC ingestion, private bundle execution, and quantitative risk guards.

---

## ⚡ Core Architecture

- **Workspace Crates**:
  - `crates/core`: Foundational types, market state representation, math engine, and risk guardrails.
  - `crates/dex`: AMM adapters for Orca Whirlpool (CLMM), Meteora (DLMM), Raydium (AMM v4 & CPMM), Pump.fun & PumpSwap.
  - `crates/pipeline`: Pipeline orchestration, order-flow routing, and execution stages.
  - `crates/solana`: Solana RPC, Yellowstone gRPC streaming client, and account decoders.
  - `crates/flashloan`: Flash loan orchestration, instruction builders, and zero-loss callback handlers.
  - `crates/executor`: MEV protection, private bundle routing (Jito Block Engine), and transaction dispatching.
  - `crates/strategy`: Graph-based arbitrage pathfinder, cyclic triangular search, and dynamic sizing.

---

## 🚀 Quick Start

### Prerequisites

- Rust (1.80+ recommended)
- `cargo` and `rustup`

### Building the Project

```bash
cargo build --release
```

### Running Tests

```bash
cargo test --workspace
```

### Static Analysis & Lints

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

---

## ⚙️ Configuration

Copy `.env.example` to `.env` and fill in your private endpoints and credentials:

```bash
cp .env.example .env
```

Review `config.toml` for engine parameters, gas/priority fee strategy, and slippage tolerances.

---

## 📚 Documentation

For detailed phase documentation, mathematical audits, and integration guides, refer to:
- [`README_PHASE09_INTEGRATED.md`](README_PHASE09_INTEGRATED.md)
- [`docs/`](docs/)
