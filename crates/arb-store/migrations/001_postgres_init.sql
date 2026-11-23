-- PostgreSQL Initial Production Schema for Solana Arbitrage Bot
-- Enforces BIGINT for Solana 64-bit integer values (Lamports, slots) and TIMESTAMPTZ.

CREATE TABLE IF NOT EXISTS attempts (
    id BIGINT PRIMARY KEY,
    slot BIGINT NOT NULL,
    route_id VARCHAR(128) NOT NULL,
    expected_net BIGINT NOT NULL,
    tip_lamports BIGINT,
    decision VARCHAR(32) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    -- Economic reconstruction fields (base-mint raw atomic units unless the
    -- column name states lamports). NULL = unknown; financial aggregation
    -- must tolerate NULLs and never assume zero costs.
    borrow_amount BIGINT,
    flash_fee_raw BIGINT,
    priority_fee_lamports BIGINT,
    expected_output_raw BIGINT,
    pool_ids TEXT
);

CREATE TABLE IF NOT EXISTS outcomes (
    id BIGSERIAL PRIMARY KEY,
    attempt_id BIGINT NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    status VARCHAR(32) NOT NULL,
    realized_net BIGINT,
    provenance VARCHAR(16),
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
    -- Post-execution reconstruction: actual route output (base-mint raw) and
    -- the machine-readable failure reason for errored outcomes.
    actual_output_raw BIGINT,
    failure_reason TEXT
);
-- Databases created before provenance existed evolve in place (idempotent).
ALTER TABLE outcomes ADD COLUMN IF NOT EXISTS provenance VARCHAR(16);
CREATE INDEX IF NOT EXISTS idx_pg_outcomes_pnl ON outcomes(status, provenance, created_at);

CREATE TABLE IF NOT EXISTS fills (
    id BIGSERIAL PRIMARY KEY,
    attempt_id BIGINT NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    token VARCHAR(64) NOT NULL,
    amount BIGINT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS token_prices (
    mint VARCHAR(64) PRIMARY KEY,
    price_usd DOUBLE PRECISION NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS circuit_breaker_events (
    id BIGSERIAL PRIMARY KEY,
    slot BIGINT NOT NULL,
    reason TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

-- Performance Indexes
CREATE INDEX IF NOT EXISTS idx_pg_attempts_slot ON attempts(slot);
CREATE INDEX IF NOT EXISTS idx_pg_attempts_created_at ON attempts(created_at);
CREATE INDEX IF NOT EXISTS idx_pg_outcomes_attempt_id ON outcomes(attempt_id);
CREATE INDEX IF NOT EXISTS idx_pg_fills_attempt_id ON fills(attempt_id);

-- Durable dispatch intents (R12) — see 001_init.sql for the reconciliation rule.
CREATE TABLE IF NOT EXISTS dispatch_intents (
    attempt_id BIGINT PRIMARY KEY REFERENCES attempts(id),
    signature TEXT NOT NULL,
    provider TEXT NOT NULL,
    dispatch_slot BIGINT NOT NULL,
    last_valid_block_height BIGINT,
    status TEXT NOT NULL DEFAULT 'pending',
    resolved_status TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP,
    -- Jito bundle id (or SWQoS acceptance id) returned by the provider after
    -- acceptance; NULL until the transport confirms acceptance.
    bundle_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_pg_dispatch_intents_status ON dispatch_intents(status);

-- Databases created before the economic-reconstruction columns existed evolve
-- in place (idempotent). New deployments get them from CREATE TABLE above.
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS borrow_amount BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS flash_fee_raw BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS priority_fee_lamports BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS expected_output_raw BIGINT;
ALTER TABLE attempts ADD COLUMN IF NOT EXISTS pool_ids TEXT;
ALTER TABLE outcomes ADD COLUMN IF NOT EXISTS actual_output_raw BIGINT;
ALTER TABLE outcomes ADD COLUMN IF NOT EXISTS failure_reason TEXT;
ALTER TABLE dispatch_intents ADD COLUMN IF NOT EXISTS bundle_id TEXT;
