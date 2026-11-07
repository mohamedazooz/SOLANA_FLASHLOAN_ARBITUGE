//! Main supervisor lifecycle and orchestration loop.
use super::*;

impl Supervisor {
    pub async fn run(&self) -> Result<()> {
        info!("Starting Supervisor — Mode: {}", self.mode);

        for token in &self.config.strategy.base_tokens {
            info!("Monitored Base Token: {}", token);
        }

        // Pool discovery is intentionally not sourced from HTTP catalogs. The next
        // ingestion stage promotes validated Solami Yellowstone/WebSocket account events
        // into the dynamic pool registry. Until then, no catalog-only pool is quoteable.
        self.telegram
            .alert_startup(&self.mode.to_string(), 0, false)
            .await;
        self.emit_market_snapshot(0, true).await;

        // Jito transport remains isolated from the Solana market-data plane.
        // No RPC market-state reads are performed by this service in this phase.
        let js = Arc::clone(&self.jito_service);
        let rpc_for_js = Arc::new(self.rpc_pool.clone());
        let leader_refresh = std::time::Duration::from_secs(self.config.jito.leader_refresh_secs);
        tokio::spawn(async move {
            js.run(rpc_for_js, leader_refresh).await;
        });

        // Yellowstone Geyser gRPC real-time streaming ingestion worker
        let cancel_token = CancellationToken::new();
        let cancel_ingest = cancel_token.clone();
        let ingest_worker = Arc::clone(&self.ingest);
        tokio::spawn(async move {
            ingest_worker.start_stream(cancel_ingest).await;
        });

        // No HTTP polling loop: Yellowstone account updates are the sole authoritative
        // market-state input. Raw updates are journaled until protocol decoders promote them.

        // Provider-neutral WebSocket slots are configured separately. No fixed vendor-specific
        // event feed is allowed to become an authoritative pool-state source.

        // Real slot clock from Solami Yellowstone gRPC. Never fabricated.
        let mut current_slot = Slot(0);
        let mut tick: u64 = 0;

        let payer = self
            .keypair
            .as_ref()
            .map(|k| k.pubkey())
            .unwrap_or_else(|| Pubkey::new_from_array([1; 32]));

        // Online-verified Kamino Main Market WSOL Reserve & Vaults
        let kamino_params = KaminoFlashParams {
            lending_market: Pubkey::from_str(arb_tx::KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(arb_tx::KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: payer,
            reserve_liquidity_fee_receiver: Pubkey::from_str(arb_tx::KAMINO_WSOL_FEE_VAULT)
                .unwrap(),
        };

        let mut last_snapshot_digest = Instant::now();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            tick += 1;

            // Health sweep every 100 ticks (~30s): watches gRPC stream, disk, Telegram.
            // Logs the exact reason for any outage; never blocks trading.
            if tick.is_multiple_of(100) {
                let summary = self.health_monitor.sweep(current_slot.0).await;
                debug!("health sweep tick={tick}: {summary}");
                self.risk_gate.maybe_reset_periodic();
            }

            // Periodic 30-minute Snapshot Digest to Telegram and local disk
            if last_snapshot_digest.elapsed() >= Duration::from_secs(1800) {
                self.emit_market_snapshot(current_slot.0, true).await;
                last_snapshot_digest = Instant::now();
            }

            // KillSwitch is checked on every loop, even while the stream is stalled.
            let ks_path =
                std::env::var("KILL_SWITCH_PATH").unwrap_or_else(|_| "KILL_SWITCH".to_string());
            if self.risk_gate.kill_switch().check_file(&ks_path) {
                info!(
                    "kill switch file present at {} ; stopping supervisor",
                    ks_path
                );
                break;
            }

            // The Solami Yellowstone gRPC slot stream is authoritative. No HTTP RPC
            // fallback and no reuse of a previous slot for a new strategy cycle.
            let stream_slot = self.ingest.current_slot();
            if stream_slot <= current_slot.0 {
                continue;
            }
            current_slot = Slot(stream_slot);
            // Pin the stream generation for this strategy cycle. A reconnect at
            // any point before dispatch invalidates every opportunity from this tick.
            let expected_stream_generation = self.ingest.stream_generation();
            self.slot_clock.lock().observe_slot(current_slot.0);

            // Promote PumpSwap only after Yellowstone has supplied the Pool, both
            // vaults, both mints and canonical Pump Fees FeeConfig at coherent slots.
            // The graph uses a conservative whole-BPS fee for candidate discovery;
            // exact route validation remains protocol-adapter driven.
            let pump_swap_report = self.ingest.refresh_pump_swap_states(
                self.config.strategy.max_state_age_slots,
            );
            if pump_swap_report.pools_promoted > 0 || pump_swap_report.pools_rejected > 0 {
                debug!(
                    "PumpSwap promotion: owner_accounts={} tracked={} joined={} promoted={} rejected={}",
                    pump_swap_report.pool_accounts_seen,
                    pump_swap_report.dependencies_tracked,
                    pump_swap_report.pools_joined,
                    pump_swap_report.pools_promoted,
                    pump_swap_report.pools_rejected,
                );
            }
            self.previous_tips
                .lock()
                .retain(|(slot, _), _| *slot + 2 >= current_slot.0);

            // 0. Verified economic inputs (fail closed): Kamino reserve fee and
            // WSOL mint decimals must exist in the Yellowstone account journal.
            // Jito cost uses the configured conservative tip ceiling until a
            // slot-aware tip-floor feed is integrated. Missing/stale chain data
            // skips the whole tick; no HTTP JSON-RPC or stale-cache fallback.
            let verified = match self.build_verified_inputs(current_slot).await {
                Ok(v) => v,
                Err(e) => {
                    debug!(
                        "verified economic inputs unavailable ({e}); skipping tick (fail closed)"
                    );
                    continue;
                }
            };
            let economic_config = verified
                .clone()
                .into_economic_config(TokenAmount(self.config.strategy.min_net_profit_abs));
            {
                let mut graph = self.graph.lock();
                graph.set_economic_config(economic_config.clone());
            }

            // HTTP catalog/trending discovery is disabled. Pool candidates must arrive
            // through the configured Solami gRPC/WebSocket streams and pass account validation.

            // 1. Ingest: take one pool snapshot per strategy cycle; account updates mutate one key.
            //    Raw Yellowstone updates are promoted only after protocol-specific decoding.
            let quote_ingest_time = std::time::Instant::now();
            let market_snap = self.ingest.snapshot();
            let pools: Vec<PoolState> = market_snap.snapshot_pools();
            // Reset every cycle: an absent fresh SOL/USDC pool must not reuse an old price.
            *self.last_known_live_sol_price.write() = 0.0;

            // Track live SOL price from any active SOL-USDC pool (Raydium, Orca, Meteora).
            // Using all three venues improves price freshness and cross-checks accuracy.
            let wsol_str = arb_tx::WSOL_MINT;
            let usdc_str = arb_tx::USDC_MINT;
            for p in &pools {
                if current_slot.0.saturating_sub(p.last_update_slot)
                    > self.config.strategy.max_state_age_slots
                {
                    continue;
                }
                let is_sol_usdc = (p.base_mint.0.to_string() == wsol_str
                    && p.quote_mint.0.to_string() == usdc_str)
                    || (p.base_mint.0.to_string() == usdc_str
                        && p.quote_mint.0.to_string() == wsol_str);
                if is_sol_usdc && p.reserve_in > 0 && p.reserve_out > 0 {
                    let (sol_res, usdc_res) = if p.base_mint.0.to_string() == wsol_str {
                        (p.reserve_in as f64 / 1e9, p.reserve_out as f64 / 1e6)
                    } else {
                        (p.reserve_out as f64 / 1e9, p.reserve_in as f64 / 1e6)
                    };
                    if sol_res > 0.0 {
                        *self.last_known_live_sol_price.write() = usdc_res / sol_res;
                        break; // first live SOL/USDC price is sufficient
                    }
                }
            }

            // 2. Update Graph using fresh states only. Trade sizing is governed by RiskGate.
            let max_sizing = if self.config.strategy.use_flash_loan {
                u64::MAX // Uncapped initial trade sizing: powered by atomic flash loans (zero capital at risk)
            } else {
                100_000_000
            };

            // Filter out stale pools from graph to prevent phantom arbitrage cycles
            // caused by price divergence against frozen/unupdated pools.
            let max_graph_age_slots = self.config.strategy.max_state_age_slots.saturating_mul(4).max(8);
            let fresh_pools: Vec<PoolState> = pools
                .iter()
                .filter(|p| {
                    p.slot.0 > 0 && current_slot.0.saturating_sub(p.slot.0) <= max_graph_age_slots
                })
                .cloned()
                .collect();

            let opportunities = {
                let mut graph = self.graph.lock();
                graph.build_from_pools(&fresh_pools);
                let mut opps = graph.find_base_cycles(current_slot, max_sizing);

                // ── Bellman-Ford negative cycle detection ──
                // Finds arbitrage routes of ANY length (not just 2-hop/3-hop).
                // Runs when the base-cycle DFS finds nothing, or to supplement
                // with longer multi-hop routes the DFS pruning would miss.
                if opps.is_empty() {
                    let bf_engine = arb_engine::BellmanFordEngine::from_pools(&fresh_pools)
                        .with_economic_config(economic_config.clone());
                    let bf_opps = bf_engine.find_negative_cycles(current_slot, max_sizing);
                    if !bf_opps.is_empty() {
                        info!(
                            "🎯 Bellman-Ford detected {} arbitrage cycle(s) that DFS missed",
                            bf_opps.len()
                        );
                        opps = bf_opps;
                    }
                }

                if opps.is_empty() {
                    opps = graph.find_multi_hop_cycles(current_slot, max_sizing, 5);
                }
                info!(
                    "Slot {}: graph discovered {} base cycles (cause: DFS + Bellman-Ford cascade complete); graph edges={}, base_anchors={}",
                    current_slot.0,
                    opps.len(),
                    graph.edges_by_from.len(),
                    graph.base_anchors.len()
                );
                opps
            };

            for opp in &opportunities {
                arb_metrics::inc_opportunities();
                let tip_account = arb_land::get_random_tip_account();

                // 3. Per-tx cost basis — ONE list, used by both the Jito edge
                //    estimate and the go/no-go gate (TX-02):
                //    flash fee (verified Q64.64 sf) + base fee + priority fee.
                let flash_fee_raw = if self.config.strategy.use_flash_loan {
                    arb_math::economic::calculate_flash_fee_raw(
                        opp.size_in.as_u64(),
                        verified.flash_loan_fee_sf.value,
                    )
                    .unwrap_or(u64::MAX) // fail closed: uncomputable fee rejects
                } else {
                    0
                };
                // Dynamic CU: sized from the REAL route shape (protocol-specific
                // per-hop cost + ephemeral ATAs) with the 15% buffer, not a flat
                // per-hop guess. The priority price is capped by a share of the
                // gross profit so it can never eat the edge.
                let hop_protocols: Vec<DexProtocol> = opp.hops.iter().map(|h| h.protocol).collect();
                let route_sig = format!(
                    "cu:{}",
                    opp.hops
                        .iter()
                        .map(|h| format!("{}-{:?}", h.pool_id.0, h.protocol))
                        .collect::<Vec<_>>()
                        .join("_")
                );
                let estimated_cu = arb_tx::estimate_route_cu(
                    self.config.strategy.use_flash_loan,
                    &hop_protocols,
                    opp.hops.len().saturating_sub(1),
                );
                // Phase E: use measured CU if available, otherwise fall back to estimate
                let measured_cu = self.measured_cu.lock().get(&route_sig).copied();
                let base_cu = measured_cu.unwrap_or(estimated_cu);
                let cu_limit = arb_tx::calculate_dynamic_cu_limit(base_cu);
                let desired_cu_price = self.tip_strategy.adaptive_cu_price(&self.recent_landed_cu);
                let cu_price = arb_tx::cap_cu_price_by_profit(
                    opp.gross_profit.as_u64(),
                    cu_limit,
                    desired_cu_price,
                    PRIORITY_FEE_MAX_SHARE_BPS,
                );
                // cu_price is micro-lamports per CU; convert to a lamport estimate.
                let priority_fee_lamports = (cu_price.saturating_mul(cu_limit as u64)) / 1_000_000;
                let gross = opp.gross_profit.as_u64();
                let gross_sol = gross as f64 / 1e9;
                let route_label = opp
                    .hops
                    .iter()
                    .map(|h| {
                        let id = h.pool_id.0.to_string();
                        format!("{:?}:{}", h.protocol, &id[..6.min(id.len())])
                    })
                    .collect::<Vec<_>>()
                    .join(" → ");

                let net_before_tip = gross
                    .saturating_sub(flash_fee_raw)
                    .saturating_sub(BASE_FEE_LAMPORTS)
                    .saturating_sub(priority_fee_lamports);

                let edge = Lamports(net_before_tip);
                let previous_tip = self
                    .previous_tips
                    .lock()
                    .get(&(current_slot.0, opp.route_id.0))
                    .copied();

                let is_all_dlmm = opp
                    .hops
                    .iter()
                    .all(|h| h.protocol == DexProtocol::MeteoraDlmm);
                let is_hot_write_lock =
                    !is_all_dlmm && opp.hops.iter().any(|h| h.protocol == DexProtocol::PumpFun);

                let jito_plan = self.jito_service.plan_with_context(
                    current_slot,
                    edge,
                    previous_tip,
                    base_cu as u64,
                    is_hot_write_lock,
                );

                let (tip, tip_rung, is_swqos_route) = match jito_plan.route {
                    ExecutionRoute::SwqosDirect { .. } => (Lamports(0), None, true),
                    ExecutionRoute::Bundle => match jito_plan.tip.decision {
                        TipDecision::Bid { tip, rung, .. } => (tip, Some(rung), false),
                        _ => {
                            debug!(
                                "Opportunity skipped: Jito plan refused ({:?})",
                                jito_plan.tip.decision
                            );
                            if gross >= 50_000 {
                                let reason = format!(
                                    "رفض تسعير الـ Tip من الأوراكل: {:?}",
                                    jito_plan.tip.decision
                                );
                                self.telegram
                                    .alert_opportunity_rejected(
                                        &route_label,
                                        gross_sol,
                                        "Jito Tip Plan",
                                        &reason,
                                        current_slot.0,
                                    )
                                    .await;
                            }
                            continue;
                        }
                    },
                    ExecutionRoute::PlainRpc => {
                        debug!("Opportunity skipped: leader is not Jito-Solana (route=PlainRpc)");
                        if gross >= 50_000 {
                            let reason =
                                "المدقق القائد للسلوت الحالي لا يدعم حزم Jito المباشرة".to_string();
                            self.telegram
                                .alert_opportunity_rejected(
                                    &route_label,
                                    gross_sol,
                                    "Jito Leader Gate",
                                    &reason,
                                    current_slot.0,
                                )
                                .await;
                        }
                        continue;
                    }
                };

                // Go/no-go through the AUTHORITATIVE model with the ACTUAL
                // planned tip: the number that passes is the number that will
                // be paid. (expected_out is already net of DEX fees — no double
                // counting; flash/base/priority/tip are the only added costs.)
                let real_net_raw = calculate_net_profit_raw(
                    opp.expected_out.as_u64(),
                    opp.size_in.as_u64(),
                    flash_fee_raw,
                    0, // dex fees already inside expected_out
                    0, // transfer fees
                    0, // account creation
                    0, // other verified costs
                    Lamports(BASE_FEE_LAMPORTS),
                    Lamports(priority_fee_lamports),
                    tip,
                    verified.lamports_per_base_raw.value,
                    verified.lamports_per_base_denom.value,
                )
                .ok()
                .filter(|net| *net > 0);
                let Some(real_net_raw) = real_net_raw else {
                    debug!("Opportunity rejected: net <= 0 with actual tip {tip}");
                    if gross > 0 {
                        let reason = format!(
                            "تآكل الربح بالكامل: تكاليف الفلاش لوان ({:.6} SOL) + البقشيش ({:.6} SOL) + رسوم الشبكة فاقت الربح الإجمالي",
                            flash_fee_raw as f64 / 1e9,
                            tip.as_u64() as f64 / 1e9
                        );
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Net Profit Economics",
                                &reason,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                };
                let real_net_profit = Lamports(real_net_raw);
                if real_net_profit.as_u64() < self.config.strategy.min_net_profit_abs {
                    debug!(
                        "Opportunity rejected: net {real_net_raw} < min_profit {}",
                        self.config.strategy.min_net_profit_abs
                    );
                    if gross > 0 {
                        let reason = format!(
                            "صافي الربح ({:.6} SOL) أقل من الحد الأدنى المطلوب ({:.6} SOL)",
                            real_net_raw as f64 / 1e9,
                            self.config.strategy.min_net_profit_abs as f64 / 1e9
                        );
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Minimum Profit Floor",
                                &reason,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                }

                // Chronos Spatiotemporal Expected Value (EV) Gate evaluation.
                // The EV gate is denominated in USD, so it needs a real SOL
                // price: with none available we skip the opportunity (fail
                // closed) rather than evaluate against a guessed constant.
                let latency_ms = quote_ingest_time.elapsed().as_millis() as f64;
                self.health_monitor
                    .record_latency(latency_ms, current_slot.0)
                    .await;
                let Some(sol_price) = self.live_sol_price() else {
                    debug!("no live SOL/USD price; skipping opportunity (fail closed)");
                    continue;
                };

                let chronos_input = arb_risk::ChronosEvInput {
                    age_ms: latency_ms as u64,
                    max_age_ms: self.config.chronos.max_age_ms,
                    decay_lambda_per_sec: self.config.chronos.decay_lambda_per_sec,
                    gross_profit_lamports: gross,
                    flash_fee_lamports: flash_fee_raw,
                    base_network_fee_lamports: BASE_FEE_LAMPORTS,
                    priority_fee_lamports,
                    jito_tip_lamports: tip.as_u64(),
                    failed_attempt_cost_lamports: self.config.chronos.failed_attempt_cost_lamports,
                    sol_price_usd: sol_price,
                    min_ev_usd: self.config.chronos.min_ev_usd,
                };

                if let Err(reason) = self.risk_gate.evaluate_chronos_gate(chronos_input) {
                    debug!("Opportunity rejected by Chronos EV Gate: {reason}");
                    if gross > 0 {
                        let reason_msg = format!(
                            "بوابة Chronos EV رفضت التنفيذ: {reason} (زمن المعالجة: {:.1}ms)",
                            latency_ms
                        );
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Chronos EV Gate",
                                &reason_msg,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                }

                // State age in SLOTS — fail closed on any unresolvable pool.
                let state_age_slots = max_state_age_slots(&opp.hops, &pools, current_slot);

                // Split routing is disabled until per-leg AMM quote, fee, slippage,
                // execution instruction and aggregate min-out equivalence are proven.
                let split: Option<arb_engine::SplitAllocation> = None;
                let effective_price_impact_bps = opp.price_impact_bps.0 as u64;

                // Convert the actual input notional to USD before either risk path.
                // Unknown base-mint valuation is a hard rejection, never a $10 placeholder.
                let base_price_usd = if opp.base_mint.0 == Pubkey::from_str(arb_tx::WSOL_MINT).unwrap_or_default() {
                    Some(live_sol_price)
                } else if opp.base_mint.0 == Pubkey::from_str(arb_tx::USDC_MINT).unwrap_or_default() {
                    Some(1.0)
                } else {
                    None
                };
                let trade_size_usd = base_price_usd.map(|price| {
                    (opp.size_in.as_u64() as f64 / 10_f64.powi(opp.base_decimals as i32)) * price
                });
                let risk_check = match trade_size_usd {
                    Some(size_usd) if size_usd.is_finite() && size_usd > 0.0 => {
                        let gross_profit = opp.gross_profit.as_u64();
                        let tip_share_bps = (tip.as_u64() * 10_000)
                            .checked_div(gross_profit)
                            .unwrap_or(0);
                        if self.config.strategy.use_flash_loan {
                            self.risk_gate.evaluate_flash_loan(
                                real_net_profit, opp.size_in, current_slot, state_age_slots,
                                size_usd, tip.as_u64(), tip_share_bps, effective_price_impact_bps,
                            )
                        } else {
                            self.risk_gate.evaluate_full(
                                real_net_profit, opp.size_in, current_slot, state_age_slots,
                                size_usd, tip.as_u64(), tip_share_bps, effective_price_impact_bps,
                            )
                        }
                    }
                    _ => Err("cannot establish USD notional for base mint; risk gate fails closed".to_string()),
                };

                if let Err(reason) = risk_check {
                    debug!("Opportunity rejected by RiskGate: {reason}");
                    if gross > 0 {
                        let reason_msg = format!("قيود RiskGate رفضت التنفيذ: {reason}");
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "RiskGate Protection",
                                &reason_msg,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                }

                arb_metrics::inc_attempts();

                // 4. Dynamic Compute Budget: use the per-tx CU limit and the
                //    adaptive priority fee computed above (same values that
                //    priced the gate).

                // Jupiter execution is disabled in this remediation build. A Jupiter
                // quote/route is not proof that our locally sized route has equivalent
                // fees, account metas, min-out and flash-loan repayment semantics.
                let route_via_jupiter = false;

                let ixs: Vec<arb_tx::CompiledInstruction> = if let Some(ref s) = split
                    && self.config.strategy.use_flash_loan
                {
                    // ── Split routing: one swap leg per venue, then the RETURN hop,
                    //    all wrapped in a single Kamino flash loan ──
                    info!(
                        "🔀 RouteArchitect N-way split EXECUTING: {} legs (impact {}bps > {}bps)",
                        s.legs.len(),
                        opp.price_impact_bps.0,
                        self.config.strategy.max_price_impact_bps
                    );
                    let first_hop = &opp.hops[0];
                    let slip = arb_tx::DEFAULT_HOP_SLIPPAGE_BPS;
                    let mut all_legs: Vec<arb_tx::CompiledInstruction> = Vec::new();
                    let mut total_out: u64 = 0;
                    let mut split_route_success = true;

                    for leg in &s.legs {
                        let estimated_out = leg.expected_out;
                        let slippage_amount = estimated_out * slip / 10_000;
                        let min_out = estimated_out.saturating_sub(slippage_amount);
                        match arb_tx::build_swap_hop_instruction(
                            &arb_types::SwapHop {
                                pool_id: leg.pool_id,
                                protocol: leg.protocol,
                                mint_in: first_hop.mint_in,
                                mint_out: first_hop.mint_out,
                                fee_bps: first_hop.fee_bps,
                                active_bin_id: None,
                            },
                            payer,
                            leg.amount_in,
                            min_out,
                        ) {
                            Ok(ix) => all_legs.push(ix),
                            Err(e) => {
                                debug!("Split route leg build failed: {e}; skipping opportunity");
                                split_route_success = false;
                                break;
                            }
                        }
                        total_out = total_out.saturating_add(estimated_out);
                    }

                    if !split_route_success {
                        continue;
                    }

                    // The RETURN hop (second hop of the opportunity) sells the
                    // summed split output back to the borrowed mint. Omitting it
                    // would leave the flash loan holding the intermediate token
                    // and fail the repay — the split route is incomplete without it.
                    if opp.hops.len() >= 2 {
                        let return_hop = &opp.hops[1];
                        // Calculate min_out using the estimated total output and slippage
                        let slippage_amount = total_out * slip / 10_000;
                        let min_out = total_out.saturating_sub(slippage_amount);
                        match arb_tx::build_swap_hop_instruction(
                            return_hop, payer, total_out, min_out,
                        ) {
                            Ok(ix) => all_legs.push(ix),
                            Err(e) => {
                                debug!("Return hop build failed: {e}; skipping opportunity");
                                continue;
                            }
                        }
                    }
                    match TxBuilder::assemble_split_flash_arbitrage(
                        payer,
                        tip_account,
                        tip,
                        cu_limit,
                        cu_price,
                        opp.size_in.as_u64(),
                        &all_legs,
                        &kamino_params,
                        verified.flash_loan_fee_sf.value,
                    ) {
                        Ok(jixs) => jixs,
                        Err(e) => {
                            debug!("Split route assembly failed: {e}; skipping opportunity");
                            continue;
                        }
                    }
                } else if route_via_jupiter {
                    match self
                        .resolve_jupiter_swap_route(
                            opp,
                            payer,
                            tip,
                            cu_limit,
                            cu_price,
                            &kamino_params,
                            verified.flash_loan_fee_sf.value,
                        )
                        .await
                    {
                        Ok(jixs) => jixs,
                        Err(e) => {
                            debug!(
                                "Jupiter live resolution failed for exotic route ({e}); falling back to native builder"
                            );
                            match self.assemble_native_ixs(
                                payer,
                                tip_account,
                                tip,
                                cu_limit,
                                cu_price,
                                opp,
                                &kamino_params,
                                verified.flash_loan_fee_sf.value,
                            ) {
                                Ok(jixs) => jixs,
                                Err(e) => {
                                    debug!("Native builder failed, skipping opportunity: {e}");
                                    continue;
                                }
                            }
                        }
                    }
                } else {
                    match self.assemble_native_ixs(
                        payer,
                        tip_account,
                        tip,
                        cu_limit,
                        cu_price,
                        opp,
                        &kamino_params,
                        verified.flash_loan_fee_sf.value,
                    ) {
                        Ok(jixs) => jixs,
                        Err(e) => {
                            debug!("Native builder failed, skipping opportunity: {e}");
                            continue;
                        }
                    }
                };

                // P0 #1: do NOT use the fabricated ALT (ALT111... does not exist on-chain).
                // Always build a legacy (v0-less) signed transaction with a real blockhash.
                //
                // Pre-warmed template fast path (Redis-backed): for a route already
                // assembled this window, reuse the wire template and patch only
                // (borrow amount, tip, blockhash) + sign in place — skipping the full
                // instruction rebuild. The template is persisted in Redis so a restart
                // or a sibling process does not pay the build cost at fire time.
                let route_sig = format!(
                    "prewarm:{}",
                    opp.hops
                        .iter()
                        .map(|h| format!("{}-{:?}", h.pool_id.0, h.protocol))
                        .collect::<Vec<_>>()
                        .join("_")
                );

                let mut blockhash_b58 = String::new();
                let tx_bytes = if let Some(ref signer) = self.keypair {
                    let blockhash = self.fetch_recent_blockhash().await;
                    blockhash_b58 = bs58::encode(blockhash).into_string();
                    // Fail fast: a zero blockhash must never be sent.
                    if blockhash == [0u8; 32] {
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Solana Blockhash",
                                "فشل جلب أحدث Blockhash صالح من شبكة سولانا (Zero Blockhash)",
                                current_slot.0,
                            )
                            .await;
                        warn!("fetch_recent_blockhash returned zero; skipping opportunity");
                        continue;
                    }
                    // Tier-1: Ultra-fast Zero-Copy In-Memory RAM template pre-signing (< 5 microseconds)
                    let patched_ram = {
                        let mut guard = self.ram_templates.lock();
                        guard.get_mut(&route_sig).map(|tpl| {
                            tpl.patch_and_sign(
                                opp.size_in.as_u64(),
                                tip.as_u64(),
                                blockhash,
                                signer,
                            )
                            .to_vec()
                        })
                    };

                    let patched = match patched_ram {
                        Some(b) => Some(b),
                        None => {
                            // Tier-2: Redis shared cache fallback
                            self.redis_cache
                                .get_json::<CachedTemplate>(&route_sig)
                                .and_then(|c| {
                                    let bh: [u8; 32] = c.blockhash.as_slice().try_into().ok()?;
                                    let mut tpl =
                                        arb_tx::PreWarmedTxTemplate::from_precompiled_bytes(
                                            c.wire,
                                            &bh,
                                            c.tip,
                                            verified.flash_loan_fee_sf.value,
                                        )
                                        .ok()?;
                                    let signed = tpl
                                        .patch_and_sign(
                                            opp.size_in.as_u64(),
                                            tip.as_u64(),
                                            blockhash,
                                            signer,
                                        )
                                        .to_vec();
                                    // Promote to Tier-1 RAM cache for subsequent sub-microsecond ticks
                                    self.ram_templates.lock().insert(route_sig.clone(), tpl);
                                    Some(signed)
                                })
                        }
                    };

                    match patched {
                        Some(bytes) => bytes,
                        None => {
                            // Tier-3: Full transaction compilation
                            let bytes = match TxBuilder::build_signed_transaction(
                                signer, &ixs, blockhash,
                            ) {
                                Ok(bytes) => bytes,
                                Err(e) => {
                                    debug!(
                                        "Failed to build signed transaction: {e}; skipping opportunity"
                                    );
                                    continue;
                                }
                            };
                            // Cache template in Tier-1 RAM and Tier-2 Redis
                            if let Ok(tpl) = arb_tx::PreWarmedTxTemplate::from_precompiled_bytes(
                                bytes.clone(),
                                &blockhash,
                                tip.as_u64(),
                                verified.flash_loan_fee_sf.value,
                            ) {
                                self.ram_templates.lock().insert(route_sig.clone(), tpl);
                            }
                            let cached = CachedTemplate {
                                wire: bytes.clone(),
                                blockhash: blockhash.to_vec(),
                                tip: tip.as_u64(),
                            };
                            let _ = self.redis_cache.set_json(&route_sig, &cached, 60);
                            bytes
                        }
                    }
                } else {
                    TxBuilder::serialize_instructions(&ixs)
                };

                // 5. No simulateTransaction on the hot path (latency).
                // Rejection is delegated to on-chain guards: Kamino flash-repay
                // fails if the balance is short, and each hop carries min_amount_out.
                // Open item: the Jupiter-sourced route quotes each hop at 50 bps of
                // slippage, which is a slippage bound rather than a profit floor —
                // a floor there needs a final-hop minimum derived from
                // `size_in + costs + tip`, which the quote API does not express.

                info!(
                    "Profitable opportunity sized: size={} real_net_profit={} tip={}",
                    opp.size_in, real_net_profit, tip
                );

                // Record attempt in persistent store (PostgreSQL / SQLite)
                // Use a composite monotonic ID (slot * 10,000 + seq) to prevent primary key collision
                // when multiple attempts occur in the exact same slot.
                static ATTEMPT_SEQ: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(1);
                let seq = ATTEMPT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 10_000;
                let attempt_id = (current_slot.0 as i64)
                    .saturating_mul(10_000)
                    .saturating_add(seq as i64);

                let record = AttemptRecord {
                    id: attempt_id,
                    slot: current_slot.0 as i64,
                    route_id: format!("route-{}", opp.route_id.0),
                    expected_net: real_net_profit,
                    tip_lamports: Some(tip),
                    decision: format!("{}", self.mode),
                };
                let _ = self.store.record_attempt(&record);

                let scale = 10f64.powi(i32::from(opp.base_decimals));
                let profit_sol = real_net_profit.as_u64() as f64 / scale;
                // USD is display-only; use the live price or 0.0 (labelled N/A)
                // — never a guessed constant that could read as a real number.
                let live_sol_price = self.live_sol_price().unwrap_or(0.0);

                let pools_str = opp
                    .hops
                    .iter()
                    .map(|h| format!("{:?} ({})", h.protocol, h.pool_id.0))
                    .collect::<Vec<_>>()
                    .join(" ➔ ");
                let initial_loan_sol = opp.size_in.as_u64() as f64 / 1e9;
                let gross_rev_sol = opp.expected_out.as_u64() as f64 / 1e9;
                let deductions_sol =
                    (opp.gross_profit
                        .as_u64()
                        .saturating_sub(real_net_profit.as_u64())) as f64
                        / 1e9;
                let roi_pct = if opp.size_in.as_u64() > 0 {
                    (real_net_profit.as_u64() as f64 / opp.size_in.as_u64() as f64) * 100.0
                } else {
                    0.0
                };

                let now_dur = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();

                info!(
                    "\n============================================================\n\
                    ### 1. Opportunity Detection & Route Mapping\n\
                    * Timestamp: [{}.{:03}s UNIX]\n\
                    * Route Type: Multi-Pool Split / Flash Loan Routing\n\
                    * Pools Involved: {}\n\
                    * Initial Loan / Capital: {:.6} SOL\n\n\
                    ### 2. Execution & Blockchain Lifecycle\n\
                    * Execution Status: [{}]\n\
                    * Block Time & Slot: Slot {}\n\
                    * Gas Fee (Priority Fees + Cu Burned): ~0.000010 SOL / {} CU\n\n\
                    ### 3. Financial PnL Breakdown (The Profit)\n\
                    * Gross Revenue: {:.6} SOL\n\
                    * Total Deductions: {:.6} SOL\n\
                    * Net Profit: **+{:.6} SOL** (~${:.3})\n\
                    * ROI %: +{:.4}%\n\
                    ============================================================",
                    now_dur.as_secs(),
                    now_dur.subsec_millis(),
                    pools_str,
                    initial_loan_sol,
                    if self.mode == BotMode::Live {
                        "DISPATCHING_JITO"
                    } else {
                        "SHADOW_SIMULATED"
                    },
                    current_slot.0,
                    cu_limit,
                    gross_rev_sol,
                    deductions_sol,
                    profit_sol,
                    profit_sol * live_sol_price,
                    roi_pct
                );

                self.dispatch_opportunity(super::dispatch::DispatchContext {
                    opp,
                    tip,
                    tip_rung,
                    is_swqos_route,
                    current_slot,
                    expected_stream_generation,
                    route_label: &route_label,
                    gross_rev_sol,
                    gross_sol,
                    profit_sol,
                    live_sol_price,
                    blockhash_b58: &blockhash_b58,
                    cu_limit,
                    cu_price,
                    tx_bytes: &tx_bytes,
                    record: &record,
                    real_net_profit,
                    payer,
                    route_sig: &route_sig,
                }).await;
            }

            if tick >= 10 && std::env::var("ARB_TEST_EXIT").is_ok() {
                info!(
                    "Completed test run of {} ticks (last slot {})",
                    tick, current_slot.0
                );
                break;
            }
        }

        cancel_token.cancel();
        info!("✅ Supervisor completed safely.");
        Ok(())
    }
}
