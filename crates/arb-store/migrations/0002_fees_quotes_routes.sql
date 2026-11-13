-- Migration 0002: fees, quotes, routes, and SDK reconciliation.
-- Double-count prevention: route_simulations stores only network, priority,
-- tip, flash-loan, and token-transfer costs; protocol fees already included
-- in quote_records amounts are never subtracted again.

CREATE TABLE fee_registry (
    fee_id          BIGSERIAL PRIMARY KEY,
    protocol_id     TEXT REFERENCES protocols(protocol_id),
    fee_component   TEXT NOT NULL,
    config_source   TEXT NOT NULL,
    calculation_version TEXT NOT NULL,
    effective_slot  BIGINT,
    fee_numerator   BIGINT,
    fee_denominator BIGINT,
    rounding_rule   TEXT NOT NULL,
    verification_status TEXT NOT NULL DEFAULT 'unverified',
    observed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (protocol_id, fee_component, config_source, effective_slot)
);

CREATE TABLE quote_records (
    quote_id        BIGSERIAL PRIMARY KEY,
    protocol_id     TEXT NOT NULL REFERENCES protocols(protocol_id),
    pool_address    TEXT NOT NULL REFERENCES pools(pool_address),
    input_mint      TEXT NOT NULL,
    output_mint     TEXT NOT NULL,
    amount_in_raw   NUMERIC(39,0) NOT NULL,
    amount_out_raw  NUMERIC(39,0) NOT NULL,
    fees_breakdown  JSONB NOT NULL,
    transfer_fee_breakdown JSONB,
    price_impact_bps BIGINT,
    unfilled_input_raw NUMERIC(39,0),
    minimum_output_raw NUMERIC(39,0),
    state_slot      BIGINT NOT NULL,
    state_hash      TEXT NOT NULL,
    quote_engine_version TEXT NOT NULL,
    sdk_reference_version TEXT,
    quote_latency_us BIGINT,
    quote_validity  TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_quotes_pool_time ON quote_records(pool_address, created_at DESC);

CREATE TABLE routes (
    route_id        BIGSERIAL PRIMARY KEY,
    route_type      TEXT NOT NULL,
    number_of_pools SMALLINT NOT NULL CHECK (number_of_pools >= 2),
    ordered_pool_addresses TEXT[] NOT NULL,
    ordered_protocol_ids   TEXT[] NOT NULL,
    input_mint      TEXT NOT NULL,
    intermediate_mints TEXT[] NOT NULL,
    output_mint     TEXT NOT NULL,
    route_version   TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    last_validation_at TIMESTAMPTZ,
    UNIQUE (ordered_pool_addresses, input_mint, route_version)
);

CREATE TABLE route_simulations (
    simulation_id   BIGSERIAL PRIMARY KEY,
    route_id        BIGINT NOT NULL REFERENCES routes(route_id),
    input_amount_raw  NUMERIC(39,0) NOT NULL,
    output_amount_raw NUMERIC(39,0),
    principal_raw     NUMERIC(39,0),
    flash_loan_fee_raw NUMERIC(39,0),
    network_fee_raw   BIGINT,
    priority_fee_raw  BIGINT,
    tip_raw           BIGINT,
    token_transfer_fees_raw JSONB,
    total_costs_raw   NUMERIC(39,0),
    net_profit_raw    BIGINT,
    transaction_size_bytes INTEGER,
    compute_units_consumed BIGINT,
    simulation_slot   BIGINT,
    simulation_result TEXT NOT NULL,
    simulation_error  TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_sims_route_time ON route_simulations(route_id, created_at DESC);
CREATE INDEX idx_sims_result ON route_simulations(simulation_result) WHERE simulation_result <> 'passed';

CREATE TABLE sdk_reconciliations (
    reconciliation_id BIGSERIAL PRIMARY KEY,
    protocol_id     TEXT NOT NULL REFERENCES protocols(protocol_id),
    pool_address    TEXT NOT NULL,
    input_amount_raw NUMERIC(39,0) NOT NULL,
    local_quote     NUMERIC(39,0) NOT NULL,
    sdk_quote       NUMERIC(39,0),
    onchain_simulated_quote NUMERIC(39,0),
    difference_raw  BIGINT,
    difference_bps  BIGINT,
    state_slot      BIGINT NOT NULL,
    result          TEXT NOT NULL,
    failure_reason  TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_recon_protocol_result ON sdk_reconciliations(protocol_id, result, created_at DESC);

-- Observability views (Gate F reporting source; no second dashboard stack).
CREATE VIEW v_protocol_coverage AS
SELECT p.protocol_id, p.verification_status, p.enabled,
       r.result AS latest_reconciliation_result, r.created_at AS last_reconciliation_at
FROM protocols p
LEFT JOIN LATERAL (
    SELECT result, created_at FROM sdk_reconciliations s
    WHERE s.protocol_id = p.protocol_id
    ORDER BY created_at DESC LIMIT 1
) r ON TRUE;

CREATE VIEW v_stale_state_events AS
SELECT pool_address, slot, state_validity, invalidation_reason, source_provider, received_at
FROM pool_state_snapshots WHERE state_validity <> 'valid';

CREATE VIEW v_latency_percentiles AS
SELECT
    percentile_cont(0.5) WITHIN GROUP (ORDER BY quote_latency_us) AS p50_us,
    percentile_cont(0.95) WITHIN GROUP (ORDER BY quote_latency_us) AS p95_us,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY quote_latency_us) AS p99_us
FROM quote_records WHERE quote_latency_us IS NOT NULL;
