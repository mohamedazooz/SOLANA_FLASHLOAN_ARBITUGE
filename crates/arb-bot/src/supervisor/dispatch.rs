//! Mode-specific execution, preflight simulation, dispatch and settlement tracking.
use super::*;
use anyhow::Context;
use arb_store::NetProvenance;
use arb_types::{InflightStatus, SignedLamports};
use base64::prelude::*;

pub(super) struct DispatchContext<'a> {
    pub opp: &'a ExecutionOpportunity,
    pub tip: Lamports,
    pub tip_rung: Option<arb_types::Percentile>,
    pub is_swqos_route: bool,
    pub current_slot: Slot,
    pub expected_stream_generation: u64,
    pub route_label: &'a str,
    pub gross_rev_sol: f64,
    #[allow(dead_code)] // logged by telemetry sinks, not read on dispatch path
    pub gross_sol: f64,
    pub profit_sol: f64,
    pub live_sol_price: f64,
    pub blockhash_b58: &'a str,
    /// `lastValidBlockHeight` of this blockhash, persisted with the dispatch
    /// intent so recovery can prove expiration with block-height evidence.
    #[allow(dead_code)] // mirrored into DispatchIntent for recovery evidence
    pub last_valid_block_height: Option<i64>,
    pub cu_limit: u32,
    pub cu_price: u64,
    pub tx_bytes: &'a Vec<u8>,
    pub record: &'a AttemptRecord,
    pub real_net_profit: Lamports,
    pub lamports_per_base_raw: u128,
    pub lamports_per_base_denom: u128,
    pub payer: Pubkey,
    pub route_sig: &'a str,
    pub t_ingest_to_quote_us: u64,
    pub t_risk_us: u64,
    pub t_assembly_us: u64,
    pub t_signing_us: u64,
    pub pools_count: usize,
    pub loan_size_sol: f64,
}

fn format_mint_short(pk: &Pubkey) -> String {
    let s = pk.to_string();
    if s == arb_constants::WSOL_MINT {
        "WSOL".to_string()
    } else if s == arb_constants::USDC_MINT {
        "USDC".to_string()
    } else if s.len() > 8 {
        format!("{}...{}", &s[..4], &s[s.len() - 4..])
    } else {
        s
    }
}

/// Signed view of a u64 lamports value for the signed-outcome store. Values
/// beyond `i64::MAX` are unreachable for bounded borrows configured by the
/// risk gate; clamp instead of silently wrapping.
fn lifecycle_to_land_status(lifecycle: arb_land::TransactionLifecycle) -> arb_land::LandStatus {
    if lifecycle.is_confirmed() {
        arb_land::LandStatus::Landed
    } else {
        match lifecycle {
            arb_land::TransactionLifecycle::Failed => arb_land::LandStatus::Failed,
            arb_land::TransactionLifecycle::Expired => arb_land::LandStatus::Expired,
            // Processed is provisional inclusion, not settlement proof.
            _ => arb_land::LandStatus::Sent,
        }
    }
}

fn signed_lamports(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Minimum payer balance after preserving the configured reserve and funding
/// the transaction's temporary rent plus non-refundable network costs.
fn required_live_payer_balance(
    reserve_lamports: u64,
    refundable_ata_rent_lamports: u64,
    base_fee_lamports: u64,
    priority_fee_lamports: u64,
    jito_tip_lamports: u64,
) -> Option<u64> {
    [
        reserve_lamports,
        refundable_ata_rent_lamports,
        base_fee_lamports,
        priority_fee_lamports,
        jito_tip_lamports,
    ]
    .into_iter()
    .try_fold(0u64, u64::checked_add)
}

fn env_flag_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

impl Supervisor {
    /// Canonical V1 transport capability gates shared identically by
    /// `opportunity.rs` (compile gate) and `dispatch.rs` (broadcast gate).
    pub(super) fn v1_transport_flags(&self) -> (bool, bool) {
        let simulate_only = matches!(self.mode(), BotMode::Simulate);
        let direct_v1_enabled = simulate_only
            || (env_flag_truthy("SOLANA_TX_V1_DIRECT_ENABLED")
                && env_flag_truthy("SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED")
                && self.rpc_pool.has_v1_rpc_allowlist());
        let jito_v1_enabled = simulate_only
            || (env_flag_truthy("SOLANA_TX_V1_JITO_ENABLED")
                && env_flag_truthy("SOLANA_TX_V1_JITO_INGRESS_VERIFIED"));
        (direct_v1_enabled, jito_v1_enabled)
    }

    /// Return the up-front refundable rent capital required for route ATAs
    /// that do not exist yet. Runs only at LIVE pre-submit time, never in the
    /// market-data or route-sizing hot path.
    ///
    /// Legacy SPL accounts use the canonical 165-byte layout and rent is read
    /// from the active RPC. Missing Token-2022 ATAs fail closed because their
    /// extension-dependent account length is not derivable from the route alone.
    async fn route_missing_ata_rent_lamports(
        &self,
        payer: Pubkey,
        hops: &[SwapHop],
        current_slot: Slot,
    ) -> anyhow::Result<u64> {
        let plan = arb_tx::build_route_ata_plan(payer, hops)?;
        let legacy_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)?;
        let token_2022_program = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID)?;

        let mut expected_accounts = Vec::with_capacity(plan.intermediate_atas.len() + 1);
        expected_accounts.push((plan.settlement_ata, plan.settlement_token_program));
        expected_accounts.extend(
            plan.intermediate_atas
                .into_iter()
                .map(|(_, address, token_program)| (address, token_program)),
        );

        let account_keys: Vec<String> = expected_accounts
            .iter()
            .map(|(address, _)| address.to_string())
            .collect();
        let account_key_refs: Vec<&str> = account_keys.iter().map(String::as_str).collect();

        let (accounts, account_slots) = tokio::time::timeout(
            Duration::from_secs(3),
            self.rpc_pool
                .get_multiple_accounts_with_slots(&account_key_refs),
        )
        .await
        .context("route ATA account lookup timed out")??;

        if accounts.len() != expected_accounts.len()
            || account_slots.len() != expected_accounts.len()
        {
            anyhow::bail!(
                "route ATA RPC returned {} accounts and {} context slots for {} requested addresses",
                accounts.len(),
                account_slots.len(),
                expected_accounts.len()
            );
        }

        let mut missing_legacy_accounts = 0u64;
        for (((address, expected_program), account), account_slot) in expected_accounts
            .iter()
            .zip(accounts.iter())
            .zip(account_slots.iter())
        {
            if *account_slot == 0
                || account_slot.saturating_add(self.config.strategy.max_state_age_slots)
                    < current_slot.0
            {
                anyhow::bail!(
                    "route ATA account snapshot for {address} is stale (RPC slot {account_slot}, current slot {})",
                    current_slot.0
                );
            }
            match account {
                Some(info) => {
                    let expected_owner = expected_program.to_string();
                    let actual_owner = info.get("owner").and_then(serde_json::Value::as_str);
                    if actual_owner != Some(expected_owner.as_str()) {
                        anyhow::bail!(
                            "route ATA {address} exists with unexpected token-program owner"
                        );
                    }
                }
                None if *expected_program == token_2022_program => {
                    anyhow::bail!(
                        "route requires creation of missing Token-2022 ATA {address}; extension-dependent account size is unknown"
                    );
                }
                None if *expected_program == legacy_program => {
                    missing_legacy_accounts = missing_legacy_accounts
                        .checked_add(1)
                        .context("missing ATA counter overflow")?;
                }
                None => {
                    anyhow::bail!("route ATA {address} uses an unsupported token program");
                }
            }
        }

        if missing_legacy_accounts == 0 {
            return Ok(0);
        }

        let minimum_balance = self
            .legacy_token_account_rent
            .get_or_try_init(|| async {
                let response = tokio::time::timeout(
                    Duration::from_secs(3),
                    self.rpc_pool.call(
                        "getMinimumBalanceForRentExemption",
                        serde_json::json!([165, {"commitment": "processed"}]),
                    ),
                )
                .await
                .context("legacy token-account rent lookup timed out")??;
                response
                    .as_u64()
                    .filter(|rent| *rent > 0)
                    .context("RPC returned an invalid legacy token-account rent minimum")
            })
            .await?;

        minimum_balance
            .checked_mul(missing_legacy_accounts)
            .context("route ATA rent requirement overflowed u64")
    }

    pub(super) async fn dispatch_opportunity(&self, context: DispatchContext<'_>) {
        let DispatchContext {
            opp,
            tip,
            tip_rung,
            is_swqos_route,
            current_slot,
            expected_stream_generation,
            route_label,
            gross_rev_sol,
            gross_sol,
            profit_sol,
            live_sol_price,
            blockhash_b58,
            last_valid_block_height,
            cu_limit,
            cu_price,
            tx_bytes,
            record,
            real_net_profit,
            lamports_per_base_raw,
            lamports_per_base_denom,
            payer,
            route_sig,
            t_ingest_to_quote_us,
            t_risk_us,
            t_assembly_us,
            t_signing_us,
            pools_count,
            loan_size_sol,
        } = context;

        let active_bins: Vec<Option<i32>> = opp.hops.iter().map(|h| h.active_bin_id).collect();

        // GUARD-01: Revalidate critical state immediately before dispatch.
        // Fail-closed: stream generation must match quoting generation, and every pool
        // must remain fresh, un-invalidated, and without active bin shifts.
        if self.ingest.stream_generation() != expected_stream_generation {
            warn!("GUARD-01: Ingest stream reconnected during cycle; rejecting dispatch");
            return;
        }

        let max_age_slots = self.config.strategy.max_state_age_slots;
        for hop in &opp.hops {
            let Some(live_pool) = self.ingest.cache().get_pool(&hop.pool_id) else {
                warn!(
                    "GUARD-01: Pool {} vanished from cache before dispatch; aborting",
                    hop.pool_id.0
                );
                return;
            };
            if current_slot.0.saturating_sub(live_pool.last_update_slot) > max_age_slots {
                warn!(
                    "GUARD-01: Pool {} became stale before dispatch (last slot {}); aborting",
                    hop.pool_id.0, live_pool.last_update_slot
                );
                return;
            }
            if self.ingest.slot_is_dead(live_pool.slot.0) {
                warn!(
                    "GUARD-01: Pool {} updated in dead slot {}; aborting dispatch",
                    hop.pool_id.0, live_pool.slot.0
                );
                return;
            }
            if let Some(ref dlmm) = live_pool.dlmm {
                if hop.active_bin_id.is_some() && hop.active_bin_id != Some(dlmm.active_bin_id) {
                    warn!(
                        "GUARD-01: DLMM pool {} active bin shifted from {:?} to {} before dispatch; aborting",
                        hop.pool_id.0, hop.active_bin_id, dlmm.active_bin_id
                    );
                    return;
                }
            }
        }

        // 6. Dispatch based on mode
        let leader_str = self
            .jito_service
            .leader_for(current_slot)
            .map(|pk| pk.to_string())
            .unwrap_or_else(|| "Unknown (Non-Jito / Stale Schedule)".to_string());

        match self.mode() {
            BotMode::Observe => {
                debug!("Observe mode active: skipping dispatch for {}", route_label);
            }
            BotMode::Analyze => {
                debug!("Analyze mode active: skipping dispatch for {}", route_label);
            }
            BotMode::PaperTrade => {
                info!(
                    "Paper trade: simulated execution recorded for route {}",
                    route_label
                );
                arb_metrics::add_simulated_net_profit(profit_sol);
                let _ = self.store.record_outcome(
                    record.id,
                    "paper_trade",
                    Some(SignedLamports::new(signed_lamports(
                        real_net_profit.as_u64(),
                    ))),
                    arb_store::NetProvenance::Estimated,
                );
                self.telegram
                    .alert_opportunity_with_telemetry(
                        &format!("PaperTrade-{}", route_label),
                        gross_rev_sol,
                        profit_sol,
                        profit_sol * live_sol_price,
                        current_slot.0,
                        blockhash_b58,
                        &leader_str,
                        cu_limit,
                        cu_price,
                        opp.price_impact_bps.0,
                        &active_bins,
                        pools_count,
                        loan_size_sol,
                    )
                    .await;
            }
            BotMode::Canary => {
                info!(
                    "canary: simulated valid execution of {} bytes (not broadcast to Jito)",
                    tx_bytes.len()
                );
                // Canary runs never broadcast, so they must never be
                // counted as fills or recorded as realized PnL. Record
                // the simulated edge under the simulated metric and a
                // distinct "canary" outcome for auditability.
                // Simulated figure with `estimated` provenance lands in the
                // simulated bucket, never in realized PnL (PIPE-01).
                arb_metrics::add_simulated_net_profit(profit_sol);
                let _ = self.store.record_outcome(
                    record.id,
                    "canary",
                    Some(SignedLamports::new(signed_lamports(
                        real_net_profit.as_u64(),
                    ))),
                    arb_store::NetProvenance::Estimated,
                );
                self.telegram
                    .alert_opportunity_with_telemetry(
                        &format!("Canary-Simulated-{}", route_label),
                        gross_rev_sol,
                        profit_sol,
                        profit_sol * live_sol_price,
                        current_slot.0,
                        blockhash_b58,
                        &leader_str,
                        cu_limit,
                        cu_price,
                        opp.price_impact_bps.0,
                        &active_bins,
                        pools_count,
                        loan_size_sol,
                    )
                    .await;
            }
            BotMode::Simulate => {
                let b64 = BASE64_STANDARD.encode(tx_bytes.as_slice());
                let max_version = arb_tx::rpc_max_supported_version_from_wire(tx_bytes)
                    .ok()
                    .flatten();
                let sim_result = self
                    .tx_rpc
                    .simulate_transaction(&b64, max_version)
                    .await
                    .and_then(|value| {
                        arb_sim::parse_simulation_response(&value)
                            .map_err(|error| anyhow::anyhow!(error))
                    });
                let mut sim = match sim_result {
                    Ok(sim) => sim,
                    Err(error) => {
                        tracing::warn!(
                            target: "arb_events",
                            "SIMULATION_UNAVAILABLE: error={}",
                            error
                        );
                        warn!("Simulation did not produce valid execution evidence: {error}");
                        let _ = self.store.record_outcome(
                            record.id,
                            "simulation_unavailable",
                            None,
                            NetProvenance::Estimated,
                        );
                        return;
                    }
                };
                sim.enforce_quote_freshness(opp.slot.0, self.config.strategy.max_state_age_slots);
                if sim.status != arb_sim::SimulationStatus::Passed {
                    let error = sim.error.as_deref().unwrap_or("simulation not passed");
                    tracing::warn!(
                        target: "arb_events",
                        "SIMULATION_REJECTED: status={:?} error={}",
                        sim.status,
                        error
                    );
                    warn!(
                        "Simulation rejected opportunity ({:?}): {error}",
                        sim.status
                    );
                    let _ = self.store.record_outcome(
                        record.id,
                        "simulation_rejected",
                        None,
                        NetProvenance::Estimated,
                    );
                    return;
                }
                tracing::info!(
                    target: "arb_events",
                    "SIMULATION_PASSED: cu_consumed={} context_slot={:?}",
                    sim.units_consumed,
                    sim.context_slot
                );
                info!(
                    "✅ Simulation passed via HTTP Transaction RPC adapter (CU={})",
                    sim.units_consumed
                );
                arb_metrics::add_simulated_net_profit(profit_sol);
                let _ = self.store.record_outcome(
                    record.id,
                    "simulated",
                    Some(SignedLamports::new(signed_lamports(
                        real_net_profit.as_u64(),
                    ))),
                    arb_store::NetProvenance::Estimated,
                );
                if real_net_profit.as_u64() >= self.config.strategy.min_net_profit_abs {
                    self.telegram
                        .alert_opportunity_with_telemetry(
                            &format!("Simulated-{}", opp.route_id.0),
                            gross_rev_sol,
                            profit_sol,
                            profit_sol * live_sol_price,
                            current_slot.0,
                            blockhash_b58,
                            &leader_str,
                            cu_limit,
                            cu_price,
                            opp.price_impact_bps.0,
                            &active_bins,
                            pools_count,
                            loan_size_sol,
                        )
                        .await;
                }
            }
            BotMode::Live => {
                if !crate::live_execution_enabled() {
                    warn!("LIVE mode dispatch rejected: ARB_LIVE=1 or LIVE_TRADING=1 is not armed");
                    return;
                }
                if !self.tx_rpc.is_configured() {
                    warn!(
                        "LIVE mode dispatch rejected: HTTP Transaction RPC adapter has no configured endpoint (cannot execute)"
                    );
                    tracing::warn!(
                        target: "arb_events",
                        "OPPORTUNITY_REJECTED: route={} stage=LiveExecution reason=Missing HTTP Transaction RPC endpoint",
                        route_label
                    );
                    return;
                }
                // Wallet balance remains a background-refreshed cache. ATA
                // existence/rent RPC is performed only in this final LIVE
                // pre-submit guard, never in market-data capture or route sizing.
                // A missing/stale balance snapshot fails closed.
                let min_reserve_lamports = self.min_sol_reserve_lamports;
                if min_reserve_lamports > 0 {
                    let current_balance = match self
                        .wallet_balance_cache
                        .snapshot(Duration::from_secs(15))
                    {
                        Some(balance) => balance,
                        None => {
                            let reason = "رصيد المحفظة الاحتياطي مفقود أو قديم؛ تم رفض الإرسال المباشر حفاظاً على هامش الرسوم".to_string();
                            warn!("Wallet reserve snapshot missing/stale; rejecting live dispatch");
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.to_string(),
                                    gross_sol,
                                    stage: "Safety Floor Snapshot".to_string(),
                                    reason: reason.clone(),
                                    slot: current_slot.0,
                                    pools_count,
                                    loan_size_sol,
                                    net_sol: Some(profit_sol),
                                },
                            );
                            self.telegram
                                .alert_opportunity_rejected(
                                    route_label,
                                    gross_sol,
                                    "Safety Floor Snapshot",
                                    &reason,
                                    current_slot.0,
                                    pools_count,
                                    loan_size_sol,
                                    Some(profit_sol),
                                )
                                .await;
                            return;
                        }
                    };
                    let ata_rent_lamports = match self
                        .route_missing_ata_rent_lamports(payer, &opp.hops, current_slot)
                        .await
                    {
                        Ok(rent) => rent,
                        Err(error) => {
                            let reason =
                                format!("route ATA rent preflight failed closed: {error:#}");
                            warn!("{reason}");
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.to_string(),
                                    gross_sol,
                                    stage: "ATA Rent Preflight".to_string(),
                                    reason: reason.clone(),
                                    slot: current_slot.0,
                                    pools_count,
                                    loan_size_sol,
                                    net_sol: Some(profit_sol),
                                },
                            );
                            self.telegram
                                .alert_opportunity_rejected(
                                    route_label,
                                    gross_sol,
                                    "ATA Rent Preflight",
                                    &reason,
                                    current_slot.0,
                                    pools_count,
                                    loan_size_sol,
                                    Some(profit_sol),
                                )
                                .await;
                            return;
                        }
                    };
                    let priority_fee_lamports = arb_tx::priority_fee_lamports(cu_price, cu_limit);
                    let required_balance = match required_live_payer_balance(
                        min_reserve_lamports,
                        ata_rent_lamports,
                        BASE_FEE_LAMPORTS,
                        priority_fee_lamports,
                        tip.as_u64(),
                    ) {
                        Some(required) => required,
                        None => {
                            warn!(
                                "reserve + refundable ATA rent + transaction fees overflowed; rejecting live dispatch"
                            );
                            return;
                        }
                    };
                    let network_cost_lamports = BASE_FEE_LAMPORTS
                        .checked_add(priority_fee_lamports)
                        .and_then(|value| value.checked_add(tip.as_u64()))
                        .unwrap_or(u64::MAX);
                    if current_balance < required_balance {
                        let reason = format!(
                            "wallet balance ({:.4} SOL) is below required reserve ({:.4} SOL) + refundable ATA rent lock ({:.6} SOL) + base/priority fees and Jito tip ({:.6} SOL)",
                            current_balance as f64 / 1e9,
                            min_reserve_lamports as f64 / 1e9,
                            ata_rent_lamports as f64 / 1e9,
                            network_cost_lamports as f64 / 1e9
                        );
                        warn!(
                            "Safety Floor Active: cached wallet balance {} lamports is below reserve + refundable ATA rent + base/priority fees + tip {} lamports; skipping execution",
                            current_balance, required_balance
                        );
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.to_string(),
                            gross_sol,
                            stage: "Safety Floor Reserve".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                            pools_count,
                            loan_size_sol,
                            net_sol: Some(profit_sol),
                        });
                        self.telegram
                            .alert_opportunity_rejected(
                                route_label,
                                gross_sol,
                                "Safety Floor Reserve",
                                &reason,
                                current_slot.0,
                                pools_count,
                                loan_size_sol,
                                Some(profit_sol),
                            )
                            .await;
                        return;
                    }
                }

                // V1-only release policy: never accept Legacy or V0 output.
                // V1 has inline accounts (no ALTs), a 4,096-byte limit, and this
                // release only uses direct RPC/SWQoS simulation/transport.
                let tx_version =
                    match arb_tx::CompiledAdaptiveTransaction::parse_version_from_wire(tx_bytes) {
                        Ok(version) => version,
                        Err(error) => {
                            warn!("Rejecting malformed/unknown transaction envelope: {error}");
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.to_string(),
                                    gross_sol,
                                    stage: "Transaction Format Check".to_string(),
                                    reason: format!("تعذر تحديد إصدار المعاملة: {error}"),
                                    slot: current_slot.0,
                                    pools_count,
                                    loan_size_sol,
                                    net_sol: Some(profit_sol),
                                },
                            );
                            return;
                        }
                    };
                if tx_version != arb_tx::TransactionVersion::V1 {
                    warn!(
                        "V1-only release rejected non-V1 transaction: {:?}",
                        tx_version
                    );
                    self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                        route: route_label.to_string(),
                        gross_sol,
                        stage: "V1-only policy".to_string(),
                        reason: format!(
                            "Only Solana transaction V1 is permitted; received {tx_version:?}"
                        ),
                        slot: current_slot.0,
                        pools_count,
                        loan_size_sol,
                        net_sol: Some(profit_sol),
                    });
                    return;
                }
                // Authoritative transport selection with explicit capability verification.
                let cap = arb_tx::TransportCapability::mainnet_v1_capable("Mainnet-RPC-Pool");
                let (direct_v1_enabled, jito_v1_enabled) = self.v1_transport_flags();

                if let Err(err) = arb_tx::select_authoritative_transport(
                    tx_bytes,
                    is_swqos_route,
                    direct_v1_enabled,
                    jito_v1_enabled,
                    &cap,
                ) {
                    let reason = format!("Transport capability validation failed: {err}");
                    warn!("{reason}");
                    self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                        route: route_label.to_string(),
                        gross_sol,
                        stage: "Transport Capability Check".to_string(),
                        reason: reason.clone(),
                        slot: current_slot.0,
                        pools_count,
                        loan_size_sol,
                        net_sol: Some(profit_sol),
                    });
                    self.telegram
                        .alert_opportunity_rejected(
                            route_label,
                            gross_sol,
                            "Transport Capability Check",
                            &reason,
                            current_slot.0,
                            pools_count,
                            loan_size_sol,
                            Some(profit_sol),
                        )
                        .await;
                    return;
                }

                // LiteSVM is an optional local accelerator. Its installed
                // transaction parser may lag V1; for V1, rely on the mandatory
                // network RPC simulation below rather than misclassifying an
                // unsupported local parser as an on-chain execution failure.
                if tx_version != arb_tx::TransactionVersion::V1 {
                    let local_sim = self.simulator.simulate_local(tx_bytes.as_slice());
                    if local_sim.success {
                        debug!(
                            "LiteSVM in-memory simulation succeeded (CU: {})",
                            local_sim.compute_units_consumed
                        );
                    } else if let Some(ref err) = local_sim.error
                        && !err.contains("litesvm feature disabled")
                    {
                        warn!(
                            "LiteSVM in-memory simulation rejected ({}) - skipping dispatch",
                            err
                        );
                        return;
                    }
                }

                // Mandatory live RPC preflight. There is no environment switch
                // that can silently turn this guard off.
                let sim_enabled = true;

                // Pre-flight provenance carried into execution records: the sim
                // outcome and wire size are known before any submission, while
                // cu_consumed is replaced by on-chain meta when the tx lands.
                let mut preflight_cu: Option<u64> = None;

                if sim_enabled {
                    // RPC simulation: fail-closed safety check before dispatch
                    let sim_started = std::time::Instant::now();
                    match self.simulator.simulate_async(tx_bytes.as_slice()).await {
                        Ok(mut sim_res) => {
                            sim_res.enforce_quote_freshness(
                                opp.slot.0,
                                self.config.strategy.max_state_age_slots,
                            );
                            let sim_us = sim_started.elapsed().as_micros() as f64;
                            arb_metrics::observe_simulation_latency_us(sim_us);
                            debug!(
                                "Pre-flight simulation latency: {:.0} µs (CU={:?})",
                                sim_us, sim_res.units_consumed
                            );
                            if sim_res.status == arb_sim::SimulationStatus::Passed {
                                self.record_measured_route_cu(
                                    route_sig,
                                    sim_res.units_consumed,
                                    current_slot,
                                );
                                preflight_cu = Some(sim_res.units_consumed);
                            }
                            if sim_res.status != arb_sim::SimulationStatus::Passed {
                                warn!(
                                    "Pre-flight simulation rejected ({:?}: {:?}) - skipping bundle dispatch; logs={:?}",
                                    sim_res.status, sim_res.error, sim_res.logs
                                );
                                let reason_msg = format!(
                                    "محاكاة المعاملة رُفضت ({:?}): {:?}",
                                    sim_res.status, sim_res.error
                                );
                                self.record_rejected_opportunity(
                                    crate::telegram::RejectedTradeSummary {
                                        route: route_label.to_string(),
                                        gross_sol,
                                        stage: "Pre-flight Simulation".to_string(),
                                        reason: reason_msg.clone(),
                                        slot: current_slot.0,
                                        pools_count,
                                        loan_size_sol,
                                        net_sol: Some(profit_sol),
                                    },
                                );
                                self.telegram
                                    .alert_opportunity_rejected(
                                        route_label,
                                        gross_sol,
                                        "Pre-flight Simulation",
                                        &reason_msg,
                                        current_slot.0,
                                        pools_count,
                                        loan_size_sol,
                                        Some(profit_sol),
                                    )
                                    .await;
                                return;
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Primary simulator RPC call failed ({e}); failing over to RpcPool::simulate_transaction..."
                            );
                            let b64 = BASE64_STANDARD.encode(tx_bytes.as_slice());
                            // Pass the wire version so the RPC can deserialize
                            // v0/v1 transactions during pre-flight.
                            let max_version = arb_tx::rpc_max_supported_version_from_wire(tx_bytes)
                                .ok()
                                .flatten();
                            let sim_started = std::time::Instant::now();
                            match self.rpc_pool.simulate_transaction(&b64, max_version).await {
                                Ok(sim_val) => {
                                    arb_metrics::observe_simulation_latency_us(
                                        sim_started.elapsed().as_micros() as f64,
                                    );
                                    let mut fallback = match arb_sim::parse_simulation_response(
                                        &sim_val,
                                    ) {
                                        Ok(result) => result,
                                        Err(parse_error) => {
                                            warn!(
                                                "Fallback simulation response was incomplete ({parse_error}); failing closed"
                                            );
                                            return;
                                        }
                                    };
                                    fallback.enforce_quote_freshness(
                                        opp.slot.0,
                                        self.config.strategy.max_state_age_slots,
                                    );
                                    if fallback.status != arb_sim::SimulationStatus::Passed {
                                        warn!(
                                            "Fallback simulation rejected ({:?}: {:?}); failing closed; logs={:?}",
                                            fallback.status, fallback.error, fallback.logs
                                        );
                                        return;
                                    }
                                    let units_consumed = fallback.units_consumed;
                                    if units_consumed > 0 {
                                        self.record_measured_route_cu(
                                            route_sig,
                                            units_consumed,
                                            current_slot,
                                        );
                                        preflight_cu = Some(units_consumed);
                                    }
                                    info!(
                                        "✅ Fallback RpcPool pre-flight simulation succeeded (CU={units_consumed})"
                                    );
                                }
                                Err(pool_err) => {
                                    warn!(
                                        "Pre-flight simulation RPC call error ({e}; fallback: {pool_err}) - failing closed for live safety"
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }

                // Fire-time gate: bypassed immediately when FAST_PATH_IMMEDIATE_DISPATCH=true.
                let immediate_dispatch = std::env::var("FAST_PATH_IMMEDIATE_DISPATCH")
                    .map(|v| {
                        matches!(
                            v.trim().to_ascii_lowercase().as_str(),
                            "1" | "true" | "yes" | "on"
                        )
                    })
                    .unwrap_or(false);
                let target_slot = Slot(current_slot.0.saturating_add(1));
                let fire_wait = if !immediate_dispatch
                    && !is_swqos_route
                    && self.jito_service.bids_for(target_slot)
                {
                    let clock = self.slot_clock.lock();
                    clock
                        .schedule(target_slot.0, &self.rtt.lock().0, 5)
                        .map(|plan| clock.wait_duration(plan.fire_at_ns))
                } else {
                    None
                };
                if let Some(wait) = fire_wait
                    && wait <= std::time::Duration::from_millis(arb_math::SLOT_MS as u64)
                {
                    tokio::time::sleep(wait).await;
                }

                // Final fire-time safety gate: the kill switch / emergency
                // latch must block in-flight submissions too, not just new
                // captures — a stop issued during simulation must win.
                if self.risk_gate.kill_switch().is_blocked()
                    || self.control().is_some_and(|c| c.is_emergency_stopped())
                {
                    warn!(route = %route_label, "Execution aborted by fire-time kill switch / emergency stop");
                    let _ = self.store.record_outcome(
                        record.id,
                        "aborted_kill_switch",
                        None,
                        NetProvenance::Estimated,
                    );
                    self.telegram
                        .alert_execution_failed(
                            "risk-gate",
                            "kill switch / emergency stop active at fire time; transaction not sent",
                            route_label,
                        )
                        .await;
                    return;
                }

                // Final fire-time state guard: simulation and the optional
                // slot-window wait may have outlived the stream generation.
                // Re-check immediately before any network submission.
                if let Err(reason) = arb_ingest::validate_execution_readiness(
                    self.ingest.stream_connected(),
                    expected_stream_generation,
                    self.ingest.stream_generation(),
                    self.ingest.current_slot(),
                    opp.slot.0,
                    self.config.strategy.max_state_age_slots,
                ) {
                    warn!(?reason, route = %route_label, "Execution rejected by final stream-readiness guard");
                    let reason_msg =
                        format!("Stream/slot integrity changed before submission: {reason:?}");
                    self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                        route: route_label.to_string(),
                        gross_sol,
                        stage: "Final Stream Readiness".to_string(),
                        reason: reason_msg.clone(),
                        slot: current_slot.0,
                        pools_count,
                        loan_size_sol,
                        net_sol: Some(profit_sol),
                    });
                    self.telegram
                        .alert_opportunity_rejected(
                            route_label,
                            gross_sol,
                            "Final Stream Readiness",
                            &reason_msg,
                            current_slot.0,
                            pools_count,
                            loan_size_sol,
                            Some(profit_sol),
                        )
                        .await;
                    return;
                }

                // R12, fail-closed: persist the dispatch intent (signature +
                // transport + slot) BEFORE the send so a crash/ambiguous
                // timeout is reconcilable by signature instead of a blind
                // resubmit. A transaction whose signature cannot be extracted,
                // or whose intent cannot be durably persisted, MUST NOT be
                // dispatched — it would be unrecoverable after a crash.
                let Some(pre_sig) = arb_tx::extract_transaction_signature(tx_bytes) else {
                    error!(
                        attempt_id = record.id,
                        route = %route_label,
                        "dispatch aborted: transaction signature not extractable (unreconcilable)"
                    );
                    let _ = self.store.record_outcome(
                        record.id,
                        "dispatch_aborted_unreconcilable",
                        None,
                        NetProvenance::Estimated,
                    );
                    return;
                };
                let provider = if is_swqos_route { "swqos" } else { "jito" };
                if let Err(err) =
                    self.store
                        .record_dispatch_intent(&arb_store::DispatchIntentRecord {
                            attempt_id: record.id,
                            signature: pre_sig.clone(),
                            provider: provider.to_string(),
                            dispatch_slot: current_slot.0 as i64,
                            last_valid_block_height,
                            resolved_status: None,
                            // The provider id only exists after acceptance;
                            // the crash-recovery trace stays signature-keyed.
                            bundle_id: None,
                        })
                {
                    error!(
                        ?err,
                        attempt_id = record.id,
                        route = %route_label,
                        "dispatch aborted: durable intent persistence failed — refusing to send an unrecoverable transaction"
                    );
                    let _ = self.store.record_outcome(
                        record.id,
                        "dispatch_aborted_unpersisted",
                        None,
                        NetProvenance::Estimated,
                    );
                    self.telegram
                        .alert_execution_failed(
                            "dispatch-intent-persistence",
                            "dispatch intent could not be persisted; transaction not sent",
                            route_label,
                        )
                        .await;
                    return;
                }

                let sent_at = Instant::now();
                let (send_result, was_swqos): (Result<String, String>, bool) = if is_swqos_route {
                    info!(
                        "⚡ Submitting transaction via direct SWQoS RPC path ({} bytes)...",
                        tx_bytes.len()
                    );
                    let b64 = BASE64_STANDARD.encode(tx_bytes.as_slice());
                    let r = self
                        .rpc_pool
                        .send_raw_transaction_v1(&b64)
                        .await
                        .map_err(|e| e.to_string());
                    (r, true)
                } else {
                    info!(
                        "Broadcasting private bundle via Jito Block Engine ({} bytes)...",
                        tx_bytes.len()
                    );
                    let bundle_txs = vec![tx_bytes.to_vec()];
                    let r = self
                        .jito_lander
                        .send_bundle_multi_region(&bundle_txs, &self.config.jito.regions)
                        .await
                        .map_err(|e| e.to_string());
                    (r, false)
                };
                let rtt = sent_at.elapsed();

                match send_result {
                    Ok(bundle_id) => {
                        self.observe_bundle_rtt(rtt);
                        if was_swqos {
                            info!("🚀 Transaction accepted via SWQoS RPC: {}", bundle_id);
                        } else {
                            info!("🚀 Bundle accepted by Jito Block Engine: {}", bundle_id);
                        }
                        // Durable provider acceptance id on the pre-send intent:
                        // reconciliation by signature plus bundle identifier.
                        let _ = self.store.set_dispatch_bundle_id(record.id, &bundle_id);
                        self.telegram
                            .alert_opportunity_with_telemetry(
                                route_label,
                                gross_rev_sol,
                                profit_sol,
                                profit_sol * live_sol_price,
                                current_slot.0,
                                blockhash_b58,
                                &leader_str,
                                cu_limit,
                                cu_price,
                                opp.price_impact_bps.0,
                                &active_bins,
                                pools_count,
                                loan_size_sol,
                            )
                            .await;
                        self.telegram
                            .alert_execution_sent(
                                &bundle_id,
                                route_label,
                                profit_sol,
                                pools_count,
                                loan_size_sol,
                                &leader_str,
                            )
                            .await;
                        // P0 #8: feed velocity tracking so the gate enforces its limit.
                        self.risk_gate.record_order();
                        // Phase E: track bundle sent for landing rate & inflight limits
                        self.risk_gate.record_bundle_sent(current_slot);
                        // A retry inside this window may not outbid
                        // the previous attempt by more than the ladder.
                        self.previous_tips
                            .lock()
                            .insert((current_slot.0, opp.route_id.0), tip);
                        let _ = self.store.record_outcome(
                            record.id,
                            "sent",
                            None,
                            NetProvenance::Estimated,
                        );

                        // Asynchronous Landing Tracker via getBundleStatuses and getSignatureStatuses (Layer 08)
                        let tx_signature = arb_tx::extract_transaction_signature(tx_bytes);
                        let signature_rpc = self.rpc_pool.clone();
                        let was_swqos_task = was_swqos;
                        let store = self.store.clone();
                        let risk_gate = self.risk_gate.clone();
                        let telegram = self.telegram.clone();
                        let js = self.jito_service.clone();
                        let rung_for_record = tip_rung;
                        let rec_id = record.id;
                        let b_id = bundle_id.clone();
                        let net_profit_val = opp.net_profit;
                        let profit_sol_val = profit_sol;
                        let sol_price_val = live_sol_price;
                        let slot_val = current_slot.0;
                        let route_label_task = route_label.to_string();
                        let loan_size_sol_val = loan_size_sol;
                        let pools_count_task = pools_count;
                        let leader_task = leader_str.clone();
                        let tokens_str_val = {
                            let mut parts = Vec::new();
                            for (i, h) in opp.hops.iter().enumerate() {
                                if i == 0 {
                                    parts.push(format_mint_short(&h.mint_in.0));
                                }
                                parts.push(format_mint_short(&h.mint_out.0));
                            }
                            parts.join(" ➔ ")
                        };
                        let venues_str_val = opp
                            .hops
                            .iter()
                            .map(|h| format!("{:?}", h.protocol))
                            .collect::<Vec<_>>()
                            .join(" ➔ ");
                        let rtt_ms = rtt.as_secs_f64() * 1000.0;
                        arb_metrics::observe_submit_rtt_ms(rtt_ms);
                        let land_start = std::time::Instant::now();
                        let recent_landed_task = self.recent_landed.clone();
                        let recent_landed_cu_task = self.recent_landed_cu.clone();
                        let encoded_cu_price_task = cu_price;
                        let recent_dropped_task = self.recent_dropped.clone();
                        let control_task = self.control().cloned();
                        let payer_pubkey_task = payer.to_string();
                        let tx_size_val = tx_bytes.len() as u64;
                        let preflight_cu_task = preflight_cu;
                        let mut actual_realized_net: Option<SignedLamports> = None;
                        let last_valid_height_task =
                            last_valid_block_height.unwrap_or(0).max(0) as u64;

                        // F-01 settlement handles (cloned before the spawn so the
                        // reconciliation task never captures `self`).
                        let settle_opp_task = opp.clone();
                        let settle_base_mint_task =
                            opp.hops.first().map(|hop| hop.mint_in.0.to_string());
                        let settle_rate_num_task = lamports_per_base_raw;
                        let settle_rate_denom_task = lamports_per_base_denom;
                        let settle_payer_task = payer;
                        let settle_rpc_task = self.rpc_pool.clone();
                        let settle_tx_rpc_task = self.tx_rpc.clone();
                        let settle_signer_task = self.keypair().cloned();
                        let settle_live_task = self.mode() == BotMode::Live;

                        tokio::spawn(async move {
                            // Dual-track landing verification:
                            // SWQoS queries getSignatureStatuses directly.
                            // Jito Bundle queries Jito getBundleStatuses AND cross-verifies with getSignatureStatuses.
                            let (status, landed_slot, _err_reason, lifecycle) = if was_swqos_task {
                                let (sig_status, slot, err) = signature_rpc
                                    .poll_signature_with_details(
                                        &b_id,
                                        20,
                                        std::time::Duration::from_millis(400),
                                    )
                                    .await
                                    .unwrap_or((arb_ingest::SignatureStatus::Pending, None, None));

                                let current_height =
                                    signature_rpc.get_block_height().await.unwrap_or(0);
                                let (lifecycle, reason) = match sig_status {
                                    arb_ingest::SignatureStatus::Finalized => {
                                        (arb_land::TransactionLifecycle::Finalized, None)
                                    }
                                    arb_ingest::SignatureStatus::Confirmed => {
                                        (arb_land::TransactionLifecycle::Confirmed, None)
                                    }
                                    arb_ingest::SignatureStatus::Processed => {
                                        (arb_land::TransactionLifecycle::Processed, None)
                                    }
                                    arb_ingest::SignatureStatus::Failed => {
                                        (arb_land::TransactionLifecycle::Failed, err)
                                    }
                                    arb_ingest::SignatureStatus::Pending => {
                                        arb_land::evaluate_lifecycle_with_block_height(
                                            None,
                                            false,
                                            None,
                                            current_height,
                                            last_valid_height_task,
                                        )
                                    }
                                };
                                let st = lifecycle_to_land_status(lifecycle);
                                (st, slot, reason, lifecycle)
                            } else {
                                let (lifecycle, slot, err) = js
                                    .verify_landing_with_height(
                                        &signature_rpc,
                                        &b_id,
                                        tx_signature.as_deref(),
                                        20,
                                        std::time::Duration::from_millis(350),
                                        last_valid_height_task,
                                    )
                                    .await;
                                let st = lifecycle_to_land_status(lifecycle);
                                (st, slot, err, lifecycle)
                            };

                            let effective_slot = landed_slot.unwrap_or(slot_val);
                            match status {
                                arb_land::LandStatus::Landed => {
                                    // Feed the landed CU price (the number actually
                                    // encoded on a confirmed bundle) into the bounded
                                    // window behind `adaptive_cu_price` — the only
                                    // confirmed-price source this ever consumes.
                                    arb_land::record_landed_cu_price(
                                        &mut recent_landed_cu_task.lock(),
                                        encoded_cu_price_task,
                                    );
                                    let mut onchain_cu: Option<u64> = None;
                                    if let Some(ref sig) = tx_signature {
                                        if let Ok(Some(meta)) =
                                            signature_rpc.get_transaction_with_metadata(sig).await
                                        {
                                            if let Some(cu) = meta
                                                .get("meta")
                                                .and_then(|m| m.get("computeUnitsConsumed"))
                                                .and_then(|c| c.as_u64())
                                            {
                                                onchain_cu = Some(cu);
                                                debug!(
                                                    "On-chain confirmed compute units consumed: {cu}"
                                                );
                                            }
                                            // Actual balance changes from on-chain metadata:
                                            // realized net recorded to the store prefers this
                                            // evidence over the pre-dispatch estimate. Signed:
                                            // losses must persist as losses (MATH-01).
                                            if let Some(base_mint) =
                                                settle_base_mint_task.as_deref()
                                            {
                                                let changes =
                                                    arb_sim::parse_landed_asset_balance_changes(
                                                        &meta,
                                                        &payer_pubkey_task,
                                                        base_mint,
                                                    );
                                                // For non-WSOL anchors, require both the tracked
                                                // token delta and native fee-payer delta. WSOL may
                                                // also settle as unwrapped native lamports.
                                                let token_delta =
                                                    changes.token_mint_delta.or_else(|| {
                                                        (base_mint == arb_constants::WSOL_MINT
                                                            && changes.fee_payer_balance_verified)
                                                            .then_some(0)
                                                    });
                                                let actual = if changes.fee_payer_balance_verified {
                                                    token_delta.and_then(|delta| {
                                                        arb_math::economic::signed_base_raw_to_lamports(
                                                            delta,
                                                            settle_rate_num_task,
                                                            settle_rate_denom_task,
                                                        )
                                                        .ok()
                                                        .and_then(|token_lamports| {
                                                            token_lamports.checked_add(
                                                                changes.fee_payer_lamport_delta,
                                                            )
                                                        })
                                                    })
                                                } else {
                                                    None
                                                };
                                                if let Some(actual) = actual {
                                                    info!(
                                                        "On-chain settlement evidence: base_mint={} token_delta_raw={:?} payer_lamport_delta={} realized_net_lamports={} (estimated {})",
                                                        base_mint,
                                                        changes.token_mint_delta,
                                                        changes.fee_payer_lamport_delta,
                                                        actual,
                                                        net_profit_val.as_u64()
                                                    );
                                                    match i64::try_from(actual) {
                                                        Ok(signed) => {
                                                            actual_realized_net =
                                                                Some(SignedLamports::new(signed));
                                                            if signed < 0 {
                                                                warn!(
                                                                    "Landed transaction realized a NET LOSS of {signed} lamports (estimated +{}); signed loss persisted",
                                                                    net_profit_val.as_u64()
                                                                );
                                                            }
                                                        }
                                                        Err(_) => {
                                                            warn!(
                                                                "Landed realized net {actual} exceeds i64 storage range; retaining estimate provenance"
                                                            );
                                                        }
                                                    }
                                                } else {
                                                    warn!(
                                                        "On-chain PnL evidence incomplete for base mint {}; retaining estimated provenance",
                                                        base_mint
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    // Reported realized figure prefers on-chain actual
                                    // evidence; the estimate is only used when no
                                    // balance metadata exists, and is then tagged
                                    // `estimated` so aggregation can never merge
                                    // it into realized PnL (MATH-01/PIPE-01/03).
                                    let realized_reported_sol = actual_realized_net
                                        .map(|n| match n {
                                            SignedLamports(v) if v >= 0 => v as f64 / 1e9,
                                            SignedLamports(v) => -(v.unsigned_abs() as f64) / 1e9,
                                        })
                                        .unwrap_or(profit_sol_val);
                                    let landing_ms = land_start.elapsed().as_secs_f64() * 1000.0;
                                    arb_metrics::observe_landing_latency_ms(landing_ms);
                                    let total_e2e_ms = (t_ingest_to_quote_us
                                        + t_risk_us
                                        + t_assembly_us
                                        + t_signing_us)
                                        as f64
                                        / 1000.0
                                        + rtt_ms
                                        + landing_ms;
                                    arb_metrics::observe_e2e_latency_ms(total_e2e_ms);

                                    let timings_str = format!(
                                        "• Ingest ➔ Quote: <code>{} µs</code>\n\
                                             • Risk & EV Gates: <code>{} µs</code>\n\
                                             • Tx Assembly & ALTs: <code>{} µs</code>\n\
                                             • Ed25519 Signing: <code>{} µs</code>\n\
                                             • Broadcast RTT: <code>{:.2} ms</code>\n\
                                             • Block Landing: <code>{:.2} ms</code>\n\
                                             • <b>Total E2E Latency:</b> <code>{:.2} ms</code>",
                                        t_ingest_to_quote_us,
                                        t_risk_us,
                                        t_assembly_us,
                                        t_signing_us,
                                        rtt_ms,
                                        landing_ms,
                                        total_e2e_ms
                                    );

                                    // Use actual realized net if available, otherwise fall back to expected
                                    let realized_sol_val =
                                        if let Some(realized) = actual_realized_net {
                                            realized.as_i64() as f64 / 1e9
                                        } else {
                                            profit_sol_val
                                        };
                                    let realized_usd_val = realized_sol_val * live_sol_price;
                                    let profit_sign =
                                        if realized_sol_val >= 0.0 { "+" } else { "" };

                                    info!(
                                        "🎯 Execution finalized/landed: {} (slot {})\n• Net Profit: {}{:.6} SOL (~${:.2})\n• Loan: {:.4} SOL\n• Tokens: {}\n• Venues: {}\n{}",
                                        b_id,
                                        effective_slot,
                                        profit_sign,
                                        realized_sol_val,
                                        realized_usd_val,
                                        loan_size_sol_val,
                                        tokens_str_val,
                                        venues_str_val,
                                        timings_str
                                            .replace("<code>", "")
                                            .replace("</code>", "")
                                            .replace("<b>", "")
                                            .replace("</b>", "")
                                    );
                                    let (realized_lamports, realized_provenance) =
                                        match actual_realized_net {
                                            Some(v) => (Some(v), arb_store::NetProvenance::Actual),
                                            None => (
                                                i64::try_from(net_profit_val.as_u64())
                                                    .ok()
                                                    .map(SignedLamports::new),
                                                arb_store::NetProvenance::Estimated,
                                            ),
                                        };
                                    let _ = store.record_outcome(
                                        rec_id,
                                        "landed",
                                        realized_lamports,
                                        realized_provenance,
                                    );
                                    let _ = store.resolve_dispatch_intent(rec_id, "landed");
                                    // F-01 post-landing settlement: reclaim rent
                                    // from intermediate ATAs whose balances ended
                                    // this route at zero. Separate live-only tx,
                                    // probe-gated — never part of the arb tx.
                                    if let Some(ref signer) = settle_signer_task {
                                        let reclaimed =
                                            Supervisor::settle_idle_intermediate_atas_with(
                                                settle_rpc_task.clone(),
                                                &settle_tx_rpc_task,
                                                signer,
                                                settle_live_task,
                                                &settle_opp_task,
                                                settle_payer_task,
                                            )
                                            .await;
                                        if reclaimed > 0 {
                                            debug!(
                                                "intermediate ATA settlement: {} accounts reclaimed",
                                                reclaimed
                                            );
                                        }
                                    }
                                    if let Some(r) = rung_for_record {
                                        js.record(
                                            r,
                                            InflightStatus::Landed {
                                                slot: effective_slot,
                                            },
                                        );
                                    }
                                    risk_gate.on_success();
                                    // Phase E: track bundle landed for landing rate
                                    risk_gate.record_bundle_landed(Slot(effective_slot), true);
                                    arb_metrics::inc_fills();
                                    // Keep evidence-backed PnL out of the estimate
                                    // series. A landed transaction without balance
                                    // metadata is still an estimate, not realized PnL.
                                    if actual_realized_net.is_some() {
                                        arb_metrics::add_net_profit(realized_reported_sol);
                                    } else {
                                        arb_metrics::add_estimated_landed_net_profit(
                                            realized_reported_sol,
                                        );
                                    }
                                    arb_metrics::set_breaker_state(0);

                                    let effective_leader = js
                                        .leader_for(Slot(effective_slot))
                                        .map(|pk| pk.to_string())
                                        .unwrap_or_else(|| leader_task.clone());
                                    // PIPE-12: report the commitment level the
                                    // verification track actually proved, never a
                                    // hardcoded Confirmed/Finalized claim.
                                    let landed_status_str = lifecycle.landing_status_str();

                                    // Record landed trade in memory for 5-minute performance digest
                                    // (realized figure — actual when on-chain evidence exists).
                                    let summary = crate::telegram::TradeSummary {
                                        route: route_label_task.clone(),
                                        profit_sol: realized_reported_sol,
                                        profit_usd: realized_reported_sol * sol_price_val,
                                        loan_sol: loan_size_sol_val,
                                        signature: tx_signature
                                            .as_deref()
                                            .unwrap_or(&b_id)
                                            .to_string(),
                                        venues: venues_str_val.clone(),
                                        slot: effective_slot,
                                        pools_count: pools_count_task,
                                        leader: effective_leader.clone(),
                                        sim_ok: Some(true),
                                        tx_size_bytes: Some(tx_size_val),
                                        cu_consumed: onchain_cu.or(preflight_cu_task),
                                    };
                                    Supervisor::record_landed_trade_task(
                                        &recent_landed_task,
                                        control_task.clone(),
                                        summary,
                                    );

                                    telegram
                                        .notify_trade_executed(
                                            tx_signature.as_deref(),
                                            &b_id,
                                            realized_reported_sol,
                                            realized_reported_sol * sol_price_val,
                                            loan_size_sol_val,
                                            pools_count_task,
                                            &tokens_str_val,
                                            &venues_str_val,
                                            effective_slot,
                                            &effective_leader,
                                            landed_status_str,
                                            &timings_str,
                                        )
                                        .await;
                                }
                                arb_land::LandStatus::Dropped | arb_land::LandStatus::Expired => {
                                    warn!("⚠️ Execution {:?} in block: {}", status, b_id);
                                    let _ = store.record_outcome(
                                        rec_id,
                                        "dropped",
                                        None,
                                        NetProvenance::Estimated,
                                    );
                                    let _ = store.resolve_dispatch_intent(
                                        rec_id,
                                        match status {
                                            arb_land::LandStatus::Dropped => "dropped",
                                            _ => "expired",
                                        },
                                    );
                                    if let Some(r) = rung_for_record {
                                        js.record(r, InflightStatus::Failed);
                                    }
                                    // Unlanded/expired Jito bundles pay 0 SOL; do not count phantom tip loss or trip breaker
                                    risk_gate.record_bundle_landed(Slot(effective_slot), false);

                                    let dropped = crate::telegram::DroppedTradeSummary {
                                        bundle_id: b_id.clone(),
                                        route: route_label_task.clone(),
                                        reason: format!("{status:?}"),
                                        slot: effective_slot,
                                        sim_ok: Some(true),
                                        tx_size_bytes: Some(tx_size_val),
                                        cu_consumed: preflight_cu_task,
                                    };
                                    Supervisor::record_dropped_bundle_task(
                                        &recent_dropped_task,
                                        control_task.clone(),
                                        dropped,
                                    );

                                    telegram
                                        .alert_execution_dropped(
                                            &b_id,
                                            &route_label_task,
                                            &format!("{status:?}"),
                                        )
                                        .await;
                                }
                                arb_land::LandStatus::Failed => {
                                    warn!("⚠️ Execution reverted on-chain: {}", b_id);
                                    if let Some(r) = rung_for_record {
                                        js.record(r, InflightStatus::Failed);
                                    }
                                    risk_gate.record_bundle_landed(Slot(effective_slot), false);
                                    // A landed-but-reverted transaction pays the
                                    // 5,000-lamport base fee AND the priority fee
                                    // (deducted pre-execution); its tip transfer is
                                    // rolled back with the rest of the tx. Read the
                                    // real on-chain fee from transaction metadata;
                                    // the flat 5,000-lamport figure is only the
                                    // no-evidence fallback (PIPE-06).
                                    let mut failure_fee_lamports = 5_000u64;
                                    let mut actual_failure_fee = None;
                                    if let Some(ref sig) = tx_signature {
                                        if let Ok(Some(meta)) =
                                            signature_rpc.get_transaction_with_metadata(sig).await
                                        {
                                            if let Some(fee) = arb_sim::meta_fee_lamports(&meta) {
                                                failure_fee_lamports = fee;
                                                actual_failure_fee = Some(fee);
                                                debug!(
                                                    "Failed-on-chain tx paid {fee} lamports (base + priority)"
                                                );
                                            }
                                        }
                                    }
                                    let realized_failure = actual_failure_fee
                                        .and_then(|fee| i64::try_from(fee).ok())
                                        .and_then(i64::checked_neg)
                                        .map(SignedLamports::new);
                                    let _ = store.record_outcome(
                                        rec_id,
                                        "failed",
                                        realized_failure,
                                        if realized_failure.is_some() {
                                            NetProvenance::Actual
                                        } else {
                                            NetProvenance::Estimated
                                        },
                                    );
                                    let _ = store.resolve_dispatch_intent(rec_id, "failed_onchain");
                                    risk_gate.record_loss(
                                        failure_fee_lamports as f64 / 1e9 * sol_price_val,
                                    );
                                    risk_gate.on_failure(Slot(effective_slot));
                                    arb_metrics::set_breaker_state(
                                        if risk_gate.is_breaker_open(Slot(effective_slot)) {
                                            1
                                        } else {
                                            0
                                        },
                                    );

                                    let dropped = crate::telegram::DroppedTradeSummary {
                                        bundle_id: b_id.clone(),
                                        route: route_label_task.clone(),
                                        reason: "Failed".to_string(),
                                        slot: effective_slot,
                                        sim_ok: Some(true),
                                        tx_size_bytes: Some(tx_size_val),
                                        cu_consumed: preflight_cu_task,
                                    };
                                    Supervisor::record_dropped_bundle_task(
                                        &recent_dropped_task,
                                        control_task.clone(),
                                        dropped,
                                    );

                                    telegram
                                        .alert_execution_dropped(&b_id, &route_label_task, "Failed")
                                        .await;
                                }
                                _ => {
                                    debug!(
                                        "Execution {} terminal check finished with status {:?}",
                                        b_id, status
                                    );
                                }
                            }
                        });
                    }
                    Err(e) => {
                        // PIPE-05: a transport error is AMBIGUOUS, never a
                        // final failure (same doctrine as recovery.rs): the
                        // bytes may already have propagated and landed. The
                        // durable dispatch intent (persisted pre-send) is NOT
                        // resolved here; a spawned evidence check polls the
                        // signature and resolves it from proof, and anything
                        // still unknown is reconciled with block-height
                        // evidence at the next startup. Never a guessed
                        // "send_failed" terminal resolution.
                        warn!(
                            error = %e,
                            attempt_id = record.id,
                            route = %route_label,
                            "send errored — outcome ambiguous; recording honestly and checking signature evidence before any terminal resolution"
                        );
                        let _ = self.store.record_outcome(
                            record.id,
                            "send_ambiguous",
                            None,
                            NetProvenance::Estimated,
                        );
                        self.telegram
                            .alert_execution_failed(
                                "dispatch",
                                &format!("send errored (ambiguous — evidence check running): {e}"),
                                route_label,
                            )
                            .await;
                        if let Some(control) = self.control() {
                            control.record_execution_with(
                                "send_ambiguous",
                                route_label,
                                &e.to_string(),
                                crate::control_service::ExecutionExtras {
                                    sim_ok: Some(true),
                                    tx_size_bytes: Some(tx_bytes.len() as u64),
                                    cu_consumed: preflight_cu,
                                    ..Default::default()
                                },
                            );
                            control.record_execution_result(false);
                        }
                        let signature_rpc = self.rpc_pool.clone();
                        let store = self.store.clone();
                        let control_task = self.control().cloned();
                        let pre_sig_task = pre_sig.clone();
                        let attempt_id = record.id;
                        let route_for_task = route_label.to_string();
                        let err_for_task = e.clone();
                        tokio::spawn(async move {
                            let poll = signature_rpc
                                .poll_signature_until_terminal(
                                    &pre_sig_task,
                                    super::recovery::SEND_ERROR_POLL_ATTEMPTS,
                                    super::recovery::SEND_ERROR_POLL_INTERVAL,
                                )
                                .await
                                .ok();
                            match super::recovery::classify_send_error(poll) {
                                Some(resolution) => {
                                    warn!(
                                        attempt_id,
                                        resolution,
                                        send_error = %err_for_task,
                                        "send errored but on-chain evidence resolved the dispatch outcome"
                                    );
                                    let (outcome, realized, provenance) = match resolution {
                                        "landed" => {
                                            let estimate = store
                                                .get_attempt(attempt_id)
                                                .ok()
                                                .flatten()
                                                .and_then(|attempt| {
                                                    i64::try_from(attempt.expected_net.as_u64())
                                                        .ok()
                                                })
                                                .map(SignedLamports::new);
                                            ("landed", estimate, NetProvenance::Estimated)
                                        }
                                        "failed_onchain" => {
                                            let fee = signature_rpc
                                                .get_transaction_with_metadata(&pre_sig_task)
                                                .await
                                                .ok()
                                                .flatten()
                                                .and_then(|meta| arb_sim::meta_fee_lamports(&meta));
                                            let realized = fee
                                                .and_then(|value| i64::try_from(value).ok())
                                                .and_then(i64::checked_neg)
                                                .map(SignedLamports::new);
                                            let provenance = if realized.is_some() {
                                                NetProvenance::Actual
                                            } else {
                                                NetProvenance::Estimated
                                            };
                                            ("failed", realized, provenance)
                                        }
                                        other => (other, None, NetProvenance::Estimated),
                                    };
                                    if let Err(store_error) = store
                                        .record_outcome(attempt_id, outcome, realized, provenance)
                                    {
                                        warn!(
                                            ?store_error,
                                            attempt_id,
                                            "terminal outcome persistence failed; dispatch intent remains pending"
                                        );
                                        return;
                                    }
                                    let _ = store.resolve_dispatch_intent(attempt_id, resolution);
                                    if let Some(control) = control_task {
                                        control.record_execution_with(
                                            resolution,
                                            &route_for_task,
                                            &format!(
                                                "send errored, resolved by on-chain evidence: {resolution}"
                                            ),
                                            crate::control_service::ExecutionExtras {
                                                sim_ok: Some(true),
                                                tx_size_bytes: None,
                                                cu_consumed: preflight_cu,
                                                ..Default::default()
                                            },
                                        );
                                    }
                                }
                                None => {
                                    // Still unknown: NOT terminal. The intent
                                    // stays pending; the startup reconciler
                                    // proves landing/expiration with
                                    // block-height evidence. No resubmit — the
                                    // same signed tx must never be re-signed.
                                    warn!(
                                        attempt_id,
                                        "send outcome unresolvable after bounded poll — dispatch intent left pending for startup reconciliation (no resubmit)"
                                    );
                                }
                            }
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod required_live_balance_tests {
    use super::required_live_payer_balance;

    #[test]
    fn preserves_reserve_and_adds_refundable_rent_and_transaction_costs() {
        assert_eq!(
            required_live_payer_balance(1_000, 100, 5, 20, 50),
            Some(1_175)
        );
    }

    #[test]
    fn processed_is_not_a_confirmed_landing_for_settlement() {
        assert_eq!(
            super::lifecycle_to_land_status(arb_land::TransactionLifecycle::Processed),
            arb_land::LandStatus::Sent
        );
        assert_eq!(
            super::lifecycle_to_land_status(arb_land::TransactionLifecycle::Confirmed),
            arb_land::LandStatus::Landed
        );
        assert_eq!(
            super::lifecycle_to_land_status(arb_land::TransactionLifecycle::Finalized),
            arb_land::LandStatus::Landed
        );
    }

    #[test]
    fn balance_requirement_overflow_fails_closed() {
        assert_eq!(required_live_payer_balance(u64::MAX, 1, 0, 0, 0), None);
    }
}
