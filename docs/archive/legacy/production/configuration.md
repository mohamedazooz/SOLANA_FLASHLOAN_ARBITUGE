> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Centralized Layered Configuration Specification

## 1. Configuration Precedence Architecture
The bot loads configuration through a single deterministic hierarchy:
1. **Compiled Protocol Invariants**: Hardcoded in `arb-constants` (program IDs, discriminators, account layouts). Operator configuration CANNOT override protocol invariants.
2. **Base Configuration**: `config/base.toml` (system defaults, timing, structures).
3. **Layered Environment TOML**: `config/{development,staging,production}.toml`.
4. **Subsystem TOMLs**: `config/risk.toml`, `config/execution.toml`, `config/providers.toml`, `config/protocols.toml`, `config/webhook.toml`.
5. **Environment Overrides**: Environment variables prefixed with `ARB_` or standard credentials (`HELIUS_API_KEY`, `WEB_ADMIN_PASSWORD`).
6. **Command-Line Overrides**: CLI flags passed to `arb-bot`.

## 2. Hardcoded Values Elimination & Rediscovery Audit
All historical magic numbers have been migrated to typed structures in `arb-constants` and `arb-config`:
- **USDC Mint Conflict**: Clarified and locked to official SPL Token USDC (`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`).
- **WSOL Mint**: Locked to Wrapped SOL (`So11111111111111111111111111111111111111112`).
- **Kamino Main Market**: Locked to `7u3HeHxatcAACvdFtHGgahLizUCkWKEmmbmjfcr2AvZh`.
- **Meteora DLMM Program**: Locked to `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`.

## 3. Strict Typings & Safe Units
Every financial, temporal, or spatial setting uses newtype wrappers:
- `TokenAmount` (atomic units)
- `Lamports` (1 SOL = 1,000,000,000 lamports)
- `Bps` (basis points, 1 bps = 0.01%)
- `Milliseconds` & `Seconds`
- `Slot` (u64 Solana slot index)
- `ComputeUnits`

## 4. Configuration Schema Validation
- All TOML files are validated on application boot against `config/config.schema.json`.
- Missing mandatory fields, out-of-range bounds, NaN, or infinite floats immediately abort startup with an actionable error.
