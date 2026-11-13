-- Initial Schema for Solana Arbitrage Bot Persistent Storage
CREATE TABLE IF NOT EXISTS attempts (
    id INTEGER PRIMARY KEY,
    slot INTEGER NOT NULL,
    route_id TEXT NOT NULL,
    expected_net INTEGER NOT NULL,
    tip_lamports INTEGER,
    decision TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    status TEXT NOT NULL,
    realized_net INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS fills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    token TEXT NOT NULL,
    amount INTEGER NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_attempts_slot ON attempts(slot);
CREATE INDEX IF NOT EXISTS idx_outcomes_attempt_id ON outcomes(attempt_id);
CREATE INDEX IF NOT EXISTS idx_fills_attempt_id ON fills(attempt_id);


-- Durable dispatch intents (R12): written before any sendTransaction so a
-- restart during an ambiguous timeout can reconcile by signature instead of
-- blindly resubmitting. Reconciliation rule: check getSignatureStatuses first;
-- only mark 'expired' when the blockhash validity window (slot + margin) has
-- provably passed.
CREATE TABLE IF NOT EXISTS dispatch_intents (
    attempt_id INTEGER PRIMARY KEY REFERENCES attempts(id),
    signature TEXT NOT NULL,
    provider TEXT NOT NULL,
    dispatch_slot INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    resolved_status TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    resolved_at TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_dispatch_intents_status ON dispatch_intents(status);
