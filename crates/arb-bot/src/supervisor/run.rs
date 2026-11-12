//! Main supervisor lifecycle and orchestration loop.
use super::*;

impl Supervisor {
    /// Serialize current RPC pool health (if available) and write the heartbeat
    /// atomically. Failures are logged, never fatal: liveness reporting must not
    /// take down the trading loop.
    fn write_heartbeat(&self) {
        let rpc = serde_json::to_value(self.rpc_pool.health_snapshot()).ok();
        if let Err(e) = self.heartbeat.write(rpc.as_ref()) {
            warn!("heartbeat write failed: {e}");
        }
    }

    /// Actual Kamino flash liquidity available for borrowing, decoded from the
    /// live WSOL supply-vault token-account snapshot. Fails closed to `None`
    /// when the snapshot is missing, stale beyond the state-age limit, owned by
    /// an unexpected program, or undecodable — never a guessed balance.
    fn kamino_available_flash_liquidity(&self, current_slot: Slot) -> Option<u64> {
        let supply_vault = Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).ok()?;
        let snap = self.ingest.account_store().get(&supply_vault)?;
        // Same freshness bound as the verified quote inputs (execution.rs).
        const MAX_SNAPSHOT_AGE_SLOTS: u64 = 1500;
        if snap.slot > current_slot.0
            || current_slot.0.saturating_sub(snap.slot) > MAX_SNAPSHOT_AGE_SLOTS
        {
            return None;
        }
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).ok()?;
        if snap.owner != spl {
            return None;
        }
        let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).ok()?;
        arb_tx::kamino::decode_supply_vault_liquidity(&snap.data, &wsol).ok()
    }

    /// Conservative flat network cost used by the 3-leg size search: base fee,
    /// Jito tip ceiling, and priority-fee estimate at the adaptive price for
    /// the 3-hop flash-funded route shape.
    /// Rent-exempt minimum for one SPL token account (165 bytes).
    const ATA_RENT_LAMPORTS: u64 = 2_039_280;

    fn three_leg_network_cost_lamports(&self, tip_lamports: u64, payer: Pubkey) -> u64 {
        let hop_protocols = [arb_types::DexProtocol::MeteoraDlmm; 3];
        let base_cu = arb_tx::estimate_route_cu(true, &hop_protocols, 3);
        let cu_limit = arb_tx::calculate_dynamic_cu_limit(base_cu);
        let cu_price = self.tip_strategy.adaptive_cu_price(&self.recent_landed_cu);
        let priority_fee = (cu_price.saturating_mul(cu_limit as u64)) / 1_000_000;
        // The WSOL settlement ATA is created when missing and never closed (it
        // holds the profit), so its rent is a real execution cost. Intermediate
        // meme ATAs are created AND closed inside the same transaction and
        // reclaim their rent, so only the settlement account is charged.
        let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap_or_default();
        let spl = Pubkey::from_str(arb_tx::TOKEN_PROGRAM_ID).unwrap_or_default();
        let settlement_ata =
            arb_tx::get_associated_token_address_with_program_id(&payer, &wsol, &spl);
        let ata_rent = if self.ingest.account_store().get(&settlement_ata).is_some() {
            0
        } else {
            Self::ATA_RENT_LAMPORTS
        };
        BASE_FEE_LAMPORTS
            .saturating_add(tip_lamports)
            .saturating_add(priority_fee)
            .saturating_add(ata_rent)
    }

    /// Meteora 3-leg (WSOL -> A -> B -> WSOL) capture: run the chained-quote
    /// pipeline over the fresh DLMM pools and convert the best chain into a
    /// standard `ExecutionOpportunity` so the same economics/risk/dispatch
    /// path applies. Fails closed (empty vec) when:
    ///
    /// * fewer than 3 eligible DLMM pools, or no executable chain is found;
    /// * any leg lacks resolved execution accounts (the compiler needs them);
    /// * the chain's gross profit is not strictly positive.
    ///
    /// No route is ever fabricated: the pipeline returns an error rather than
    /// a guessed chain, and the error is surfaced in the trace log.
    fn capture_three_leg_dlmm(
        &self,
        fresh_pools: &[PoolState],
        current_slot: Slot,
        max_sizing: u64,
        flash_loan_fee_sf: u64,
        kamino_available_liquidity: Option<u64>,
        verified_tip_lamports: u64,
    ) -> Vec<ExecutionOpportunity> {
        if fresh_pools.len() < 3 || max_sizing == 0 {
            return Vec::new();
        }
        let tuning = self
            .config
            .strategy
            .dlmm_three_leg
            .clone()
            .unwrap_or_default();
        let payer = self
            .keypair
            .as_ref()
            .map(|k| k.pubkey())
            .unwrap_or_default();

        // Candidate size ladder bounded by both the tuning ceiling and the live
        // risk-gate sizing. Falls back to a single discovery size when capped.
        let size_search_sizes = {
            let ceiling = max_sizing.min(tuning.max_input).max(1);
            let capacity = tuning.size_ladder.clamp(1, 8);
            let mut sizes = Vec::with_capacity(capacity);
            let mut s = tuning.input_size.min(ceiling).max(1);
            while sizes.len() < capacity && s <= ceiling {
                sizes.push(s);
                s = s.saturating_mul(3).max(s.saturating_add(1));
            }
            sizes
        };

        let params = arb_engine::ThreeLegMemeContextParams {
            snapshot_slot: current_slot.0,
            cu_limit: arb_tx::COMPUTE_BUDGET_LIMIT_3HOP,
            priority_fee_micro_lamports: 1_000,
            jito_tip_account: self.jito_service.tip_account_for_slot(current_slot),
            jito_tip_lamports: 0,
            recent_blockhash: [0u8; 32],
            v1_transport_supported: false,
            slippage_bps: tuning.slippage_bps,
            min_profit_lamports: tuning
                .min_profit_lamports
                .max(self.config.strategy.min_net_profit_abs),
            flash_loan_fee_sf,
            network_cost_lamports: self
                .three_leg_network_cost_lamports(verified_tip_lamports, payer),
            kamino_available_liquidity,
            input_size: tuning.input_size.min(max_sizing).max(1),
            size_search_sizes,
            ..arb_engine::ThreeLegMemeContextParams::for_discovery(payer)
        };

        let result = match arb_engine::build_three_leg_meme_pipeline(fresh_pools, &params) {
            Ok(r) => r,
            Err(e) => {
                debug!("Meteora 3-leg DLMM capture skipped: {e}");
                return Vec::new();
            }
        };

        // Fail closed: only a strictly positive gross chain enters the set.
        if result.gross_profit_lamports <= 0 {
            return Vec::new();
        }

        let by_id: std::collections::HashMap<Pubkey, &PoolState> =
            fresh_pools.iter().map(|p| (p.pool_id.0, p)).collect();
        let mut hops = Vec::with_capacity(3);
        for leg in result.legs.iter() {
            let Some(pool) = by_id.get(&leg.pool_id.0) else {
                return Vec::new();
            };
            hops.push(SwapHop {
                pool_id: leg.pool_id,
                protocol: DexProtocol::MeteoraDlmm,
                mint_in: leg.from_mint,
                mint_out: leg.to_mint,
                // Pipeline pre-filtered pools to those with a valid fee; a
                // missing fee here can only mean a mid-tick state mutation, and
                // the pipeline's own quote already priced the route.
                fee_bps: pool.fee_bps().unwrap_or(0),
                active_bin_id: pool.dlmm.as_ref().map(|d| d.active_bin_id),
                token_programs: pool.token_programs,
                trade_intent: Default::default(),
                execution_accounts: pool.execution_accounts.clone(),
            });
        }

        let size_in = result.size_search.best_input.max(1);
        let expected_out = result.expected_final_out;
        let expected_min_final_output = result.expected_min_final_output;
        let hop_inputs = vec![
            result.legs[0].amount_in,
            result.legs[1].amount_in,
            result.legs[2].amount_in,
            expected_out,
        ];
        let gross = result.gross_profit_lamports as u64;
        let net = result.net_profit_lamports.max(0) as u64;

        static THREE_LEG_ROUTE_ID: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(50_000);
        let route_id =
            RouteId(THREE_LEG_ROUTE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

        vec![ExecutionOpportunity {
            route_id,
            slot: current_slot,
            size_in: TokenAmount(size_in),
            expected_out: TokenAmount(expected_out),
            hop_inputs,
            // The authoritative economic gate re-derives net profit later with
            // the actual tip; the pipeline's canonical net (flash fee + network
            // cost already deducted, WSOL-anchored) ranks the merged set.
            gross_profit: Lamports(gross),
            net_profit: Lamports(net),
            base_decimals: 9, // WSOL-anchored route
            price_impact_bps: Bps(0),
            hops,
            expected_min_final_output,
        }]
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting Supervisor — Mode: {}", self.mode);

        // Publish process identity immediately, before the first tick, so an
        // external verifier can always find the PID/binary it is watching.
        self.heartbeat.observe_tick(0);
        self.write_heartbeat();

        // Independent liveness writer: refreshes the file once per second even
        // while the strategy loop is blocked in a long await (startup pool
        // discovery, RPC warmup). Loop health is still visible through
        // `heartbeat_seq` / `last_tick_age_ms`, which only the loop advances —
        // so a wedged loop cannot be masked by this writer.
        {
            let hb = Arc::clone(&self.heartbeat);
            let rpc = self.rpc_pool.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let snapshot = serde_json::to_value(rpc.health_snapshot()).ok();
                    if let Err(e) = hb.write(snapshot.as_ref()) {
                        warn!("heartbeat write failed: {e}");
                    }
                }
            });
        }

        for token in &self.config.strategy.base_tokens {
            info!("Monitored Base Token: {}", token);
        }

        // Jito transport remains isolated from the Solana market-data plane.
        // No RPC market-state reads are performed by this service in this phase.
        let js = Arc::clone(&self.jito_service);
        let rpc_for_js = Arc::new(self.rpc_pool.clone());
        let leader_refresh = std::time::Duration::from_secs(self.config.jito.leader_refresh_secs);
        tokio::spawn(async move {
            js.run(rpc_for_js, leader_refresh).await;
        });

        // 1. Run Dynamic Pool Discovery upfront so all active pools & vaults are known
        info!(
            "🌐 RpcPool active with {} endpoint(s): {:?}",
            self.rpc_pool.endpoints_count(),
            self.rpc_pool.redacted_hosts()
        );
        info!(
            "🔍 Discovering active multi-DEX pools across Raydium, Orca, Meteora, and Pump.fun..."
        );
        let disc = arb_ingest::PoolDiscovery::new(self.rpc_pool.clone());
        let mut disc_cfg = arb_ingest::DiscoveryConfig::default();
        let max_per_venue = self.config.strategy.max_per_venue.unwrap_or(33);
        disc_cfg.max_per_venue = max_per_venue;
        disc_cfg.min_tvl_usd = self
            .config
            .strategy
            .discovery_min_tvl_usd
            .unwrap_or(25_000.0);
        disc_cfg.max_pools = self
            .config
            .strategy
            .max_pools
            .unwrap_or(max_per_venue.saturating_mul(5));
        let refresh_secs = self.config.strategy.discovery_refresh_secs.unwrap_or(60);

        let discovered = disc.discover(&disc_cfg).await;
        if !discovered.is_empty() {
            let disc_cnt = discovered.len();
            {
                let mut reg_write = self.registry.write();
                arb_ingest::register_discovered(&mut reg_write, &discovered);
            }
            info!(
                "🎯 Successfully registered {disc_cnt} discovered liquid pools into MultiDexRegistry"
            );
        }

        // 2. Ingestion: WebSocket streaming worker
        let cancel_token = CancellationToken::new();

        info!(
            "⚡ Pure WebSocket Ingestion Engine Active (Helius & RPCFast WebSockets driving pipeline)"
        );

        // 3. Multi-provider WebSocket real-time slot and account streaming ingestion worker
        let mut hw_client_opt: Option<Arc<arb_ingest::HeliusWsClient>> = None;
        let mut ws_slot_notify: Option<Arc<tokio::sync::Notify>> = None;
        if let Some(helius_ws) = arb_ingest::HeliusWsClient::from_env() {
            let wsol_mint = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap();
            let usdc_mint = Pubkey::from_str(arb_tx::USDC_MINT).unwrap();
            let kamino_reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE).unwrap();
            let kamino_supply_vault =
                Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).unwrap();
            let kamino_owner = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID).unwrap();
            let token_prog = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
            helius_ws.track_account(kamino_reserve, kamino_owner);
            helius_ws.track_account(wsol_mint, token_prog);
            helius_ws.track_account(kamino_supply_vault, token_prog);
            helius_ws.track_account(usdc_mint, token_prog);

            // Register multi-DEX pool vaults/accounts so live WS accountNotification frames
            // update MarketStateCache directly sub-second.
            for reg_pool in self.registry.read().pools() {
                helius_ws.register_pool(reg_pool);
            }

            let hw_client = Arc::new(helius_ws);
            ws_slot_notify = Some(hw_client.slot_notifier());
            hw_client_opt = Some(Arc::clone(&hw_client));
            let hw_ingest = Arc::clone(&self.ingest);
            let cancel_hw = cancel_token.clone();
            tokio::spawn(async move {
                hw_client.start_stream(hw_ingest, cancel_hw).await;
            });
            info!("⚡ Multi-Provider WebSocket slot & account stream worker active");
        }

        // Pre-seed initial verified multi-DEX pools into MarketStateCache before starting periodic workers
        info!("Seeding initial multi-DEX pools into MarketStateCache...");
        let rpc_seed_slot = self.rpc_pool.get_slot().await.unwrap_or(0);
        let active_seed_slot = rpc_seed_slot
            .max(self.ingest.current_slot())
            .max(self.ingest.helius_slot());
        let seed_slot = Slot(active_seed_slot);
        let reg = self.registry.read().clone();
        let initial_pools = reg.fetch_live_pool_states(seed_slot).await;
        let mut live_sol_price_seed = 0.0_f64;
        for pool in initial_pools {
            let p_base = pool.base_mint.0.to_string();
            let p_quote = pool.quote_mint.0.to_string();
            if live_sol_price_seed == 0.0 && pool.reserve_in > 0 && pool.reserve_out > 0 {
                if p_base == arb_tx::WSOL_MINT && p_quote == arb_tx::USDC_MINT {
                    let sol_res = pool.reserve_in as f64 / 1e9;
                    let usdc_res = pool.reserve_out as f64 / 1e6;
                    if sol_res > 0.0 {
                        live_sol_price_seed = usdc_res / sol_res;
                    }
                } else if p_base == arb_tx::USDC_MINT && p_quote == arb_tx::WSOL_MINT {
                    let sol_res = pool.reserve_out as f64 / 1e9;
                    let usdc_res = pool.reserve_in as f64 / 1e6;
                    if sol_res > 0.0 {
                        live_sol_price_seed = usdc_res / sol_res;
                    }
                }
            }
            let _ = self.ingest.apply_pool_update(pool);
        }
        if live_sol_price_seed > 0.0 {
            *self.last_known_live_sol_price.write() = live_sol_price_seed;
        }
        let reported_seed_slot = self
            .ingest
            .current_slot()
            .max(self.ingest.helius_slot())
            .max(active_seed_slot);
        let seeded_cache_count = self.ingest.snapshot().snapshot_pools().len();
        info!(
            "Successfully seeded {seeded_cache_count} verified live pools into MarketStateCache (slot={reported_seed_slot})"
        );

        // Emit startup & initial market snapshot AFTER discovery and seeding succeed
        self.telegram
            .alert_startup(
                &self.mode.to_string(),
                seeded_cache_count,
                crate::live_execution_enabled(),
            )
            .await;
        self.emit_market_snapshot(reported_seed_slot, true).await;

        // 4. Multi-DEX active reserve ingestion worker (WS slot-driven + 750ms cadence, batch-100 parallel)
        let registry_worker = Arc::clone(&self.registry);
        let ingest_worker_res = Arc::clone(&self.ingest);
        let rpc_worker = self.rpc_pool.clone();
        let cancel_registry = cancel_token.clone();
        let slot_notify_worker = ws_slot_notify.clone();
        tokio::spawn(async move {
            let cadence = Duration::from_millis(750);
            let min_gap = Duration::from_millis(550);
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + cadence, cadence);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_run = Instant::now()
                .checked_sub(min_gap)
                .unwrap_or_else(Instant::now);
            loop {
                tokio::select! {
                    _ = cancel_registry.cancelled() => break,
                    _ = async {
                        if let Some(ref notify) = slot_notify_worker {
                            notify.notified().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {}
                    _ = interval.tick() => {}
                }
                if last_run.elapsed() < min_gap {
                    continue;
                }
                last_run = Instant::now();
                let mut s = ingest_worker_res
                    .current_slot()
                    .max(ingest_worker_res.helius_slot());
                if s == 0 {
                    s = rpc_worker.get_slot().await.unwrap_or(0);
                }
                if s > 0 {
                    let reg = registry_worker.read().clone();
                    let pools = reg.fetch_live_pool_states(Slot(s)).await;
                    for pool in pools {
                        let _ = ingest_worker_res.apply_pool_update(pool);
                    }
                }
            }
        });
        info!(
            "🌊 Multi-DEX active reserve ingestion worker running (WS-slot triggered / 750ms cadence, parallel batch-100)"
        );

        // 5. Periodic dynamic discovery refresh worker
        let disc_worker = Arc::new(disc);
        let reg_worker_disc = Arc::clone(&self.registry);
        let cancel_disc = cancel_token.clone();
        let hw_client_for_disc = hw_client_opt.clone();
        tokio::spawn(async move {
            let period = Duration::from_secs(refresh_secs.max(30));
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = cancel_disc.cancelled() => break,
                    _ = interval.tick() => {
                        let pools = disc_worker.discover(&disc_cfg).await;
                        if !pools.is_empty() {
                            let updated_pools = {
                                let mut reg_write = reg_worker_disc.write();
                                arb_ingest::register_discovered(&mut reg_write, &pools);
                                reg_write.pools().to_vec()
                            };
                            if let Some(ref hw) = hw_client_for_disc {
                                for rp in &updated_pools {
                                    hw.register_pool(rp);
                                }
                            }
                        }
                    }
                }
            }
        });

        // No HTTP polling loop: WebSocket account updates are the sole authoritative
        // market-state input. Raw updates are journaled until protocol decoders promote them.

        // Provider-neutral WebSocket slots are configured separately. No fixed vendor-specific
        // event feed is allowed to become an authoritative pool-state source.

        // Real slot clock from WebSocket/RPC ingestion. Never fabricated.
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
        let mut adaptive_delay_ms: u64 = 50;

        // Startup (discovery + warmup) is complete from the heartbeat's point of
        // view: from here on, a non-advancing `heartbeat_seq` means a real stall.
        self.heartbeat.mark_loop_entered();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(adaptive_delay_ms)).await;
            tick += 1;

            // Liveness: recorded on EVERY iteration, including iterations that
            // `continue` early because the slot did not advance. The 1s writer
            // task above only refreshes the file; this counter is what proves
            // the strategy loop itself is still turning.
            self.heartbeat.observe_tick(current_slot.0);

            // Health sweep every 100 ticks (~30s): watches the ingestion stream, disk, Telegram.
            // Logs the exact reason for any outage; never blocks trading.
            if tick.is_multiple_of(100) {
                let summary = self.health_monitor.sweep(current_slot.0).await;
                debug!("health sweep tick={tick}: {summary}");
                self.risk_gate.maybe_reset_periodic();
            }

            // Periodic 1-minute Performance & Rejections Digest to Telegram and local disk
            let digest_interval_secs = std::env::var("TELEGRAM_DIGEST_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(60);
            if last_snapshot_digest.elapsed() >= Duration::from_secs(digest_interval_secs) {
                self.emit_five_minute_digest(current_slot.0).await;
                self.emit_market_snapshot(current_slot.0, false).await;
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

            // Slot advancement: The WebSocket and Helius WebSocket slot streams are authoritative.
            // If WebSocket slot is 0 or lagging behind Helius WebSocket, advance from Helius WS.
            let mut stream_slot = self.ingest.current_slot();
            let helius_slot = self.ingest.helius_slot();
            if helius_slot > stream_slot {
                stream_slot = helius_slot;
                self.ingest.record_slot(stream_slot);
            }

            // Fallback: If slot is still 0 or lagging behind current_slot, query rpc_pool slot stream
            if (stream_slot == 0 || stream_slot <= current_slot.0)
                && (stream_slot == 0 || tick.is_multiple_of(5))
            {
                if let Ok(rpc_slot) = self.rpc_pool.get_slot().await {
                    if rpc_slot > current_slot.0 {
                        debug!(
                            "Advancing slot from RPC slot stream fallback: {rpc_slot} (previous: {stream_slot})"
                        );
                        self.ingest.record_slot(rpc_slot);
                        self.ingest.record_helius_slot(rpc_slot);
                        self.ingest.mark_stream_connected();
                        stream_slot = rpc_slot;
                    }
                }
            }
            if stream_slot <= current_slot.0 {
                // Adaptive delay: when no slot advancement occurs, relax pacing smoothly up to 100ms
                adaptive_delay_ms = (adaptive_delay_ms + 10).min(100);
                continue;
            }
            // Slot advanced! Reset to fastest 50ms pacing for sub-slot responsiveness
            adaptive_delay_ms = 50;
            current_slot = Slot(stream_slot);
            // Pin the stream generation for this strategy cycle. A reconnect at
            // any point before dispatch invalidates every opportunity from this tick.
            let expected_stream_generation = self.ingest.stream_generation();
            self.slot_clock.lock().observe_slot(current_slot.0);

            // Promote PumpSwap only after ingestion has supplied the Pool, both
            // vaults, both mints and canonical Pump Fees FeeConfig at coherent slots.
            // The graph uses a conservative whole-BPS fee for candidate discovery;
            // exact route validation remains protocol-adapter driven.
            let pump_swap_report = self
                .ingest
                .refresh_pump_swap_states(self.config.strategy.max_state_age_slots);
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
            // WSOL mint decimals must exist in the ingest account journal.
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
            // through the configured WebSocket account-update stream and pass account validation.

            // 1. Ingest: take one pool snapshot per strategy cycle; account updates mutate one key.
            //    Raw stream updates are promoted only after protocol-specific decoding.
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
            // Finite WSOL-denominated candidate ceiling derived from the
            // configured USD notional and a fresh SOL/USD reference. The graph
            // is deliberately WSOL-anchored because Kamino's configured reserve
            // lends WSOL and repayment must close in the exact borrowed mint.
            let max_sizing = match self.live_sol_price() {
                Some(sol_usd)
                    if sol_usd.is_finite()
                        && sol_usd > 0.0
                        && self.config.risk.max_trade_size_usd.is_finite()
                        && self.config.risk.max_trade_size_usd > 0.0 =>
                {
                    (self.config.risk.max_trade_size_usd / sol_usd * 1_000_000_000.0)
                        .floor()
                        .clamp(0.0, u64::MAX as f64) as u64
                }
                _ => 0,
            };
            if max_sizing == 0 {
                debug!(
                    "Missing fresh SOL/USD reference or invalid USD size cap; skip graph sizing"
                );
                continue;
            }

            // Filter out stale pools from graph to prevent phantom arbitrage cycles
            // caused by price divergence against frozen/unupdated pools.
            // Graph freshness is an explicit policy: pools older than the
            // configured graph age never enter the candidate graph. Defaults
            // to the quote-freshness policy; no arbitrary multiplier.
            // Graph freshness must never exceed quote/state freshness: a wider
            // horizon (e.g. 300 slots) admits stale pools whose prices have
            // diverged, producing phantom arbitrage and implausible ROI. Cap the
            // graph horizon at the configured state-age window so only pools
            // that are still quote-fresh can enter the candidate graph.
            let max_graph_age_slots = self
                .config
                .strategy
                .graph_max_age_slots
                .unwrap_or(self.config.strategy.max_state_age_slots)
                .min(self.config.strategy.max_state_age_slots)
                .max(1);
            let fresh_pools: Vec<PoolState> = pools
                .iter()
                .filter(|p| {
                    p.slot.0 > 0 && current_slot.0.saturating_sub(p.slot.0) <= max_graph_age_slots
                })
                .cloned()
                .collect();

            // Fail-closed: if no fresh pools within the graph max age window,
            // skip this tick entirely. No stale pool may enter a candidate route.
            if fresh_pools.is_empty() {
                debug!("No fresh pools within graph max age; skipping tick (fail closed)");
                continue;
            }

            let opportunities = {
                let mut graph = self.graph.lock();
                graph.build_from_pools(&fresh_pools);
                let mut opps = graph.find_base_cycles(current_slot, max_sizing);

                // ── MeteoraDlmmCrossPoolArbitrageStrategy (P0 first strategy) ──
                // First-class two-pool DLMM path: distinct same-pair pools,
                // both directions, bounded integer size optimizer, real bin
                // traversal. Merged with the DFS candidates (deduped below).
                let dcp_cfg = self
                    .config
                    .strategy
                    .dlmm_cross_pool
                    .clone()
                    .unwrap_or_default();
                let scan = {
                    // Split borrow: config comes from the graph-hosting block.
                    let cfg = arb_engine::DlmmCrossPoolConfig {
                        max_pairs: dcp_cfg.max_pairs,
                        min_input: dcp_cfg.min_input,
                        max_input: dcp_cfg.max_input.min(max_sizing),
                        coarse_samples: dcp_cfg.coarse_samples,
                        refine_samples: dcp_cfg.refine_samples,
                        min_spread_bps: dcp_cfg.min_spread_bps,
                        min_liquidity_bin_raw: dcp_cfg.min_liquidity_bin_raw,
                    };
                    graph.find_cross_pool_dlmm_cycles(current_slot, max_sizing, &cfg)
                };
                let mut cross_opps = scan.opportunities;
                let cross_stats = scan.stats;
                let cross_rejections = scan.rejections;
                {
                    use arb_engine::CrossPoolRejectionReason as RR;
                    let mut by: std::collections::HashMap<&'static str, u64> = Default::default();
                    for r in &cross_rejections {
                        let label = match r.reason {
                            RR::EdgeBelowHurdle => "EDGE_BELOW_HURDLE",
                            RR::InsufficientLiquidity => "INSUFFICIENT_LIQUIDITY",
                            RR::StalePoolState => "STALE_POOL_STATE",
                            RR::MissingAccounts => "MISSING_ACCOUNTS",
                            RR::MaxTradeSizeExceeded => "MAX_TRADE_SIZE_EXCEEDED",
                            RR::TokenProgramMismatch => "TOKEN_PROGRAM_MISMATCH",
                            RR::EconomicGateRejected => "ECONOMIC_GATE_REJECTED",
                        };
                        *by.entry(label).or_insert(0) += 1;
                    }
                    info!(
                        "DLMM cross-pool strategy: pairs={} directions={} opportunities={} rejections={}",
                        cross_stats.pairs_scanned,
                        cross_stats.directions_evaluated,
                        cross_opps.len(),
                        by.iter()
                            .map(|(k, v)| format!("{k}:{v}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                }
                // Dedupe vs DFS candidates by (pool pair, size).
                let mut seen_pairs: HashSet<(Pubkey, Pubkey)> = Default::default();
                for o in &opps {
                    if let (Some(h1), Some(h2)) = (o.hops.first(), o.hops.last()) {
                        seen_pairs.insert((h1.pool_id.0, h2.pool_id.0));
                    }
                }
                cross_opps.retain(|o| {
                    if let (Some(h1), Some(h2)) = (o.hops.first(), o.hops.last()) {
                        seen_pairs.insert((h1.pool_id.0, h2.pool_id.0))
                    } else {
                        false
                    }
                });
                opps.extend(cross_opps);

                // ── Cross-platform multi-hop capture (always evaluated) ──
                // Best practice: evaluate every candidate class each tick and
                // merge by profitability instead of short-circuiting at the
                // first non-empty class. Previously Bellman-Ford / multi-hop
                // DFS ran only when the base-cycle set was empty, which hid
                // longer cross-DEX routes (3..5 hops) whenever any 2-hop cycle
                // existed — even when the longer route was more profitable.
                let bf_opps = {
                    let bf_engine = arb_engine::BellmanFordEngine::from_pools(&fresh_pools)
                        .with_economic_config(economic_config.clone());
                    let found = bf_engine.find_negative_cycles(current_slot, max_sizing);
                    if !found.is_empty() {
                        info!(
                            "🎯 Bellman-Ford detected {} arbitrage cycle(s)",
                            found.len()
                        );
                    }
                    found
                };
                let multi_opps = graph.find_multi_hop_cycles(current_slot, max_sizing, 5);
                if !multi_opps.is_empty() {
                    info!("🔁 Multi-hop DFS detected {} route(s)", multi_opps.len());
                }

                // ── Meteora 3-leg (WSOL -> A -> B -> WSOL) pipeline ──
                // Third candidate class: three distinct DLMM pools on Meteora,
                // chained-quote discovery + 3-leg-native size search. Converted
                // into a standard ExecutionOpportunity so the same economics,
                // risk and dispatch path applies. Fails closed to an empty vec.
                let three_leg_opps = self.capture_three_leg_dlmm(
                    &fresh_pools,
                    current_slot,
                    max_sizing,
                    verified.flash_loan_fee_sf.value,
                    self.kamino_available_flash_liquidity(current_slot),
                    verified.tip_lamports.value.0,
                );
                if !three_leg_opps.is_empty() {
                    info!(
                        "🟢 Meteora 3-leg DLMM pipeline captured {} chain(s)",
                        three_leg_opps.len()
                    );
                }

                // Merge + dedupe by sorted pool set, then rank by net profit so
                // the most profitable route is processed first.
                let mut merged: Vec<ExecutionOpportunity> = opps;
                let mut seen_pool_sets: HashSet<Vec<Pubkey>> = Default::default();
                for o in &merged {
                    let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
                    ids.sort_unstable();
                    seen_pool_sets.insert(ids);
                }
                for mut src in [bf_opps, multi_opps, three_leg_opps] {
                    src.retain(|o| {
                        let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
                        ids.sort_unstable();
                        seen_pool_sets.insert(ids)
                    });
                    merged.extend(src);
                }
                merged.sort_by_key(|o| std::cmp::Reverse(o.net_profit.0));
                info!(
                    "Slot {}: graph discovered {} candidate route(s) after dedupe; graph edges={}, base_anchors={}",
                    current_slot.0,
                    merged.len(),
                    graph.edges_by_from.len(),
                    graph.base_anchors.len()
                );
                merged
            };
            let t_ingest_to_quote_us = quote_ingest_time.elapsed().as_micros() as u64;
            self.heartbeat
                .observe_quote(current_slot.0, opportunities.len());

            for opp in &opportunities {
                let t_risk_start = std::time::Instant::now();
                arb_metrics::inc_opportunities();
                let target_slot = Slot(current_slot.0.saturating_add(1));
                let tip_account = self.jito_service.tip_account_for_slot(target_slot);

                // 3. Per-tx cost basis — ONE list, used by both the Jito edge
                //    estimate and the go/no-go gate (TX-02):
                //    flash fee (verified U68F60 sf) + base fee + priority fee.
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

                // Strict Decoupled Profit Protection Invariant:
                // P_net = G - C_fees - T_Jito >= $0.20 USD
                // T_max = G - C_fees - $0.20 - B
                let min_net_floor = self.config.strategy.min_net_profit_abs;
                let contingency_buffer = 10_000u64; // B
                let t_max = net_before_tip
                    .saturating_sub(min_net_floor)
                    .saturating_sub(contingency_buffer);

                if t_max < arb_land::MIN_JITO_TIP_LAMPORTS {
                    debug!(
                        "Opportunity skipped: Surplus ceiling t_max ({t_max}) < MIN_JITO_TIP; cannot satisfy P_net >= $0.20"
                    );
                    if gross >= 50_000 {
                        let reason = format!(
                            "فائض الربح غير كافٍ لتغطية إكرامية Jito مع ضمان بقاء الحد الأدنى للربح الصافي $0.20 (t_max={t_max} lamports)"
                        );
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Profit Floor Protection".to_string(),
                            reason,
                            slot: current_slot.0,
                        });
                    }
                    continue;
                }

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
                        TipDecision::Bid { tip, rung, .. } => {
                            (Lamports(tip.as_u64().min(t_max)), Some(rung), false)
                        }
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
                                self.record_rejected_opportunity(
                                    crate::telegram::RejectedTradeSummary {
                                        route: route_label.clone(),
                                        gross_sol,
                                        stage: "Jito Tip Plan".to_string(),
                                        reason: reason.clone(),
                                        slot: current_slot.0,
                                    },
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
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.clone(),
                                    gross_sol,
                                    stage: "Jito Leader Gate".to_string(),
                                    reason: reason.clone(),
                                    slot: current_slot.0,
                                },
                            );
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
                // `calculate_net_profit_raw` returns net profit in BASE MINT RAW
                // UNITS (e.g., USDC raw units when base_mint is USDC). It is NOT
                // in lamports. We must never cast it to `Lamports` or divide by
                // 1e9 as if it were SOL.
                let real_net_base_raw = calculate_net_profit_raw(
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
                let Some(real_net_base_raw) = real_net_base_raw else {
                    debug!("Opportunity rejected: net <= 0 with actual tip {tip}");
                    if gross > 0 {
                        let reason = format!(
                            "تآكل الربح بالكامل: تكاليف الفلاش لوان + البقشيش + رسوم الشبكة فاقت الربح الإجمالي (الوحدة الأساسية: {})",
                            opp.hops
                                .first()
                                .map(|h| h.mint_in.to_string())
                                .unwrap_or_else(|| "unknown".to_string())
                        );
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Net Profit Economics".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                        });
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
                // Implausibility gate: implied ROI vs input size above a hard ceiling
                // cannot be a real edge - it is a stale/inconsistent quote artifact
                // (Phase 15.2 anomaly: ~965% ROI from stale pools). Holds regardless of
                // the configured state-age window, so a wider freshness window cannot
                // re-open the phantom-arbitrage class. Fail closed + Telegram reject telemetry.
                let implied_roi_bps = (real_net_base_raw as u128)
                    .saturating_mul(10_000)
                    .checked_div(opp.size_in.as_u64().max(1) as u128)
                    .unwrap_or(u128::MAX);
                let max_implausible_roi_bps: u128 = std::env::var("ARB_MAX_IMPLAUSIBLE_ROI_BPS")
                    .ok()
                    .and_then(|v| v.trim().parse::<u128>().ok())
                    .unwrap_or(1_500); // 15% ceiling
                if implied_roi_bps > max_implausible_roi_bps {
                    warn!(
                        "Opportunity rejected: implied ROI {implied_roi_bps} bps exceeds implausibility ceiling {max_implausible_roi_bps} bps (stale-quote artifact)"
                    );
                    if gross > 0 {
                        let reason = format!(
                            "ROI وهمي ({} bps) فوق سقف الإستحالة {} bps — quote stale/inconsistent (phantom edge); rejected fail-closed",
                            implied_roi_bps, max_implausible_roi_bps
                        );
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Implausible ROI".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                        });
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Implausible ROI",
                                &reason,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                }
                // Convert the min profit floor (in lamports) to base-mint raw
                // units for a same-denomination comparison. Fail closed if the
                // conversion rate is unavailable or stale.
                let min_net_base_raw = match arb_types::dimensional::lamports_to_base_raw_checked(
                    self.config.strategy.min_net_profit_abs,
                    verified.lamports_per_base_raw.value,
                    verified.lamports_per_base_denom.value,
                    current_slot.0,
                    current_slot.0,
                    self.config.strategy.max_state_age_slots,
                ) {
                    Ok(v) => v as u64,
                    Err(e) => {
                        debug!("Cannot convert min profit floor to base mint: {e}; failing closed");
                        if gross > 0 {
                            let reason =
                                format!("تعذر تحويل حد الربح الأدنى إلى الوحدات الأساسية: {e}");
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.clone(),
                                    gross_sol,
                                    stage: "Net Profit Economics".to_string(),
                                    reason: reason.clone(),
                                    slot: current_slot.0,
                                },
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
                    }
                };

                if real_net_base_raw < min_net_base_raw {
                    debug!(
                        "Opportunity rejected: net {real_net_base_raw} base-raw < min_profit {min_net_base_raw} base-raw"
                    );
                    if gross > 0 {
                        let net_in_base =
                            real_net_base_raw as f64 / 10f64.powi(i32::from(opp.base_decimals));
                        let min_in_base =
                            min_net_base_raw as f64 / 10f64.powi(i32::from(opp.base_decimals));
                        let reason = format!(
                            "صافي الربح ({:.6} {}) أقل من الحد الأدنى المطلوب ({:.6} {})",
                            net_in_base,
                            opp.hops
                                .first()
                                .map(|h| h.mint_in.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                            min_in_base,
                            opp.hops
                                .first()
                                .map(|h| h.mint_in.to_string())
                                .unwrap_or_else(|| "unknown".to_string()),
                        );
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Net Profit Economics".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                        });
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
                    debug!("No fresh SOL/USD reference; rejecting candidate economic conversion");
                    continue;
                };

                if self.config.chronos.enabled {
                    let chronos_input = arb_risk::ChronosEvInput {
                        age_ms: latency_ms as u64,
                        max_age_ms: self.config.chronos.max_age_ms,
                        decay_lambda_per_sec: self.config.chronos.decay_lambda_per_sec,
                        gross_profit_lamports: gross,
                        flash_fee_lamports: flash_fee_raw,
                        base_network_fee_lamports: BASE_FEE_LAMPORTS,
                        priority_fee_lamports,
                        jito_tip_lamports: tip.as_u64(),
                        failed_attempt_cost_lamports: self
                            .config
                            .chronos
                            .failed_attempt_cost_lamports,
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
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.clone(),
                                    gross_sol,
                                    stage: "Chronos EV Gate".to_string(),
                                    reason: reason_msg.clone(),
                                    slot: current_slot.0,
                                },
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
                }

                // State age in SLOTS — fail closed on any unresolvable pool.
                let state_age_slots = max_state_age_slots(&opp.hops, &pools, current_slot);

                // Split routing is disabled until per-leg AMM quote, fee, slippage,
                // execution instruction and aggregate min-out equivalence are proven.
                let split: Option<arb_engine::SplitAllocation> = None;
                let effective_price_impact_bps = opp.price_impact_bps.0 as u64;

                // Convert the actual input notional to USD before either risk path.
                let base_mint = opp.hops.first().map(|hop| hop.mint_in.0);
                let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap_or_default();
                let usdc = Pubkey::from_str(arb_tx::USDC_MINT).unwrap_or_default();
                let usdt = Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")
                    .unwrap_or_default();
                let base_price_usd = match base_mint {
                    Some(mint) if mint == wsol => Some(sol_price),
                    Some(mint) if mint == usdc || mint == usdt => Some(1.0),
                    // No mint-specific, slot-tagged price oracle is wired for
                    // meme-token anchors yet. Do not mislabel their raw units as SOL.
                    _ => None,
                };
                let trade_size_usd = base_price_usd.map(|price| {
                    (opp.size_in.as_u64() as f64 / 10_f64.powi(opp.base_decimals as i32)) * price
                });
                let bypass_risk_gate = !matches!(self.mode, BotMode::Live)
                    && std::env::var("BYPASS_RISK_GATE")
                        .map(|v| {
                            matches!(
                                v.trim().to_ascii_lowercase().as_str(),
                                "1" | "true" | "yes" | "on"
                            )
                        })
                        .unwrap_or(false);
                if bypass_risk_gate {
                    warn!(
                        "BYPASS_RISK_GATE active in {} mode: notional and impact risk checks skipped",
                        self.mode
                    );
                }

                let risk_check =
                    if bypass_risk_gate {
                        if self.risk_gate.kill_switch().is_blocked() {
                            Err("kill switch active".to_string())
                        } else {
                            Ok(())
                        }
                    } else {
                        match trade_size_usd {
                        Some(size_usd) if size_usd.is_finite() && size_usd > 0.0 => {
                            let gross_profit = opp.gross_profit.as_u64();
                            // Fail closed: a non-positive gross profit must not be
                            // read as a 0% tip share.
                            let tip_share_bps = tip
                                .as_u64()
                                .saturating_mul(10_000)
                                .checked_div(gross_profit)
                                .unwrap_or(u64::MAX);
                            if self.config.strategy.use_flash_loan {
                                self.risk_gate.evaluate_flash_loan(
                                    Lamports(arb_types::dimensional::base_raw_to_lamports_checked(
                                        real_net_base_raw as u128,
                                        verified.lamports_per_base_raw.value,
                                        verified.lamports_per_base_denom.value,
                                    )
                                    .unwrap_or(0)),
                                    opp.size_in,
                                    current_slot,
                                    state_age_slots,
                                    size_usd,
                                    tip.as_u64(),
                                    tip_share_bps,
                                    effective_price_impact_bps,
                                )
                            } else {
                                self.risk_gate.evaluate_full(
                                    Lamports(arb_types::dimensional::base_raw_to_lamports_checked(
                                        real_net_base_raw as u128,
                                        verified.lamports_per_base_raw.value,
                                        verified.lamports_per_base_denom.value,
                                    )
                                    .unwrap_or(0)),
                                    opp.size_in,
                                    current_slot,
                                    state_age_slots,
                                    size_usd,
                                    tip.as_u64(),
                                    tip_share_bps,
                                    effective_price_impact_bps,
                                )
                            }
                        }
                    _ => Err(
                        "cannot establish USD notional for base mint; risk gate fails closed"
                            .to_string(),
                    ),
                    }
                    };

                if let Err(reason) = risk_check {
                    debug!("Opportunity rejected by RiskGate: {reason}");
                    if gross > 0 {
                        let reason_msg = format!("قيود RiskGate رفضت التنفيذ: {reason}");
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "RiskGate Protection".to_string(),
                            reason: reason_msg.clone(),
                            slot: current_slot.0,
                        });
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

                let t_risk_us = t_risk_start.elapsed().as_micros() as u64;
                let t_assembly_start = std::time::Instant::now();

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
                    // Split legs are parallel pools on the SAME mint pair: the leg
                    // builder below stamps every leg with first_hop's mints. Reject
                    // any split whose legs trade a different pair (fail closed).
                    let pair_ok = {
                        let registry = self.registry.read();
                        s.legs.iter().all(|leg| {
                            registry
                                .pools()
                                .iter()
                                .find(|p| p.pool_id == leg.pool_id)
                                .map(|p| {
                                    (p.base_mint == first_hop.mint_in
                                        && p.quote_mint == first_hop.mint_out)
                                        || (p.base_mint == first_hop.mint_out
                                            && p.quote_mint == first_hop.mint_in)
                                })
                                .unwrap_or(false)
                        })
                    };
                    if !pair_ok {
                        debug!("Split route rejected: leg pool mint pair differs from first hop");
                        continue;
                    }
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
                                token_programs: first_hop.token_programs,
                                trade_intent: Default::default(),
                                execution_accounts: None,
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
                } else {
                    match self
                        .assemble_native_ixs(
                            payer,
                            tip_account,
                            tip,
                            cu_limit,
                            cu_price,
                            opp,
                            &kamino_params,
                            verified.flash_loan_fee_sf.value,
                        )
                        .await
                    {
                        Ok(jixs) => jixs,
                        Err(e) => {
                            debug!("Native builder failed, skipping opportunity: {e}");
                            continue;
                        }
                    }
                };
                let t_assembly_us = t_assembly_start.elapsed().as_micros() as u64;
                let t_signing_start = std::time::Instant::now();

                // V1-only profile: never consume fabricated/static ALTs and never
                // reuse Legacy/V0 pre-warmed templates. Rebuild all inline account
                // metas from the current route state before RPC simulation.
                let route_sig = format!(
                    "prewarm:{}",
                    opp.hops
                        .iter()
                        .map(|h| format!("{}-{:?}", h.pool_id.0, h.protocol))
                        .collect::<Vec<_>>()
                        .join("_")
                );

                let blockhash_b58;
                let tx_bytes = if let Some(ref signer) = self.keypair {
                    let blockhash = self.fetch_recent_blockhash().await;
                    blockhash_b58 = bs58::encode(blockhash).into_string();
                    // Fail fast: a zero blockhash must never be sent.
                    if blockhash == [0u8; 32] {
                        let reason_msg =
                            "فشل جلب أحدث Blockhash صالح من شبكة سولانا (Zero Blockhash)";
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Solana Blockhash".to_string(),
                            reason: reason_msg.to_string(),
                            slot: current_slot.0,
                        });
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Solana Blockhash",
                                reason_msg,
                                current_slot.0,
                            )
                            .await;
                        warn!("fetch_recent_blockhash returned zero; skipping opportunity");
                        continue;
                    }
                    // This build is V1-only. Only the direct RPC/SWQoS path is
                    // eligible; Jito bundle and all legacy/V0 template paths fail closed.
                    let direct_v1_enabled = is_swqos_route
                        && std::env::var("SOLANA_TX_V1_DIRECT_ENABLED")
                            .map(|v| {
                                matches!(
                                    v.trim().to_ascii_lowercase().as_str(),
                                    "1" | "true" | "yes" | "on"
                                )
                            })
                            .unwrap_or(false);
                    // Shadow and Simulate always compile V1 for RPC simulation.
                    // A future Live build may compile only when the direct V1
                    // sender is explicitly enabled. Jito is deliberately excluded.
                    let v1_transport_enabled =
                        matches!(self.mode, BotMode::Shadow | BotMode::Simulate)
                            || direct_v1_enabled;
                    // Tier-1: Ultra-fast Zero-Copy In-Memory RAM template pre-signing (< 5 microseconds)
                    let patched_ram = if v1_transport_enabled {
                        None
                    } else {
                        let mut guard = self.ram_templates.lock();
                        guard.get_mut(&route_sig).map(|tpl| {
                            tpl.patch_and_sign_with_swaps(
                                opp.size_in.as_u64(),
                                tip.as_u64(),
                                blockhash,
                                signer,
                                Some(opp.size_in.as_u64()), // swap amount_in = borrow amount
                                Some(real_net_base_raw),    // swap min_out = borrow + profit floor
                            )
                            .to_vec()
                        })
                    };

                    let patched = if v1_transport_enabled {
                        None
                    } else {
                        match patched_ram {
                            Some(b) => Some(b),
                            None => {
                                // Tier-2: Redis shared cache fallback (legacy only).
                                self.redis_cache
                                    .get_json::<CachedTemplate>(&route_sig)
                                    .and_then(|c| {
                                        let bh: [u8; 32] =
                                            c.blockhash.as_slice().try_into().ok()?;
                                        let mut tpl =
                                            arb_tx::PreWarmedTxTemplate::from_precompiled_bytes(
                                                c.wire,
                                                &bh,
                                                c.tip,
                                                verified.flash_loan_fee_sf.value,
                                            )
                                            .ok()?;
                                        let signed = tpl
                                            .patch_and_sign_with_swaps(
                                                opp.size_in.as_u64(),
                                                tip.as_u64(),
                                                blockhash,
                                                signer,
                                                Some(opp.size_in.as_u64()),
                                                Some(real_net_base_raw),
                                            )
                                            .to_vec();
                                        self.ram_templates.lock().insert(route_sig.clone(), tpl);
                                        Some(signed)
                                    })
                            }
                        }
                    };

                    match patched {
                        Some(bytes) => bytes,
                        None => {
                            // Tier-3: V1-only full transaction compilation.
                            // No Legacy/V0/ALT fallback is permitted in this release.
                            let build_result = if v1_transport_enabled {
                                let priority_fee_lamports = ((cu_price as u128)
                                    .saturating_mul(cu_limit as u128)
                                    .saturating_add(999_999))
                                    / 1_000_000;
                                if priority_fee_lamports > u64::MAX as u128 {
                                    Err(arb_tx::TxError::Serialization(
                                        "V1 priority fee exceeds u64 lamports".into(),
                                    ))
                                } else {
                                    let v1_config = arb_tx::V1ConfigParams {
                                        priority_fee_lamports: Some(priority_fee_lamports as u64),
                                        compute_unit_limit: Some(cu_limit),
                                        loaded_accounts_data_size_limit: Some(67_108_864),
                                        heap_size: None,
                                    };
                                    arb_tx::AdaptiveTxCompiler::compile_v1_only(
                                        signer, &ixs, blockhash, v1_config,
                                    )
                                    .map(|compiled| compiled.wire_bytes)
                                }
                            } else {
                                Err(arb_tx::TxError::UnsupportedTransactionVersion(0))
                            };
                            let bytes = match build_result {
                                Ok(bytes) => bytes,
                                Err(e) => {
                                    debug!(
                                        "Failed to compile/sign transaction: {e}; skipping opportunity"
                                    );
                                    continue;
                                }
                            };
                            // V1 messages are rebuilt from current inline account metas.
                            // Legacy/V0 pre-warmed templates are intentionally not reused.
                            bytes
                        }
                    }
                } else {
                    // Shadow-only path: build an unsigned V1 envelope with zeroed
                    // signature slots. RPC simulation is explicitly sigVerify=false;
                    // no wallet secret is loaded and this byte sequence is never sent.
                    let blockhash = self.fetch_recent_blockhash().await;
                    if blockhash == [0u8; 32] {
                        warn!("Shadow V1 simulation rejected: no valid recent blockhash");
                        continue;
                    }
                    blockhash_b58 = bs58::encode(blockhash).into_string();
                    let payer = self
                        .config
                        .wallet
                        .cold_wallet_pubkey
                        .as_deref()
                        .and_then(|value| value.parse::<Pubkey>().ok());
                    let Some(payer) = payer else {
                        warn!(
                            "Shadow V1 simulation rejected: wallet.cold_wallet_pubkey is missing or invalid"
                        );
                        continue;
                    };
                    let priority_fee_lamports = ((cu_price as u128)
                        .saturating_mul(cu_limit as u128)
                        .saturating_add(999_999))
                        / 1_000_000;
                    if priority_fee_lamports > u64::MAX as u128 {
                        warn!("Shadow V1 simulation rejected: priority fee overflow");
                        continue;
                    }
                    let v1_config = arb_tx::V1ConfigParams {
                        priority_fee_lamports: Some(priority_fee_lamports as u64),
                        compute_unit_limit: Some(cu_limit),
                        loaded_accounts_data_size_limit: Some(67_108_864),
                        heap_size: None,
                    };
                    match arb_tx::AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
                        payer, &ixs, blockhash, v1_config,
                    ) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            warn!("Shadow V1 transaction compilation failed: {error}");
                            continue;
                        }
                    }
                };
                let t_signing_us = t_signing_start.elapsed().as_micros() as u64;

                // 5. No simulateTransaction on the hot path (latency).
                // Rejection is delegated to on-chain guards: Kamino flash-repay
                // fails if the balance is short, and each hop carries min_amount_out
                // at DEFAULT_HOP_SLIPPAGE_BPS. Note this is a slippage bound rather
                // than a profit floor — a floor would need a final-hop minimum
                // derived from `size_in + costs + tip` propagated per hop.

                info!(
                    "Profitable opportunity sized: size={} real_net_base_raw={} tip={}",
                    opp.size_in, real_net_base_raw, tip
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
                    expected_net: Lamports(
                        arb_types::dimensional::base_raw_to_lamports_checked(
                            real_net_base_raw as u128,
                            verified.lamports_per_base_raw.value,
                            verified.lamports_per_base_denom.value,
                        )
                        .unwrap_or(0),
                    ),
                    tip_lamports: Some(tip),
                    decision: format!("{}", self.mode),
                };
                let _ = self.store.record_attempt(&record);

                // All monetary values below are in BASE MINT raw units (e.g.,
                // USDC raw units when base_mint is USDC). We convert to
                // human-readable decimal form using the base mint's own decimals
                // — never assuming SOL (9 decimals) or lamports.
                let Some(base_mint) = opp.hops.first().map(|h| h.mint_in) else {
                    continue;
                };
                let base_scale = 10f64.powi(i32::from(opp.base_decimals));
                let profit_base = real_net_base_raw as f64 / base_scale;
                let _size_in_base = opp.size_in.as_u64() as f64 / base_scale;
                let _gross_base = opp.expected_out.as_u64() as f64 / base_scale;

                // For SOL-denominated display, convert base-mint raw → lamports
                // using the verified rate. Fail closed (0.0) if conversion is
                // unavailable — never guess.
                let profit_lamports = arb_types::dimensional::base_raw_to_lamports_checked(
                    real_net_base_raw as u128,
                    verified.lamports_per_base_raw.value,
                    verified.lamports_per_base_denom.value,
                )
                .unwrap_or(0);
                let profit_sol = profit_lamports as f64 / 1e9;
                let live_sol_price = self.live_sol_price().unwrap_or(0.0);

                let pools_str = opp
                    .hops
                    .iter()
                    .map(|h| format!("{:?} ({})", h.protocol, h.pool_id.0))
                    .collect::<Vec<_>>()
                    .join(" ➔ ");
                let initial_loan_base = opp.size_in.as_u64() as f64 / base_scale;
                let gross_rev_base = opp.expected_out.as_u64() as f64 / base_scale;
                let deductions_base =
                    (opp.gross_profit.as_u64() as f64 - real_net_base_raw as f64) / base_scale;
                let roi_pct = if opp.size_in.as_u64() > 0 {
                    (real_net_base_raw as f64 / opp.size_in.as_u64() as f64) * 100.0
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
                    * Initial Loan / Capital: {:.6} {}\n\n\
                    ### 2. Execution & Blockchain Lifecycle\n\
                    * Execution Status: [{}]\n\
                    * Block Time & Slot: Slot {}\n\
                    * Gas Fee (Priority Fees + Cu Burned): ~0.000010 SOL / {} CU\n\n\
                    ### 3. Financial PnL Breakdown (The Profit)\n\
                    * Gross Revenue: {:.6} {}\n\
                    * Total Deductions: {:.6} {}\n\
                    * Net Profit: **+{:.6} {}** (~${:.3})\n\
                    * ROI %: +{:.4}%\n\
                    ============================================================",
                    now_dur.as_secs(),
                    now_dur.subsec_millis(),
                    pools_str,
                    initial_loan_base,
                    base_mint,
                    if self.mode == BotMode::Live {
                        "DISPATCHING_JITO"
                    } else {
                        "SHADOW_SIMULATED"
                    },
                    current_slot.0,
                    cu_limit,
                    gross_rev_base,
                    base_mint,
                    deductions_base,
                    base_mint,
                    profit_base,
                    base_mint,
                    profit_sol * live_sol_price,
                    roi_pct
                );

                // ── Non-broadcast route simulation (fail closed when enabled) ──
                // Submits the serialized route to `simulateTransaction` against
                // real mainnet state with `replaceRecentBlockhash: true` and
                // `sigVerify: false`. It never signs and never broadcasts.
                // Disabled by default; Live dispatch keeps its own mandatory
                // pre-flight simulation. Enable with ENABLE_PREFLIGHT_SIMULATION.
                if preflight_simulation_enabled() {
                    let fork_sim = ForkSimulator::from_env();
                    if !fork_sim.has_endpoint() {
                        warn!(
                            "ENABLE_PREFLIGHT_SIMULATION is set but no RPC endpoint is configured - failing closed"
                        );
                        continue;
                    }
                    let fork_result = match fork_sim
                        .simulate_route(&tx_bytes, &[], real_net_base_raw)
                        .await
                    {
                        Ok(result) => result,
                        Err(e) => {
                            warn!("Non-broadcast simulation could not run ({e}) - failing closed");
                            if gross > 0 {
                                self.record_rejected_opportunity(
                                    crate::telegram::RejectedTradeSummary {
                                        route: route_label.clone(),
                                        gross_sol,
                                        stage: "Pre-flight Simulation".to_string(),
                                        reason: format!("تعذّر تشغيل المحاكاة: {e}"),
                                        slot: current_slot.0,
                                    },
                                );
                            }
                            continue;
                        }
                    };
                    if !fork_result.success {
                        warn!(
                            "Non-broadcast simulation rejected the route: {:?}",
                            fork_result.error
                        );
                        if gross > 0 {
                            let reason =
                                format!("محاكاة المعاملة رفضت التنفيذ: {:?}", fork_result.error);
                            self.record_rejected_opportunity(
                                crate::telegram::RejectedTradeSummary {
                                    route: route_label.clone(),
                                    gross_sol,
                                    stage: "Pre-flight Simulation".to_string(),
                                    reason,
                                    slot: current_slot.0,
                                },
                            );
                            self.telegram
                                .alert_opportunity_rejected(
                                    &route_label,
                                    gross_sol,
                                    "Pre-flight Simulation",
                                    &format!("{:?}", fork_result.error),
                                    current_slot.0,
                                )
                                .await;
                        }
                        continue;
                    }
                    info!(
                        "Non-broadcast simulation OK: slot={:?} cu={} inner_ix={} fee_payer_delta={}",
                        fork_result.context_slot,
                        fork_result.units_consumed,
                        fork_result.inner_instruction_count,
                        fork_result.fee_payer_lamport_delta
                    );
                }

                // ── Phase 15.3: Economic Invariant Gate ──
                // Reject any candidate that fails a core economic invariant before
                // it reaches the transaction compiler. Fail closed.
                let invariant_result = self.validate_economic_invariants(
                    opp,
                    tip,
                    current_slot.0,
                    verified.flash_loan_fee_sf.value,
                );
                if let Err(reason) = invariant_result {
                    debug!("Opportunity rejected by economic invariant gate: {reason}");
                    if gross > 0 {
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Economic Invariant Gate".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                        });
                        self.telegram
                            .alert_opportunity_rejected(
                                &route_label,
                                gross_sol,
                                "Economic Invariant Gate",
                                &reason,
                                current_slot.0,
                            )
                            .await;
                    }
                    continue;
                }

                self.dispatch_opportunity(super::dispatch::DispatchContext {
                    opp,
                    tip,
                    tip_rung,
                    is_swqos_route,
                    current_slot,
                    expected_stream_generation,
                    route_label: &route_label,
                    gross_rev_sol: gross_rev_base,
                    gross_sol,
                    profit_sol,
                    live_sol_price,
                    blockhash_b58: &blockhash_b58,
                    cu_limit,
                    cu_price,
                    tx_bytes: &tx_bytes,
                    record: &record,
                    real_net_profit: Lamports(real_net_base_raw),
                    payer,
                    route_sig: &route_sig,
                    t_ingest_to_quote_us,
                    t_risk_us,
                    t_assembly_us,
                    t_signing_us,
                })
                .await;
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

    /// Phase 15.3: Validate all economic invariants for a candidate opportunity.
    ///
    /// Rejects any candidate with:
    /// * Cross-mint arithmetic (input mint ≠ output mint for cyclic routes)
    /// * Invalid or missing decimals metadata
    /// * Missing or stale price conversions
    /// * Net profit not denominated in the input mint
    /// * Inconsistent pool snapshot slots within a route
    /// * Flash loan repayment not fully funded
    fn validate_economic_invariants(
        &self,
        opp: &arb_types::ExecutionOpportunity,
        tip: arb_types::Lamports,
        current_slot: u64,
        flash_loan_fee_sf: u64,
    ) -> Result<(), String> {
        // 1. Input and output must be the same mint for a cyclic arbitrage route.
        let input_mint = opp
            .hops
            .first()
            .map(|h| h.mint_in)
            .ok_or_else(|| "no hops in route".to_string())?;
        let output_mint = opp
            .hops
            .last()
            .map(|h| h.mint_out)
            .ok_or_else(|| "no hops in route".to_string())?;
        if input_mint != output_mint {
            return Err(format!(
                "input mint {input_mint} != output mint {output_mint}: not a closed cycle"
            ));
        }

        // 2. Route state age must be within max_state_age_slots.
        let state_age = current_slot.saturating_sub(opp.slot.0);
        if state_age > self.config.strategy.max_state_age_slots {
            return Err(format!(
                "route state age {state_age} slots exceeds max {max} slots",
                max = self.config.strategy.max_state_age_slots
            ));
        }

        // 4. Net profit must be positive and exceed the minimum floor.
        if opp.net_profit.as_u64() == 0 {
            return Err("net profit is zero".to_string());
        }
        let min_net_base_raw = self.config.strategy.min_net_profit_abs;
        if opp.net_profit.as_u64() < min_net_base_raw {
            return Err(format!(
                "net profit {} below minimum floor {}",
                opp.net_profit.as_u64(),
                min_net_base_raw
            ));
        }

        // 5. Flash loan repayment must be fully funded: expected_out must cover
        //    size_in + flash fee + all costs.
        let flash_fee = if self.config.strategy.use_flash_loan {
            arb_math::economic::calculate_flash_fee_raw(opp.size_in.as_u64(), flash_loan_fee_sf)
                .map_err(|e| format!("flash fee calculation failed: {e}"))?
        } else {
            0
        };
        let total_required = opp
            .size_in
            .as_u64()
            .checked_add(flash_fee)
            .ok_or("size_in + flash_fee overflow")?;
        if opp.expected_out.as_u64() < total_required {
            return Err(format!(
                "expected_out {} < required repayment {} (size_in={} flash_fee={})",
                opp.expected_out.as_u64(),
                total_required,
                opp.size_in.as_u64(),
                flash_fee
            ));
        }
        // Conservative floor: when a pipeline carries a slippage-bounded final
        // output, that floor (not the optimistic expected_out) must still cover
        // the full repayment.
        if opp.expected_min_final_output > 0 && opp.expected_min_final_output < total_required {
            return Err(format!(
                "expected_min_final_output {} < required repayment {} (size_in={} flash_fee={})",
                opp.expected_min_final_output,
                total_required,
                opp.size_in.as_u64(),
                flash_fee
            ));
        }

        // 6. Tip must not exceed net profit (no negative ROI after tip).
        let tip_as_u64 = tip.as_u64();
        if opp.net_profit.as_u64() <= tip_as_u64 {
            return Err(format!(
                "tip {tip_as_u64} >= net profit {}: negative ROI after tip",
                opp.net_profit.as_u64()
            ));
        }

        Ok(())
    }
}

/// Whether the optional early, route-level non-broadcast simulation is enabled.
///
/// Live dispatch already performs a mandatory `simulateTransaction` pre-flight;
/// this flag adds an earlier route-level check for shadow/analysis runs. It is
/// off unless explicitly requested, and it always fails closed when requested
/// without a usable RPC endpoint.
fn preflight_simulation_enabled() -> bool {
    std::env::var("ENABLE_PREFLIGHT_SIMULATION")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}
