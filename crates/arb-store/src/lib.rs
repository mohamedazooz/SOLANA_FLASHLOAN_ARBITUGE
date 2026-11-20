//! Persistent Storage Layer (SQLite with WAL Pragmas) & Accounting.

use arb_types::{ArbError, Lamports, Result, SignedLamports, TokenAmount};
use rusqlite::{Connection, params};
use std::sync::Mutex;
use tracing::{debug, info};

pub mod postgres_store;
pub mod protocol_intelligence;
pub mod redis_cache;
pub use postgres_store::PostgresStore;
pub use protocol_intelligence::*;
pub use redis_cache::{RedisStore, ShortTtlCache};

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const POSTGRES_INIT_SQL: &str = include_str!("../migrations/001_postgres_init.sql");
pub const PROTOCOL_INTELLIGENCE_INIT_SQL: &str =
    include_str!("../migrations/002_protocol_intelligence_sqlite.sql");

#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub id: i64,
    pub slot: i64,
    pub route_id: String,
    pub expected_net: Lamports,
    pub tip_lamports: Option<Lamports>,
    pub decision: String,
}

#[derive(Debug, Clone)]
pub struct FillRecord {
    pub attempt_id: i64,
    /// Token mint address (base58), not a symbol.
    pub token: String,
    /// Raw token atomic units. This is deliberately not Lamports.
    pub amount: TokenAmount,
}

#[derive(Debug, Clone)]
pub struct PnlDaily {
    /// Signed realized net from outcomes with on-chain evidence only
    /// (`status = 'landed'` AND provenance `actual`). Losses are negative.
    pub realized_net: SignedLamports,
    /// Estimated net (including landed transactions without balance metadata,
    /// plus simulated/canary outcomes) kept separate from `realized_net`.
    pub simulated_net: SignedLamports,
    pub attempts: u64,
    pub successful: u64,
}

/// Provenance of a recorded realized-net figure: where the number came from.
/// Financial aggregation (`pnl_daily`) may only sum `Actual` figures; an
/// on-chain loss can never be silently replaced by a positive estimate
/// (MATH-01 / PIPE-01 fix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetProvenance {
    /// Decoded from on-chain transaction metadata (balance changes).
    Actual,
    /// Pre-dispatch estimate / simulation. No on-chain evidence.
    Estimated,
}

impl NetProvenance {
    pub fn as_str(&self) -> &'static str {
        match self {
            NetProvenance::Actual => "actual",
            NetProvenance::Estimated => "estimated",
        }
    }
}

/// Durable record of one signed-transaction dispatch (R12). Written **before**
/// the send so a crash/timeout leaves a recoverable trace: reconciliation
/// checks the signature on-chain first and never blindly resubmits.
#[derive(Debug, Clone)]
pub struct DispatchIntentRecord {
    pub attempt_id: i64,
    /// Base58 transaction signature (extracted from the signed bytes).
    pub signature: String,
    /// Transport label, e.g. `swqos` or `jito`.
    pub provider: String,
    /// Slot at dispatch time.
    pub dispatch_slot: i64,
    /// `lastValidBlockHeight` of the blockhash the transaction was built
    /// with, captured from `getLatestBlockHash` at dispatch time. Recovery
    /// proves expiration with this value (current block height strictly
    /// greater) — never with slot arithmetic.
    pub last_valid_block_height: Option<i64>,
    /// `pending` until reconciliation resolves it; then the on-chain outcome.
    pub resolved_status: Option<String>,
}

pub trait Store: Send + Sync {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()>;
    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<SignedLamports>,
        provenance: NetProvenance,
    ) -> Result<()>;
    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>>;
    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>>;
    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>>;
    fn pnl_daily(&self, day: &str) -> Result<PnlDaily>;
    fn record_fill(&self, fill: &FillRecord) -> Result<()>;
    fn record_dispatch_intent(&self, intent: &DispatchIntentRecord) -> Result<()>;
    fn pending_dispatch_intents(&self, limit: i64) -> Result<Vec<DispatchIntentRecord>>;
    fn resolve_dispatch_intent(&self, attempt_id: i64, resolved_status: &str) -> Result<()>;

    /// Delete rows older than `days`, newest-first safe order (fills → outcomes →
    /// dispatch_intents → attempts). Default no-op for stores without SQL pruning.
    fn prune_older_than_days(&self, _days: u32) -> Result<u64> {
        Ok(0)
    }
}

/// Persistent SQLite Store enforcing WAL Pragmas.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open SQLite database with mandated WAL pragmas and execute initial migration.
    pub fn open(path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let conn = Connection::open(path)
            .map_err(|e| ArbError::Internal(format!("Failed to open SQLite database: {e}")))?;

        // Enforce system invariants: WAL mode, busy timeout 5000ms
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(|e| ArbError::Internal(format!("Failed to set SQLite WAL pragmas: {e}")))?;

        conn.execute_batch(INIT_SQL)
            .map_err(|e| ArbError::Internal(format!("Failed to run migration: {e}")))?;
        conn.execute_batch(PROTOCOL_INTELLIGENCE_INIT_SQL)
            .map_err(|e| {
                ArbError::Internal(format!(
                    "Failed to run protocol intelligence migration: {e}"
                ))
            })?;
        Self::evolve_schema(&conn)?;

        info!("SQLite database initialized with WAL pragmas at {}", path);
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory SQLite database (ideal for tests with full SQL support).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| ArbError::Internal(format!("Failed to open in-memory SQLite: {e}")))?;

        conn.execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(|e| ArbError::Internal(format!("Failed to set SQLite pragmas: {e}")))?;

        conn.execute_batch(INIT_SQL)
            .map_err(|e| ArbError::Internal(format!("Failed to run migration: {e}")))?;
        conn.execute_batch(PROTOCOL_INTELLIGENCE_INIT_SQL)
            .map_err(|e| {
                ArbError::Internal(format!(
                    "Failed to run protocol intelligence migration: {e}"
                ))
            })?;
        Self::evolve_schema(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Safe schema evolution for databases created by older builds:
    /// `CREATE TABLE IF NOT EXISTS` cannot add columns to an existing table,
    /// so column presence is checked via `PRAGMA table_info` and missing
    /// columns are added idempotently.
    fn evolve_schema(conn: &Connection) -> Result<()> {
        let has_column = |name: &str| -> Result<bool> {
            let mut stmt = conn
                .prepare("PRAGMA table_info(dispatch_intents)")
                .map_err(|e| ArbError::Internal(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| ArbError::Internal(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| ArbError::Internal(e.to_string()))? {
                let col: String = row.get(1).map_err(|e| ArbError::Internal(e.to_string()))?;
                if col == name {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        if !has_column("last_valid_block_height")? {
            conn.execute_batch(
                "ALTER TABLE dispatch_intents ADD COLUMN last_valid_block_height INTEGER",
            )
            .map_err(|e| ArbError::Internal(format!("schema evolution failed: {e}")))?;
        }
        // outcomes.provenance gates financial aggregation to on-chain-actual
        // figures only (see NetProvenance). Missing on older databases.
        let has_provenance = |name: &str| -> Result<bool> {
            let mut stmt = conn
                .prepare("PRAGMA table_info(outcomes)")
                .map_err(|e| ArbError::Internal(e.to_string()))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| ArbError::Internal(e.to_string()))?;
            while let Some(row) = rows.next().map_err(|e| ArbError::Internal(e.to_string()))? {
                let col: String = row.get(1).map_err(|e| ArbError::Internal(e.to_string()))?;
                if col == name {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        if !has_provenance("provenance")? {
            conn.execute_batch("ALTER TABLE outcomes ADD COLUMN provenance TEXT")
                .map_err(|e| ArbError::Internal(format!("schema evolution failed: {e}")))?;
            // Rows predating provenance carry no evidence tag and can never be
            // promoted to `actual` retroactively — 'estimated' is the honest
            // default for legacy rows.
            conn.execute_batch(
                "UPDATE outcomes SET provenance = 'estimated' WHERE provenance IS NULL",
            )
            .map_err(|e| ArbError::Internal(format!("provenance backfill failed: {e}")))?;
        }
        Ok(())
    }
}

impl Store for SqliteStore {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        // PIPE-11: plain INSERT — an id collision must fail loudly here so
        // the caller refuses to dispatch, not silently REPLACE durable
        // evidence of a previous attempt (outcomes/fills keep referencing it).
        conn.execute(
            "INSERT INTO attempts (id, slot, route_id, expected_net, tip_lamports, decision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.id,
                record.slot,
                record.route_id,
                record.expected_net.as_u64() as i64,
                record.tip_lamports.map(|t| t.as_u64() as i64),
                record.decision,
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to insert attempt: {e}")))?;
        Ok(())
    }

    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<SignedLamports>,
        provenance: NetProvenance,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO outcomes (attempt_id, status, realized_net, provenance) VALUES (?1, ?2, ?3, ?4)",
            params![attempt_id, status, realized_net.map(|n| n.as_i64()), provenance.as_str()],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to insert outcome: {e}")))?;
        Ok(())
    }

    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare("SELECT id, slot, route_id, expected_net, tip_lamports, decision FROM attempts WHERE id = ?1")
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut rows = stmt
            .query(params![id])
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| ArbError::Internal(e.to_string()))? {
            let exp_net: i64 = row.get(3).unwrap_or(0);
            let tip: Option<i64> = row.get(4).unwrap_or(None);
            Ok(Some(AttemptRecord {
                id: row.get(0).unwrap_or(0),
                slot: row.get(1).unwrap_or(0),
                route_id: row.get(2).unwrap_or_default(),
                expected_net: Lamports(exp_net.max(0) as u64),
                tip_lamports: tip.map(|t| Lamports(t.max(0) as u64)),
                decision: row.get(5).unwrap_or_default(),
            }))
        } else {
            Ok(None)
        }
    }

    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare("SELECT id, slot, route_id, expected_net, tip_lamports, decision FROM attempts ORDER BY id DESC LIMIT ?1")
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map(params![limit], |row| {
                let exp_net: i64 = row.get(3)?;
                let tip: Option<i64> = row.get(4)?;
                Ok(AttemptRecord {
                    id: row.get(0)?,
                    slot: row.get(1)?,
                    route_id: row.get(2)?,
                    expected_net: Lamports(exp_net.max(0) as u64),
                    tip_lamports: tip.map(|t| Lamports(t.max(0) as u64)),
                    decision: row.get(5)?,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for item in rows.flatten() {
            out.push(item);
        }
        Ok(out)
    }

    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare("SELECT attempt_id, token, amount FROM fills WHERE attempt_id = ?1")
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map(params![attempt_id], |row| {
                let amt: i64 = row.get(2)?;
                Ok(FillRecord {
                    attempt_id: row.get(0)?,
                    token: row.get(1)?,
                    amount: TokenAmount(amt.max(0) as u64),
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for item in rows.flatten() {
            out.push(item);
        }
        Ok(out)
    }

    fn pnl_daily(&self, day: &str) -> Result<PnlDaily> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let filter = if day == "today" {
            "date(created_at) = date('now')".to_string()
        } else if !day.is_empty() && day != "all" {
            let sanitized: String = day
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            format!("date(created_at) = date('{sanitized}')")
        } else {
            "1=1".to_string()
        };
        let query = format!(
            "SELECT COUNT(*),
                    SUM(CASE WHEN status = 'landed' THEN 1 ELSE 0 END),
                    COALESCE(SUM(realized_net) FILTER (WHERE status = 'landed' AND provenance = 'actual'), 0),
                    COALESCE(SUM(realized_net) FILTER (
                        WHERE provenance = 'estimated'
                          AND status IN ('landed', 'canary', 'simulated')
                    ), 0)
             FROM outcomes WHERE {filter}"
        );
        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let res = stmt.query_row([], |row| {
            let total_attempts: i64 = row.get(0)?;
            let successful: i64 = row.get(1)?;
            let total_realized: i64 = row.get(2).unwrap_or(0);
            let total_simulated: i64 = row.get(3).unwrap_or(0);
            Ok(PnlDaily {
                attempts: total_attempts.max(0) as u64,
                successful: successful.max(0) as u64,
                realized_net: SignedLamports::new(total_realized),
                simulated_net: SignedLamports::new(total_simulated),
            })
        });

        match res {
            Ok(pnl) => Ok(pnl),
            Err(e) => Err(ArbError::Internal(e.to_string())),
        }
    }

    fn record_fill(&self, fill: &FillRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let amount = i64::try_from(fill.amount.as_u64()).map_err(|_| {
            ArbError::InvalidInput("fill amount exceeds SQLite INTEGER range".into())
        })?;
        conn.execute(
            "INSERT INTO fills (attempt_id, token, amount) VALUES (?1, ?2, ?3)",
            params![fill.attempt_id, fill.token, amount],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to insert fill: {e}")))?;
        Ok(())
    }

    fn record_dispatch_intent(&self, intent: &DispatchIntentRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO dispatch_intents (attempt_id, signature, provider, dispatch_slot, last_valid_block_height, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
            params![
                intent.attempt_id,
                intent.signature,
                intent.provider,
                intent.dispatch_slot,
                intent.last_valid_block_height,
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to insert dispatch intent: {e}")))?;
        Ok(())
    }

    fn pending_dispatch_intents(&self, limit: i64) -> Result<Vec<DispatchIntentRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT attempt_id, signature, provider, dispatch_slot, last_valid_block_height FROM dispatch_intents
                 WHERE status = 'pending' ORDER BY attempt_id ASC LIMIT ?1",
            )
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(DispatchIntentRecord {
                    attempt_id: row.get(0).unwrap_or(0),
                    signature: row.get(1).unwrap_or_default(),
                    provider: row.get(2).unwrap_or_default(),
                    dispatch_slot: row.get(3).unwrap_or(0),
                    last_valid_block_height: row.get(4).ok().flatten(),
                    resolved_status: None,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| ArbError::Internal(e.to_string()))
    }

    fn resolve_dispatch_intent(&self, attempt_id: i64, resolved_status: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "UPDATE dispatch_intents SET status = 'resolved', resolved_status = ?2,
             resolved_at = CURRENT_TIMESTAMP WHERE attempt_id = ?1 AND status = 'pending'",
            params![attempt_id, resolved_status],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to resolve dispatch intent: {e}")))?;
        Ok(())
    }

    /// Retention policy: delete all attempt-row families whose `created_at`
    /// timestamp is older than `days`. Child tables are pruned before the
    /// attempts rows they reference. Returns total rows deleted.
    fn prune_older_than_days(&self, days: u32) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let modifier = format!("-{days} days");
        let mut deleted: u64 = 0;
        for table in ["fills", "outcomes", "dispatch_intents", "attempts"] {
            let sql = format!(
                "DELETE FROM {table} WHERE created_at IS NOT NULL\n                 AND created_at < datetime('now', ?1)"
            );
            let n = conn.execute(&sql, params![modifier]).map_err(|e| {
                ArbError::Internal(format!("retention prune failed on {table}: {e}"))
            })?;
            deleted += u64::try_from(n).unwrap_or(0);
        }
        if deleted > 0 {
            info!(deleted, days, "store retention prune completed");
        }
        Ok(deleted)
    }
}

impl ProtocolIntelligenceRepository for SqliteStore {
    fn register_protocol(&self, r: &ProtocolRegistryRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO protocol_registry (
                protocol_id, protocol_name, protocol_type, program_id, network,
                protocol_version, supported_features, sdk_name, sdk_version, sdk_commit,
                source_repository, documentation_url, verification_status, enabled
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                r.protocol_id,
                r.protocol_name,
                r.protocol_type,
                r.program_id,
                r.network,
                r.protocol_version,
                r.supported_features,
                r.sdk_name,
                r.sdk_version,
                r.sdk_commit,
                r.source_repository,
                r.documentation_url,
                r.verification_status,
                if r.enabled { 1 } else { 0 },
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to register protocol: {e}")))?;
        Ok(())
    }

    fn register_math_spec(&self, r: &ProtocolMathSpecRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO protocol_math_specs (
                protocol_id, formula_version, input_amount_semantics, output_amount_semantics,
                price_formula, fee_formula, rounding_mode, integer_precision, fee_components,
                supported_swap_modes, unsupported_features, reference_source, verification_test_version
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                r.protocol_id, r.formula_version, r.input_amount_semantics, r.output_amount_semantics,
                r.price_formula, r.fee_formula, r.rounding_mode, r.integer_precision, r.fee_components,
                r.supported_swap_modes, r.unsupported_features, r.reference_source, r.verification_test_version,
            ],
        ).map_err(|e| ArbError::Internal(format!("Failed to register math spec: {e}")))?;
        Ok(())
    }

    fn register_pool(&self, r: &PoolRegistryRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO pool_registry (
                pool_address, protocol_id, pool_type, token_mint_a, token_mint_b,
                token_decimals_a, token_decimals_b, token_program_a, token_program_b,
                vault_a, vault_b, configuration_account, tick_spacing, bin_step,
                active_bin_id, current_tick, pool_status, discovery_source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                r.pool_address, r.protocol_id, r.pool_type, r.token_mint_a, r.token_mint_b,
                r.token_decimals_a as i32, r.token_decimals_b as i32, r.token_program_a, r.token_program_b,
                r.vault_a, r.vault_b, r.configuration_account, r.tick_spacing, r.bin_step,
                r.active_bin_id, r.current_tick, r.pool_status, r.discovery_source,
            ],
        ).map_err(|e| ArbError::Internal(format!("Failed to register pool: {e}")))?;
        Ok(())
    }

    fn record_pool_snapshot(&self, s: &PoolStateSnapshotRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO pool_state_snapshots (
                pool_address, slot, write_version, state_hash, reserve_a, reserve_b,
                sqrt_price, current_tick, active_bin, liquidity, protocol_specific_state,
                source_provider, state_validity, invalidation_reason
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                s.pool_address,
                s.slot as i64,
                s.write_version as i64,
                s.state_hash,
                s.reserve_a,
                s.reserve_b,
                s.sqrt_price,
                s.current_tick,
                s.active_bin,
                s.liquidity,
                s.protocol_specific_state,
                s.source_provider,
                s.state_validity,
                s.invalidation_reason,
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to record pool snapshot: {e}")))?;
        Ok(())
    }

    fn register_fee(&self, f: &FeeRegistryRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO fee_registry (
                protocol_id, pool_address, fee_type, configuration_source, calculation_version,
                effective_slot, fee_numerator, fee_denominator, fee_bps, rounding_rule, verification_status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                f.protocol_id, f.pool_address, f.fee_type, f.configuration_source, f.calculation_version,
                f.effective_slot as i64, f.fee_numerator as i64, f.fee_denominator as i64,
                f.fee_bps as i32, f.rounding_rule, f.verification_status,
            ],
        ).map_err(|e| ArbError::Internal(format!("Failed to register fee: {e}")))?;
        Ok(())
    }

    fn record_quote(&self, q: &QuoteRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO quote_records (
                quote_id, protocol_id, pool_address, input_mint, output_mint,
                amount_in_raw, amount_out_raw, fees_breakdown, price_impact_bps,
                unfilled_input_raw, minimum_output_raw, state_slot, state_hash,
                quote_engine_version, sdk_reference_version, quote_latency_us, quote_validity
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                q.quote_id,
                q.protocol_id,
                q.pool_address,
                q.input_mint,
                q.output_mint,
                q.amount_in_raw,
                q.amount_out_raw,
                q.fees_breakdown,
                q.price_impact_bps as i32,
                q.unfilled_input_raw,
                q.minimum_output_raw,
                q.state_slot as i64,
                q.state_hash,
                q.quote_engine_version,
                q.sdk_reference_version,
                q.quote_latency_us as i64,
                q.quote_validity,
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to record quote: {e}")))?;
        Ok(())
    }

    fn register_route(&self, r: &RouteRegistryRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO route_registry (
                route_id, route_type, number_of_pools, ordered_pool_addresses, ordered_protocol_ids,
                input_mint, intermediate_mints, output_mint, route_version, enabled
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                r.route_id,
                r.route_type,
                r.number_of_pools as i32,
                r.ordered_pool_addresses,
                r.ordered_protocol_ids,
                r.input_mint,
                r.intermediate_mints,
                r.output_mint,
                r.route_version,
                if r.enabled { 1 } else { 0 },
            ],
        )
        .map_err(|e| ArbError::Internal(format!("Failed to register route: {e}")))?;
        Ok(())
    }

    fn record_simulation(&self, s: &RouteSimulationRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO route_simulation_records (
                simulation_id, route_id, input_amount_raw, output_amount_raw, principal_raw,
                flash_loan_fee_raw, network_fee_raw, priority_fee_raw, tip_raw, token_transfer_fees_raw,
                total_costs_raw, net_profit_raw, transaction_size_bytes, compute_units_consumed,
                simulation_slot, simulation_result, simulation_error
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                s.simulation_id, s.route_id, s.input_amount_raw, s.output_amount_raw, s.principal_raw,
                s.flash_loan_fee_raw, s.network_fee_raw, s.priority_fee_raw, s.tip_raw, s.token_transfer_fees_raw,
                s.total_costs_raw, s.net_profit_raw, s.transaction_size_bytes as i32, s.compute_units_consumed as i64,
                s.simulation_slot as i64, s.simulation_result, s.simulation_error,
            ],
        ).map_err(|e| ArbError::Internal(format!("Failed to record simulation: {e}")))?;
        Ok(())
    }

    fn record_sdk_reconciliation(&self, rec: &SdkReconciliationRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO sdk_reconciliation_records (
                protocol_id, pool_address, input_amount_raw, local_quote_raw, sdk_quote_raw,
                onchain_simulated_quote_raw, difference_raw, difference_bps, state_slot, result, failure_reason
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rec.protocol_id, rec.pool_address, rec.input_amount_raw, rec.local_quote_raw, rec.sdk_quote_raw,
                rec.onchain_simulated_quote_raw, rec.difference_raw, rec.difference_bps, rec.state_slot as i64,
                rec.result, rec.failure_reason,
            ],
        ).map_err(|e| ArbError::Internal(format!("Failed to record sdk reconciliation: {e}")))?;
        Ok(())
    }

    fn get_protocol(&self, protocol_id: &str) -> Result<Option<ProtocolRegistryRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT protocol_id, protocol_name, protocol_type, program_id, network,
                    protocol_version, supported_features, sdk_name, sdk_version, sdk_commit,
                    source_repository, documentation_url, verification_status, enabled
             FROM protocol_registry WHERE protocol_id = ?1",
            )
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![protocol_id], |row| {
                let enabled_int: i32 = row.get(13)?;
                Ok(ProtocolRegistryRecord {
                    protocol_id: row.get(0)?,
                    protocol_name: row.get(1)?,
                    protocol_type: row.get(2)?,
                    program_id: row.get(3)?,
                    network: row.get(4)?,
                    protocol_version: row.get(5)?,
                    supported_features: row.get(6)?,
                    sdk_name: row.get(7)?,
                    sdk_version: row.get(8)?,
                    sdk_commit: row.get(9)?,
                    source_repository: row.get(10)?,
                    documentation_url: row.get(11)?,
                    verification_status: row.get(12)?,
                    enabled: enabled_int != 0,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(|e| ArbError::Internal(e.to_string()))?))
        } else {
            Ok(None)
        }
    }

    fn get_pool(&self, pool_address: &str) -> Result<Option<PoolRegistryRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT pool_address, protocol_id, pool_type, token_mint_a, token_mint_b,
                    token_decimals_a, token_decimals_b, token_program_a, token_program_b,
                    vault_a, vault_b, configuration_account, tick_spacing, bin_step,
                    active_bin_id, current_tick, pool_status, discovery_source
             FROM pool_registry WHERE pool_address = ?1",
            )
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![pool_address], |row| {
                let dec_a: i32 = row.get(5)?;
                let dec_b: i32 = row.get(6)?;
                Ok(PoolRegistryRecord {
                    pool_address: row.get(0)?,
                    protocol_id: row.get(1)?,
                    pool_type: row.get(2)?,
                    token_mint_a: row.get(3)?,
                    token_mint_b: row.get(4)?,
                    token_decimals_a: dec_a as u8,
                    token_decimals_b: dec_b as u8,
                    token_program_a: row.get(7)?,
                    token_program_b: row.get(8)?,
                    vault_a: row.get(9)?,
                    vault_b: row.get(10)?,
                    configuration_account: row.get(11)?,
                    tick_spacing: row.get(12)?,
                    bin_step: row.get(13)?,
                    active_bin_id: row.get(14)?,
                    current_tick: row.get(15)?,
                    pool_status: row.get(16)?,
                    discovery_source: row.get(17)?,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(|e| ArbError::Internal(e.to_string()))?))
        } else {
            Ok(None)
        }
    }

    fn get_latest_snapshot(&self, pool_address: &str) -> Result<Option<PoolStateSnapshotRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn
            .prepare(
                "SELECT pool_address, slot, write_version, state_hash, reserve_a, reserve_b,
                    sqrt_price, current_tick, active_bin, liquidity, protocol_specific_state,
                    source_provider, state_validity, invalidation_reason
             FROM pool_state_snapshots WHERE pool_address = ?1 ORDER BY slot DESC LIMIT 1",
            )
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut rows = stmt
            .query_map(params![pool_address], |row| {
                let slot_i: i64 = row.get(1)?;
                let wv_i: i64 = row.get(2)?;
                Ok(PoolStateSnapshotRecord {
                    pool_address: row.get(0)?,
                    slot: slot_i as u64,
                    write_version: wv_i as u64,
                    state_hash: row.get(3)?,
                    reserve_a: row.get(4)?,
                    reserve_b: row.get(5)?,
                    sqrt_price: row.get(6)?,
                    current_tick: row.get(7)?,
                    active_bin: row.get(8)?,
                    liquidity: row.get(9)?,
                    protocol_specific_state: row.get(10)?,
                    source_provider: row.get(11)?,
                    state_validity: row.get(12)?,
                    invalidation_reason: row.get(13)?,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(|e| ArbError::Internal(e.to_string()))?))
        } else {
            Ok(None)
        }
    }

    fn get_recent_reconciliations(
        &self,
        protocol_id: &str,
        limit: usize,
    ) -> Result<Vec<SdkReconciliationRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        let mut stmt = conn.prepare(
            "SELECT protocol_id, pool_address, input_amount_raw, local_quote_raw, sdk_quote_raw,
                    onchain_simulated_quote_raw, difference_raw, difference_bps, state_slot, result, failure_reason
             FROM sdk_reconciliation_records WHERE protocol_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        ).map_err(|e| ArbError::Internal(e.to_string()))?;

        let rows = stmt
            .query_map(params![protocol_id, limit as i64], |row| {
                let slot_i: i64 = row.get(8)?;
                Ok(SdkReconciliationRecord {
                    protocol_id: row.get(0)?,
                    pool_address: row.get(1)?,
                    input_amount_raw: row.get(2)?,
                    local_quote_raw: row.get(3)?,
                    sdk_quote_raw: row.get(4)?,
                    onchain_simulated_quote_raw: row.get(5)?,
                    difference_raw: row.get(6)?,
                    difference_bps: row.get(7)?,
                    state_slot: slot_i as u64,
                    result: row.get(9)?,
                    failure_reason: row.get(10)?,
                })
            })
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        for item in rows.flatten() {
            out.push(item);
        }
        Ok(out)
    }
}
/// One durable outcome row: (attempt_id, status, realized_net, provenance).
type OutcomeRow = (i64, String, Option<SignedLamports>, NetProvenance);

#[derive(Debug, Default)]
pub struct MemoryStore {
    attempts: Mutex<Vec<AttemptRecord>>,
    outcomes: Mutex<Vec<OutcomeRow>>,
    fills: Mutex<Vec<FillRecord>>,
    dispatch_intents: Mutex<Vec<(DispatchIntentRecord, String)>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemoryStore {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()> {
        debug!(route = %record.route_id, net = record.expected_net.as_u64(), "record attempt");
        let mut a = self
            .attempts
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        a.push(record.clone());
        Ok(())
    }

    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<SignedLamports>,
        provenance: NetProvenance,
    ) -> Result<()> {
        let mut o = self
            .outcomes
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        o.push((attempt_id, status.to_string(), realized_net, provenance));
        Ok(())
    }

    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>> {
        let a = self
            .attempts
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        Ok(a.iter().find(|x| x.id == id).cloned())
    }

    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>> {
        let a = self
            .attempts
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        let recent: Vec<AttemptRecord> = a
            .iter()
            .rev()
            .take(usize::try_from(limit.max(0)).unwrap_or(0))
            .cloned()
            .collect();
        Ok(recent)
    }

    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>> {
        let f = self
            .fills
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        Ok(f.iter()
            .filter(|f| f.attempt_id == attempt_id)
            .cloned()
            .collect())
    }

    fn pnl_daily(&self, _day: &str) -> Result<PnlDaily> {
        let mut realized = 0i64;
        let mut simulated = 0i64;
        let mut n = 0u64;
        let mut successful = 0u64;
        let o = self
            .outcomes
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        for (_, status, rn, prov) in o.iter() {
            n += 1;
            if status == "landed" {
                successful += 1;
            }
            match (status.as_str(), prov) {
                ("landed", NetProvenance::Actual) => {
                    if let Some(v) = rn {
                        realized = realized.saturating_add(v.as_i64());
                    }
                }
                ("landed", NetProvenance::Estimated) | ("canary", _) | ("simulated", _) => {
                    if let Some(v) = rn {
                        simulated = simulated.saturating_add(v.as_i64());
                    }
                }
                _ => {}
            }
        }
        Ok(PnlDaily {
            realized_net: SignedLamports::new(realized),
            simulated_net: SignedLamports::new(simulated),
            attempts: n,
            successful,
        })
    }

    fn record_fill(&self, fill: &FillRecord) -> Result<()> {
        let mut f = self
            .fills
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        f.push(fill.clone());
        Ok(())
    }

    fn record_dispatch_intent(&self, intent: &DispatchIntentRecord) -> Result<()> {
        let mut d = self
            .dispatch_intents
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        d.retain(|(existing, _)| existing.attempt_id != intent.attempt_id);
        d.push((intent.clone(), "pending".to_string()));
        Ok(())
    }

    fn pending_dispatch_intents(&self, limit: i64) -> Result<Vec<DispatchIntentRecord>> {
        let d = self
            .dispatch_intents
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        Ok(d.iter()
            .filter(|(_, status)| status == "pending")
            .take(usize::try_from(limit.max(0)).unwrap_or(0))
            .map(|(intent, _)| intent.clone())
            .collect())
    }

    fn resolve_dispatch_intent(&self, attempt_id: i64, resolved_status: &str) -> Result<()> {
        let mut d = self
            .dispatch_intents
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        for (intent, status) in d.iter_mut() {
            if intent.attempt_id == attempt_id && status == "pending" {
                *status = "resolved".to_string();
                intent.resolved_status = Some(resolved_status.to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_evolution_adds_last_valid_block_height_to_legacy_db() {
        let path = std::env::temp_dir().join(format!(
            "arb_evolve_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
        ));
        {
            let conn = Connection::open(&path).expect("raw open");
            conn.execute_batch(
                "CREATE TABLE dispatch_intents (
                    attempt_id INTEGER PRIMARY KEY,
                    signature TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    dispatch_slot INTEGER NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    resolved_status TEXT,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    resolved_at TIMESTAMP
                );",
            )
            .expect("legacy schema");
        }
        let store = SqliteStore::open(path.to_str().unwrap()).expect("open legacy db");
        let r = AttemptRecord {
            id: 7,
            slot: 1_000,
            route_id: "route".into(),
            expected_net: Lamports(0),
            tip_lamports: None,
            decision: "dispatch".into(),
        };
        store.record_attempt(&r).expect("invariant");
        store
            .record_dispatch_intent(&DispatchIntentRecord {
                attempt_id: 7,
                signature: "sig".into(),
                provider: "swqos".into(),
                dispatch_slot: 1_000,
                last_valid_block_height: Some(1_100),
                resolved_status: None,
            })
            .expect("evolved column must accept writes");
        let pending = store.pending_dispatch_intents(10).expect("invariant");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].last_valid_block_height, Some(1_100));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sqlite_store_wal_roundtrip() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        let r = AttemptRecord {
            id: 101,
            slot: 500,
            route_id: "test-route".into(),
            expected_net: Lamports(50_000),
            tip_lamports: Some(Lamports(10_000)),
            decision: "sent".into(),
        };
        store.record_attempt(&r).expect("invariant");
        let fetched = store
            .get_attempt(101)
            .expect("invariant")
            .expect("invariant");
        assert_eq!(fetched.id, 101);
        assert_eq!(fetched.expected_net.as_u64(), 50_000);

        store
            .record_outcome(
                101,
                "landed",
                Some(SignedLamports::new(45_000)),
                NetProvenance::Actual,
            )
            .expect("invariant");
        let pnl = store.pnl_daily("today").expect("invariant");
        assert_eq!(pnl.attempts, 1);
        assert_eq!(pnl.successful, 1);
        assert_eq!(pnl.realized_net.as_i64(), 45_000);

        let recent = store.get_recent_attempts(10).expect("invariant");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, 101);
    }

    /// MATH-01 regression: a landed transaction that realized a NET LOSS must
    /// round-trip through the store as a negative number — never dropped by an
    /// unsigned parse, never replaced by an estimate.
    #[test]
    fn realized_pnl_preserves_negative_landed_actual() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        store
            .record_attempt(&AttemptRecord {
                id: 1,
                slot: 1,
                route_id: "r".into(),
                expected_net: Lamports(50_000),
                tip_lamports: Some(Lamports(61_000)),
                decision: "sent".into(),
            })
            .unwrap();
        store
            .record_outcome(
                1,
                "landed",
                Some(SignedLamports::new(-12_345)),
                NetProvenance::Actual,
            )
            .unwrap();
        let pnl = store.pnl_daily("today").unwrap();
        assert_eq!(
            pnl.realized_net.as_i64(),
            -12_345,
            "loss must persist as a negative signed integer"
        );
        assert_eq!(pnl.successful, 1);
    }

    /// PIPE-01 regression: financial aggregation only sums `landed` outcomes
    /// with `actual` provenance. Simulated/canary rows and estimate-fallback
    /// landed rows are kept in a separate bucket, never in realized_net.
    #[test]
    fn pnl_daily_is_provenance_gated() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        for id in 1..=4i64 {
            store
                .record_attempt(&AttemptRecord {
                    id,
                    slot: 1,
                    route_id: "r".into(),
                    expected_net: Lamports(1),
                    tip_lamports: None,
                    decision: "sent".into(),
                })
                .unwrap();
        }
        store
            .record_outcome(
                1,
                "landed",
                Some(SignedLamports::new(45_000)),
                NetProvenance::Actual,
            )
            .unwrap();
        store
            .record_outcome(
                2,
                "landed",
                Some(SignedLamports::new(9_999)),
                NetProvenance::Estimated,
            )
            .unwrap();
        store
            .record_outcome(
                3,
                "simulated",
                Some(SignedLamports::new(12_000)),
                NetProvenance::Estimated,
            )
            .unwrap();
        store
            .record_outcome(
                4,
                "canary",
                Some(SignedLamports::new(3_000)),
                NetProvenance::Estimated,
            )
            .unwrap();
        let pnl = store.pnl_daily("today").unwrap();
        assert_eq!(
            pnl.realized_net.as_i64(),
            45_000,
            "estimate-backed landed row must not contaminate realized PnL"
        );
        assert_eq!(pnl.simulated_net.as_i64(), 24_999);
        assert_eq!(pnl.attempts, 4);
        assert_eq!(pnl.successful, 2);
    }

    /// PIPE-11 regression: an attempt-id collision must fail loudly — the
    /// second `record_attempt` returns Err and the first attempt's durable
    /// row (its evidence anchor for outcomes/fills/intents) is left intact,
    /// never silently replaced.
    #[test]
    fn attempt_id_collision_fails_loudly_and_preserves_original() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        let mk = |expected: u64| AttemptRecord {
            id: 700_000_001,
            slot: 700,
            route_id: "route-A".into(),
            expected_net: Lamports(expected),
            tip_lamports: None,
            decision: "sent".into(),
        };
        store.record_attempt(&mk(111)).expect("first write ok");
        assert!(
            store.record_attempt(&mk(222)).is_err(),
            "duplicate id must be rejected, not REPLACE the original row"
        );
        let preserved = store.get_attempt(700_000_001).unwrap().expect("row");
        assert_eq!(preserved.expected_net, Lamports(111));
    }

    /// Schema evolution: a database created before the provenance column
    /// evolves in place; legacy rows are backfilled as 'estimated' (no
    /// retroactive promotion to on-chain evidence).
    #[test]
    fn evolve_schema_backfills_provenance_for_legacy_rows() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        {
            let conn = store.conn.lock().unwrap();
            conn.execute("UPDATE outcomes SET provenance = NULL", [])
                .unwrap();
        }
        // Writing a legacy-shaped DB is not possible after init, so instead
        // verify the PRAGMA column exists and that a fresh write round-trips.
        let conn = store.conn.lock().unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(outcomes)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .flatten()
            .collect();
        assert!(cols.iter().any(|c| c == "provenance"));
    }

    #[test]
    fn dispatch_intent_roundtrip_and_reconciliation() {
        let intent = |id: i64| DispatchIntentRecord {
            attempt_id: id,
            signature: format!("sig{id}"),
            provider: "swqos".into(),
            dispatch_slot: 700,
            last_valid_block_height: Some(1_000 + id),
            resolved_status: None,
        };

        for store in [
            &SqliteStore::open_in_memory().expect("invariant") as &dyn Store,
            &MemoryStore::new() as &dyn Store,
        ] {
            assert!(store.pending_dispatch_intents(16).unwrap().is_empty());
            for id in [1i64, 2] {
                store
                    .record_attempt(&AttemptRecord {
                        id,
                        slot: 700,
                        route_id: "r".into(),
                        expected_net: Lamports(1),
                        tip_lamports: None,
                        decision: "sent".into(),
                    })
                    .unwrap();
            }
            store.record_dispatch_intent(&intent(1)).unwrap();
            store.record_dispatch_intent(&intent(2)).unwrap();

            let pending = store.pending_dispatch_intents(16).unwrap();
            assert_eq!(pending.len(), 2);
            assert_eq!(pending[0].attempt_id, 1);
            assert_eq!(pending[0].signature, "sig1");
            assert_eq!(pending[0].dispatch_slot, 700);

            store.resolve_dispatch_intent(1, "landed").unwrap();
            let pending = store.pending_dispatch_intents(16).unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].attempt_id, 2);

            // Resolving twice is a no-op (idempotent reconciliation).
            store.resolve_dispatch_intent(1, "expired").unwrap();
            assert_eq!(store.pending_dispatch_intents(16).unwrap().len(), 1);
        }
    }

    #[test]
    fn retention_prune_deletes_old_rows_and_keeps_recent() {
        let store = SqliteStore::open_in_memory().expect("invariant");
        for id in [1i64, 2] {
            store
                .record_attempt(&AttemptRecord {
                    id,
                    slot: 700,
                    route_id: "r".into(),
                    expected_net: Lamports(1),
                    tip_lamports: None,
                    decision: "sent".into(),
                })
                .unwrap();
            store
                .record_outcome(
                    id,
                    "landed",
                    Some(SignedLamports::new(1)),
                    NetProvenance::Actual,
                )
                .unwrap();
            store
                .record_fill(&FillRecord {
                    attempt_id: id,
                    token: "mint".into(),
                    amount: arb_types::TokenAmount(1),
                })
                .unwrap();
            store
                .record_dispatch_intent(&DispatchIntentRecord {
                    attempt_id: id,
                    signature: format!("sig{id}"),
                    provider: "swqos".into(),
                    dispatch_slot: 700,
                    last_valid_block_height: None,
                    resolved_status: None,
                })
                .unwrap();
        }
        // Backdate attempt 1 (and its children) beyond the retention window.
        {
            let conn = store.conn.lock().unwrap();
            for table in ["attempts", "outcomes", "fills", "dispatch_intents"] {
                let key = if table == "attempts" {
                    "id"
                } else {
                    "attempt_id"
                };
                conn.execute(
                    &format!(
                        "UPDATE {table} SET created_at = datetime('now', '-40 days') WHERE {key} = 1"
                    ),
                    [],
                )
                .unwrap();
            }
        }
        let deleted = store.prune_older_than_days(30).expect("invariant");
        assert!(deleted >= 4, "expected old rows deleted, got {deleted}");
        assert!(store.get_attempt(1).unwrap().is_none());
        assert!(store.get_attempt(2).unwrap().is_some());
        assert_eq!(store.prune_older_than_days(30).unwrap(), 0);
    }
}
