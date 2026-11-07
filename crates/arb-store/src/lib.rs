//! Persistent Storage Layer (SQLite with WAL Pragmas) & Accounting.

use arb_types::{ArbError, Lamports, Result, TokenAmount};
use rusqlite::{Connection, params};
use std::sync::Mutex;
use tracing::{debug, info};

pub mod postgres_store;
pub mod redis_cache;
pub use postgres_store::PostgresStore;
pub use redis_cache::{RedisStore, ShortTtlCache};

pub const INIT_SQL: &str = include_str!("../migrations/001_init.sql");
pub const POSTGRES_INIT_SQL: &str = include_str!("../migrations/001_postgres_init.sql");

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
    pub realized_net: Lamports,
    pub attempts: u64,
    pub successful: u64,
}

pub trait Store: Send + Sync {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()>;
    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<Lamports>,
    ) -> Result<()>;
    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>>;
    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>>;
    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>>;
    fn pnl_daily(&self, day: &str) -> Result<PnlDaily>;
    fn record_fill(&self, fill: &FillRecord) -> Result<()>;
}

/// Persistent SQLite Store enforcing WAL Pragmas.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open SQLite database with mandated WAL pragmas and execute initial migration.
    pub fn open(path: &str) -> Result<Self> {
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

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl Store for SqliteStore {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT OR REPLACE INTO attempts (id, slot, route_id, expected_net, tip_lamports, decision)
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
        realized_net: Option<Lamports>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| ArbError::Internal("Database lock poisoned".into()))?;
        conn.execute(
            "INSERT INTO outcomes (attempt_id, status, realized_net) VALUES (?1, ?2, ?3)",
            params![attempt_id, status, realized_net.map(|n| n.as_u64() as i64),],
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
                    COALESCE(SUM(realized_net), 0)
             FROM outcomes WHERE {filter}"
        );
        let mut stmt = conn
            .prepare(&query)
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let res = stmt.query_row([], |row| {
            let total_attempts: i64 = row.get(0)?;
            let successful: i64 = row.get(1)?;
            let total_realized: i64 = row.get(2)?;
            Ok(PnlDaily {
                attempts: total_attempts.max(0) as u64,
                successful: successful.max(0) as u64,
                realized_net: Lamports(total_realized.max(0) as u64),
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
}

/// In-Memory store implementation for backward compatibility.
#[derive(Debug, Default)]
pub struct MemoryStore {
    attempts: Mutex<Vec<AttemptRecord>>,
    outcomes: Mutex<Vec<(i64, String, Option<Lamports>)>>,
    fills: Mutex<Vec<FillRecord>>,
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
        realized_net: Option<Lamports>,
    ) -> Result<()> {
        let mut o = self
            .outcomes
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        o.push((attempt_id, status.to_string(), realized_net));
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
        let mut total = Lamports::ZERO;
        let mut n = 0u64;
        let mut successful = 0u64;
        let o = self
            .outcomes
            .lock()
            .map_err(|_| ArbError::Internal("lock".into()))?;
        for (_, status, rn) in o.iter() {
            n += 1;
            if status == "landed" {
                successful += 1;
            }
            if let Some(v) = rn {
                total = total.checked_add(*v).unwrap_or(total);
            }
        }
        Ok(PnlDaily {
            realized_net: total,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .record_outcome(101, "landed", Some(Lamports(45_000)))
            .expect("invariant");
        let pnl = store.pnl_daily("today").expect("invariant");
        assert_eq!(pnl.attempts, 1);
        assert_eq!(pnl.successful, 1);
        assert_eq!(pnl.realized_net.as_u64(), 45_000);

        let recent = store.get_recent_attempts(10).expect("invariant");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, 101);
    }
}
