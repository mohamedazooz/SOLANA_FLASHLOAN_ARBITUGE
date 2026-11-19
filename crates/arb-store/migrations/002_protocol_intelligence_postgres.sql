-- PostgreSQL Protocol Intelligence & Historical Verification Schema
-- Version: 002
-- Complies with SOLANA_FLASHLOAN_ARBITUGE Protocol Intelligence Specifications

-- 1. Protocol Registry
CREATE TABLE IF NOT EXISTS protocol_registry (
    protocol_id VARCHAR(64) PRIMARY KEY,
    protocol_name VARCHAR(128) NOT NULL,
    protocol_type VARCHAR(32) NOT NULL, -- CPMM, CLMM, DLMM, BONDING_CURVE
    program_id VARCHAR(64) NOT NULL,
    network VARCHAR(32) NOT NULL DEFAULT 'mainnet-beta',
    protocol_version VARCHAR(32) NOT NULL,
    supported_features TEXT NOT NULL,
    sdk_name VARCHAR(128),
    sdk_version VARCHAR(64),
    sdk_commit VARCHAR(64),
    source_repository TEXT,
    documentation_url TEXT,
    verification_status VARCHAR(32) NOT NULL DEFAULT 'IMPLEMENTED_UNVERIFIED',
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_verified_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 2. Protocol Mathematical Specifications
CREATE TABLE IF NOT EXISTS protocol_math_specs (
    spec_id BIGSERIAL PRIMARY KEY,
    protocol_id VARCHAR(64) NOT NULL REFERENCES protocol_registry(protocol_id) ON DELETE CASCADE,
    formula_version VARCHAR(32) NOT NULL,
    input_amount_semantics VARCHAR(64) NOT NULL,
    output_amount_semantics VARCHAR(64) NOT NULL,
    price_formula TEXT NOT NULL,
    fee_formula TEXT NOT NULL,
    rounding_mode VARCHAR(64) NOT NULL,
    integer_precision VARCHAR(64) NOT NULL,
    fee_components TEXT NOT NULL,
    supported_swap_modes TEXT NOT NULL,
    unsupported_features TEXT,
    reference_source TEXT NOT NULL,
    verification_test_version VARCHAR(64) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 3. Pool Registry
CREATE TABLE IF NOT EXISTS pool_registry (
    pool_address VARCHAR(64) PRIMARY KEY,
    protocol_id VARCHAR(64) NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_type VARCHAR(32) NOT NULL,
    token_mint_a VARCHAR(64) NOT NULL,
    token_mint_b VARCHAR(64) NOT NULL,
    token_decimals_a SMALLINT NOT NULL,
    token_decimals_b SMALLINT NOT NULL,
    token_program_a VARCHAR(64) NOT NULL,
    token_program_b VARCHAR(64) NOT NULL,
    vault_a VARCHAR(64) NOT NULL,
    vault_b VARCHAR(64) NOT NULL,
    configuration_account VARCHAR(64),
    tick_spacing INTEGER,
    bin_step INTEGER,
    active_bin_id INTEGER,
    current_tick INTEGER,
    pool_status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE',
    discovery_source VARCHAR(64) NOT NULL,
    discovered_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    last_verified_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 4. Pool State Snapshots
CREATE TABLE IF NOT EXISTS pool_state_snapshots (
    snapshot_id BIGSERIAL PRIMARY KEY,
    pool_address VARCHAR(64) NOT NULL REFERENCES pool_registry(pool_address) ON DELETE CASCADE,
    slot BIGINT NOT NULL,
    write_version BIGINT NOT NULL DEFAULT 0,
    state_hash VARCHAR(64) NOT NULL,
    reserve_a NUMERIC(39, 0) NOT NULL,
    reserve_b NUMERIC(39, 0) NOT NULL,
    sqrt_price VARCHAR(64),
    current_tick INTEGER,
    active_bin INTEGER,
    liquidity VARCHAR(64),
    protocol_specific_state TEXT,
    source_provider VARCHAR(64) NOT NULL,
    received_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    state_validity VARCHAR(32) NOT NULL DEFAULT 'VALID',
    invalidation_reason TEXT
);

-- 5. Fee Registry
CREATE TABLE IF NOT EXISTS fee_registry (
    fee_id BIGSERIAL PRIMARY KEY,
    protocol_id VARCHAR(64) NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address VARCHAR(64),
    fee_type VARCHAR(32) NOT NULL,
    configuration_source VARCHAR(64) NOT NULL,
    calculation_version VARCHAR(32) NOT NULL,
    effective_slot BIGINT NOT NULL DEFAULT 0,
    fee_numerator BIGINT NOT NULL DEFAULT 0,
    fee_denominator BIGINT NOT NULL DEFAULT 10000,
    fee_bps INTEGER NOT NULL DEFAULT 0,
    rounding_rule VARCHAR(32) NOT NULL DEFAULT 'CEILING',
    verification_status VARCHAR(32) NOT NULL DEFAULT 'VERIFIED',
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 6. Quote Records
CREATE TABLE IF NOT EXISTS quote_records (
    quote_id VARCHAR(64) PRIMARY KEY,
    protocol_id VARCHAR(64) NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address VARCHAR(64) NOT NULL,
    input_mint VARCHAR(64) NOT NULL,
    output_mint VARCHAR(64) NOT NULL,
    amount_in_raw VARCHAR(64) NOT NULL,
    amount_out_raw VARCHAR(64) NOT NULL,
    fees_breakdown TEXT NOT NULL,
    price_impact_bps INTEGER NOT NULL DEFAULT 0,
    unfilled_input_raw VARCHAR(64) NOT NULL DEFAULT '0',
    minimum_output_raw VARCHAR(64) NOT NULL,
    state_slot BIGINT NOT NULL,
    state_hash VARCHAR(64) NOT NULL,
    quote_engine_version VARCHAR(32) NOT NULL,
    sdk_reference_version VARCHAR(32),
    quote_latency_us BIGINT NOT NULL,
    quote_validity VARCHAR(32) NOT NULL DEFAULT 'VALID',
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 7. Route Registry
CREATE TABLE IF NOT EXISTS route_registry (
    route_id VARCHAR(128) PRIMARY KEY,
    route_type VARCHAR(32) NOT NULL, -- SPATIAL_2HOP, TRIANGULAR_3HOP
    number_of_pools SMALLINT NOT NULL,
    ordered_pool_addresses TEXT NOT NULL,
    ordered_protocol_ids TEXT NOT NULL,
    input_mint VARCHAR(64) NOT NULL,
    intermediate_mints TEXT NOT NULL,
    output_mint VARCHAR(64) NOT NULL,
    route_version VARCHAR(32) NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_validation_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 8. Route Simulation Records
CREATE TABLE IF NOT EXISTS route_simulation_records (
    simulation_id VARCHAR(64) PRIMARY KEY,
    route_id VARCHAR(128) NOT NULL REFERENCES route_registry(route_id) ON DELETE CASCADE,
    input_amount_raw VARCHAR(64) NOT NULL,
    output_amount_raw VARCHAR(64) NOT NULL,
    principal_raw VARCHAR(64) NOT NULL,
    flash_loan_fee_raw VARCHAR(64) NOT NULL DEFAULT '0',
    network_fee_raw VARCHAR(64) NOT NULL DEFAULT '5000',
    priority_fee_raw VARCHAR(64) NOT NULL DEFAULT '0',
    tip_raw VARCHAR(64) NOT NULL DEFAULT '0',
    token_transfer_fees_raw VARCHAR(64) NOT NULL DEFAULT '0',
    total_costs_raw VARCHAR(64) NOT NULL,
    net_profit_raw VARCHAR(64) NOT NULL,
    transaction_size_bytes INTEGER NOT NULL,
    compute_units_consumed BIGINT NOT NULL,
    simulation_slot BIGINT NOT NULL,
    simulation_result VARCHAR(32) NOT NULL, -- SUCCESS, FAILED
    simulation_error TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- 9. SDK Reconciliation Records
CREATE TABLE IF NOT EXISTS sdk_reconciliation_records (
    reconciliation_id BIGSERIAL PRIMARY KEY,
    protocol_id VARCHAR(64) NOT NULL REFERENCES protocol_registry(protocol_id),
    pool_address VARCHAR(64) NOT NULL,
    input_amount_raw VARCHAR(64) NOT NULL,
    local_quote_raw VARCHAR(64) NOT NULL,
    sdk_quote_raw VARCHAR(64) NOT NULL,
    onchain_simulated_quote_raw VARCHAR(64),
    difference_raw VARCHAR(64) NOT NULL,
    difference_bps INTEGER NOT NULL,
    state_slot BIGINT NOT NULL,
    result VARCHAR(32) NOT NULL, -- EXACT_MATCH, WITHIN_TOLERANCE, MISMATCH, FAILED
    failure_reason TEXT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Performance Indexes
CREATE INDEX IF NOT EXISTS idx_pool_snapshots_slot ON pool_state_snapshots(pool_address, slot DESC);
CREATE INDEX IF NOT EXISTS idx_pool_snapshots_created ON pool_state_snapshots(received_at);
CREATE INDEX IF NOT EXISTS idx_quote_records_pool_slot ON quote_records(pool_address, state_slot DESC);
CREATE INDEX IF NOT EXISTS idx_quote_records_created ON quote_records(created_at);
CREATE INDEX IF NOT EXISTS idx_sim_records_route ON route_simulation_records(route_id, simulation_slot DESC);
CREATE INDEX IF NOT EXISTS idx_sim_records_created ON route_simulation_records(created_at);
CREATE INDEX IF NOT EXISTS idx_reconciliation_protocol ON sdk_reconciliation_records(protocol_id, created_at DESC);
