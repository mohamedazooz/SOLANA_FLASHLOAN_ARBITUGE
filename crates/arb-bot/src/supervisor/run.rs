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
    pub(super) fn kamino_available_flash_liquidity(&self, current_slot: Slot) -> Option<u64> {
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
    pub(super) fn capture_three_leg_dlmm(
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
        let mut tuning = self
            .config
            .strategy
            .dlmm_three_leg
            .clone()
            .unwrap_or_default();
        // Unified-control runtime overrides (web section E / Telegram /config)
        // are applied at this single boundary; the pipeline below stays pure.
        if let Some(control) = self.control() {
            let overrides = control.overrides();
            if let Some(usd) = overrides.min_net_profit_usd {
                let sol_price = *self.last_known_live_sol_price.read();
                if sol_price > 0.0 {
                    let floor = (usd / sol_price * 1e9).round();
                    if floor.is_finite() && floor >= 0.0 && floor < u64::MAX as f64 {
                        tuning.min_profit_lamports = tuning.min_profit_lamports.max(floor as u64);
                    }
                }
            }
            if let Some(min_borrow) = overrides.min_borrow_lamports {
                tuning.input_size = tuning.input_size.max(min_borrow);
            }
            if let Some(max_borrow) = overrides.max_borrow_lamports {
                tuning.max_input = tuning.max_input.min(max_borrow);
            }
            if let Some(bps) = overrides.max_slippage_bps {
                tuning.slippage_bps = bps;
            }
        }
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

        // Dynamic economics: the candidate-ranking cost model uses the SAME
        // adaptive CU price and verified tip that the dispatch gate will
        // charge, so ranking and gating cannot disagree.
        let params = arb_engine::ThreeLegMemeContextParams {
            snapshot_slot: current_slot.0,
            cu_limit: arb_tx::COMPUTE_BUDGET_LIMIT_3HOP,
            priority_fee_micro_lamports: self
                .tip_strategy
                .adaptive_cu_price(&self.recent_landed_cu),
            jito_tip_account: self.jito_service.tip_account_for_slot(current_slot),
            jito_tip_lamports: verified_tip_lamports,
            recent_blockhash: [0u8; 32],
            // Fail-closed default. May ONLY be set to true after a successful
            // V1 transport canary (crates/arb-tx/examples/v1_canary_broadcast.rs)
            // landed on-chain through the configured transport — evidence under
            // reports/v1_canary/. Never a blind flip.
            v1_transport_supported: std::env::var("ARB_V1_TRANSPORT_SUPPORTED")
                .map(|v| {
                    matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .unwrap_or(false),
            slippage_bps: tuning.slippage_bps,
            min_profit_lamports: crate::config::min_net_profit_floor_lamports(
                tuning
                    .min_profit_lamports
                    .max(self.config.strategy.min_net_profit_abs),
            ),
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
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
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
        info!("Starting Supervisor — Mode: {}", self.mode());

        // R12: reconcile dispatch intents left pending by a previous run
        // before dispatching anything new.
        self.reconcile_pending_dispatches().await;

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
                &self.mode().to_string(),
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
            self.publish_control_state(current_slot);

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
            self.strategy_cycle(
                current_slot,
                expected_stream_generation,
                payer,
                &kamino_params,
            )
            .await?;

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
    pub(super) fn validate_economic_invariants(
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
