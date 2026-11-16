//! PostgreSQL Production Store implementation using tokio-postgres.
//! Provides persistence for attempts, outcomes, fills, token prices, and circuit breaker events.
//!
//! Transport note: connections use `NoTls`. The supported deployment topology
//! is a PostgreSQL instance reachable on a trusted network (docker-compose /
//! localhost / private VPC). Hosts that require TLS are rejected at connect
//! time (fail-closed); point the store at a TLS-terminating proxy or extend
//! this module with a rustls connector before using such a host.

use crate::{AttemptRecord, DispatchIntentRecord, FillRecord, POSTGRES_INIT_SQL, PnlDaily, Store};
use arb_types::{ArbError, Lamports, Result, TokenAmount};
use std::sync::{Arc, Mutex};
use tokio_postgres::Row;
use tracing::info;

pub struct PostgresStore {
    client: Arc<tokio_postgres::Client>,
    /// Drives the connection task on its worker threads. Must outlive
    /// `client`: dropping the runtime closes the connection.
    _runtime: tokio::runtime::Runtime,
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
        let (client, runtime) = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to build PostgreSQL runtime: {e}"))
                })?;

            let (client, connection) = rt
                .block_on(tokio_postgres::connect(&conn_str, tokio_postgres::NoTls))
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to connect to PostgreSQL: {e}"))
                })?;

            // The connection must be actively polled for the client to work;
            // the multi-thread runtime drives it on its worker threads.
            rt.spawn(async move {
                if let Err(e) = connection.await {
                    tracing::warn!("PostgreSQL connection closed: {e}");
                }
            });

            rt.block_on(async {
                client
                    .batch_execute(POSTGRES_INIT_SQL)
                    .await
                    .map_err(|e| {
                        ArbError::Internal(format!("Failed to execute PostgreSQL migration: {e}"))
                    })?;
                // Safe schema evolution for pre-existing databases: CREATE
                // TABLE IF NOT EXISTS cannot add columns to an existing table.
                client
                    .batch_execute(
                        "ALTER TABLE dispatch_intents ADD COLUMN IF NOT EXISTS last_valid_block_height BIGINT",
                    )
                    .await
                    .map_err(|e| {
                        ArbError::Internal(format!("Failed to evolve PostgreSQL schema: {e}"))
                    })?;
                Ok::<_, ArbError>(())
            })?;

            Ok::<_, ArbError>((client, rt))
        })
        .join()
        .map_err(|_| ArbError::Internal("Postgres init thread panicked".to_string()))??;

        info!(
            "PostgreSQL store initialized successfully at {}",
            connection_str
        );
        Ok(Self {
            client: Arc::new(client),
            _runtime: runtime,
            _guard: Mutex::new(()),
        })
    }

    /// Record a live token price.
    pub fn record_token_price(&self, mint: &str, price_usd: f64) -> Result<()> {
        let client = self.client.clone();
        let mint = mint.to_string();
        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO token_prices (mint, price_usd, updated_at)
                     VALUES ($1, $2, CURRENT_TIMESTAMP)
                     ON CONFLICT (mint) DO UPDATE SET price_usd = $2, updated_at = CURRENT_TIMESTAMP",
                    &[&mint, &price_usd],
                )
                .await
                .map_err(|e| ArbError::Internal(format!("Failed to record token price: {e}")))?;
            Ok(())
        })
    }

    /// Record circuit breaker event.
    pub fn record_circuit_breaker(&self, slot: i64, reason: &str) -> Result<()> {
        let client = self.client.clone();
        let reason = reason.to_string();
        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO circuit_breaker_events (slot, reason) VALUES ($1, $2)",
                    &[&slot, &reason],
                )
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to record circuit breaker event: {e}"))
                })?;
            Ok(())
        })
    }

    fn row_to_attempt(row: &Row) -> AttemptRecord {
        let exp_net: i64 = row.try_get(3).unwrap_or(0);
        let tip: Option<i64> = row.try_get(4).unwrap_or(None);
        AttemptRecord {
            id: row.try_get(0).unwrap_or(0),
            slot: row.try_get(1).unwrap_or(0),
            route_id: row.try_get(2).unwrap_or_default(),
            expected_net: Lamports(exp_net.max(0) as u64),
            tip_lamports: tip.map(|t| Lamports(t.max(0) as u64)),
            decision: row.try_get(5).unwrap_or_default(),
        }
    }
}

impl Store for PostgresStore {
    fn record_attempt(&self, record: &AttemptRecord) -> Result<()> {
        let client = self.client.clone();
        let id = record.id;
        let slot = record.slot;
        let route_id = record.route_id.clone();
        let exp_net = record.expected_net.as_u64() as i64;
        let tip: Option<i64> = record.tip_lamports.map(|t| t.as_u64() as i64);
        let decision = record.decision.clone();

        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO attempts (id, slot, route_id, expected_net, tip_lamports, decision)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (id) DO UPDATE SET
                        expected_net = $4,
                        tip_lamports = $5,
                        decision = $6",
                    &[&id, &slot, &route_id, &exp_net, &tip, &decision],
                )
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
        let client = self.client.clone();
        let status = status.to_string();
        let net: Option<i64> = realized_net.map(|n| n.as_u64() as i64);

        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO outcomes (attempt_id, status, realized_net) VALUES ($1, $2, $3)",
                    &[&attempt_id, &status, &net],
                )
                .await
                .map_err(|e| ArbError::Internal(format!("Failed to insert PG outcome: {e}")))?;
            Ok(())
        })
    }

    fn get_attempt(&self, id: i64) -> Result<Option<AttemptRecord>> {
        let client = self.client.clone();
        self.block_on(async move {
            let row = client
                .query_opt(
                    "SELECT id, slot, route_id, expected_net, tip_lamports, decision
                     FROM attempts WHERE id = $1",
                    &[&id],
                )
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            Ok(row.as_ref().map(Self::row_to_attempt))
        })
    }

    fn get_recent_attempts(&self, limit: i64) -> Result<Vec<AttemptRecord>> {
        let client = self.client.clone();
        self.block_on(async move {
            let rows = client
                .query(
                    "SELECT id, slot, route_id, expected_net, tip_lamports, decision
                     FROM attempts ORDER BY id DESC LIMIT $1",
                    &[&limit],
                )
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            Ok(rows.iter().map(Self::row_to_attempt).collect())
        })
    }

    fn fills(&self, attempt_id: i64) -> Result<Vec<FillRecord>> {
        let client = self.client.clone();
        self.block_on(async move {
            let rows = client
                .query(
                    "SELECT attempt_id, token, amount FROM fills WHERE attempt_id = $1",
                    &[&attempt_id],
                )
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            let mut out = Vec::new();
            for row in &rows {
                let amt: i64 = row.try_get(2).unwrap_or(0);
                out.push(FillRecord {
                    attempt_id: row.try_get(0).unwrap_or(0),
                    token: row.try_get(1).unwrap_or_default(),
                    amount: TokenAmount(amt.max(0) as u64),
                });
            }
            Ok(out)
        })
    }

    fn pnl_daily(&self, day: &str) -> Result<PnlDaily> {
        let client = self.client.clone();
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
            let row = client
                .query_one(&query_str, &[])
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            let total_attempts: i64 = row.try_get(0).unwrap_or(0);
            let successful: i64 = row.try_get(1).unwrap_or(0);
            let total_realized: i64 = row.try_get(2).unwrap_or(0);

            Ok(PnlDaily {
                attempts: total_attempts.max(0) as u64,
                successful: successful.max(0) as u64,
                realized_net: Lamports(total_realized.max(0) as u64),
            })
        })
    }

    fn record_fill(&self, fill: &FillRecord) -> Result<()> {
        let client = self.client.clone();
        let amt = i64::try_from(fill.amount.as_u64()).map_err(|_| {
            ArbError::InvalidInput("fill amount exceeds PostgreSQL BIGINT range".into())
        })?;
        let attempt_id = fill.attempt_id;
        let token = fill.token.clone();

        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO fills (attempt_id, token, amount) VALUES ($1, $2, $3)",
                    &[&attempt_id, &token, &amt],
                )
                .await
                .map_err(|e| ArbError::Internal(format!("Failed to insert PG fill: {e}")))?;
            Ok(())
        })
    }

    fn record_dispatch_intent(&self, intent: &DispatchIntentRecord) -> Result<()> {
        let client = self.client.clone();
        let (attempt_id, signature, provider, dispatch_slot, last_valid_block_height) = (
            intent.attempt_id,
            intent.signature.clone(),
            intent.provider.clone(),
            intent.dispatch_slot,
            intent.last_valid_block_height,
        );
        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO dispatch_intents (attempt_id, signature, provider, dispatch_slot, last_valid_block_height, status)
                     VALUES ($1, $2, $3, $4, $5, 'pending')
                     ON CONFLICT (attempt_id) DO UPDATE SET
                        signature = $2, provider = $3, dispatch_slot = $4,
                        last_valid_block_height = $5, status = 'pending',
                        resolved_status = NULL, resolved_at = NULL",
                    &[
                        &attempt_id,
                        &signature,
                        &provider,
                        &dispatch_slot,
                        &last_valid_block_height,
                    ],
                )
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to insert PG dispatch intent: {e}"))
                })?;
            Ok(())
        })
    }

    fn pending_dispatch_intents(&self, limit: i64) -> Result<Vec<DispatchIntentRecord>> {
        let client = self.client.clone();
        self.block_on(async move {
            let rows = client
                .query(
                    "SELECT attempt_id, signature, provider, dispatch_slot, last_valid_block_height FROM dispatch_intents
                     WHERE status = 'pending' ORDER BY attempt_id ASC LIMIT $1",
                    &[&limit],
                )
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to query PG dispatch intents: {e}"))
                })?;
            Ok(rows
                .iter()
                .map(|row| DispatchIntentRecord {
                    attempt_id: row.try_get(0).unwrap_or(0),
                    signature: row.try_get(1).unwrap_or_default(),
                    provider: row.try_get(2).unwrap_or_default(),
                    dispatch_slot: row.try_get(3).unwrap_or(0),
                    last_valid_block_height: row.try_get(4).unwrap_or(None),
                    resolved_status: None,
                })
                .collect())
        })
    }

    fn resolve_dispatch_intent(&self, attempt_id: i64, resolved_status: &str) -> Result<()> {
        let client = self.client.clone();
        let resolved_status = resolved_status.to_string();
        self.block_on(async move {
            client
                .execute(
                    "UPDATE dispatch_intents SET status = 'resolved', resolved_status = $2,
                     resolved_at = CURRENT_TIMESTAMP WHERE attempt_id = $1 AND status = 'pending'",
                    &[&attempt_id, &resolved_status],
                )
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("Failed to resolve PG dispatch intent: {e}"))
                })?;
            Ok(())
        })
    }
}
