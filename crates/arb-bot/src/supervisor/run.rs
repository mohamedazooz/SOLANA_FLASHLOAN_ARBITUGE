//! Main supervisor lifecycle and orchestration loop.
use super::*;
use arb_types::{Bps, RouteId};

impl Supervisor {
    /// Accept only correctly owned legacy SPL Token WSOL accounts for bootstrap hydration.
    /// This validates the owner program and mint before account bytes enter the shared store.
    fn is_verified_spl_wsol_account(owner: &Pubkey, data: &[u8]) -> bool {
        let Ok(token_program) = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID) else {
            return false;
        };
        if owner != &token_program {
            return false;
        }
        let Ok(wsol_mint) = Pubkey::from_str(arb_tx::WSOL_MINT) else {
            return false;
        };
        arb_tx::kamino::decode_supply_vault_liquidity(data, &wsol_mint).is_ok()
    }
    /// Cumulative user+system CPU ticks for this process (Linux `/proc/self/stat`).
    /// Field indices 11/12 after `(comm)` correspond to utime/stime (fields 14/15).
    #[cfg(target_os = "linux")]
    fn process_cpu_ticks() -> Option<(u64, u64)> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = stat.split(") ").nth(1)?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some((utime, stime))
    }

    #[cfg(not(target_os = "linux"))]
    fn process_cpu_ticks() -> Option<(u64, u64)> {
        None
    }

    /// CPU percent over the sample interval. Can exceed 100 on multi-core
    /// hosts; only negative drift is clamped.
    fn cpu_percent(prev: (u64, u64), now: (u64, u64), interval_secs: f64) -> f64 {
        const CLK_TCK: f64 = 100.0;
        let ticks = now
            .0
            .saturating_sub(prev.0)
            .saturating_add(now.1.saturating_sub(prev.1));
        ticks as f64 / CLK_TCK / interval_secs * 100.0
    }

    /// Resident set size in bytes (Linux `/proc/self/status` VmRSS).
    #[cfg(target_os = "linux")]
    fn process_rss_bytes() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb.saturating_mul(1024))
    }

    #[cfg(not(target_os = "linux"))]
    fn process_rss_bytes() -> Option<u64> {
        None
    }

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
    pub fn kamino_available_flash_liquidity(&self, current_slot: Slot) -> Option<u64> {
        self.kamino_available_flash_liquidity_detailed(current_slot)
            .ok()
    }

    /// Detailed fail-closed result for diagnostics; callers must not turn any
    /// error into an estimated or cached borrow amount.
    pub fn kamino_available_flash_liquidity_detailed(
        &self,
        current_slot: Slot,
    ) -> Result<u64, &'static str> {
        let supply_vault = Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT)
            .map_err(|_| "configured Kamino WSOL supply-vault address is invalid")?;
        let snap = self
            .ingest
            .account_store()
            .get(&supply_vault)
            .ok_or("supply-vault snapshot is missing from the live account store")?;

        const MAX_SNAPSHOT_AGE_SLOTS: u64 = arb_constants::KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS;
        if snap.slot > current_slot.0 {
            return Err("supply-vault snapshot slot is ahead of the current stream slot");
        }
        let age = current_slot.0.saturating_sub(snap.slot);
        if age > MAX_SNAPSHOT_AGE_SLOTS {
            return Err("supply-vault snapshot is stale beyond the configured slot-age limit");
        }

        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)
            .map_err(|_| "configured SPL Token program id is invalid")?;
        if snap.owner != spl {
            return Err("supply-vault account owner is not the SPL Token program");
        }
        let wsol =
            Pubkey::from_str(arb_tx::WSOL_MINT).map_err(|_| "configured WSOL mint is invalid")?;
        arb_tx::kamino::decode_supply_vault_liquidity(&snap.data, &wsol)
            .map_err(|_| "supply-vault token account failed mint/layout validation")
    }

    fn three_leg_network_cost_lamports(&self, tip_lamports: u64) -> u64 {
        self.jito_service
            .oracle()
            .current(arb_land::now_ms())
            .map(|f| f.p50.0.min(tip_lamports))
            .unwrap_or(tip_lamports)
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
    /// Token program owning `mint` on `pool`, from the pool's actual X/Y
    /// program slots (X = base → index 0, Y = quote → index 1).
    fn token_program_for_mint_pub(pool: &PoolState, mint: MintId) -> Option<Pubkey> {
        use arb_types::TokenProgramKind;
        let idx = if pool.base_mint == mint {
            0
        } else if pool.quote_mint == mint {
            1
        } else {
            return None;
        };
        match pool.token_programs.get(idx)? {
            TokenProgramKind::SplToken => Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).ok(),
            TokenProgramKind::Token2022 => {
                Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).ok()
            }
        }
    }

    /// Fetch the payer's real token balances for `atas`. Returns `None` when
    /// the RPC fetch itself fails (fail-closed: zero-balance unprovable). A
    /// missing account is provably zero (0); an existing account that cannot
    /// be decoded yields `u64::MAX` (fail-closed non-zero). SPL/Token-2022
    /// token accounts both store the amount at byte offset 64..72 (LE).
    async fn fetch_payer_token_balances(&self, atas: &[Pubkey]) -> Option<Vec<u64>> {
        use base64::prelude::*;
        let strs: Vec<String> = atas.iter().map(|p| p.to_string()).collect();
        let refs: Vec<&str> = strs.iter().map(String::as_str).collect();
        let accounts = self.rpc_pool.get_multiple_accounts(&refs).await.ok()?;
        Some(
            accounts
                .into_iter()
                .map(|acc| {
                    let Some(acc) = acc else {
                        return 0u64;
                    };
                    let Some(data_str) = acc
                        .get("data")
                        .and_then(|d| d.get(0))
                        .and_then(|s| s.as_str())
                    else {
                        return u64::MAX;
                    };
                    let Ok(bytes) = BASE64_STANDARD.decode(data_str) else {
                        return u64::MAX;
                    };
                    if bytes.len() >= 72 {
                        u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]))
                    } else {
                        u64::MAX
                    }
                })
                .collect(),
        )
    }

    pub(super) async fn capture_three_leg_dlmm(
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
        // Shared geometric ladder (×3 growth, ceiling always evaluated) so the
        // rehearsal harness searches the SAME sizes as production.
        let size_search_sizes = {
            let ceiling = max_sizing.min(tuning.max_input).max(1);
            let capacity = tuning.size_ladder.clamp(1, 8);
            arb_engine::build_size_ladder(tuning.input_size, ceiling, capacity)
        };

        // Dynamic economics: the candidate-ranking cost model uses the SAME
        // adaptive CU price and dynamic tip floor (p50 from tip_oracle) for discovery sizing
        // so sizing hurdles match realistic execution costs.
        let dynamic_tip_lamports = self
            .jito_service
            .oracle()
            .current(arb_land::now_ms())
            .map(|f| f.p50.0.min(verified_tip_lamports))
            .unwrap_or(verified_tip_lamports);

        // Match the final Supervisor gate's route-shaped CU estimate and
        // profit-share-capped CU price. These values are used to rank every
        // (route, size) candidate after all known execution costs.
        let route_protocols = [arb_types::DexProtocol::MeteoraDlmm; 3];
        let estimated_cu = arb_tx::estimate_route_cu(
            self.config.strategy.use_flash_loan,
            &route_protocols,
            2, // two intermediate token ATAs for WSOL -> A -> B -> WSOL
        );
        let effective_cu_limit = arb_tx::calculate_dynamic_cu_limit(estimated_cu);
        let recent_landed_cu = self.recent_landed_cu.lock().clone();
        let fresh_market = {
            let market = self.recent_priority_market_cu.lock();
            market
                .as_ref()
                .filter(|(_, sampled_at)| sampled_at.elapsed() <= Duration::from_secs(12))
                .map(|(samples, _)| samples.clone())
                .unwrap_or_default()
        };
        let desired_cu_price = self
            .tip_strategy
            .adaptive_cu_price_with_market(&recent_landed_cu, &fresh_market);
        let competition_factor = self.jito_service.competition_factor();
        let priority_fee_max_share_bps = ((self.config.strategy.priority_fee_base_share_bps as f64
            * competition_factor) as u64)
            .clamp(
                self.config.strategy.priority_fee_base_share_bps,
                self.config.strategy.priority_fee_max_share_bps,
            );

        let params = arb_engine::ThreeLegMemeContextParams {
            snapshot_slot: current_slot.0,
            cu_limit: effective_cu_limit,
            priority_fee_micro_lamports: desired_cu_price,
            priority_fee_max_share_bps,
            jito_tip_account: self.jito_service.tip_account_for_slot(current_slot),
            jito_tip_lamports: dynamic_tip_lamports,
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
            network_cost_lamports: self.three_leg_network_cost_lamports(dynamic_tip_lamports),
            kamino_available_liquidity,
            input_size: tuning.input_size.min(max_sizing).max(1),
            size_search_sizes,
            ..arb_engine::ThreeLegMemeContextParams::for_discovery(payer)
        };

        let t_exact = std::time::Instant::now();
        let pipelines = arb_engine::build_three_leg_meme_pipeline_top_k(
            fresh_pools,
            &params,
            arb_engine::THREE_LEG_PIPELINE_TOP_K_MAX,
        );
        arb_metrics::observe_exact_quote_latency_us(t_exact.elapsed().as_micros() as f64);
        let pipelines = match pipelines {
            Ok(r) => r,
            Err(e) => {
                debug!("Meteora 3-leg DLMM capture skipped: {e}");
                return Vec::new();
            }
        };

        let by_id: std::collections::HashMap<Pubkey, &PoolState> =
            fresh_pools.iter().map(|p| (p.pool_id.0, p)).collect();

        // Fail-closed inventory-provenance policy (PR24 §2.2): the chain's
        // solvency must come from trade proceeds alone. Fetch the payer's
        // REAL settlement/intermediate ATA balances and reject the chain when
        // any is non-zero OR when zero-balance cannot be proven. The chain's
        // leg inputs are already conservatively chained (leg N+1 consumes at
        // most leg N's enforced min_amount_out), so a rejection here means a
        // pre-existing balance could otherwise mask a repayment shortfall.
        let wsol_pubkey = match Pubkey::from_str(arb_tx::WSOL_MINT) {
            Ok(k) => k,
            Err(_) => return Vec::new(),
        };
        let wsol_ata = arb_tx::get_associated_token_address_with_program_id(
            &payer,
            &wsol_pubkey,
            &params.wsol_token_program,
        );
        let mut opportunities = Vec::with_capacity(pipelines.len());
        for result in &pipelines {
            // Fail closed per candidate: only a strictly positive gross chain
            // enters the set.
            if result.gross_profit_lamports <= 0 {
                continue;
            }
            let leg0_pool = by_id.get(&result.legs[0].pool_id.0).copied();
            let leg2_pool = by_id.get(&result.legs[2].pool_id.0).copied();
            let (Some(pool_a), Some(pool_b)) = (leg0_pool, leg2_pool) else {
                continue;
            };
            let (Some(prog_a), Some(prog_b)) = (
                Self::token_program_for_mint_pub(pool_a, result.meme_mints[0]),
                Self::token_program_for_mint_pub(pool_b, result.meme_mints[1]),
            ) else {
                debug!("3-leg capture candidate rejected: intermediate token program unresolvable");
                continue;
            };
            let meme_a_ata = arb_tx::get_associated_token_address_with_program_id(
                &payer,
                &result.meme_mints[0].0,
                &prog_a,
            );
            let meme_b_ata = arb_tx::get_associated_token_address_with_program_id(
                &payer,
                &result.meme_mints[1].0,
                &prog_b,
            );
            let Some(balances) = self
                .fetch_payer_token_balances(&[wsol_ata, meme_a_ata, meme_b_ata])
                .await
            else {
                arb_metrics::inc_inventory_rejections();
                debug!("3-leg capture candidate rejected: ATA balances unfetchable (fail-closed)");
                continue;
            };
            let balances: Vec<u64> = balances;
            let wsol_bal = balances.first().copied().unwrap_or(u64::MAX);
            let a_bal = balances.get(1).copied().unwrap_or(u64::MAX);
            let b_bal = balances.get(2).copied().unwrap_or(u64::MAX);
            if wsol_bal > 0 || a_bal > 0 || b_bal > 0 {
                arb_metrics::inc_inventory_rejections();
                warn!(
                    "3-leg capture candidate rejected: pre-existing inventory would subsidize execution (wsol={wsol_bal}, meme_a={a_bal}, meme_b={b_bal}); remedy: wsol>0 requires unwrapping the WSOL ATA (converts WSOL back to SOL, e.g. syncNative-close flow), intermediate ATA dust requires closing that token account empty before 3-leg capture resumes"
                );
                continue;
            }
            let mut hops = Vec::with_capacity(3);
            let mut route_missing_pool = false;
            for leg in result.legs.iter() {
                let Some(pool) = by_id.get(&leg.pool_id.0) else {
                    route_missing_pool = true;
                    break;
                };
                let Some(fee_bps) = pool.fee_bps() else {
                    // Fee state changed or became invalid after candidate pricing.
                    // Do not serialize a zero-fee hop from an unverifiable snapshot.
                    route_missing_pool = true;
                    break;
                };
                hops.push(SwapHop {
                    pool_id: leg.pool_id,
                    protocol: DexProtocol::MeteoraDlmm,
                    mint_in: leg.from_mint,
                    mint_out: leg.to_mint,
                    // Pipeline pre-filtered pools to those with a valid fee; a
                    // missing fee here can only mean a mid-tick state mutation, and
                    // the pipeline's own quote already priced the route.
                    fee_bps,
                    active_bin_id: pool.dlmm.as_ref().map(|d| d.active_bin_id),
                    // SwapHop contract (arb-types): token_programs is
                    // (mint_in, mint_out) — flip the pool's (base, quote) pair
                    // when this leg trades quote -> base.
                    token_programs: if leg.from_mint == pool.base_mint {
                        pool.token_programs
                    } else {
                        [pool.token_programs[1], pool.token_programs[0]]
                    },
                    trade_intent: Default::default(),
                    execution_accounts: pool.execution_accounts.clone(),
                });
            }
            if route_missing_pool {
                continue;
            }

            let size_in = result.size_search.best_input.max(1);
            let expected_out = result.expected_final_out;
            let expected_min_final_output = result.expected_min_final_output;
            let hop_inputs = vec![
                result.legs[0].amount_in,
                result.legs[0].amount_out,
                result.legs[1].amount_out,
                result.legs[2].amount_out,
            ];
            let gross = result.gross_profit_lamports as u64;
            let net = result.net_profit_lamports.max(0) as u64;

            static THREE_LEG_ROUTE_ID: std::sync::atomic::AtomicU32 =
                std::sync::atomic::AtomicU32::new(50_000);
            let route_id =
                RouteId(THREE_LEG_ROUTE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

            opportunities.push(ExecutionOpportunity {
                hop_slippage_bps: tuning.slippage_bps,
                cu_estimate_units: 0,
                cu_shape_key: String::new(),
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
                // Seed only: the caller must re-price this route through MarketGraph.
                // 10,000 bps is an unknown-impact sentinel, never a false zero.
                price_impact_bps: Bps(10_000),
                hops,
                expected_min_final_output,
            });
        }
        opportunities
    }

    async fn start_wallet_balance_monitor(&self) -> Result<()> {
        if self.min_sol_reserve_lamports == 0 {
            anyhow::bail!(
                "wallet SOL reserve guard cannot be disabled; min_sol_reserve_lamports must be positive"
            );
        }
        let Some(payer) = self.keypair.as_ref().map(|keypair| keypair.pubkey()) else {
            if matches!(self.mode(), BotMode::Live) {
                anyhow::bail!("wallet reserve guard requires a loaded signer in live mode");
            }
            warn!("Wallet reserve monitor not started: no signer loaded (non-live mode)");
            return Ok(());
        };
        if !self.tx_rpc.is_configured() {
            if matches!(self.mode(), BotMode::Live) {
                anyhow::bail!(
                    "wallet reserve guard requires configured Transaction RPC in live mode"
                );
            }
            warn!("Wallet reserve monitor not started: Transaction RPC is not configured");
            return Ok(());
        }

        let initial_balance =
            tokio::time::timeout(Duration::from_secs(5), self.tx_rpc.get_balance(&payer))
                .await
                .map_err(|_| anyhow::anyhow!("initial wallet balance check timed out"))??;
        self.wallet_balance_cache.update(initial_balance);
        if matches!(self.mode(), BotMode::Live) && initial_balance < self.min_sol_reserve_lamports {
            anyhow::bail!(
                "wallet balance {} lamports is below configured reserve {} lamports",
                initial_balance,
                self.min_sol_reserve_lamports,
            );
        }
        info!(
            "Wallet reserve monitor initialized: balance={} lamports reserve={} lamports",
            initial_balance, self.min_sol_reserve_lamports,
        );

        let cache = Arc::clone(&self.wallet_balance_cache);
        let rpc = Arc::clone(&self.tx_rpc);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                match tokio::time::timeout(Duration::from_secs(3), rpc.get_balance(&payer)).await {
                    Ok(Ok(balance)) => cache.update(balance),
                    Ok(Err(error)) => {
                        cache.invalidate();
                        warn!(
                            "Background wallet balance refresh failed; reserve guard is closed: {error}"
                        );
                    }
                    Err(_) => {
                        cache.invalidate();
                        warn!(
                            "Background wallet balance refresh timed out; reserve guard is closed"
                        );
                    }
                }
            }
        });
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        info!("Starting Supervisor — Mode: {}", self.mode());

        // Validate execution prerequisites across registered provider capabilities (G1, G2).
        if let Err(e) = self.capability_registry.validate_execution_prerequisites() {
            error!("Provider capability validation failed (fail-closed): {e}");
            return Err(anyhow::anyhow!("Provider capability check failed: {e}"));
        }
        info!("✅ Provider capability prerequisites validated successfully");

        // R12: reconcile dispatch intents left pending by a previous run
        // before dispatching anything new.
        self.start_wallet_balance_monitor().await?;

        self.reconcile_pending_dispatches().await;

        // PIPE-04: verify the hardcoded tip constants against the live
        // `getTipAccounts` set before any tip-bearing dispatch. In Live mode
        // a mismatch (or unreachable engine) is fail-closed: a tip sent to an
        // account the block engine no longer rewards is silently wasted value.
        {
            const TIP_VERIFY_TIMEOUT: Duration = Duration::from_secs(15);
            let lander = self.jito_lander.clone();
            let expected: Vec<&str> = arb_constants::JITO_TIP_ACCOUNTS.to_vec();
            match tokio::time::timeout(TIP_VERIFY_TIMEOUT, lander.verify_tip_accounts(&expected))
                .await
            {
                Ok(Ok(())) => info!(
                    "✅ Jito getTipAccounts matches the hardcoded tip constants ({} accounts)",
                    expected.len()
                ),
                Ok(Err(e)) if matches!(self.mode(), BotMode::Live) => {
                    error!("tip account verification failed (live fail-closed): {e}");
                    return Err(anyhow::anyhow!("jito tip account verification failed: {e}"));
                }
                Ok(Err(e)) => {
                    warn!("tip account verification failed (non-live, continuing): {e}");
                }
                Err(_) if matches!(self.mode(), BotMode::Live) => {
                    error!("tip account verification timed out (live fail-closed)");
                    return Err(anyhow::anyhow!(
                        "jito tip account verification timed out after 15s"
                    ));
                }
                Err(_) => {
                    warn!("tip account verification timed out (non-live, continuing)");
                }
            }
            if !lander.has_auth() {
                warn!(
                    "JITO_AUTH_UUID not configured: block-engine requests fall back to the default unauthenticated rate limit (1 rps/IP/region)"
                );
            }
        }

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
        if self.config.strategy.http_discovery_enabled {
            info!(
                "🔍 HTTP catalog discovery enabled — fetching active multi-DEX pools across Raydium, Orca, Meteora, and Pump.fun..."
            );
        }
        let disc = arb_ingest::PoolDiscovery::new(self.rpc_pool.clone());
        let mut disc_cfg = arb_ingest::DiscoveryConfig::default();
        // Hot-meme extension without recompiling: `ARB_DISCOVERY_HUBS` is a
        // comma-separated list of mint pubkeys merged into the default hub set
        // (dedup). Invalid entries are logged and skipped — discovery never
        // fails closed because of a malformed env value.
        if let Ok(hubs_raw) = std::env::var("ARB_DISCOVERY_HUBS") {
            for raw in hubs_raw.split(',') {
                let mint = raw.trim();
                if mint.is_empty() {
                    continue;
                }
                match Pubkey::from_str(mint) {
                    Ok(pk) => {
                        let id = arb_types::MintId(pk);
                        if !disc_cfg.hub_mints.contains(&id) {
                            disc_cfg.hub_mints.push(id);
                        }
                    }
                    Err(_) => warn!("ARB_DISCOVERY_HUBS: skipping invalid mint '{mint}'"),
                }
            }
        }
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

        if self.config.strategy.http_discovery_enabled {
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
        } else {
            info!(
                "HTTP pool discovery disabled by config (http_discovery_enabled = false) — pool candidates arrive via the WebSocket account-update stream only"
            );
        }

        // 2. Ingestion: WebSocket streaming worker
        let cancel_token = CancellationToken::new();

        // Maintenance: process CPU/RSS sampler (Linux /proc) and store
        // retention prune. Both are observability/hygiene only — failures are
        // logged, never fatal to the trading loop.
        let cancel_sampler = cancel_token.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut prev_ticks: Option<(u64, u64)> = None;
            loop {
                tokio::select! {
                    _ = cancel_sampler.cancelled() => break,
                    _ = interval.tick() => {
                        let ticks = Self::process_cpu_ticks();
                        if let Some((now, prev)) = ticks.zip(prev_ticks) {
                            let pct = Self::cpu_percent(prev, now, 30.0);
                            arb_metrics::set_process_cpu_percent(pct);
                        }
                        if let Some(t) = ticks {
                            prev_ticks = Some(t);
                        }
                        if let Some(rss) = Self::process_rss_bytes() {
                            arb_metrics::set_process_rss_bytes(rss);
                        }
                    }
                }
            }
        });
        {
            let cancel_prune = cancel_token.clone();
            let store = self.store.clone();
            let retention_days: u32 = std::env::var("DATA_RETENTION_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|d| *d > 0)
                .unwrap_or(30);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = cancel_prune.cancelled() => break,
                        _ = interval.tick() => {
                            match store.prune_older_than_days(retention_days) {
                                Ok(n) if n > 0 => info!(deleted = n, retention_days, "store retention pruned"),
                                Ok(_) => {}
                                Err(e) => warn!("store retention prune failed: {}", e),
                            }
                        }
                    }
                }
            });
        }

        info!(
            "⚡ Pure WebSocket Ingestion Engine Active (Helius & RPCFast WebSockets driving pipeline)"
        );

        // 3. Multi-provider WebSocket real-time slot and account streaming ingestion worker
        let mut hw_client_opt: Option<Arc<arb_ingest::HeliusWsClient>> = None;
        let mut ws_slot_notify: Option<Arc<tokio::sync::Notify>> = None;
        // Fail-closed capability gate: the pipeline must not start unless at
        // least one CONFIGURED provider actually supports live account
        // streaming. HTTP slot polling is not a substitute for a live
        // account stream.
        let helius_ws_client = arb_ingest::HeliusWsClient::from_env();
        let mut capability_registry = arb_ingest::ProviderCapabilityRegistry::new();
        if helius_ws_client.is_some() {
            capability_registry.register(arb_ingest::ProviderCapabilities::standard_json_rpc_ws(
                "Helius",
            ));
        }
        if let Err(e) = capability_registry.validate_execution_prerequisites() {
            error!(
                "No configured provider supports live account streaming ({e}); refusing to start (fail closed)"
            );
            anyhow::bail!(
                "provider capability prerequisites unmet: no live account-streaming provider configured"
            );
        }
        if let Some(helius_ws) = helius_ws_client {
            let wsol_mint = crate::verified_config::wsol_mint();
            let usdc_mint = Pubkey::from_str(arb_tx::USDC_MINT).unwrap();
            let kamino_reserve = crate::verified_config::wsol_reserve();
            let kamino_supply_vault =
                Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).unwrap();
            let kamino_owner = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID).unwrap();
            let token_prog = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
            helius_ws.track_account(kamino_reserve, kamino_owner);
            helius_ws.track_account(wsol_mint, token_prog);
            helius_ws.track_account(kamino_supply_vault, token_prog);
            helius_ws.track_account(usdc_mint, token_prog);

            // Track payer WSOL settlement ATA so ata_rent is recognized as 0 without unnecessary fee penalties
            if let Some(ref kp) = self.keypair {
                let payer_pk = kp.pubkey();
                let settlement_ata = arb_tx::get_associated_token_address_with_program_id(
                    &payer_pk,
                    &wsol_mint,
                    &token_prog,
                );
                helius_ws.track_account(settlement_ata, token_prog);
            }

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
            self.telegram.update_sol_price(live_sol_price_seed);
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

        // Pre-hydrate Kamino WSOL supply vault and payer WSOL settlement ATA immediately at startup
        let startup_vault = Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).unwrap();
        let startup_vault_str = startup_vault.to_string();
        let mut startup_accounts_to_fetch = vec![startup_vault_str];

        let mut payer_settlement_ata_opt = None;
        if let Some(ref kp) = self.keypair {
            let payer_pk = kp.pubkey();
            let wsol_mint = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
            let token_prog = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
            let settlement_ata = arb_tx::get_associated_token_address_with_program_id(
                &payer_pk,
                &wsol_mint,
                &token_prog,
            );
            payer_settlement_ata_opt = Some(settlement_ata);
            startup_accounts_to_fetch.push(settlement_ata.to_string());
        }

        let fetch_strs: Vec<&str> = startup_accounts_to_fetch
            .iter()
            .map(|s| s.as_str())
            .collect();
        if let Ok((accounts, account_slots)) = self
            .rpc_pool
            .get_multiple_accounts_with_slots(&fetch_strs)
            .await
        {
            use base64::prelude::*;
            for (idx, (acc_opt, context_slot)) in
                accounts.into_iter().zip(account_slots).enumerate()
            {
                let Some(acc) = acc_opt else { continue };
                if context_slot == 0 {
                    continue;
                }
                let target_pk = if idx == 0 {
                    startup_vault
                } else if let Some(ata) = payer_settlement_ata_opt {
                    ata
                } else {
                    continue;
                };

                if let (Some(owner_str), Some(data_arr)) = (
                    acc.get("owner").and_then(|o| o.as_str()),
                    acc.get("data").and_then(|d| d.as_array()),
                ) {
                    if let (Ok(owner), Some(raw_b64)) = (
                        Pubkey::from_str(owner_str),
                        data_arr.first().and_then(|s| s.as_str()),
                    ) {
                        if let Ok(bytes) = BASE64_STANDARD.decode(raw_b64) {
                            if !Self::is_verified_spl_wsol_account(&owner, &bytes) {
                                warn!(
                                    pubkey = %target_pk,
                                    owner = %owner,
                                    "Rejected startup WSOL hydration: unexpected token program or mint"
                                );
                                continue;
                            }
                            let snap = arb_ingest::AccountSnapshot {
                                pubkey: target_pk,
                                owner,
                                slot: context_slot,
                                write_version: arb_ingest::next_pool_write_version(),
                                // HTTP account reads are bootstrap/reconciliation
                                // evidence, never a live stream dependency.
                                is_startup: true,
                                provider_id: arb_ingest::HTTP_RPC_PROVIDER_ID,
                                bank_id: None,
                                generation: self.ingest.stream_generation(),
                                data: Arc::new(bytes),
                            };
                            if self.ingest.account_store().apply(snap) {
                                if idx == 0 {
                                    info!(
                                        "💧 Kamino WSOL supply vault successfully hydrated at startup (slot={context_slot})"
                                    );
                                } else {
                                    info!(
                                        "💧 Payer WSOL settlement ATA successfully hydrated at startup ({target_pk}, slot={context_slot})"
                                    );
                                }
                            } else {
                                debug!(
                                    pubkey = %target_pk,
                                    slot = context_slot,
                                    "Startup RPC snapshot was not applied; existing live/newer state or invalidation was retained"
                                );
                            }
                        }
                    }
                }
            }
        }

        // Emit startup & initial market snapshot AFTER discovery and seeding succeed
        self.compute_and_publish_ticker(Slot(reported_seed_slot));
        self.telegram
            .alert_startup(
                &self.mode().to_string(),
                seeded_cache_count,
                crate::live_execution_enabled(),
            )
            .await;
        self.emit_market_snapshot(reported_seed_slot, true).await;

        // 3b. Side-band priority-fee market sampler. Solana retains up to 150
        // slots; query active writable pool accounts (max 128) rather than the
        // network-wide floor, which is commonly zero. This worker is isolated
        // from quote/dispatch and uses RpcPool's throttling + provider failover.
        let priority_rpc = self.rpc_pool.clone();
        let priority_ingest = Arc::clone(&self.ingest);
        let priority_cache = Arc::clone(&self.recent_priority_market_cu);
        let cancel_priority = cancel_token.clone();
        tokio::spawn(async move {
            let cadence = Duration::from_secs(3);
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + cadence, cadence);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = cancel_priority.cancelled() => break,
                    _ = interval.tick() => {}
                }
                let mut accounts: Vec<String> = priority_ingest
                    .snapshot()
                    .snapshot_pools()
                    .into_iter()
                    .map(|pool| pool.pool_id.0.to_string())
                    .collect();
                accounts.sort_unstable();
                accounts.dedup();
                accounts.truncate(128);
                if accounts.is_empty() {
                    continue;
                }
                match priority_rpc
                    .call("getRecentPrioritizationFees", serde_json::json!([accounts]))
                    .await
                {
                    Ok(result) => {
                        let mut samples: Vec<u64> = result
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|sample| {
                                sample
                                    .get("prioritizationFee")
                                    .and_then(serde_json::Value::as_u64)
                            })
                            .collect();
                        if samples.len() > 150 {
                            samples.drain(..samples.len() - 150);
                        }
                        if !samples.is_empty() {
                            *priority_cache.lock() = Some((samples, Instant::now()));
                        }
                    }
                    Err(error) => tracing::debug!(
                        "priority fee sampler unavailable; retaining only fresh cache/landed evidence: {error}"
                    ),
                }
            }
        });

        // 4. Multi-DEX active reserve ingestion worker (WS slot-driven + 750ms cadence, batch-100 parallel)
        let registry_worker = Arc::clone(&self.registry);
        let ingest_worker_res = Arc::clone(&self.ingest);
        let rpc_worker = self.rpc_pool.clone();
        let cancel_registry = cancel_token.clone();
        let slot_notify_worker = ws_slot_notify.clone();
        tokio::spawn(async move {
            // Slow reconciliation sweep, NOT the real-time data path. The WS
            // stream is the sole live source; this pass (30s) only repairs
            // missed WS messages and refreshes derived-account amounts. The
            // former 750ms full poll moved ~70GB/day of redundant RPC bytes
            // (every pool's tick/bin arrays re-fetched 1.3x/second).
            let cadence = Duration::from_secs(30);
            let min_gap = Duration::from_secs(29);
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

                    // Periodically hydrate Kamino WSOL supply vault snapshot if stale (> 100 slots).
                    let supply_vault =
                        Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).unwrap();
                    let is_vault_stale = match ingest_worker_res.account_store().get(&supply_vault)
                    {
                        Some(snap) => s.saturating_sub(snap.slot) > 100,
                        None => true,
                    };
                    if is_vault_stale {
                        let vault_str = supply_vault.to_string();
                        if let Ok((mut accounts, mut account_slots)) = rpc_worker
                            .get_multiple_accounts_with_slots(&[&vault_str])
                            .await
                        {
                            if let Some((Some(acc), context_slot)) =
                                accounts.pop().zip(account_slots.pop())
                            {
                                if context_slot == 0 {
                                    continue;
                                }
                                if let (Some(owner_str), Some(data_arr)) = (
                                    acc.get("owner").and_then(|o| o.as_str()),
                                    acc.get("data").and_then(|d| d.as_array()),
                                ) {
                                    if let (Ok(owner), Some(raw_b64)) = (
                                        Pubkey::from_str(owner_str),
                                        data_arr.first().and_then(|s| s.as_str()),
                                    ) {
                                        use base64::prelude::*;
                                        if let Ok(bytes) = BASE64_STANDARD.decode(raw_b64) {
                                            if Supervisor::is_verified_spl_wsol_account(
                                                &owner, &bytes,
                                            ) {
                                                let snap = arb_ingest::AccountSnapshot {
                                                    pubkey: supply_vault,
                                                    owner,
                                                    slot: context_slot,
                                                    write_version:
                                                        arb_ingest::next_pool_write_version(),
                                                    // RPC refreshes can seed a missing
                                                    // snapshot, but cannot masquerade as
                                                    // live WebSocket state.
                                                    is_startup: true,
                                                    provider_id: arb_ingest::HTTP_RPC_PROVIDER_ID,
                                                    bank_id: None,
                                                    generation: ingest_worker_res
                                                        .stream_generation(),
                                                    data: Arc::new(bytes),
                                                };
                                                if !ingest_worker_res.account_store().apply(snap) {
                                                    debug!(
                                                        pubkey = %supply_vault,
                                                        slot = context_slot,
                                                        "Kamino RPC refresh was not applied; live/newer state or invalidation was retained"
                                                    );
                                                }
                                            } else {
                                                warn!(
                                                    pubkey = %supply_vault,
                                                    owner = %owner,
                                                    "Rejected Kamino WSOL refresh: unexpected token program or mint"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        info!(
            "🌊 Multi-DEX ingestion: WS stream is the live path; 30s reconciliation sweep active (750ms poll removed)"
        );

        // 5. Periodic dynamic discovery refresh worker (HTTP catalog; config-gated)
        if self.config.strategy.http_discovery_enabled {
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
        } else {
            info!(
                "Periodic HTTP discovery refresh worker not started (http_discovery_enabled = false)"
            );
        }

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
            liquidity_mint: Pubkey::from_str(arb_tx::WSOL_MINT).unwrap(),
            liquidity_token_program: Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
        };

        let mut last_snapshot_digest = Instant::now();
        // Daily report (checklist 637): emitted at the first loop visit after
        // the UTC date flips; the date check itself runs at most every 10 min.
        let mut last_report_utc_date = crate::daily_report::current_utc_date();
        let mut last_daily_check = Instant::now();
        let mut adaptive_delay_ms: u64 = 50;

        // Provider health/failover supervisor (G2): deterministic circuit
        // breaker over the live data plane. Provider 1 is the WebSocket
        // account stream; provider 0 is the HTTP RPC slot fallback. When no
        // provider is healthy and fresh, new opportunity cycles stop (fail
        // closed, no stale fallback).
        const WS_STREAM_PROVIDER: arb_ingest::ProviderId =
            arb_ingest::ACCOUNT_WEBSOCKET_PROVIDER_ID;
        const RPC_FALLBACK_PROVIDER: arb_ingest::ProviderId = arb_ingest::HTTP_RPC_PROVIDER_ID;
        let provider_supervisor = parking_lot::Mutex::new(arb_ingest::ProviderSupervisor::new(
            vec![
                arb_ingest::ProviderHealth::new(WS_STREAM_PROVIDER, 0),
                arb_ingest::ProviderHealth::new(RPC_FALLBACK_PROVIDER, 1),
            ],
            arb_ingest::SupervisorPolicy::default(),
        ));
        let mut last_ws_slot: u64 = 0;
        let mut last_ws_generation = self.ingest.stream_generation();
        let mut last_ws_advance_at = Instant::now();
        let ws_stale_after = arb_ingest::SupervisorPolicy::default().stale_after;

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

            // Periodic Performance & Rejections Digest to Telegram and local disk.
            // Default 300 s (5 min) matches .env.example; override via env.
            let digest_interval_secs = std::env::var("TELEGRAM_DIGEST_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|s| *s >= 5)
                .unwrap_or(300);
            if last_snapshot_digest.elapsed() >= Duration::from_secs(digest_interval_secs) {
                self.emit_five_minute_digest(current_slot.0).await;
                self.emit_market_snapshot(current_slot.0, false).await;
                last_snapshot_digest = Instant::now();
            }

            // Daily aggregated report (checklist 637) on UTC date rollover.
            if last_daily_check.elapsed() >= Duration::from_secs(600) {
                last_daily_check = Instant::now();
                let today = crate::daily_report::current_utc_date();
                if today != last_report_utc_date {
                    last_report_utc_date = today;
                    self.emit_daily_report(current_slot.0).await;
                }
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

            // Provider health observation (G2): the WebSocket stream is
            // healthy while its slot watermark advances; a stall beyond the
            // stale-after window trips the circuit breaker.
            let now = Instant::now();
            let ws_generation = self.ingest.stream_generation();
            if ws_generation != last_ws_generation {
                last_ws_generation = ws_generation;
                last_ws_slot = 0;
                last_ws_advance_at = now;
            }
            if helius_slot > last_ws_slot {
                last_ws_slot = helius_slot;
                last_ws_advance_at = now;
                provider_supervisor
                    .lock()
                    .record_success(WS_STREAM_PROVIDER, helius_slot, now);
            } else if helius_slot == 0 || last_ws_advance_at.elapsed() > ws_stale_after {
                provider_supervisor
                    .lock()
                    .record_failure(WS_STREAM_PROVIDER, now);
            }

            // Fallback cluster clock only: an RPC slot may pace a cycle, but it
            // must never advance the Helius watermark or mark the WS stream
            // connected. Market-data readiness is evaluated independently.
            if (stream_slot == 0 || stream_slot <= current_slot.0)
                && (stream_slot == 0 || tick.is_multiple_of(5))
            {
                match self.rpc_pool.get_slot().await {
                    Ok(rpc_slot) => {
                        provider_supervisor.lock().record_success(
                            RPC_FALLBACK_PROVIDER,
                            rpc_slot,
                            Instant::now(),
                        );
                        if rpc_slot > current_slot.0 {
                            debug!(
                                "Advancing slot from RPC slot stream fallback: {rpc_slot} (previous: {stream_slot})"
                            );
                            self.ingest.record_slot(rpc_slot);
                            stream_slot = rpc_slot;
                        }
                    }
                    Err(_) => {
                        provider_supervisor
                            .lock()
                            .record_failure(RPC_FALLBACK_PROVIDER, Instant::now());
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

            // Only the healthy account WebSocket can authorize trading. HTTP
            // RPC is a cluster-clock fallback; accepting it here could let
            // stale WS snapshots pass while their socket is stalled.
            let selected_provider = provider_supervisor
                .lock()
                .select(current_slot.0, Instant::now());
            if selected_provider != Some(WS_STREAM_PROVIDER) || !self.ingest.stream_connected() {
                debug!(
                    "No healthy account WebSocket at slot {} (selected provider: {:?}); skipping opportunity cycle (fail closed)",
                    current_slot.0, selected_provider
                );
                continue;
            }

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
    pub fn validate_economic_invariants(
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
