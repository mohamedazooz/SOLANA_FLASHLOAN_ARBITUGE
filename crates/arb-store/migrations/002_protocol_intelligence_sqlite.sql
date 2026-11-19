-- SQLite Protocol Intelligence & Historical Verification Schema
-- Version: 002
-- Complies with SOLANA_FLASHLOAN_ARBITUGE Protocol Intelligence Specifications

-- 1. Protocol Registry
CREATE TABLE IF NOT EXISTS protocol_registry (
    protocol_id TEXT PRIMARY KEY,
    protocol_name TEXT NOT NULL,
    protocol_type TEXT NOT NULL, -- CPMM, CLMM, DLMM, BONDING_CURVE
    program_id TEXT NOT NULL,
    network TEXT NOT NULL DEFAULT 'mainnet-beta',
    protocol_version TEXT NOT NULL,
    supported_features TEXT NOT NULL,
    sdk_name TEXT,
    sdk_version TEXT,
    sdk_commit TEXT,
    source_repository TEXT,
    documentation_url TEXT,
    verification_status TEXT NOT NULL DEFAULT 'IMPLEMENTED_UNVERIFIED',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 2. Protocol Mathematical Specifications
CREATE TABLE IF NOT EXISTS protocol_math_specs (
    spec_id INTEGER PRIMARY KEY AUTOINCREMENT,
    protocol_id TEXT NOT NULL REFERENCES protocol_registry(protocol_id) ON DELETE CASCADE,
    formula_version TEXT NOT NULL,
    input_amount_semantics TEXT NOT NULL,
    output_amount_semantics TEXT NOT NULL,
    price_formula TEXT NOT NULL,
    fee_formula TEXT NOT NULL,
    rounding_mode TEXT NOT NULL,
    integer_precision TEXT NOT NULL,
    fee_components TEXT NOT NULL,
    supported_swap_modes TEXT NOT NULL,
    unsupported_features TEXT,
    reference_source TEXT NOT NULL,
    verification_test_version TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 3. Pool Registry
CREATE TABLE IF NOT EXISTS pool_registry (
    pool_address TEXT PRIMARY KEY,
    protocol_id TEXT NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_type TEXT NOT NULL,
    token_mint_a TEXT NOT NULL,
    token_mint_b TEXT NOT NULL,
    token_decimals_a INTEGER NOT NULL,
    token_decimals_b INTEGER NOT NULL,
    token_program_a TEXT NOT NULL,
    token_program_b TEXT NOT NULL,
    vault_a TEXT NOT NULL,
    vault_b TEXT NOT NULL,
    configuration_account TEXT,
    tick_spacing INTEGER,
    bin_step INTEGER,
    active_bin_id INTEGER,
    current_tick INTEGER,
    pool_status TEXT NOT NULL DEFAULT 'ACTIVE',
    discovery_source TEXT NOT NULL,
    discovered_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 4. Pool State Snapshots
CREATE TABLE IF NOT EXISTS pool_state_snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    pool_address TEXT NOT NULL REFERENCES pool_registry(pool_address) ON DELETE CASCADE,
    slot INTEGER NOT NULL,
    write_version INTEGER NOT NULL DEFAULT 0,
    state_hash TEXT NOT NULL,
    reserve_a TEXT NOT NULL,
    reserve_b TEXT NOT NULL,
    sqrt_price TEXT,
    current_tick INTEGER,
    active_bin INTEGER,
    liquidity TEXT,
    protocol_specific_state TEXT,
    source_provider TEXT NOT NULL,
    received_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    state_validity TEXT NOT NULL DEFAULT 'VALID',
    invalidation_reason TEXT
);

-- 5. Fee Registry
CREATE TABLE IF NOT EXISTS fee_registry (
    fee_id INTEGER PRIMARY KEY AUTOINCREMENT,
    protocol_id TEXT NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address TEXT,
    fee_type TEXT NOT NULL,
    configuration_source TEXT NOT NULL,
    calculation_version TEXT NOT NULL,
    effective_slot INTEGER NOT NULL DEFAULT 0,
    fee_numerator INTEGER NOT NULL DEFAULT 0,
    fee_denominator INTEGER NOT NULL DEFAULT 10000,
    fee_bps INTEGER NOT NULL DEFAULT 0,
    rounding_rule TEXT NOT NULL DEFAULT 'CEILING',
    verification_status TEXT NOT NULL DEFAULT 'VERIFIED',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 6. Quote Records
CREATE TABLE IF NOT EXISTS quote_records (
    quote_id TEXT PRIMARY KEY,
    protocol_id TEXT NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address TEXT NOT NULL,
    input_mint TEXT NOT NULL,
    output_mint TEXT NOT NULL,
    amount_in_raw TEXT NOT NULL,
    amount_out_raw TEXT NOT NULL,
    fees_breakdown TEXT NOT NULL,
    price_impact_bps INTEGER NOT NULL DEFAULT 0,
    unfilled_input_raw TEXT NOT NULL DEFAULT '0',
    minimum_output_raw TEXT NOT NULL,
    state_slot INTEGER NOT NULL,
    state_hash TEXT NOT NULL,
    quote_engine_version TEXT NOT NULL,
    sdk_reference_version TEXT,
    quote_latency_us INTEGER NOT NULL,
    quote_validity TEXT NOT NULL DEFAULT 'VALID',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 7. Route Registry
CREATE TABLE IF NOT EXISTS route_registry (
    route_id TEXT PRIMARY KEY,
    route_type TEXT NOT NULL, -- SPATIAL_2HOP, TRIANGULAR_3HOP
    number_of_pools INTEGER NOT NULL,
    ordered_pool_addresses TEXT NOT NULL,
    ordered_protocol_ids TEXT NOT NULL,
    input_mint TEXT NOT NULL,
    intermediate_mints TEXT NOT NULL,
    output_mint TEXT NOT NULL,
    route_version TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_validation_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 8. Route Simulation Records
CREATE TABLE IF NOT EXISTS route_simulation_records (
    simulation_id TEXT PRIMARY KEY,
    route_id TEXT NOT NULL REFERENCES route_registry(route_id) ON DELETE CASCADE,
    input_amount_raw TEXT NOT NULL,
    output_amount_raw TEXT NOT NULL,
    principal_raw TEXT NOT NULL,
    flash_loan_fee_raw TEXT NOT NULL DEFAULT '0',
    network_fee_raw TEXT NOT NULL DEFAULT '5000',
    priority_fee_raw TEXT NOT NULL DEFAULT '0',
    tip_raw TEXT NOT NULL DEFAULT '0',
    token_transfer_fees_raw TEXT NOT NULL DEFAULT '0',
    total_costs_raw TEXT NOT NULL,
    net_profit_raw TEXT NOT NULL,
    transaction_size_bytes INTEGER NOT NULL,
    compute_units_consumed INTEGER NOT NULL,
    simulation_slot INTEGER NOT NULL,
    simulation_result TEXT NOT NULL, -- SUCCESS, FAILED
    simulation_error TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 9. SDK Reconciliation Records
CREATE TABLE IF NOT EXISTS sdk_reconciliation_records (
    reconciliation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    protocol_id TEXT NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address TEXT NOT NULL,
    input_amount_raw TEXT NOT NULL,
    local_quote_raw TEXT NOT NULL,
    sdk_quote_raw TEXT NOT NULL,
    onchain_simulated_quote_raw TEXT,
    difference_raw TEXT NOT NULL,
    difference_bps INTEGER NOT NULL,
    state_slot INTEGER NOT NULL,
    result TEXT NOT NULL, -- EXACT_MATCH, WITHIN_TOLERANCE, MISMATCH, FAILED
    failure_reason TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Performance Indexes
CREATE INDEX IF NOT EXISTS idx_sq_pool_snapshots_slot ON pool_state_snapshots(pool_address, slot DESC);
CREATE INDEX IF NOT EXISTS idx_sq_pool_snapshots_created ON pool_state_snapshots(received_at);
CREATE INDEX IF NOT EXISTS idx_sq_quote_records_pool_slot ON quote_records(pool_address, state_slot DESC);
CREATE INDEX IF NOT EXISTS idx_sq_quote_records_created ON quote_records(created_at);
CREATE INDEX IF NOT EXISTS idx_sq_sim_records_route ON route_simulation_records(route_id, simulation_slot DESC);
CREATE INDEX IF NOT EXISTS idx_sq_sim_records_created ON route_simulation_records(created_at);
CREATE INDEX IF NOT EXISTS idx_sq_reconciliation_protocol ON sdk_reconciliation_records(protocol_id, created_at DESC);
