> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Migration Guide

**Repository:** Solana Arbitrage Bot  
**Purpose:** Migrate from the old flat `config.toml` + hardcoded values to the
new strongly-typed `arb-config` system.

---

## 1. Before You Start

- Back up your existing `config.toml` and `.env` (Rule 10: never overwrite
  existing configuration without backups).
- Do not delete existing work, dirty files, or untracked files (Rule 9).
- This refactor does not enable live trading (Rule 7).

## 2. File Mapping

| Old location | New location | Notes |
|--------------|--------------|-------|
| `config.toml` (root) | `config/base.toml` | Complete default config |
| Hardcoded risk defaults in `arb-risk` | `config/*.toml` + `arb-config::RiskConfig` | Single authoritative source |
| Hardcoded Jito defaults in `arb-constants` | `config/*.toml` + `arb-config::JitoConfig` | Constants stay; operational defaults move |
| `.env` | `.env.example` + real `.env` | Never commit real `.env` |
| `config.toml` secret references | `config/config.schema.json` | Secret env names only |

## 3. Steps

1. **Copy** `config/config.example.toml` → `config.toml` and fill in values.
2. **Copy** `.env.example` → `.env` and fill in real secret values (never commit).
3. **Remove** any hardcoded operational defaults that now live in config (e.g.
   duplicate `max_orders_per_hour` defaults — keep only `RiskConfig` = 120).
4. **Verify** address consistency: the regression tests in `arb-types` lock
   canonical Kamino USDC reserve and Raydium vault addresses.
5. **Run** `cargo test -p arb-config -p arb-types` to confirm validation and
   regression tests pass.
6. **Run** the full workspace test suite before deploying.

## 4. Backward Compatibility

The old `crates/arb-bot/src/main.rs::config::load` flat struct still exists for
the running binary. The new `arb-config` crate is additive; migration of the
bot to consume `arb-config` is a follow-up step (Phase 3). Config files in
`config/` are compatible with both during the transition.

## 5. Rollback

If validation fails at startup, the bot reports an actionable error identifying
the config key (no secrets revealed) and exits without trading (fail closed).
Restore the backed-up `config.toml` and retry.
