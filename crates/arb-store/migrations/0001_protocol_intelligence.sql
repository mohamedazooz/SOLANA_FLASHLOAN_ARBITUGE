-- Migration 0001: protocol intelligence core.
-- Applied after POSTGRES_INIT_SQL by the arb-store migration runner.
-- Amounts are raw integers; NUMERIC(39,0) covers the u128 range.
-- Executable math stays in arb-math; this schema stores metadata,
-- parameters, provenance, and verification results only.

CREATE TABLE protocols (
    protocol_id        TEXT PRIMARY KEY,
    protocol_name      TEXT NOT NULL,
    protocol_type      TEXT NOT NULL,
    program_id         TEXT NOT NULL,
    network            TEXT NOT NULL DEFAULT 'mainnet',
    protocol_version   TEXT,
    supported_features JSONB NOT NULL DEFAULT '[]',
    sdk_name           TEXT,
    sdk_version        TEXT,
    sdk_commit         TEXT,
    source_repository  TEXT,
    documentation_url  TEXT,
    verification_status TEXT NOT NULL DEFAULT 'NOT_IMPLEMENTED',
    enabled            BOOLEAN NOT NULL DEFAULT FALSE,
    last_verified_at   TIMESTAMPTZ
);

CREATE TABLE protocol_math_specs (
    protocol_id       TEXT NOT NULL REFERENCES protocols(protocol_id),
    formula_version   TEXT NOT NULL,
    input_amount_semantics  TEXT NOT NULL,
    output_amount_semantics TEXT NOT NULL,
    price_formula_ref TEXT NOT NULL,
    fee_formula_ref   TEXT NOT NULL,
    rounding_mode     TEXT NOT NULL,
    integer_precision TEXT NOT NULL,
    fee_components    JSONB NOT NULL,
    supported_swap_modes JSONB NOT NULL,
    unsupported_features JSONB NOT NULL,
    reference_source  TEXT NOT NULL,
    verification_test_version TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (protocol_id, formula_version)
);

CREATE TABLE pools (
    pool_address      TEXT PRIMARY KEY,
    protocol_id       TEXT NOT NULL REFERENCES protocols(protocol_id),
    pool_type         TEXT NOT NULL,
    token_mint_a      TEXT NOT NULL,
    token_mint_b      TEXT NOT NULL,
    token_decimals_a  SMALLINT NOT NULL CHECK (token_decimals_a BETWEEN 0 AND 255),
    token_decimals_b  SMALLINT NOT NULL CHECK (token_decimals_b BETWEEN 0 AND 255),
    token_program_a   TEXT NOT NULL,
    token_program_b   TEXT NOT NULL,
    vault_a           TEXT,
    vault_b           TEXT,
    configuration_account TEXT,
    tick_spacing      INTEGER,
    bin_step          INTEGER,
    active_bin_id     BIGINT,
    current_tick      INTEGER,
    pool_status       TEXT NOT NULL DEFAULT 'unknown',
    discovery_source  TEXT NOT NULL,
    discovered_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_verified_at  TIMESTAMPTZ
);
CREATE INDEX idx_pools_protocol ON pools(protocol_id);
CREATE INDEX idx_pools_mints ON pools(token_mint_a, token_mint_b);

CREATE TABLE pool_state_snapshots (
    snapshot_id     BIGSERIAL PRIMARY KEY,
    pool_address    TEXT NOT NULL REFERENCES pools(pool_address),
    slot            BIGINT NOT NULL,
    write_version   BIGINT,
    state_hash      TEXT NOT NULL,
    reserve_a       BIGINT,
    reserve_b       BIGINT,
    sqrt_price      NUMERIC(39,0),
    current_tick    INTEGER,
    active_bin      BIGINT,
    liquidity       NUMERIC(39,0),
    protocol_specific_state JSONB,
    source_provider TEXT NOT NULL,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    state_validity  TEXT NOT NULL DEFAULT 'valid',
    invalidation_reason TEXT
);
CREATE INDEX idx_snapshots_pool_slot ON pool_state_snapshots(pool_address, slot DESC);
CREATE INDEX idx_snapshots_validity ON pool_state_snapshots(state_validity) WHERE state_validity <> 'valid';
