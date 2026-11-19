# Remediation Changelog

## [2026-11-16] — Production Economic Engine & Hardening Remediation

### Added
- **`calibrated_inclusion_probability` in `crates/arb-land/src/bidder.rs`**: Dynamically recalculates monotonic landing probability for any clamped or intermediate bid amount.
- **`calibrated_inclusion_probability` on `JitoService`**: Cleanly exposes calibrated probability calculation without breaking encapsulation of `LandingLedger`.
- **`SOLANA_RUNTIME_MAX_COMPUTE_UNITS` (1,400,000)** and **`APPLICATION_SAFETY_CU_LIMIT` (350,000)** in `crates/arb-constants/src/lib.rs`: Explicitly distinguishes consensus protocol limits from application safety ceilings.
- **`SWQOS_DEFAULT_PRIORITY_FEE_LAMPORTS` (2,000)** and **`SWQOS_MIN_EDGE_HURDLE_LAMPORTS` (8,000)** in `crates/arb-bot/src/jito_service.rs`: Replaced unlinked hardcoded magic numbers with named, typed constants.
- **Automated Regression Test**: `bidder::tests::capped_tip_recalculates_landing_probability_and_ev` verifying post-selection clipping recalculation.
- **Deliverables**: Comprehensive audit reports `PHASE_00_BASELINE.md` through `CHANGELOG.md` in `reports/remediation/`.

### Fixed
- **Post-Selection Tip Clipping**: Fixed bug where retry ladder cap or supervisor $t_{max}$ reduced the tip while retaining upper-rung inclusion probability. Inclusion probability and expected value are now immediately recalculated whenever a tip is modified.
- **USD / Lamport Floor Wiring**: Fixed `crates/arb-bot/src/config.rs:424` to route `from_arb_config` through `min_net_profit_floor_lamports`, ensuring `MIN_NET_PROFIT_USD` and `SOL_USD_PRICE` properly elevate the execution floor.
- **Hardcoded 50k Alert Checks**: Replaced hardcoded `gross >= 50_000` checks in `crates/arb-bot/src/supervisor/opportunity.rs` with `gross >= min_net_floor` (`self.config.strategy.min_net_profit_abs`).
- **Defaults Drift**: Harmonized `three_leg_default_min_profit` to `DEFAULT_MIN_PROFIT_LAMPORTS = 50_000`, and reconciled `DEFAULT_KEEP_SHARE` (0.60), `DEFAULT_MAX_TIP_SHARE_BPS` (7,000), and `DEFAULT_MIN_KEEP_LAMPORTS` (10,000) with `config/base.toml`.

### Changed
- Refactored `AdaptiveBidder::calculate_contention_hurdle` to cleanly handle low-contention DLMM pool execution while respecting configured profit floor bounds.
