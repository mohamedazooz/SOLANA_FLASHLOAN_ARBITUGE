//! Mode-specific execution, preflight simulation, dispatch and settlement tracking.
use super::*;

pub(super) struct DispatchContext<'a> {
    pub opp: &'a ExecutionOpportunity,
    pub tip: Lamports,
    pub tip_rung: Option<arb_types::Percentile>,
    pub is_swqos_route: bool,
    pub current_slot: Slot,
    pub expected_stream_generation: u64,
    pub route_label: &'a str,
    pub gross_rev_sol: f64,
    pub gross_sol: f64,
    pub profit_sol: f64,
    pub live_sol_price: f64,
    pub blockhash_b58: &'a str,
    pub cu_limit: u32,
    pub cu_price: u64,
    pub tx_bytes: &'a Vec<u8>,
    pub record: &'a AttemptRecord,
    pub real_net_profit: Lamports,
    pub payer: Pubkey,
    pub route_sig: &'a str,
    pub t_ingest_to_quote_us: u64,
    pub t_risk_us: u64,
    pub t_assembly_us: u64,
    pub t_signing_us: u64,
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

impl Supervisor {
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
            cu_limit,
            cu_price,
            tx_bytes,
            record,
            real_net_profit,
            payer,
            route_sig,
            t_ingest_to_quote_us,
            t_risk_us,
            t_assembly_us,
            t_signing_us,
        } = context;

        #[cfg(feature = "shadow-only")]
        let _ = (
            tip,
            tip_rung,
            is_swqos_route,
            expected_stream_generation,
            payer,
            route_sig,
            t_ingest_to_quote_us,
            t_risk_us,
            t_assembly_us,
            t_signing_us,
        );

        // Compile-time shadow-only containment. This guard is intentionally
        // inside the dispatch boundary, not merely a CLI/config convention.
        #[cfg(feature = "shadow-only")]
        if self.mode() == BotMode::Live {
            error!("shadow-only build rejected Live dispatch before any transport call");
            let _ = self
                .store
                .record_outcome(record.id, "blocked_shadow_only", None);
            return;
        }

        // 6. Dispatch based on mode
        let leader_str = self
            .jito_service
            .leader_for(current_slot)
            .map(|pk| pk.to_string())
            .unwrap_or_else(|| "Unknown (Non-Jito / Stale Schedule)".to_string());

        match self.mode() {
            BotMode::Canary => {
                info!(
                    "canary: simulated valid execution of {} bytes (not broadcast to Jito)",
                    tx_bytes.len()
                );
                // Canary runs never broadcast, so they must never be
                // counted as fills or recorded as realized PnL. Record
                // the simulated edge under the simulated metric and a
                // distinct "canary" outcome for auditability.
                arb_metrics::add_simulated_net_profit(profit_sol);
                let _ = self
                    .store
                    .record_outcome(record.id, "canary", Some(real_net_profit));
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
                    )
                    .await;
            }
            BotMode::Shadow | BotMode::Simulate => {
                arb_metrics::add_simulated_net_profit(profit_sol);
                let _ = self
                    .store
                    .record_outcome(record.id, "simulated", Some(real_net_profit));
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
                        )
                        .await;
                }
            }
            #[cfg(feature = "shadow-only")]
            BotMode::Live => {
                error!("shadow-only binary has no Live dispatch implementation");
                let _ = self
                    .store
                    .record_outcome(record.id, "blocked_shadow_only", None);
            }
            #[cfg(not(feature = "shadow-only"))]
            BotMode::Live => {
                // Safety Floor Check: Enforce minimum reserve only if configured > 0.
                // When MIN_SOL_RESERVE_LAMPORTS=0 (fast-path flash loan default), skip the blocking RPC getBalance check completely.
                let min_reserve_lamports = std::env::var("MIN_SOL_RESERVE_LAMPORTS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(self.config.wallet.min_sol_balance_lamports);
                if min_reserve_lamports > 0
                    && let Ok(current_balance) = self.rpc_pool.get_balance(&payer).await
                    && current_balance < min_reserve_lamports
                {
                    warn!(
                        "🚨 Safety Floor Active: Wallet balance ({} lamports / {:.4} SOL) is below {:.4} SOL minimum reserve — skipping execution to protect fees",
                        current_balance,
                        current_balance as f64 / 1e9,
                        min_reserve_lamports as f64 / 1e9
                    );
                    self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                        route: route_label.to_string(),
                        gross_sol,
                        stage: "Safety Floor Reserve".to_string(),
                        reason: format!(
                            "رصيد المحفظة ({:.4} SOL) أقل من صمام الأمان الأدنى ({:.4} SOL)",
                            current_balance as f64 / 1e9,
                            min_reserve_lamports as f64 / 1e9
                        ),
                        slot: current_slot.0,
                    });
                    self.telegram
                        .alert_opportunity_rejected(
                            route_label,
                            gross_sol,
                            "Safety Floor Reserve",
                            &format!(
                                "رصيد المحفظة ({:.4} SOL) أقل من صمام الأمان الأدنى ({:.4} SOL)",
                                current_balance as f64 / 1e9,
                                min_reserve_lamports as f64 / 1e9
                            ),
                            current_slot.0,
                        )
                        .await;
                    return;
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
                    });
                    return;
                }
                // No Jito bundle fallback in the V1-only profile. V1 is
                // simulated over RPC; any future live transport requires a
                // separately verified V1-capable direct sender.
                let v1_transport_enabled = is_swqos_route
                    && std::env::var("SOLANA_TX_V1_DIRECT_ENABLED")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false);
                let max_wire_size = arb_tx::SOLANA_PACKET_V1_MTU;
                // Shadow/simulate mode only needs RPC simulation. Live direct
                // transport remains disabled in this package and must not fall
                // back to Jito, V0, or legacy when its capability flag is absent.
                if self.mode() == BotMode::Live && !v1_transport_enabled {
                    warn!("V1 direct transport not enabled; refusing dispatch");
                    return;
                }
                if tx_bytes.len() > max_wire_size {
                    let reason = format!(
                        "transaction {:?} size {} exceeds format limit {} bytes",
                        tx_version,
                        tx_bytes.len(),
                        max_wire_size
                    );
                    warn!("{reason}");
                    self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                        route: route_label.to_string(),
                        gross_sol,
                        stage: "Transaction Size Check".to_string(),
                        reason: reason.clone(),
                        slot: current_slot.0,
                    });
                    self.telegram
                        .alert_opportunity_rejected(
                            route_label,
                            gross_sol,
                            "Transaction Size Check",
                            &reason,
                            current_slot.0,
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

                if sim_enabled {
                    // RPC simulation: fail-closed safety check before dispatch
                    match self.simulator.simulate_async(tx_bytes.as_slice()).await {
                        Ok(sim_res) => {
                            if sim_res.success && sim_res.units_consumed > 0 {
                                self.measured_cu.lock().insert(
                                    route_sig.to_owned(),
                                    u32::try_from(sim_res.units_consumed).unwrap_or(u32::MAX),
                                );
                            }
                            if !sim_res.success {
                                warn!(
                                    "Pre-flight simulation failed ({:?}) - skipping bundle dispatch",
                                    sim_res.error
                                );
                                let reason_msg =
                                    format!("محاكاة المعاملة فشلت على الشبكة: {:?}", sim_res.error);
                                self.record_rejected_opportunity(
                                    crate::telegram::RejectedTradeSummary {
                                        route: route_label.to_string(),
                                        gross_sol,
                                        stage: "Pre-flight Simulation".to_string(),
                                        reason: reason_msg.clone(),
                                        slot: current_slot.0,
                                    },
                                );
                                self.telegram
                                    .alert_opportunity_rejected(
                                        route_label,
                                        gross_sol,
                                        "Pre-flight Simulation",
                                        &reason_msg,
                                        current_slot.0,
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
                            match self.rpc_pool.simulate_transaction(&b64, max_version).await {
                                Ok(sim_val) => {
                                    let sim_err = sim_val.get("err").and_then(|ev| {
                                        if ev.is_null() {
                                            None
                                        } else {
                                            Some(ev.to_string())
                                        }
                                    });
                                    let units_consumed = sim_val
                                        .get("unitsConsumed")
                                        .and_then(|u| u.as_u64())
                                        .unwrap_or(0);
                                    if sim_err.is_none() && units_consumed > 0 {
                                        self.measured_cu.lock().insert(
                                            route_sig.to_owned(),
                                            u32::try_from(units_consumed).unwrap_or(u32::MAX),
                                        );
                                    }
                                    if let Some(err_str) = sim_err {
                                        warn!(
                                            "Pre-flight fallback simulation failed ({err_str}) - skipping bundle dispatch"
                                        );
                                        let reason_msg = format!(
                                            "محاكاة المعاملة فشلت على الشبكة (Fallback RPC): {err_str}"
                                        );
                                        self.record_rejected_opportunity(
                                            crate::telegram::RejectedTradeSummary {
                                                route: route_label.to_string(),
                                                gross_sol,
                                                stage: "Pre-flight Simulation".to_string(),
                                                reason: reason_msg.clone(),
                                                slot: current_slot.0,
                                            },
                                        );
                                        self.telegram
                                            .alert_opportunity_rejected(
                                                route_label,
                                                gross_sol,
                                                "Pre-flight Simulation",
                                                &reason_msg,
                                                current_slot.0,
                                            )
                                            .await;
                                        return;
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
                    });
                    self.telegram
                        .alert_opportunity_rejected(
                            route_label,
                            gross_sol,
                            "Final Stream Readiness",
                            &reason_msg,
                            current_slot.0,
                        )
                        .await;
                    return;
                }

                // R12: persist the dispatch intent (signature + transport +
                // slot) BEFORE the send so a crash/ambiguous timeout is
                // reconcilable by signature instead of a blind resubmit.
                let pre_sig = arb_tx::extract_transaction_signature(tx_bytes);
                if let Some(sig) = pre_sig.as_deref() {
                    let provider = if is_swqos_route { "swqos" } else { "jito" };
                    if let Err(err) =
                        self.store
                            .record_dispatch_intent(&arb_store::DispatchIntentRecord {
                                attempt_id: record.id,
                                signature: sig.to_string(),
                                provider: provider.to_string(),
                                dispatch_slot: current_slot.0 as i64,
                                resolved_status: None,
                            })
                    {
                        warn!(
                            ?err,
                            attempt_id = record.id,
                            "failed to persist dispatch intent"
                        );
                    }
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
                        .send_raw_transaction(&b64)
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
                            )
                            .await;
                        self.telegram
                            .alert_execution_sent(&bundle_id, route_label, profit_sol)
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
                        let _ = self.store.record_outcome(record.id, "sent", None);

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
                        let profit_usd_val = profit_sol * live_sol_price;
                        let sol_price_val = live_sol_price;
                        let slot_val = current_slot.0;
                        let route_label_task = route_label.to_string();
                        let loan_size_sol_val = opp.size_in.as_u64() as f64 / 1e9;
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
                        let land_start = std::time::Instant::now();
                        let recent_landed_task = self.recent_landed.clone();
                        let recent_dropped_task = self.recent_dropped.clone();

                        tokio::spawn(async move {
                            // Dual-track landing verification:
                            // SWQoS queries getSignatureStatuses directly.
                            // Jito Bundle queries Jito getBundleStatuses AND cross-verifies with getSignatureStatuses.
                            let (status, landed_slot) = if was_swqos_task {
                                let sig_status = signature_rpc
                                    .poll_signature_until_terminal(
                                        &b_id,
                                        20,
                                        std::time::Duration::from_millis(400),
                                    )
                                    .await;
                                let st = match sig_status {
                                    Ok(s) if s.is_landed() => arb_land::LandStatus::Landed,
                                    Ok(arb_ingest::SignatureStatus::Failed) => {
                                        arb_land::LandStatus::Failed
                                    }
                                    _ => arb_land::LandStatus::Expired,
                                };
                                (st, None)
                            } else {
                                let (st, slot, _err) = js
                                    .verify_landing(
                                        &signature_rpc,
                                        &b_id,
                                        tx_signature.as_deref(),
                                        20,
                                        std::time::Duration::from_millis(350),
                                    )
                                    .await;
                                (st, slot)
                            };

                            let effective_slot = landed_slot.unwrap_or(slot_val);
                            match status {
                                arb_land::LandStatus::Landed => {
                                    let landing_ms = land_start.elapsed().as_secs_f64() * 1000.0;
                                    let total_e2e_ms = (t_ingest_to_quote_us
                                        + t_risk_us
                                        + t_assembly_us
                                        + t_signing_us)
                                        as f64
                                        / 1000.0
                                        + rtt_ms
                                        + landing_ms;

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

                                    info!(
                                        "🎯 Execution finalized/landed: {} (slot {})\n• Net Profit: +{:.6} SOL (~${:.2})\n• Loan: {:.4} SOL\n• Tokens: {}\n• Venues: {}\n{}",
                                        b_id,
                                        effective_slot,
                                        profit_sol_val,
                                        profit_usd_val,
                                        loan_size_sol_val,
                                        tokens_str_val,
                                        venues_str_val,
                                        timings_str
                                            .replace("<code>", "")
                                            .replace("</code>", "")
                                            .replace("<b>", "")
                                            .replace("</b>", "")
                                    );
                                    let _ = store.record_outcome(
                                        rec_id,
                                        "landed",
                                        Some(net_profit_val),
                                    );
                                    let _ = store.resolve_dispatch_intent(rec_id, "landed");
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
                                    arb_metrics::add_net_profit(profit_sol_val);
                                    arb_metrics::set_breaker_state(0);

                                    // Record landed trade in memory for 5-minute performance digest
                                    let summary = crate::telegram::TradeSummary {
                                        route: route_label_task.clone(),
                                        profit_sol: profit_sol_val,
                                        profit_usd: profit_usd_val,
                                        loan_sol: loan_size_sol_val,
                                        signature: tx_signature
                                            .as_deref()
                                            .unwrap_or(&b_id)
                                            .to_string(),
                                        venues: venues_str_val.clone(),
                                        slot: effective_slot,
                                    };
                                    {
                                        let mut guard = recent_landed_task.lock();
                                        if guard.len() >= 50 {
                                            guard.remove(0);
                                        }
                                        guard.push(summary);
                                    }

                                    telegram
                                        .alert_trade_landed(
                                            tx_signature.as_deref(),
                                            &b_id,
                                            profit_sol_val,
                                            profit_usd_val,
                                            loan_size_sol_val,
                                            &tokens_str_val,
                                            &venues_str_val,
                                            effective_slot,
                                            &timings_str,
                                        )
                                        .await;
                                }
                                arb_land::LandStatus::Dropped | arb_land::LandStatus::Expired => {
                                    warn!("⚠️ Execution {:?} in block: {}", status, b_id);
                                    let _ = store.record_outcome(rec_id, "dropped", None);
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
                                    };
                                    {
                                        let mut guard = recent_dropped_task.lock();
                                        if guard.len() >= 50 {
                                            guard.remove(0);
                                        }
                                        guard.push(dropped);
                                    }

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
                                    let _ = store.record_outcome(rec_id, "failed", None);
                                    let _ = store.resolve_dispatch_intent(rec_id, "failed_onchain");
                                    if let Some(r) = rung_for_record {
                                        js.record(r, InflightStatus::Failed);
                                    }
                                    risk_gate.record_bundle_landed(Slot(effective_slot), false);
                                    risk_gate.record_loss(5_000.0 / 1e9 * sol_price_val);
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
                                    };
                                    {
                                        let mut guard = recent_dropped_task.lock();
                                        if guard.len() >= 50 {
                                            guard.remove(0);
                                        }
                                        guard.push(dropped);
                                    }

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
                        error!("Bundle dispatch failed: {}", e);
                        self.telegram
                            .alert_execution_failed("dispatch", &e.to_string(), route_label)
                            .await;
                        let _ = self.store.record_outcome(record.id, "failed", None);
                        let _ = self.store.resolve_dispatch_intent(record.id, "send_failed");
                        // Unaccepted bundle / transport error does not spend tip or base tx fee (0 SOL loss)
                    }
                }
            }
        }
    }
}
