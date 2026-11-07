//! PostgreSQL Production Store implementation using sqlx.
//! Provides persistence for attempts, outcomes, fills, token prices, and circuit breaker events.

use crate::{AttemptRecord, FillRecord, POSTGRES_INIT_SQL, PnlDaily, Store};
use arb_types::{ArbError, Lamports, Result, TokenAmount};
use sqlx::postgres::PgPoolOptions;
use std::sync::Mutex;
use tracing::info;

pub struct PostgresStore {
    pool: sqlx::PgPool,
    _guard: Mutex<()>,
}

impl PostgresStore {
    fn block_on<T: Send + 'static, F>(&self, f: F) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>> + Send + 'static,
    {
        // Spawning on a dedicated OS thread bypasses Tokio's
        // "Cannot start a runtime from within a runtime" scheduler panic.
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to build current_thread runtime: {e}"))
                })?;
            rt.block_on(f)
        })
        .join()
        .map_err(|_| ArbError::Internal("Postgres worker thread panicked".to_string()))?
    }

    /// Connect to PostgreSQL and apply initial migrations.
    pub fn open(connection_str: &str) -> Result<Self> {
        let conn_str = connection_str.to_string();
        let pool = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to build current_thread runtime: {e}"))
                })?;

            rt.block_on(async {
                let pool = PgPoolOptions::new()
                    .max_connections(5)
                    .connect(&conn_str)
                    .await
                    .map_err(|e| {
                        ArbError::Internal(format!("Failed to connect to PostgreSQL: {e}"))
                    })?;

                sqlx::raw_sql(POSTGRES_INIT_SQL)
                    .execute(&pool)
                    .await
                    .map_err(|e| {
                        ArbError::Internal(format!("Failed to execute PostgreSQL migration: {e}"))
                    })?;
                Ok::<_, ArbError>(pool)
            })
        })
        .join()
        .map_err(|_| ArbError::Internal("Postgres init thread panicked".to_string()))??;

        info!(
            "PostgreSQL store initialized successfully at {}",
            connection_str
        );
        Ok(Self {
            pool,
            _guard: Mutex::new(()),
        })
    }

    /// Record a live token price.
    pub fn record_token_price(&self, mint: &str, price_usd: f64) -> Result<()> {
        let pool = self.pool.clone();
        let mint = mint.to_string();
        self.block_on(async move {
            sqlx::query(
                "INSERT INTO token_prices (mint, price_usd, updated_at)
                 VALUES ($1, $2, CURRENT_TIMESTAMP)
                 ON CONFLICT (mint) DO UPDATE SET price_usd = $2, updated_at = CURRENT_TIMESTAMP",
            )
            .bind(mint)
            .bind(price_usd)
            .execute(&pool)
            .await
            .map_err(|e| ArbError::Internal(format!("Failed to record token price: {e}")))?;
            Ok(())
        })
    }

    /// Record circuit breaker event.
    pub fn record_circuit_breaker(&self, slot: i64, reason: &str) -> Result<()> {
        let pool = self.pool.clone();
        let reason = reason.to_string();
        self.block_on(async move {
            sqlx::query("INSERT INTO circuit_breaker_events (slot, reason) VALUES ($1, $2)")
                .bind(slot)
                .bind(reason)
                .execute(&pool)
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to record circuit breaker event: {e}"))
                })?;
            Ok(())
        })
    }
}

impl Store for PostgresStore {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()> {
        let pool = self.pool.clone();
        let id = record.id;
        let slot = record.slot;
        let route_id = record.route_id.clone();
        let exp_net = record.expected_net.as_u64() as i64;
        let tip: Option<i64> = record.tip_lamports.map(|t| t.as_u64() as i64);
        let decision = record.decision.clone();

        self.block_on(async move {
            sqlx::query(
                "INSERT INTO attempts (id, slot, route_id, expected_net, tip_lamports, decision)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO UPDATE SET
                    expected_net = $4,
                    tip_lamports = $5,
                    decision = $6",
            )
            .bind(id)
            .bind(slot)
            .bind(route_id)
            .bind(exp_net)
            .bind(tip)
            .bind(decision)
            .execute(&pool)
            .await
            .map_err(|e| ArbError::Internal(format!("Failed to insert PG attempt: {e}")))?;
            Ok(())
        })
    }

    fn record_outcome(
        &self,
        attempt_id: i64,
        status: &str,
        realized_net: Option<Lamports>,
    ) -> Result<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        let net: Option<i64> = realized_net.map(|n| n.as_u64() as i64);

        self.block_on(async move {
            sqlx::query(
                "INSERT INTO outcomes (attempt_id, status, realized_net) VALUES ($1, $2, $3)",
            )
            .bind(attempt_id)
            .bind(status)
            .bind(net)
            .execute(&pool)
            .await
            .map_err(|e| ArbError::Internal(format!("Failed to insert PG outcome: {e}")))?;
            Ok(())
        })
    }

    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>> {
        let pool = self.pool.clone();
        self.block_on(async move {
            let row = sqlx::query(
                "SELECT id, slot, route_id, expected_net, tip_lamports, decision
                 FROM attempts WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;

            if let Some(row) = row {
                use sqlx::Row;
                let exp_net: i64 = row.get(3);
                let tip: Option<i64> = row.get(4);
                Ok(Some(AttemptRecord {
                    id: row.get(0),
                    slot: row.get(1),
                    route_id: row.get(2),
                    expected_net: Lamports(exp_net.max(0) as u64),
                    tip_lamports: tip.map(|t| Lamports(t.max(0) as u64)),
                    decision: row.get(5),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>> {
        let pool = self.pool.clone();
        self.block_on(async move {
            let rows = sqlx::query(
                "SELECT id, slot, route_id, expected_net, tip_lamports, decision
                 FROM attempts ORDER BY id DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&pool)
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;

            let mut out = Vec::new();
            for row in rows {
                use sqlx::Row;
                let exp_net: i64 = row.get(3);
                let tip: Option<i64> = row.get(4);
                out.push(AttemptRecord {
                    id: row.get(0),
                    slot: row.get(1),
                    route_id: row.get(2),
                    expected_net: Lamports(exp_net.max(0) as u64),
                    tip_lamports: tip.map(|t| Lamports(t.max(0) as u64)),
                    decision: row.get(5),
                });
            }
            Ok(out)
        })
    }

    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>> {
        let pool = self.pool.clone();
        self.block_on(async move {
            let rows =
                sqlx::query("SELECT attempt_id, token, amount FROM fills WHERE attempt_id = $1")
                    .bind(attempt_id)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| ArbError::Internal(e.to_string()))?;

            let mut out = Vec::new();
            for row in rows {
                use sqlx::Row;
                let amt: i64 = row.get(2);
                out.push(FillRecord {
                    attempt_id: row.get(0),
                    token: row.get(1),
                    amount: TokenAmount(amt.max(0) as u64),
                });
            }
            Ok(out)
        })
    }

    fn pnl_daily(&self, day: &str) -> Result<PnlDaily> {
        let pool = self.pool.clone();
        let day_str = day.to_string();
        self.block_on(async move {
            let filter = if day_str == "today" {
                "created_at::date = CURRENT_DATE".to_string()
            } else if !day_str.is_empty() && day_str != "all" {
                let sanitized: String = day_str
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                format!("created_at::date = '{sanitized}'::date")
            } else {
                "1=1".to_string()
            };
            let query_str = format!(
                "SELECT COUNT(*)::BIGINT,
                        COALESCE(SUM(CASE WHEN status = 'landed' THEN 1 ELSE 0 END),0)::BIGINT,
                        COALESCE(SUM(realized_net), 0)::BIGINT
                 FROM outcomes WHERE {filter}"
            );
            let row = sqlx::query(&query_str)
                .fetch_one(&pool)
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            use sqlx::Row;
            let total_attempts: i64 = row.get(0);
            let successful: i64 = row.get(1);
            let total_realized: i64 = row.get(2);

            Ok(PnlDaily {
                attempts: total_attempts.max(0) as u64,
                successful: successful.max(0) as u64,
                realized_net: Lamports(total_realized.max(0) as u64),
            })
        })
    }

    fn record_fill(&self, fill: &FillRecord) -> Result<()> {
        let pool = self.pool.clone();
        let amt = i64::try_from(fill.amount.as_u64()).map_err(|_| {
            ArbError::InvalidInput("fill amount exceeds PostgreSQL BIGINT range".into())
        })?;
        let attempt_id = fill.attempt_id;
        let token = fill.token.clone();

        self.block_on(async move {
            sqlx::query("INSERT INTO fills (attempt_id, token, amount) VALUES ($1, $2, $3)")
                .bind(attempt_id)
                .bind(token)
                .bind(amt)
                .execute(&pool)
                .await
                .map_err(|e| ArbError::Internal(format!("Failed to insert PG fill: {e}")))?;
            Ok(())
        })
    }
}
