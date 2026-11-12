# Release notes — F2.2 (2026-11-07)

Status: **Shadow only / not approved for live execution**.

## Changes

- PumpSwap Pool joins verify the canonical PDA, stored bump, and canonical base/quote vault ATAs before accepting account state.
- PumpSwap quote adapter now carries GlobalConfig direction flags and rejects only the disabled direction.
- Mayhem and Holder Rewards no longer get dropped solely because of their mode flag when quote fees are verified.
- Custom-pair creator-fee override is gated separately from SOL/USDC fee tiers.
- FeeConfig and GlobalConfig snapshot freshness is checked before pool promotion.
- Supervisor implementation was separated into lifecycle, execution-helper and dispatch modules; no `include!`-based code splitting was used.
- Pump state-join tests were separated from production state code to keep both files under 1,000 lines.
- PumpSwap remains candidate/quote-adapter only: exact quote is not yet wired into the generic route evaluator, and transaction construction remains disabled.
- Added protocol reference and phase acceptance documents.

## Validation

- ZIP archive integrity: to be verified during packaging.
- Static TOML/JSON/YAML and source-structure checks: to be run during packaging.
- Rust build, unit/integration tests, clippy and Docker build: not run because the environment has no Cargo/Rust/Docker toolchain.
- No live transaction signing or submission was performed.
