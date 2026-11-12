-- PostgreSQL Initial Production Schema for Solana Arbitrage Bot
-- Enforces BIGINT for Solana 64-bit integer values (Lamports, slots) and TIMESTAMPTZ.

CREATE TABLE IF NOT EXISTS attempts (
    id BIGINT PRIMARY KEY,
    slot BIGINT NOT NULL,
    route_id VARCHAR(128) NOT NULL,
    expected_net BIGINT NOT NULL,
    tip_lamports BIGINT,
    decision VARCHAR(32) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS outcomes (
    id BIGSERIAL PRIMARY KEY,
    attempt_id BIGINT NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
    status VARCHAR(32) NOT NULL,
    realized_net BIGINT,
    created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);

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
