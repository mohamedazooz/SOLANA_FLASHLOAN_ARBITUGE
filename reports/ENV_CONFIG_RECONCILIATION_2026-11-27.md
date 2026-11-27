# Environment / runtime configuration reconciliation — 2026-11-27

## Findings

- The repository's runtime entry point selects `config/config.toml` by default. The local checkout did not contain that canonical file, so startup would fail unless an explicit config path was supplied.
- `config/production.toml` is the matching legacy runtime schema and is now copied locally to the ignored `config/config.toml` path. No credentials are stored in that file.
- The local `.env` contained several permissive overrides far above the actual production policy. The loader intersects USD notional limits with the configured risk cap, but the environment values obscured the effective limits and were unsafe to reason about.
- The USD profit override was present without `SOL_USD_PRICE`; by design it had no effect. The local environment now uses the explicit lamport-denominated floor instead.
- Several old AMM/Jito/EVM-style tuning aliases had no reader in the active Rust Solana workspace. They were commented out locally rather than left as apparently active controls. Secret values, provider URLs, wallet values, and unrelated service settings were not changed.

## Local `.env` reconciliation

| Setting | Effective production policy |
|---|---:|
| `MAX_TRADE_SIZE_USD` | 1.5 USD |
| `MAX_FLASH_BORROW_USD` | 1.5 USD |
| `MAX_SLIPPAGE_BPS` | 10 bps |
| `MIN_NET_PROFIT_LAMPORTS` | 10,000 lamports |
| `MIN_NET_PROFIT_BPS` | 1 bps |
| `MAX_PRICE_IMPACT_BPS` | 40 bps |
| `MAX_DAILY_LOSS_USD` | 0.10 USD |
| `MAX_HOURLY_LOSS_USD` | 0.03 USD |
| `MAX_INFLIGHT_BUNDLES` | 1 |
| `MAX_CONSECUTIVE_FAILURES` | 3 |
| `MAX_PRIORITY_FEE_SHARE_BPS` | 1,000 bps |
| `JITO_MAX_TIP_SHARE_BPS` | 1,000 bps |
| `ARB_MAX_IMPLAUSIBLE_ROI_BPS` | 1,500 bps |
| `MIN_SOL_RESERVE_LAMPORTS` | 10,000,000 lamports |

`MAX_TRADE_SIZE_SOL=2.0` remains an additional raw-SOL ceiling; the tighter USD risk/borrow cap remains authoritative. `CHRONOS_*` values already matched the production configuration and were left unchanged. Existing live-mode/arming flags were not changed, and the bot was not started.

## Repository hygiene and verification

- `.env` and `config/config.toml` are ignored local runtime files and are not staged or committed.
- `.env.example` is updated with the canonical numeric safety overrides.
- A regression test checks these documented values to prevent future drift.
- No transaction was signed or broadcast. No secrets or provider URLs are included in this report.
