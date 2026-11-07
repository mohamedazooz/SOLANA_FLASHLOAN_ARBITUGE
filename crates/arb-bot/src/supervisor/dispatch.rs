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
}

impl Supervisor {
    pub(super) async fn dispatch_opportunity(&self, context: DispatchContext<'_>) {
        let DispatchContext {
            opp, tip, tip_rung, is_swqos_route, current_slot, expected_stream_generation, route_label,
            gross_rev_sol, gross_sol, profit_sol, live_sol_price, blockhash_b58, cu_limit,
            cu_price, tx_bytes, record, real_net_profit, payer, route_sig,
        } = context;
                // 6. Dispatch based on mode
                let leader_str = self
                    .jito_service
                    .leader_for(current_slot)
                    .map(|pk| pk.to_string())
                    .unwrap_or_else(|| "Unknown (Non-Jito / Stale Schedule)".to_string());

                if self.mode == BotMode::Live && std::env::var("ARB_LIVE").is_ok() {
                    self.telegram
                        .alert_opportunity_with_telemetry(
                            &route_label,
                            gross_rev_sol,
                            profit_sol,
                            profit_sol * live_sol_price,
                            current_slot.0,
                            &blockhash_b58,
                            &leader_str,
                            cu_limit,
                            cu_price,
                        )
                        .await;
                }
                match self.mode {
                    BotMode::Canary => {
                        info!(
                            "canary: simulated valid execution of {} bytes (not broadcast to Jito)",
                            tx_bytes.len()
                        );
                        arb_metrics::inc_fills();
                        arb_metrics::add_net_profit(profit_sol);
                        let _ =
                            self.store
                                .record_outcome(record.id, "canary", Some(real_net_profit));
                        self.telegram
                            .alert_opportunity_with_telemetry(
                                &format!("Canary-{}", route_label),
                                gross_rev_sol,
                                profit_sol,
                                profit_sol * live_sol_price,
                                current_slot.0,
                                &blockhash_b58,
                                &leader_str,
                                cu_limit,
                                cu_price,
                            )
                            .await;
                    }
                    BotMode::Shadow | BotMode::Simulate => {
                        arb_metrics::add_simulated_net_profit(profit_sol);
                        let _ = self.store.record_outcome(
                            record.id,
                            "simulated",
                            Some(real_net_profit),
                        );
                        if real_net_profit.as_u64() >= self.config.strategy.min_net_profit_abs {
                            self.telegram
                                .alert_opportunity_with_telemetry(
                                    &format!("Simulated-{}", opp.route_id.0),
                                    gross_rev_sol,
                                    profit_sol,
                                    profit_sol * live_sol_price,
                                    current_slot.0,
                                    &blockhash_b58,
                                    &leader_str,
                                    cu_limit,
                                    cu_price,
                                )
                                .await;
                        }
                    }
                    BotMode::Live => {
                        if std::env::var("ARB_LIVE").is_ok() {
                            // Safety Floor Check: Enforce minimum 0.05 SOL reserve (50_000_000 lamports)
                            let current_balance =
                                self.rpc_pool.get_balance(&payer).await.unwrap_or(0);
                            let min_reserve_lamports = 50_000_000u64;
                            if current_balance < min_reserve_lamports {
                                warn!(
                                    "🚨 Safety Floor Active: Wallet balance ({} lamports / {:.4} SOL) is below 0.05 SOL minimum reserve — skipping execution to protect fees",
                                    current_balance,
                                    current_balance as f64 / 1e9
                                );
                                self.telegram
                                    .alert_opportunity_rejected(
                                        &route_label,
                                        gross_sol,
                                        "Safety Floor Reserve",
                                        &format!("رصيد المحفظة ({:.4} SOL) أقل من صمام الأمان الأدنى (0.05 SOL)", current_balance as f64 / 1e9),
                                        current_slot.0,
                                    )
                                    .await;
                                return;
                            }

                            // Cold Wallet Auto-Sweep: Transfer accrued profit above reserve
                            let target_cold = std::env::var("COLD_WALLET_ADDRESS")
                                .or_else(|_| std::env::var("PROFIT_RECEIVER_ADDRESS"))
                                .ok()
                                .or_else(|| self.config.wallet.cold_wallet_pubkey.clone());

                            if let Some(cold_wallet_str) = target_cold
                                && let Ok(cold_wallet) = Pubkey::from_str(&cold_wallet_str)
                            {
                                if let Some(ref signer) = self.keypair {
                                    if cold_wallet == signer.pubkey() {
                                        debug!(
                                            "Cold wallet destination is identical to signer hot wallet; skipping self-sweep"
                                        );
                                        return;
                                    }
                                }
                                let sweep_threshold = 250_000_000u64; // 0.25 SOL total (0.05 reserve + 0.20 profit)
                                if current_balance > sweep_threshold {
                                    let sweep_lamports = current_balance
                                        .saturating_sub(min_reserve_lamports)
                                        .saturating_sub(10_000);
                                    let sweep_sol = sweep_lamports as f64 / 1e9;
                                    info!(
                                        "💰 Cold Wallet Sweep: Executing on-chain transfer of {:.4} SOL -> {}",
                                        sweep_sol, cold_wallet
                                    );
                                    if let Some(ref signer) = self.keypair {
                                        let blockhash = self.fetch_recent_blockhash().await;
                                        let transfer_ix = arb_tx::build_system_transfer(
                                            signer.pubkey(),
                                            cold_wallet,
                                            sweep_lamports,
                                        );
                                        if let Ok(signed_tx) = TxBuilder::build_signed_transaction(
                                            signer,
                                            &[transfer_ix],
                                            blockhash,
                                        ) {
                                            use base64::Engine;
                                            let b64 = base64::engine::general_purpose::STANDARD
                                                .encode(&signed_tx);
                                            match self.rpc_pool.send_raw_transaction(&b64).await {
                                                Ok(sig) => {
                                                    info!(
                                                        "💰 Cold Wallet Sweep confirmed tx: {sig}"
                                                    );
                                                    self.telegram
                                                        .send_message(&format!(
                                                            "🛡️ <b>تم ترحيل أرباح إلى المحفظة الباردة</b>\n\n\
                                                             💵 <b>المبلغ:</b> <code>{:.4} SOL</code>\n\
                                                             🏦 <b>المحفظة المستلمة:</b> <code>{}</code>\n\
                                                             🔗 <b>التوقيع:</b> <code>{}</code>",
                                                            sweep_sol, cold_wallet, sig
                                                        ))
                                                        .await
                                                        .ok();
                                                }
                                                Err(e) => {
                                                    warn!("Cold Wallet Sweep failed: {e}");
                                                }
                                            }
                                        }
                                    } else {
                                        self.telegram
                                            .send_message(&format!(
                                                "🛡️ <b>[Simulated] ترحيل أرباح إلى المحفظة الباردة</b>\n\n\
                                                 💵 <b>المبلغ:</b> <code>{:.4} SOL</code>\n\
                                                 🏦 <b>المحفظة:</b> <code>{}</code>",
                                                sweep_sol, cold_wallet
                                            ))
                                            .await
                                            .ok();
                                    }
                                }
                            }

                            if tx_bytes.len() > 1232 {
                                warn!(
                                    "⚠️ Transaction size ({} bytes) exceeds Solana MTU (1232 bytes) — skipping to prevent drop",
                                    tx_bytes.len()
                                );
                                self.telegram
                                    .alert_opportunity_rejected(
                                        &route_label,
                                        gross_sol,
                                        "Solana MTU Check",
                                        &format!("حجم المعاملة ({} bytes) تجاوز حد MTU لشبكة سولانا (1232 bytes)", tx_bytes.len()),
                                        current_slot.0,
                                    )
                                    .await;
                                return;
                            }

                            // Pre-Flight Simulation Guard: Mandatory in Live mode, or when explicitly enabled
                            let sim_enabled = self.mode == BotMode::Live
                                || std::env::var("ENABLE_PREFLIGHT_SIMULATION").as_deref()
                                    == Ok("true");

                            if sim_enabled {
                                // 1. Fast in-memory LiteSVM simulation check (sub-millisecond)
                                let local_sim = self.simulator.simulate_local(tx_bytes.as_slice());
                                if local_sim.success {
                                    debug!(
                                        "LiteSVM in-memory simulation succeeded (CU: {})",
                                        local_sim.compute_units_consumed
                                    );
                                } else if let Some(ref err) = local_sim.error {
                                    if !err.contains("litesvm feature disabled") {
                                        warn!(
                                            "LiteSVM in-memory simulation rejected ({}) - skipping dispatch",
                                            err
                                        );
                                        return;
                                    }
                                }

                                // 2. RPC simulation: fail-closed safety check before dispatch
                                match self.simulator.simulate_async(tx_bytes.as_slice()).await {
                                    Ok(sim_res) => {
                                        if sim_res.success && sim_res.units_consumed > 0 {
                                            // Persist actual RPC simulation CU for this route shape.
                                            // This is simulation evidence, not a claim about landed CU.
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
                                            self.telegram
                                                .alert_opportunity_rejected(
                                                    &route_label,
                                                    gross_sol,
                                                    "Pre-flight Simulation",
                                                    &format!(
                                                        "محاكاة المعاملة فشلت على الشبكة: {:?}",
                                                        sim_res.error
                                                    ),
                                                    current_slot.0,
                                                )
                                                .await;
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        warn!(
                                            "Pre-flight simulation RPC call error ({e}) - failing closed for live safety"
                                        );
                                        return;
                                    }
                                }
                            }

                            // Fire-time gate: a bundle competes for the *next*
                            // slot, so leave at `window_start - p95 - 3σ` rather
                            // than whenever this tick happened to get here. The
                            // wait is skipped when the target slot has no Jito
                            // leader to bid against, and when the clock is cold.
                            let target_slot = Slot(current_slot.0.saturating_add(1));
                            let fire_wait =
                                if !is_swqos_route && self.jito_service.bids_for(target_slot) {
                                    let clock = self.slot_clock.lock();
                                    clock
                                        .schedule(target_slot.0, &self.rtt.lock().0, 5)
                                        .map(|plan| clock.wait_duration(plan.fire_at_ns))
                                } else {
                                    None
                                };
                            if let Some(wait) = fire_wait
                                && wait
                                    <= std::time::Duration::from_millis(arb_math::SLOT_MS as u64)
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
                                self.telegram.alert_opportunity_rejected(
                                    &route_label, gross_sol, "Final Stream Readiness",
                                    &format!("Stream/slot integrity changed before submission: {reason:?}"),
                                    current_slot.0,
                                ).await;
                                return;
                            }

                            let sent_at = Instant::now();
                            let (send_result, was_swqos): (Result<String, String>, bool) =
                                if is_swqos_route {
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
                                        .send_bundle_multi_region(
                                            &bundle_txs,
                                            &self.config.jito.regions,
                                        )
                                        .await
                                        .map_err(|e| e.to_string());
                                    (r, false)
                                };
                            let rtt = sent_at.elapsed();

                            // Route signature for CU tracking
                            let route_sig = format!(
                                "cu:{}",
                                opp.hops
                                    .iter()
                                    .map(|h| format!("{}-{:?}", h.pool_id.0, h.protocol))
                                    .collect::<Vec<_>>()
                                    .join("_")
                            );

                            match send_result {
                                Ok(bundle_id) => {
                                    self.observe_bundle_rtt(rtt);
                                    if was_swqos {
                                        info!(
                                            "🚀 Transaction accepted via SWQoS RPC: {}",
                                            bundle_id
                                        );
                                    } else {
                                        info!(
                                            "🚀 Bundle accepted by Jito Block Engine: {}",
                                            bundle_id
                                        );
                                    }
                                    self.telegram
                                        .alert_execution_sent(&bundle_id, &route_label, profit_sol)
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

                                    // Asynchronous Landing Tracker via getBundleStatuses (Layer 08)
                                    let lander = self.jito_lander.clone();
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
                                    let tip_val = tip.as_u64();
                                    let sol_price_val = live_sol_price;
                                    let slot_val = current_slot.0;
                                    let route_label_task = route_label.clone();

                                    tokio::spawn(async move {
                                        // SWQoS returns a transaction signature, not a Jito bundle ID.
                                        // Query getSignatureStatuses for that path; only bundle submissions
                                        // may be queried through getBundleStatuses.
                                        let status = if was_swqos_task {
                                            match signature_rpc.poll_signature_until_terminal(
                                                &b_id, 20, std::time::Duration::from_millis(400),
                                            ).await {
                                                Ok(arb_ingest::SignatureStatus::Finalized) => arb_land::LandStatus::Landed,
                                                Ok(arb_ingest::SignatureStatus::Failed) => arb_land::LandStatus::Failed,
                                                Ok(arb_ingest::SignatureStatus::Processed) => {
                                                    info!("Transaction signature {} is processed; not yet finalized", b_id);
                                                    arb_land::LandStatus::Sent
                                                }
                                                Ok(arb_ingest::SignatureStatus::Confirmed) => {
                                                    info!("Transaction signature {} is confirmed; awaiting finalization", b_id);
                                                    arb_land::LandStatus::Sent
                                                }
                                                Ok(arb_ingest::SignatureStatus::Pending) => arb_land::LandStatus::Sent,
                                                Err(error) => {
                                                    warn!("Signature status query failed for {}: {}", b_id, error);
                                                    arb_land::LandStatus::Sent
                                                }
                                            }
                                        } else {
                                            lander.poll_bundle_until_terminal(
                                                &b_id, 8, std::time::Duration::from_millis(400),
                                            ).await
                                        };

                                        match status {
                                            arb_land::LandStatus::Landed => {
                                                info!(
                                                    "🎯 Execution finalized/landed: {}",
                                                    b_id
                                                );
                                                let _ = store.record_outcome(
                                                    rec_id,
                                                    "landed",
                                                    Some(net_profit_val),
                                                );
                                                if let Some(r) = rung_for_record {
                                                    js.record(
                                                        r,
                                                        InflightStatus::Landed { slot: slot_val },
                                                    );
                                                }
                                                risk_gate.on_success();
                                                // Phase E: track bundle landed for landing rate
                                                risk_gate
                                                    .record_bundle_landed(Slot(slot_val), true);
                                                arb_metrics::inc_fills();
                                                arb_metrics::add_net_profit(profit_sol_val);
                                                arb_metrics::set_breaker_state(0);
                                                telegram
                                                    .alert_trade_landed(
                                                        &b_id,
                                                        profit_sol_val,
                                                        profit_usd_val,
                                                    )
                                                    .await;
                                            }
                                            arb_land::LandStatus::Dropped
                                            | arb_land::LandStatus::Expired
                                            | arb_land::LandStatus::Failed => {
                                                warn!(
                                                    "⚠️ Jito Bundle {:?} in block: {}",
                                                    status, b_id
                                                );
                                                let _ =
                                                    store.record_outcome(rec_id, "dropped", None);
                                                if let Some(r) = rung_for_record {
                                                    js.record(r, InflightStatus::Failed);
                                                }
                                                // Phase E: track bundle not landed for landing rate
                                                risk_gate
                                                    .record_bundle_landed(Slot(slot_val), false);
                                                risk_gate.record_loss(
                                                    (tip_val as f64 + 5_000.0) / 1e9
                                                        * sol_price_val,
                                                );
                                                risk_gate.on_failure(current_slot);
                                                arb_metrics::set_breaker_state(
                                                    if risk_gate.is_breaker_open(current_slot) {
                                                        1
                                                    } else {
                                                        0
                                                    },
                                                );
                                                telegram
                                                    .alert_execution_dropped(
                                                        &b_id,
                                                        &route_label_task,
                                                        &format!("{status:?}"),
                                                    )
                                                    .await;
                                            }
                                            _ => {
                                                debug!(
                                                    "Jito Bundle {} terminal check finished with status {:?}",
                                                    b_id, status
                                                );
                                            }
                                        }
                                    });
                                }
                                Err(e) => {
                                    error!("Bundle dispatch failed: {}", e);
                                    self.telegram
                                        .alert_execution_failed(
                                            "dispatch",
                                            &e.to_string(),
                                            &route_label,
                                        )
                                        .await;
                                    let _ = self.store.record_outcome(record.id, "failed", None);
                                    self.risk_gate.record_loss(
                                        (tip.as_u64() as f64 + 5_000.0) / 1e9 * live_sol_price,
                                    );
                                    self.risk_gate.on_failure(current_slot);
                                    arb_metrics::set_breaker_state(
                                        if self.risk_gate.is_breaker_open(current_slot) {
                                            1
                                        } else {
                                            0
                                        },
                                    );

                                    // Alert if circuit breaker tripped
                                    if self.risk_gate.is_breaker_open(current_slot) {
                                        self.telegram.alert_circuit_breaker(&e.to_string()).await;
                                    }
                                }
                            }
                        } else {
                            warn!("Live mode active but ARB_LIVE=1 is missing!");
                        }
                    }
                }
    }
}
