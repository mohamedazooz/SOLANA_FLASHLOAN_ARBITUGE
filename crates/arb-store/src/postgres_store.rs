//! PostgreSQL Production Store implementation using tokio-postgres.
//! Provides persistence for attempts, outcomes, fills, token prices, and circuit breaker events.
//!
//! Transport note: connections use `NoTls`. The supported deployment topology
//! is a PostgreSQL instance reachable on a trusted network (docker-compose /
//! localhost / private VPC). Hosts that require TLS are rejected at connect
//! time (fail-closed); point the store at a TLS-terminating proxy or extend
//! this module with a rustls connector before using such a host.

use crate::{
    AttemptRecord, DispatchIntentRecord, FeeRegistryRecord, FillRecord, NetProvenance,
    POSTGRES_INIT_SQL, PROTOCOL_INTELLIGENCE_POSTGRES_MIGRATION, PnlDaily, PoolRegistryRecord,
    PoolStateSnapshotRecord, ProtocolIntelligenceRepository, ProtocolMathSpecRecord,
    ProtocolRegistryRecord, QuoteRecord, RouteRegistryRecord, RouteSimulationRecord,
    SdkReconciliationRecord, Store,
};
use arb_types::{ArbError, Lamports, Result, SignedLamports, TokenAmount};
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
                client
                    .batch_execute(PROTOCOL_INTELLIGENCE_POSTGRES_MIGRATION)
                    .await
                    .map_err(|e| {
                        ArbError::Internal(format!("Failed to execute PostgreSQL protocol intelligence migration: {e}"))
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
                    // PIPE-11: plain INSERT — an id collision must fail loudly
                    // (caller refuses to dispatch) instead of overwriting a
                    // previous attempt's durable evidence.
                    "INSERT INTO attempts (id, slot, route_id, expected_net, tip_lamports, decision)
                     VALUES ($1, $2, $3, $4, $5, $6)",
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
        realized_net: Option<SignedLamports>,
        provenance: NetProvenance,
    ) -> Result<()> {
        let client = self.client.clone();
        let status = status.to_string();
        let provenance = provenance.as_str().to_string();
        let net: Option<i64> = realized_net.map(|n| n.as_i64());

        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO outcomes (attempt_id, status, realized_net, provenance) VALUES ($1, $2, $3, $4)",
                    &[&attempt_id, &status, &net, &provenance],
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
                        COALESCE(SUM(realized_net) FILTER (WHERE status = 'landed' AND provenance = 'actual'), 0)::BIGINT,
                        COALESCE(SUM(realized_net) FILTER (WHERE status IN ('canary', 'simulated')), 0)::BIGINT
                 FROM outcomes WHERE {filter}"
            );
            let row = client
                .query_one(&query_str, &[])
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            let total_attempts: i64 = row.try_get(0).unwrap_or(0);
            let successful: i64 = row.try_get(1).unwrap_or(0);
            let total_realized: i64 = row.try_get(2).unwrap_or(0);
            let total_simulated: i64 = row.try_get(3).unwrap_or(0);

            Ok(PnlDaily {
                attempts: total_attempts.max(0) as u64,
                successful: successful.max(0) as u64,
                realized_net: SignedLamports::new(total_realized),
                simulated_net: SignedLamports::new(total_simulated),
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

impl ProtocolIntelligenceRepository for PostgresStore {
    fn register_protocol(&self, r: &ProtocolRegistryRecord) -> Result<()> {
        let client = self.client.clone();
        let (protocol_id, protocol_name, protocol_type, program_id, network) = (
            r.protocol_id.clone(),
            r.protocol_name.clone(),
            r.protocol_type.clone(),
            r.program_id.clone(),
            r.network.clone(),
        );
        let (protocol_version, supported_features, sdk_name, sdk_version, sdk_commit) = (
            r.protocol_version.clone(),
            r.supported_features.clone(),
            r.sdk_name.clone(),
            r.sdk_version.clone(),
            r.sdk_commit.clone(),
        );
        let (source_repository, documentation_url, verification_status, enabled) = (
            r.source_repository.clone(),
            r.documentation_url.clone(),
            r.verification_status.clone(),
            r.enabled,
        );
        self.block_on(async move {
            client
                .execute(
                    "INSERT INTO protocol_registry (
                    protocol_id, protocol_name, protocol_type, program_id, network,
                    protocol_version, supported_features, sdk_name, sdk_version, sdk_commit,
                    source_repository, documentation_url, verification_status, enabled
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                ON CONFLICT (protocol_id) DO UPDATE SET
                    protocol_name = $2, protocol_type = $3, program_id = $4, network = $5,
                    protocol_version = $6, supported_features = $7, sdk_name = $8,
                    sdk_version = $9, sdk_commit = $10, source_repository = $11,
                    documentation_url = $12, verification_status = $13, enabled = $14,
                    last_verified_at = CURRENT_TIMESTAMP",
                    &[
                        &protocol_id,
                        &protocol_name,
                        &protocol_type,
                        &program_id,
                        &network,
                        &protocol_version,
                        &supported_features,
                        &sdk_name,
                        &sdk_version,
                        &sdk_commit,
                        &source_repository,
                        &documentation_url,
                        &verification_status,
                        &enabled,
                    ],
                )
                .await
                .map_err(|e| ArbError::Internal(format!("Failed to register PG protocol: {e}")))?;
            Ok(())
        })
    }

    fn register_math_spec(&self, r: &ProtocolMathSpecRecord) -> Result<()> {
        let client = self.client.clone();
        let (protocol_id, formula_version, input_amount_semantics, output_amount_semantics) = (
            r.protocol_id.clone(),
            r.formula_version.clone(),
            r.input_amount_semantics.clone(),
            r.output_amount_semantics.clone(),
        );
        let (price_formula, fee_formula, rounding_mode, integer_precision, fee_components) = (
            r.price_formula.clone(),
            r.fee_formula.clone(),
            r.rounding_mode.clone(),
            r.integer_precision.clone(),
            r.fee_components.clone(),
        );
        let (
            supported_swap_modes,
            unsupported_features,
            reference_source,
            verification_test_version,
        ) = (
            r.supported_swap_modes.clone(),
            r.unsupported_features.clone(),
            r.reference_source.clone(),
            r.verification_test_version.clone(),
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO protocol_math_specs (
                    protocol_id, formula_version, input_amount_semantics, output_amount_semantics,
                    price_formula, fee_formula, rounding_mode, integer_precision, fee_components,
                    supported_swap_modes, unsupported_features, reference_source, verification_test_version
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                &[
                    &protocol_id, &formula_version, &input_amount_semantics, &output_amount_semantics,
                    &price_formula, &fee_formula, &rounding_mode, &integer_precision, &fee_components,
                    &supported_swap_modes, &unsupported_features, &reference_source, &verification_test_version,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to register PG math spec: {e}")))?;
            Ok(())
        })
    }

    fn register_pool(&self, r: &PoolRegistryRecord) -> Result<()> {
        let client = self.client.clone();
        let (pool_address, protocol_id, pool_type, token_mint_a, token_mint_b) = (
            r.pool_address.clone(),
            r.protocol_id.clone(),
            r.pool_type.clone(),
            r.token_mint_a.clone(),
            r.token_mint_b.clone(),
        );
        let (token_decimals_a, token_decimals_b, token_program_a, token_program_b) = (
            r.token_decimals_a as i16,
            r.token_decimals_b as i16,
            r.token_program_a.clone(),
            r.token_program_b.clone(),
        );
        let (vault_a, vault_b, configuration_account, tick_spacing, bin_step) = (
            r.vault_a.clone(),
            r.vault_b.clone(),
            r.configuration_account.clone(),
            r.tick_spacing,
            r.bin_step,
        );
        let (active_bin_id, current_tick, pool_status, discovery_source) = (
            r.active_bin_id,
            r.current_tick,
            r.pool_status.clone(),
            r.discovery_source.clone(),
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO pool_registry (
                    pool_address, protocol_id, pool_type, token_mint_a, token_mint_b,
                    token_decimals_a, token_decimals_b, token_program_a, token_program_b,
                    vault_a, vault_b, configuration_account, tick_spacing, bin_step,
                    active_bin_id, current_tick, pool_status, discovery_source
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
                ON CONFLICT (pool_address) DO UPDATE SET
                    protocol_id = $2, pool_type = $3, token_mint_a = $4, token_mint_b = $5,
                    token_decimals_a = $6, token_decimals_b = $7, token_program_a = $8,
                    token_program_b = $9, vault_a = $10, vault_b = $11, configuration_account = $12,
                    tick_spacing = $13, bin_step = $14, active_bin_id = $15, current_tick = $16,
                    pool_status = $17, discovery_source = $18, last_verified_at = CURRENT_TIMESTAMP",
                &[
                    &pool_address, &protocol_id, &pool_type, &token_mint_a, &token_mint_b,
                    &token_decimals_a, &token_decimals_b, &token_program_a, &token_program_b,
                    &vault_a, &vault_b, &configuration_account, &tick_spacing, &bin_step,
                    &active_bin_id, &current_tick, &pool_status, &discovery_source,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to register PG pool: {e}")))?;
            Ok(())
        })
    }

    fn record_pool_snapshot(&self, s: &PoolStateSnapshotRecord) -> Result<()> {
        let client = self.client.clone();
        let (pool_address, slot, write_version, state_hash) = (
            s.pool_address.clone(),
            s.slot as i64,
            s.write_version as i64,
            s.state_hash.clone(),
        );
        let (reserve_a, reserve_b, sqrt_price, current_tick, active_bin) = (
            s.reserve_a.clone(),
            s.reserve_b.clone(),
            s.sqrt_price.clone(),
            s.current_tick,
            s.active_bin,
        );
        let (
            liquidity,
            protocol_specific_state,
            source_provider,
            state_validity,
            invalidation_reason,
        ) = (
            s.liquidity.clone(),
            s.protocol_specific_state.clone(),
            s.source_provider.clone(),
            s.state_validity.clone(),
            s.invalidation_reason.clone(),
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO pool_state_snapshots (
                    pool_address, slot, write_version, state_hash, reserve_a, reserve_b,
                    sqrt_price, current_tick, active_bin, liquidity, protocol_specific_state,
                    source_provider, state_validity, invalidation_reason
                ) VALUES ($1, $2, $3, $4, $5::NUMERIC, $6::NUMERIC, $7, $8, $9, $10, $11, $12, $13, $14)",
                &[
                    &pool_address, &slot, &write_version, &state_hash, &reserve_a, &reserve_b,
                    &sqrt_price, &current_tick, &active_bin, &liquidity, &protocol_specific_state,
                    &source_provider, &state_validity, &invalidation_reason,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to record PG pool snapshot: {e}")))?;
            Ok(())
        })
    }

    fn register_fee(&self, f: &FeeRegistryRecord) -> Result<()> {
        let client = self.client.clone();
        let (protocol_id, pool_address, fee_type, configuration_source, calculation_version) = (
            f.protocol_id.clone(),
            f.pool_address.clone(),
            f.fee_type.clone(),
            f.configuration_source.clone(),
            f.calculation_version.clone(),
        );
        let (effective_slot, fee_numerator, fee_denominator, fee_bps) = (
            f.effective_slot as i64,
            f.fee_numerator as i64,
            f.fee_denominator as i64,
            f.fee_bps as i32,
        );
        let (rounding_rule, verification_status) =
            (f.rounding_rule.clone(), f.verification_status.clone());
        self.block_on(async move {
            client.execute(
                "INSERT INTO fee_registry (
                    protocol_id, pool_address, fee_type, configuration_source, calculation_version,
                    effective_slot, fee_numerator, fee_denominator, fee_bps, rounding_rule, verification_status
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &protocol_id, &pool_address, &fee_type, &configuration_source, &calculation_version,
                    &effective_slot, &fee_numerator, &fee_denominator, &fee_bps, &rounding_rule, &verification_status,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to register PG fee: {e}")))?;
            Ok(())
        })
    }

    fn record_quote(&self, q: &QuoteRecord) -> Result<()> {
        let client = self.client.clone();
        let (quote_id, protocol_id, pool_address, input_mint, output_mint) = (
            q.quote_id.clone(),
            q.protocol_id.clone(),
            q.pool_address.clone(),
            q.input_mint.clone(),
            q.output_mint.clone(),
        );
        let (amount_in_raw, amount_out_raw, fees_breakdown, price_impact_bps) = (
            q.amount_in_raw.clone(),
            q.amount_out_raw.clone(),
            q.fees_breakdown.clone(),
            q.price_impact_bps as i32,
        );
        let (unfilled_input_raw, minimum_output_raw, state_slot, state_hash) = (
            q.unfilled_input_raw.clone(),
            q.minimum_output_raw.clone(),
            q.state_slot as i64,
            q.state_hash.clone(),
        );
        let (quote_engine_version, sdk_reference_version, quote_latency_us, quote_validity) = (
            q.quote_engine_version.clone(),
            q.sdk_reference_version.clone(),
            q.quote_latency_us as i64,
            q.quote_validity.clone(),
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO quote_records (
                    quote_id, protocol_id, pool_address, input_mint, output_mint,
                    amount_in_raw, amount_out_raw, fees_breakdown, price_impact_bps,
                    unfilled_input_raw, minimum_output_raw, state_slot, state_hash,
                    quote_engine_version, sdk_reference_version, quote_latency_us, quote_validity
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
                ON CONFLICT (quote_id) DO UPDATE SET
                    amount_in_raw = $6, amount_out_raw = $7, fees_breakdown = $8,
                    price_impact_bps = $9, unfilled_input_raw = $10, minimum_output_raw = $11,
                    state_slot = $12, state_hash = $13, quote_latency_us = $16, quote_validity = $17",
                &[
                    &quote_id, &protocol_id, &pool_address, &input_mint, &output_mint,
                    &amount_in_raw, &amount_out_raw, &fees_breakdown, &price_impact_bps,
                    &unfilled_input_raw, &minimum_output_raw, &state_slot, &state_hash,
                    &quote_engine_version, &sdk_reference_version, &quote_latency_us, &quote_validity,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to record PG quote: {e}")))?;
            Ok(())
        })
    }

    fn register_route(&self, r: &RouteRegistryRecord) -> Result<()> {
        let client = self.client.clone();
        let (route_id, route_type, number_of_pools) = (
            r.route_id.clone(),
            r.route_type.clone(),
            r.number_of_pools as i32,
        );
        let (ordered_pool_addresses, ordered_protocol_ids, input_mint) = (
            r.ordered_pool_addresses.clone(),
            r.ordered_protocol_ids.clone(),
            r.input_mint.clone(),
        );
        let (intermediate_mints, output_mint, route_version, enabled) = (
            r.intermediate_mints.clone(),
            r.output_mint.clone(),
            r.route_version.clone(),
            r.enabled,
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO route_registry (
                    route_id, route_type, number_of_pools, ordered_pool_addresses, ordered_protocol_ids,
                    input_mint, intermediate_mints, output_mint, route_version, enabled
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT (route_id) DO UPDATE SET
                    number_of_pools = $3, ordered_pool_addresses = $4, ordered_protocol_ids = $5,
                    intermediate_mints = $7, route_version = $9, enabled = $10, last_validation_at = CURRENT_TIMESTAMP",
                &[
                    &route_id, &route_type, &number_of_pools, &ordered_pool_addresses,
                    &ordered_protocol_ids, &input_mint, &intermediate_mints, &output_mint,
                    &route_version, &enabled,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to register PG route: {e}")))?;
            Ok(())
        })
    }

    fn record_simulation(&self, s: &RouteSimulationRecord) -> Result<()> {
        let client = self.client.clone();
        let (simulation_id, route_id, input_amount_raw, output_amount_raw, principal_raw) = (
            s.simulation_id.clone(),
            s.route_id.clone(),
            s.input_amount_raw.clone(),
            s.output_amount_raw.clone(),
            s.principal_raw.clone(),
        );
        let (
            flash_loan_fee_raw,
            network_fee_raw,
            priority_fee_raw,
            tip_raw,
            token_transfer_fees_raw,
        ) = (
            s.flash_loan_fee_raw.clone(),
            s.network_fee_raw.clone(),
            s.priority_fee_raw.clone(),
            s.tip_raw.clone(),
            s.token_transfer_fees_raw.clone(),
        );
        let (total_costs_raw, net_profit_raw, transaction_size_bytes, compute_units_consumed) = (
            s.total_costs_raw.clone(),
            s.net_profit_raw.clone(),
            s.transaction_size_bytes as i32,
            s.compute_units_consumed as i64,
        );
        let (simulation_slot, simulation_result, simulation_error) = (
            s.simulation_slot as i64,
            s.simulation_result.clone(),
            s.simulation_error.clone(),
        );
        self.block_on(async move {
            client.execute(
                "INSERT INTO route_simulation_records (
                    simulation_id, route_id, input_amount_raw, output_amount_raw, principal_raw,
                    flash_loan_fee_raw, network_fee_raw, priority_fee_raw, tip_raw, token_transfer_fees_raw,
                    total_costs_raw, net_profit_raw, transaction_size_bytes, compute_units_consumed,
                    simulation_slot, simulation_result, simulation_error
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
                ON CONFLICT (simulation_id) DO UPDATE SET
                    output_amount_raw = $4, total_costs_raw = $11, net_profit_raw = $12,
                    transaction_size_bytes = $13, compute_units_consumed = $14,
                    simulation_result = $16, simulation_error = $17",
                &[
                    &simulation_id, &route_id, &input_amount_raw, &output_amount_raw, &principal_raw,
                    &flash_loan_fee_raw, &network_fee_raw, &priority_fee_raw, &tip_raw, &token_transfer_fees_raw,
                    &total_costs_raw, &net_profit_raw, &transaction_size_bytes, &compute_units_consumed,
                    &simulation_slot, &simulation_result, &simulation_error,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to record PG simulation: {e}")))?;
            Ok(())
        })
    }

    fn record_sdk_reconciliation(&self, rec: &SdkReconciliationRecord) -> Result<()> {
        let client = self.client.clone();
        let (protocol_id, pool_address, input_amount_raw, local_quote_raw, sdk_quote_raw) = (
            rec.protocol_id.clone(),
            rec.pool_address.clone(),
            rec.input_amount_raw.clone(),
            rec.local_quote_raw.clone(),
            rec.sdk_quote_raw.clone(),
        );
        let (onchain_simulated_quote_raw, difference_raw, difference_bps, state_slot) = (
            rec.onchain_simulated_quote_raw.clone(),
            rec.difference_raw.clone(),
            rec.difference_bps,
            rec.state_slot as i64,
        );
        let (result, failure_reason) = (rec.result.clone(), rec.failure_reason.clone());
        self.block_on(async move {
            client.execute(
                "INSERT INTO sdk_reconciliation_records (
                    protocol_id, pool_address, input_amount_raw, local_quote_raw, sdk_quote_raw,
                    onchain_simulated_quote_raw, difference_raw, difference_bps, state_slot, result, failure_reason
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &protocol_id, &pool_address, &input_amount_raw, &local_quote_raw, &sdk_quote_raw,
                    &onchain_simulated_quote_raw, &difference_raw, &difference_bps, &state_slot,
                    &result, &failure_reason,
                ],
            ).await.map_err(|e| ArbError::Internal(format!("Failed to record PG sdk reconciliation: {e}")))?;
            Ok(())
        })
    }

    fn get_protocol(&self, protocol_id: &str) -> Result<Option<ProtocolRegistryRecord>> {
        let client = self.client.clone();
        let pid = protocol_id.to_string();
        self.block_on(async move {
            let row = client
                .query_opt(
                    "SELECT protocol_id, protocol_name, protocol_type, program_id, network,
                        protocol_version, supported_features, sdk_name, sdk_version, sdk_commit,
                        source_repository, documentation_url, verification_status, enabled
                 FROM protocol_registry WHERE protocol_id = $1",
                    &[&pid],
                )
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            if let Some(row) = row {
                Ok(Some(ProtocolRegistryRecord {
                    protocol_id: row.get(0),
                    protocol_name: row.get(1),
                    protocol_type: row.get(2),
                    program_id: row.get(3),
                    network: row.get(4),
                    protocol_version: row.get(5),
                    supported_features: row.get(6),
                    sdk_name: row.get(7),
                    sdk_version: row.get(8),
                    sdk_commit: row.get(9),
                    source_repository: row.get(10),
                    documentation_url: row.get(11),
                    verification_status: row.get(12),
                    enabled: row.get(13),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_pool(&self, pool_address: &str) -> Result<Option<PoolRegistryRecord>> {
        let client = self.client.clone();
        let addr = pool_address.to_string();
        self.block_on(async move {
            let row = client
                .query_opt(
                    "SELECT pool_address, protocol_id, pool_type, token_mint_a, token_mint_b,
                        token_decimals_a, token_decimals_b, token_program_a, token_program_b,
                        vault_a, vault_b, configuration_account, tick_spacing, bin_step,
                        active_bin_id, current_tick, pool_status, discovery_source
                 FROM pool_registry WHERE pool_address = $1",
                    &[&addr],
                )
                .await
                .map_err(|e| ArbError::Internal(e.to_string()))?;

            if let Some(row) = row {
                let dec_a: i16 = row.get(5);
                let dec_b: i16 = row.get(6);
                Ok(Some(PoolRegistryRecord {
                    pool_address: row.get(0),
                    protocol_id: row.get(1),
                    pool_type: row.get(2),
                    token_mint_a: row.get(3),
                    token_mint_b: row.get(4),
                    token_decimals_a: dec_a as u8,
                    token_decimals_b: dec_b as u8,
                    token_program_a: row.get(7),
                    token_program_b: row.get(8),
                    vault_a: row.get(9),
                    vault_b: row.get(10),
                    configuration_account: row.get(11),
                    tick_spacing: row.get(12),
                    bin_step: row.get(13),
                    active_bin_id: row.get(14),
                    current_tick: row.get(15),
                    pool_status: row.get(16),
                    discovery_source: row.get(17),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_latest_snapshot(&self, pool_address: &str) -> Result<Option<PoolStateSnapshotRecord>> {
        let client = self.client.clone();
        let addr = pool_address.to_string();
        self.block_on(async move {
            let row = client.query_opt(
                "SELECT pool_address, slot, write_version, state_hash, reserve_a::TEXT, reserve_b::TEXT,
                        sqrt_price, current_tick, active_bin, liquidity, protocol_specific_state,
                        source_provider, state_validity, invalidation_reason
                 FROM pool_state_snapshots WHERE pool_address = $1 ORDER BY slot DESC LIMIT 1",
                &[&addr],
            ).await.map_err(|e| ArbError::Internal(e.to_string()))?;

            if let Some(row) = row {
                let slot: i64 = row.get(1);
                let wv: i64 = row.get(2);
                Ok(Some(PoolStateSnapshotRecord {
                    pool_address: row.get(0),
                    slot: slot as u64,
                    write_version: wv as u64,
                    state_hash: row.get(3),
                    reserve_a: row.get(4),
                    reserve_b: row.get(5),
                    sqrt_price: row.get(6),
                    current_tick: row.get(7),
                    active_bin: row.get(8),
                    liquidity: row.get(9),
                    protocol_specific_state: row.get(10),
                    source_provider: row.get(11),
                    state_validity: row.get(12),
                    invalidation_reason: row.get(13),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_recent_reconciliations(
        &self,
        protocol_id: &str,
        limit: usize,
    ) -> Result<Vec<SdkReconciliationRecord>> {
        let client = self.client.clone();
        let pid = protocol_id.to_string();
        let lim = limit as i64;
        self.block_on(async move {
            let rows = client.query(
                "SELECT protocol_id, pool_address, input_amount_raw, local_quote_raw, sdk_quote_raw,
                        onchain_simulated_quote_raw, difference_raw, difference_bps, state_slot, result, failure_reason
                 FROM sdk_reconciliation_records WHERE protocol_id = $1 ORDER BY created_at DESC LIMIT $2",
                &[&pid, &lim],
            ).await.map_err(|e| ArbError::Internal(e.to_string()))?;

            Ok(rows.iter().map(|row| {
                let slot: i64 = row.get(8);
                SdkReconciliationRecord {
                    protocol_id: row.get(0),
                    pool_address: row.get(1),
                    input_amount_raw: row.get(2),
                    local_quote_raw: row.get(3),
                    sdk_quote_raw: row.get(4),
                    onchain_simulated_quote_raw: row.get(5),
                    difference_raw: row.get(6),
                    difference_bps: row.get(7),
                    state_slot: slot as u64,
                    result: row.get(9),
                    failure_reason: row.get(10),
                }
            }).collect())
        })
    }
}
