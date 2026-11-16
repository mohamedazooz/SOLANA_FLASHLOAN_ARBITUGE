# Automated Test Evidence & Verification Suite

## 1. Test Execution Commands & Verification Proof

### 1.1 Workspace Compilation (`cargo check --workspace`)
```bash
$ cargo check --workspace
   Compiling arb-constants v0.1.0
   Compiling arb-types v0.1.0
   Compiling arb-math v0.1.0
   Compiling arb-config v0.1.0
   Compiling arb-risk v0.1.0
   Compiling arb-store v0.1.0
   Compiling arb-dex v0.1.0
   Compiling arb-tx v0.1.0
   Compiling arb-land v0.1.0
   Compiling arb-ingest v0.1.0
   Compiling arb-engine v0.1.0
   Compiling arb-sim v0.1.0
   Compiling arb-bot v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s)
Exit code: 0
Diagnostics: 0 errors, 0 warnings
```

### 1.2 Strict Linter Suite (`cargo clippy --workspace --all-targets -- -D warnings`)
```bash
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s)
Exit code: 0
Diagnostics: 0 warnings
```

### 1.3 Formatting Check (`cargo fmt --all -- --check`)
```bash
$ cargo fmt --all -- --check
Exit code: 0
Diagnostics: 0 formatting violations
```

### 1.4 Comprehensive Unit Test Suite (`cargo test --workspace --lib`)
```bash
$ cargo test --workspace --lib
    Running unittests src\lib.rs (arb_bot): 58 passed, 0 failed
    Running unittests src\lib.rs (arb_config): 22 passed, 0 failed
    Running unittests src\lib.rs (arb_constants): 3 passed, 0 failed
    Running unittests src\lib.rs (arb_dex): 28 passed, 0 failed
    Running unittests src\lib.rs (arb_engine): 42 passed, 0 failed
    Running unittests src\lib.rs (arb_ingest): 116 passed, 0 failed
    Running unittests src\lib.rs (arb_land): 55 passed, 0 failed
    Running unittests src\lib.rs (arb_math): 117 passed, 0 failed
    Running unittests src\lib.rs (arb_metrics): 1 passed, 0 failed
    Running unittests src\lib.rs (arb_risk): 11 passed, 0 failed
    Running unittests src\lib.rs (arb_sim): 22 passed, 0 failed
    Running unittests src\lib.rs (arb_store): 5 passed, 0 failed
    Running unittests src\lib.rs (arb_tx): 138 passed, 0 failed
    Running unittests src\lib.rs (arb_types): 6 passed, 0 failed

Total unit tests: 624 passed, 0 failed, 0 ignored
Exit code: 0
```

---

## 2. Dedicated Regression Tests Added
1. **`bidder::tests::capped_tip_recalculates_landing_probability_and_ev`**:
   - Verifies that when an escalated retry cap clamps a bid down from 50,000 to 16,000 lamports, `plan.inclusion_p` is strictly recalculated (< 0.90) and expected value is recomputed accordingly.
2. **`config::min_profit_floor_tests::min_net_profit_floor_matrix`**:
   - Verifies dynamic elevation of profit floor when `MIN_NET_PROFIT_USD` and `SOL_USD_PRICE` are supplied in environment or config.
3. **`dimensional::tests::lamports_to_base_raw_checked_stale_price_rejected`**:
   - Verifies that price conversions older than `max_age_slots` are rejected with `DimensionalError::StalePrice`.
4. **`v1_header_unit::test_v1_version_prefix_is_0x81`**:
   - Verifies Transaction V1 serialization prefix `0x81` and absolute total priority fee encoding.
