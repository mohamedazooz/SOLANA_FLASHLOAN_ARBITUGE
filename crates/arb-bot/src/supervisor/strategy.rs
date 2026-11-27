//! Per-tick strategy cycle (R9 split from run.rs): one slot-advanced tick —
//! verified economic inputs, market snapshot, candidate graph, opportunity
//! capture (2-pool DLMM / Bellman-Ford / multi-hop / 3-leg), risk gates,
//! transaction assembly and dispatch. Extracted verbatim; outer-tick `continue`
//! statements became `return Ok(())`, which is behavior-preserving because the
//! only code after the call site in `run()` is the loop bottom.
use super::*;
fn parse_positive_env_limit(name: &str) -> anyhow::Result<Option<f64>> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(None);
    };
    let value: f64 = raw
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{name} must be numeric"))?;
    if !value.is_finite() || value <= 0.0 {
        anyhow::bail!("{name} must be positive and finite");
    }
    Ok(Some(value))
}

fn apply_borrow_size_caps(
    max_sizing_lamports: u64,
    sol_usd: f64,
    max_trade_sol: Option<f64>,
    portfolio_capital_sol: Option<f64>,
    max_flash_borrow_usd: Option<f64>,
) -> u64 {
    if !sol_usd.is_finite() || sol_usd <= 0.0 {
        return 0;
    }
    let mut cap = max_sizing_lamports;
    for sol_cap in [max_trade_sol, portfolio_capital_sol].into_iter().flatten() {
        if !sol_cap.is_finite() || sol_cap <= 0.0 {
            return 0;
        }
        let lamports = (sol_cap * 1_000_000_000.0).floor();
        if !lamports.is_finite() || lamports < 1.0 || lamports > u64::MAX as f64 {
            return 0;
        }
        cap = cap.min(lamports as u64);
    }
    if let Some(usd_cap) = max_flash_borrow_usd {
        if !usd_cap.is_finite() || usd_cap <= 0.0 {
            return 0;
        }
        let lamports = (usd_cap / sol_usd * 1_000_000_000.0).floor();
        if !lamports.is_finite() || lamports < 1.0 || lamports > u64::MAX as f64 {
            return 0;
        }
        cap = cap.min(lamports as u64);
    }
    cap
}

fn apply_environment_borrow_caps(max_sizing_lamports: u64, sol_usd: f64) -> anyhow::Result<u64> {
    let max_trade_sol = parse_positive_env_limit("MAX_TRADE_SIZE_SOL")?;
    let portfolio_capital_sol = parse_positive_env_limit("PORTFOLIO_CAPITAL_SOL")?;
    let max_flash_borrow_usd = parse_positive_env_limit("MAX_FLASH_BORROW_USD")?;
    Ok(apply_borrow_size_caps(
        max_sizing_lamports,
        sol_usd,
        max_trade_sol,
        portfolio_capital_sol,
        max_flash_borrow_usd,
    ))
}
pub fn extract_pool_mid_price(
    p: &PoolState,
    expected_base_mint: &str,
    expected_quote_mint: &str,
    base_decimals: u8,
    quote_decimals: u8,
) -> Option<(f64, &'static str)> {
    let b_str = p.base_mint.0.to_string();
    let q_str = p.quote_mint.0.to_string();

    let is_direct = b_str == expected_base_mint && q_str == expected_quote_mint;
    let is_inverted = b_str == expected_quote_mint && q_str == expected_base_mint;

    if !is_direct && !is_inverted {
        return None;
    }

    let venue = match p.protocol {
        DexProtocol::OrcaWhirlpool => "Orca Whirlpool",
        DexProtocol::MeteoraDlmm => "Meteora DLMM",
        DexProtocol::RaydiumV4 | DexProtocol::RaydiumClmm | DexProtocol::RaydiumCpmm => "Raydium",
        DexProtocol::PumpSwap => "PumpSwap",
        DexProtocol::PumpFun => "Pump.fun",
        _ => "DEX Pool",
    };

    // p.base_mint corresponds to pool token A (decimals_a)
    // p.quote_mint corresponds to pool token B (decimals_b)
    let pool_dec_a = if p.decimals_a > 0 {
        p.decimals_a
    } else if is_direct {
        base_decimals
    } else {
        quote_decimals
    };
    let pool_dec_b = if p.decimals_b > 0 {
        p.decimals_b
    } else if is_direct {
        quote_decimals
    } else {
        base_decimals
    };

    let dec_diff = (pool_dec_a as i32) - (pool_dec_b as i32);

    // 1. CLMM (Orca Whirlpools, Raydium CLMM) via sqrt_price (Q64.64)
    if let Some(clmm) = &p.clmm {
        if clmm.sqrt_price > 0 {
            let sqrt_p = (clmm.sqrt_price as f64) / 18446744073709551616.0;
            let p_raw = sqrt_p * sqrt_p;
            let human_b_per_a = p_raw * 10f64.powi(dec_diff);
            if human_b_per_a > 0.0 && human_b_per_a.is_finite() {
                let price = if is_direct {
                    human_b_per_a
                } else {
                    1.0 / human_b_per_a
                };
                if price > 0.0 && price.is_finite() {
                    return Some((price, venue));
                }
            }
        }
    }

    // 2. Meteora DLMM via active_bin_id
    if let Some(dlmm) = &p.dlmm {
        if let Ok(price_q64) =
            arb_math::dlmm::get_price_from_id(dlmm.active_bin_id, dlmm.bin_step_bps)
        {
            if price_q64 > 0 {
                let p_raw = (price_q64 as f64) / 18446744073709551616.0;
                let human_b_per_a = p_raw * 10f64.powi(dec_diff);
                if human_b_per_a > 0.0 && human_b_per_a.is_finite() {
                    let price = if is_direct {
                        human_b_per_a
                    } else {
                        1.0 / human_b_per_a
                    };
                    if price > 0.0 && price.is_finite() {
                        return Some((price, venue));
                    }
                }
            }
        }
    }

    // 3. Constant Product / Standard reserves
    if p.reserve_in > 0 && p.reserve_out > 0 {
        let res_a = (p.reserve_in as f64) / 10f64.powi(pool_dec_a as i32);
        let res_b = (p.reserve_out as f64) / 10f64.powi(pool_dec_b as i32);
        if res_a > 0.0 && res_b > 0.0 {
            let human_b_per_a = res_b / res_a;
            if human_b_per_a > 0.0 && human_b_per_a.is_finite() {
                let price = if is_direct {
                    human_b_per_a
                } else {
                    1.0 / human_b_per_a
                };
                if price > 0.0 && price.is_finite() {
                    return Some((price, venue));
                }
            }
        }
    }

    None
}

impl Supervisor {
    pub async fn strategy_cycle(
        &self,
        current_slot: Slot,
        expected_stream_generation: u64,
        payer: Pubkey,
        kamino_params: &KaminoFlashParams,
    ) -> Result<()> {
        // Promote PumpSwap only after ingestion has supplied the Pool, both
        // vaults, both mints and canonical Pump Fees FeeConfig at coherent slots.
        // The graph uses a conservative whole-BPS fee for candidate discovery;
        // exact route validation remains protocol-adapter driven.
        let state_start = std::time::Instant::now();
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

        // Refresh on-chain SOL/USD spot reference and ticker BEFORE building
        // anchor economic inputs so cross-mint rate conversion sees current-slot
        // pool prices instead of a previous tick's stale observation slot.
        self.compute_and_publish_ticker(current_slot);

        // 0. Verified economic inputs (fail closed): hydrate live Kamino
        // reserve fee and decimals per configured anchor (WSOL + USDC).
        // Each anchor receives its own denomination-exact EconomicConfig; if
        // an anchor cannot be verified at current_slot, only that anchor fails
        // closed. If NO anchor is verifiable, skip the whole tick.
        let configured_anchors = {
            let g = self.graph.lock();
            g.base_anchors.clone()
        };
        let recent_landed_cu = self.recent_landed_cu.lock().clone();
        let fresh_market = {
            let market = self.recent_priority_market_cu.lock();
            market
                .as_ref()
                .filter(|(_, sampled_at)| {
                    sampled_at.elapsed() <= std::time::Duration::from_secs(12)
                })
                .map(|(samples, _)| samples.clone())
                .unwrap_or_default()
        };
        let desired_cu_price = self
            .tip_strategy
            .adaptive_cu_price_with_market(&recent_landed_cu, &fresh_market);

        let mut verified_anchors: Vec<(
            MintId,
            VerifiedQuoteInputs,
            arb_math::economic::EconomicConfig,
        )> = Vec::new();
        for anchor in &configured_anchors {
            match self
                .build_verified_inputs_for_mint(anchor.0, current_slot)
                .await
            {
                Ok(v) => {
                    let min_profit_base_raw =
                        match arb_types::dimensional::lamports_to_base_raw_checked(
                            self.config.strategy.min_net_profit_abs,
                            v.lamports_per_base_raw.value,
                            v.lamports_per_base_denom.value,
                            v.lamports_per_base_raw.observed_slot.0,
                            current_slot.0,
                            self.config.strategy.max_state_age_slots,
                        ) {
                            Ok(raw) => u64::try_from(raw).unwrap_or(u64::MAX),
                            Err(e) => {
                                debug!(
                                    anchor = %anchor.0,
                                    "cannot convert min_net_profit_abs for anchor ({e}); anchor disabled this tick"
                                );
                                continue;
                            }
                        };
                    let mut eco = v.to_economic_config(
                        TokenAmount(min_profit_base_raw),
                        self.config.strategy.hop_slippage_bps,
                    );
                    eco.priority_fee_micro_lamports = desired_cu_price;
                    verified_anchors.push((*anchor, v, eco));
                }
                Err(e) => {
                    debug!(
                        anchor = %anchor.0,
                        "verified economic inputs unavailable for anchor ({e}); anchor disabled this tick"
                    );
                }
            }
        }
        if verified_anchors.is_empty() {
            debug!(
                "verified economic inputs unavailable for all anchors; skipping tick (fail closed)"
            );
            return Ok(());
        }
        let wsol_mint = crate::verified_config::wsol_mint();
        let usdc_mint = crate::verified_config::usdc_mint();
        let (_, verified, primary_economic_config) = verified_anchors
            .iter()
            .find(|(m, _, _)| m.0 == wsol_mint)
            .unwrap_or(&verified_anchors[0])
            .clone();
        {
            let mut graph = self.graph.lock();
            graph.set_economic_config(primary_economic_config);
            for anchor in &configured_anchors {
                graph.clear_anchor_economic_config(anchor);
            }
            for (anchor, _, eco) in &verified_anchors {
                graph.set_anchor_economic_config(*anchor, *eco);
            }
            graph.set_priority_fee_policy(self.current_priority_fee_policy());
        }

        // HTTP catalog discovery is config-gated (strategy.http_discovery_enabled,
        // default false). When disabled, pool candidates arrive only through the
        // configured WebSocket account-update stream and must pass account
        // validation before entering the registry.

        // 1. Ingest: take one pool snapshot per strategy cycle; account updates mutate one key.
        //    Raw stream updates are promoted only after protocol-specific decoding.
        let quote_ingest_time = std::time::Instant::now();
        let market_snap = self.ingest.snapshot();
        let pools: Vec<PoolState> = market_snap.snapshot_pools();
        arb_metrics::observe_state_processing_latency_us(state_start.elapsed().as_micros() as f64);

        let discovery_start = std::time::Instant::now();

        // 2. Update Graph using fresh states only. Trade sizing is governed by RiskGate.
        // Finite WSOL-denominated candidate ceiling derived from the
        // configured USD notional and a fresh SOL/USD reference. The graph
        // is deliberately WSOL-anchored because Kamino's configured reserve
        // lends WSOL and repayment must close in the exact borrowed mint.
        let live_sol_usd = self.live_sol_price();
        let mut max_sizing = match live_sol_usd {
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

        if max_sizing > 0 {
            max_sizing = match apply_environment_borrow_caps(
                max_sizing,
                live_sol_usd.unwrap_or_default(),
            ) {
                Ok(capped) => capped,
                Err(error) => {
                    warn!(error = %error, "invalid configured borrow-size cap; sizing disabled");
                    0
                }
            };
        }

        // Compute per-anchor raw borrow ceilings (`WSOL` 9 decimals, `USDC` 6 decimals),
        // strictly clamped by verified live Kamino supply-vault liquidity per mint.
        let mut anchor_caps: std::collections::HashMap<MintId, u64> =
            std::collections::HashMap::new();
        for (anchor, _, _) in &verified_anchors {
            let mut cap_raw = if anchor.0 == wsol_mint {
                max_sizing
            } else if anchor.0 == usdc_mint {
                if self.config.risk.max_trade_size_usd.is_finite()
                    && self.config.risk.max_trade_size_usd > 0.0
                {
                    let mut usdc_cap = (self.config.risk.max_trade_size_usd * 1_000_000.0)
                        .floor()
                        .clamp(0.0, u64::MAX as f64) as u64;
                    if let Ok(Some(env_usd)) = parse_positive_env_limit("MAX_FLASH_BORROW_USD") {
                        let env_raw =
                            (env_usd * 1_000_000.0).floor().clamp(0.0, u64::MAX as f64) as u64;
                        usdc_cap = usdc_cap.min(env_raw);
                    }
                    usdc_cap
                } else {
                    0
                }
            } else {
                0
            };

            if cap_raw > 0 && self.config.strategy.use_flash_loan {
                match self
                    .kamino_available_flash_liquidity_for_mint_detailed(&anchor.0, current_slot)
                {
                    Ok(liq) => {
                        cap_raw = cap_raw.min(liq);
                    }
                    Err(reason) => {
                        debug!(
                            anchor = %anchor.0,
                            reason,
                            slot = current_slot.0,
                            "Kamino flash liquidity unavailable for anchor; fail closed"
                        );
                        cap_raw = 0;
                    }
                }
            }

            if anchor.0 == wsol_mint {
                max_sizing = cap_raw;
            }
            anchor_caps.insert(*anchor, cap_raw);
        }

        if anchor_caps.values().all(|&cap| cap == 0) {
            debug!(
                "Missing fresh SOL/USD reference, invalid USD size cap, or zero/unverified flash liquidity across all anchors; skip graph sizing"
            );
            return Ok(());
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
            return Ok(());
        }

        // Record protocol-native, all-cost diagnostic pair probes across fresh pools only.
        self.evaluate_and_record_cross_pool_routes(
            current_slot,
            &fresh_pools,
            max_sizing,
            &verified,
            desired_cu_price,
        );

        let mut merged = {
            let mut graph = self.graph.lock();
            let build = graph.build_from_pools(&fresh_pools);
            let (mut candidates, scan) = graph.find_cycles_with_anchor_caps(
                current_slot,
                &anchor_caps,
                self.config.strategy.max_route_hops,
                arb_engine::MAX_ROUTE_CANDIDATES_PER_TICK,
            );

            // DLMM cross-pool discovery remains a venue-specific candidate
            // generator. Every result is re-quoted and re-sized by the same
            // canonical route evaluator before it can enter the opportunity set.
            let dcp_cfg = self
                .config
                .strategy
                .dlmm_cross_pool
                .clone()
                .unwrap_or_default();
            let cross_cfg = arb_engine::DlmmCrossPoolConfig {
                max_pairs: dcp_cfg.max_pairs,
                min_input: dcp_cfg.min_input,
                max_input: dcp_cfg.max_input.min(max_sizing),
                coarse_samples: dcp_cfg.coarse_samples,
                refine_samples: dcp_cfg.refine_samples,
                min_spread_bps: dcp_cfg.min_spread_bps,
                min_liquidity_bin_raw: dcp_cfg.min_liquidity_bin_raw,
            };
            let mut cross_accepted = 0usize;
            if max_sizing > 0 {
                let cross_scan =
                    graph.find_cross_pool_dlmm_cycles(current_slot, max_sizing, &cross_cfg);
                for candidate in cross_scan.opportunities {
                    if let Some(requoted) = graph.evaluate_directed_route(
                        current_slot,
                        max_sizing,
                        candidate.route_id,
                        &candidate.hops,
                    ) {
                        candidates.push(requoted);
                        cross_accepted += 1;
                    }
                }
                if let Some(control) = self.control() {
                    control.record_evaluations(cross_scan.stats.directions_evaluated);
                }
                info!(
                    "DLMM cross-pool seed scan: pairs={} directions={} seeds={} canonical_accepted={} rejections={}",
                    cross_scan.stats.pairs_scanned,
                    cross_scan.stats.directions_evaluated,
                    cross_scan.stats.opportunities,
                    cross_accepted,
                    cross_scan.rejections.len(),
                );
            }
            info!(
                "Unified route engine slot={}: pools={}/{} edges={} excluded[inactive={},raydium_clmm={},reserves={},risky={},transfer_fee={},fee={},unsupported={}] anchors={} cycles={} gross_negative={} quote_rejected={} economic_rejected={} opportunities={} truncated={}",
                current_slot.0,
                build.pools_indexed,
                build.pools_seen,
                build.edges_indexed,
                build.excluded_inactive,
                build.excluded_raydium_clmm,
                build.excluded_missing_reserves,
                build.excluded_risky_token,
                build.excluded_unmodeled_transfer_fee,
                build.excluded_invalid_fee,
                build.excluded_unsupported_protocol,
                scan.base_anchors,
                scan.cycles_evaluated,
                scan.gross_negative,
                scan.quote_rejected,
                scan.gate_rejected,
                scan.opportunities,
                scan.truncated,
            );
            super::route_dedup::dedupe_best_by_route(candidates)
        };

        // Specialized Meteora 3-leg discovery proves chained DLMM bin quotes
        // and wallet inventory provenance. Its routes are then re-evaluated by
        // the canonical MarketGraph quote/sizing/economics path.
        let flash_liquidity = if max_sizing > 0 {
            self.kamino_available_flash_liquidity(current_slot)
        } else {
            None
        };
        let flash_liquidity_unverified =
            max_sizing == 0 || (self.config.strategy.use_flash_loan && flash_liquidity.is_none());
        if flash_liquidity_unverified {
            debug!(
                "3-leg capture skipped: WSOL sizing zero or flash liquidity unverifiable at slot {}",
                current_slot.0
            );
        } else {
            let three_leg_seeds = self
                .capture_three_leg_dlmm(
                    &fresh_pools,
                    current_slot,
                    max_sizing,
                    verified.flash_loan_fee_sf.value,
                    flash_liquidity,
                    verified.tip_lamports.value.0,
                )
                .await;
            let mut canonical_accepted = 0usize;
            {
                let graph = self.graph.lock();
                for seed in three_leg_seeds {
                    if let Some(requoted) = graph.evaluate_directed_route(
                        current_slot,
                        max_sizing,
                        seed.route_id,
                        &seed.hops,
                    ) {
                        merged.push(requoted);
                        canonical_accepted += 1;
                    }
                }
            }
            info!(
                "Meteora 3-leg DLMM: canonical_accepted={} (inventory proof and exact chained quote required)",
                canonical_accepted,
            );
            merged = super::route_dedup::dedupe_best_by_route(merged);
        }

        arb_metrics::observe_route_discovery_latency_us(
            discovery_start.elapsed().as_micros() as f64
        ); // ── Pump.fun BOOST Migration surveillance pass ──
        // Scan active Pump.fun curves nearing completion in the sweet-spot band [40..75 SOL].
        // Log detected targets and record them in the ControlService dashboard.
        let boost_targets = {
            let pump_discovery = arb_ingest::PumpFunDiscoveryClient::default();
            // Convert known pools into PumpFunCoins for sweet-spot evaluation
            let mut pump_coins = Vec::new();
            for p in &fresh_pools {
                if p.protocol == DexProtocol::PumpFun {
                    if let Some(curve) = &p.pump_fun {
                        pump_coins.push(arb_ingest::PumpFunCoin {
                            mint: p.quote_mint.0.to_string(),
                            name: None,
                            symbol: None,
                            description: None,
                            bonding_curve: Some(p.pool_id.0.to_string()),
                            associated_bonding_curve: None,
                            virtual_sol_reserves: Some(curve.virtual_sol_reserves),
                            virtual_token_reserves: Some(curve.virtual_token_reserves),
                            total_supply: Some(curve.token_total_supply),
                            complete: curve.complete,
                            market_cap: None,
                            usd_market_cap: None,
                            reply_count: None,
                            last_trade_timestamp: None,
                            raydium_pool: None,
                        });
                    }
                }
            }

            pump_discovery.find_boost_migration_targets(
                &pump_coins,
                verified.flash_loan_fee_sf.value, // raw U68F60 sf — canonical fee path (MATH-03)
                verified.tip_lamports.value.0,
                self.config.strategy.min_net_profit_abs,
            )
        };

        if !boost_targets.is_empty() {
            info!(
                "🔥 Pump.fun BOOST surveillance detected {} target curve(s) in migration sweet spot",
                boost_targets.len()
            );
            if let Some(control) = self.control() {
                for t in boost_targets.iter().take(5) {
                    control.record_opportunity(crate::control_service::OpportunityRecord {
                        captured_at_unix: 0,
                        slot: current_slot.0,
                        route: format!("PumpFunCurve({}) \u{2192} BOOST PumpSwap", t.symbol),
                        protocol: "pumpfun-boost-migration".to_string(),
                        borrow_lamports: t.working_capital_eval.completion_cost_lamports,
                        expected_output_lamports: t.working_capital_eval.gross_sol_out_lamports,
                        gross_profit_lamports: 0,
                        net_profit_lamports: t.working_capital_eval.net_profit_lamports,
                        eligible: t.working_capital_eval.is_executable,
                        hop_quotes: Vec::new(),
                        reject_reason: if t.working_capital_eval.is_executable {
                            None
                        } else {
                            Some("BELOW_MIN_PROFIT_HURDLE".to_string())
                        },
                    });
                }
            }
        }

        let opportunities = rank_opportunities_cross_mint(merged);

        // Feed evaluated cross-pool 2-leg and multi-hop opportunities into the control service & live WS / telemetry
        if let Some(control) = self.control() {
            for o in opportunities.iter().take(24) {
                let proto_str = if o.hops.len() == 2 {
                    if o.hops[0].protocol == o.hops[1].protocol {
                        format!("{:?}", o.hops[0].protocol).to_lowercase()
                    } else {
                        format!("{:?}-to-{:?}", o.hops[0].protocol, o.hops[1].protocol)
                            .to_lowercase()
                    }
                } else if o.hops.len() == 1 {
                    format!("{:?}", o.hops[0].protocol).to_lowercase()
                } else {
                    format!("multi-hop-{}leg", o.hops.len())
                };

                let hop_quotes: Vec<crate::control_service::HopQuoteRecord> = o
                    .hops
                    .iter()
                    .enumerate()
                    .map(|(i, h)| {
                        let amount_in = o.hop_inputs.get(i).copied().unwrap_or(o.size_in.0);
                        let amount_out =
                            o.hop_inputs.get(i + 1).copied().unwrap_or(o.expected_out.0);
                        crate::control_service::HopQuoteRecord {
                            protocol: format!("{:?}", h.protocol),
                            pool: h.pool_id.0.to_string(),
                            amount_in,
                            amount_out,
                        }
                    })
                    .collect();

                let route_label = o
                    .hops
                    .iter()
                    .map(|h| format!("{}", h.pool_id))
                    .collect::<Vec<_>>()
                    .join("\u{2192}");

                let base_mint = o.hops.first().map(|h| h.mint_in);
                let min_hurdle_raw = if base_mint == Some(MintId(wsol_mint)) {
                    arb_risk::minimum_net_profit_hurdle_lamports(
                        Lamports(o.size_in.0),
                        Lamports(self.config.strategy.min_net_profit_abs),
                        self.config.strategy.min_net_profit_bps,
                    )
                    .as_u64()
                } else {
                    let abs_floor = base_mint
                        .and_then(|m| {
                            verified_anchors
                                .iter()
                                .find(|(a, _, _)| *a == m)
                                .map(|(_, _, eco)| eco.min_profit_raw.0)
                        })
                        .unwrap_or(u64::MAX);
                    let bps_floor = o
                        .size_in
                        .0
                        .saturating_mul(self.config.strategy.min_net_profit_bps)
                        .div_ceil(10_000);
                    abs_floor.max(bps_floor)
                };
                let is_eligible = o.net_profit.0 >= min_hurdle_raw && o.net_profit.0 > 0;

                control.record_opportunity(crate::control_service::OpportunityRecord {
                    captured_at_unix: 0,
                    slot: current_slot.0,
                    route: route_label,
                    protocol: proto_str,
                    borrow_lamports: o.size_in.0,
                    expected_output_lamports: o.expected_out.0,
                    gross_profit_lamports: o.gross_profit.0 as i64,
                    net_profit_lamports: o.net_profit.0 as i64,
                    eligible: is_eligible,
                    hop_quotes,
                    reject_reason: if is_eligible {
                        None
                    } else {
                        Some("BELOW_MIN_PROFIT_HURDLE".to_string())
                    },
                });
            }
        }

        let t_ingest_to_quote_us = quote_ingest_time.elapsed().as_micros() as u64;
        self.heartbeat
            .observe_quote(current_slot.0, opportunities.len());

        self.process_opportunities(
            &opportunities,
            super::opportunity::OpportunityPass {
                current_slot,
                expected_stream_generation,
                verified: verified.clone(),
                payer,
                kamino_params,
                t_ingest_to_quote_us,
                pools: &pools,
            },
        )
        .await;

        Ok(())
    }
}

/// PIPE-13: cross-mint ranking for the merged opportunity set.
///
/// `net_profit` values from different sources are denominated in their own
/// base mint's raw lamport-equivalent unit. WSOL-anchored routes are exact
/// lamports; for non-WSOL bases the conversion rides a price assumption
/// (`derive_rational_rate`), so comparing raw units across mints mixes
/// apples and oranges — a 6-decimal meme token's raw net can trivially
/// dwarf a real WSOL profit by scale alone.
///
/// Deterministic, conservative order:
/// 1. WSOL-anchored opportunities (exact common quote) first, by net profit
///    descending — verified economics always outrank assumed conversions.
/// 2. Every other base mint as its own group (first-seen insertion order),
///    each ranked internally by net profit descending. Group-local ranking
///    only affects processing latency, never per-dispatch accounting: every
///    opportunity is independently re-validated and re-costed before
///    dispatch, so a locally-imprecise group order is honest and safe.
pub(super) fn rank_opportunities_cross_mint(
    opps: Vec<ExecutionOpportunity>,
) -> Vec<ExecutionOpportunity> {
    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).expect("WSOL mint constant parses");
    let by_net_desc =
        |a: &ExecutionOpportunity, b: &ExecutionOpportunity| b.net_profit.0.cmp(&a.net_profit.0);

    let mut wsol_group: Vec<ExecutionOpportunity> = Vec::new();
    let mut other_groups: Vec<(MintId, Vec<ExecutionOpportunity>)> = Vec::new();
    let mut unanchored: Vec<ExecutionOpportunity> = Vec::new();
    for o in opps {
        match o.hops.first().map(|h| h.mint_in) {
            Some(m) if m.0 == wsol => wsol_group.push(o),
            Some(m) => {
                if let Some(group) = other_groups.iter_mut().find(|(gm, _)| *gm == m) {
                    group.1.push(o);
                } else {
                    other_groups.push((m, vec![o]));
                }
            }
            // No hops => no base mint to group by; keep in arrival order at
            // the tail (nothing comparable was averred about their profits).
            None => unanchored.push(o),
        }
    }
    wsol_group.sort_by(by_net_desc);
    for group in other_groups.iter_mut().map(|(_, g)| g) {
        group.sort_by(by_net_desc);
    }

    let mut ranked = wsol_group;
    ranked.extend(other_groups.into_iter().flat_map(|(_, g)| g));
    ranked.extend(unanchored);
    ranked
}

#[cfg(test)]
mod rank_tests {
    use super::*;
    #[cfg(test)]
    use arb_types::{Bps, RouteId};
    use arb_types::{PoolId, PumpSwapTradeIntent, TokenProgramKind};

    fn hop(mint_in: MintId, mint_out: MintId) -> SwapHop {
        SwapHop {
            pool_id: PoolId::new([1u8; 32]),
            protocol: DexProtocol::MeteoraDlmm,
            mint_in,
            mint_out,
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [TokenProgramKind::Token2022, TokenProgramKind::Token2022],
            trade_intent: PumpSwapTradeIntent::default(),
            execution_accounts: None,
        }
    }

    fn opp(base: MintId, net: u64) -> ExecutionOpportunity {
        let wsol = MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap());
        ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(1),
            size_in: TokenAmount(1),
            expected_out: TokenAmount(1),
            hop_inputs: Vec::new(),
            gross_profit: Lamports(net),
            net_profit: Lamports(net),
            base_decimals: 9,
            price_impact_bps: Bps(0),
            hops: vec![hop(base, wsol)],
            expected_min_final_output: 0,
            hop_slippage_bps: 0,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
        }
    }

    /// PIPE-13 regression: a huge raw-unit net on a 6-decimal meme base must
    /// not outrank an exact WSOL-lamport profit (the old direct sort placed
    /// it first by scale alone); same-mint ordering is respected.
    #[test]
    fn borrow_size_uses_the_smallest_configured_cap() {
        let capped =
            apply_borrow_size_caps(1_000_000_000, 100.0, Some(0.50), Some(0.20), Some(10.0));
        assert_eq!(
            capped, 100_000_000,
            "$10 flash-borrow cap is the tightest at 0.10 SOL"
        );
    }

    #[test]
    fn borrow_size_converts_flash_usd_cap_at_the_live_sol_price() {
        let capped = apply_borrow_size_caps(1_000_000_000, 100.0, Some(1.0), Some(2.0), Some(25.0));
        assert_eq!(capped, 250_000_000, "$25 at $100/SOL is 0.25 SOL");
    }

    #[test]
    fn invalid_price_or_nonpositive_cap_fails_closed() {
        assert_eq!(
            apply_borrow_size_caps(1_000_000_000, 0.0, None, None, None),
            0
        );
        assert_eq!(
            apply_borrow_size_caps(1_000_000_000, 100.0, Some(0.0), None, None),
            0
        );
        assert_eq!(
            apply_borrow_size_caps(1_000_000_000, 100.0, None, Some(f64::NAN), None),
            0
        );
        assert_eq!(
            apply_borrow_size_caps(1_000_000_000, 100.0, None, None, Some(-1.0)),
            0
        );
    }
    #[test]
    fn wsol_anchored_profits_outrank_assumed_cross_mint_conversions() {
        let wsol = MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap());
        let meme_6dec = MintId(Pubkey::new_from_array([9u8; 32]));
        let meme_b = MintId(Pubkey::new_from_array([8u8; 32]));

        let ranked = rank_opportunities_cross_mint(vec![
            opp(meme_6dec, u64::MAX / 2), // raw-scale inflated
            opp(wsol, 100),
            opp(meme_b, u64::MAX - 1), // different meme mint
            opp(wsol, 200),
        ]);

        assert_eq!(ranked[0].net_profit, Lamports(200));
        assert_eq!(ranked[1].net_profit, Lamports(100));
        // Assumed-conversion groups keep deterministic first-seen order and
        // are ranked internally, after every exact-WSOL profit.
        let meme_group_ids: Vec<[u8; 32]> = ranked[2..]
            .iter()
            .map(|o| o.hops[0].mint_in.0.to_bytes())
            .collect();
        assert_eq!(meme_group_ids[0], [9u8; 32]);
        assert_eq!(meme_group_ids[1], [8u8; 32]);
    }

    fn test_pool(
        protocol: DexProtocol,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        decimals_a: u8,
        decimals_b: u8,
        reserve_in: u128,
        reserve_out: u128,
    ) -> PoolState {
        PoolState {
            pool_id: arb_types::PoolId::new([1u8; 32]),
            protocol,
            slot: Slot(100),
            write_version: 1,
            base_mint: MintId(base_mint),
            quote_mint: MintId(quote_mint),
            decimals_a,
            decimals_b,
            reserve_in,
            reserve_out,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 100,
            last_update_ms: 0,
        }
    }

    #[test]
    fn test_extract_pool_mid_price_direct_and_inverted() {
        use arb_types::ClmmState;

        let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();

        // 1. Direct CPMM pool: base = WSOL (9 dec), quote = USDC (6 dec)
        // 100 SOL (100 * 10^9 atomic) and 20,000 USDC (20,000 * 10^6 atomic)
        // Price should be 20,000 / 100 = 200.0 USD per SOL.
        let mut pool_direct = test_pool(
            DexProtocol::RaydiumV4,
            wsol,
            usdc,
            9,
            6,
            100_000_000_000,
            20_000_000_000,
        );

        let res = extract_pool_mid_price(
            &pool_direct,
            arb_constants::WSOL_MINT,
            arb_constants::USDC_MINT,
            9,
            6,
        );
        assert!(res.is_some());
        let (price, venue) = res.unwrap();
        assert_eq!(venue, "Raydium");
        assert!(
            (price - 200.0).abs() < 1e-4,
            "direct CPMM pool price expected ~200.0, got {price}"
        );

        // 2. Inverted CPMM pool: base = USDC (6 dec), quote = WSOL (9 dec)
        // 20,000 USDC (20,000 * 10^6 atomic) and 100 SOL (100 * 10^9 atomic)
        // Price of SOL in USDC must still be 200.0!
        let mut pool_inverted = test_pool(
            DexProtocol::RaydiumV4,
            usdc,
            wsol,
            6,
            9,
            20_000_000_000,
            100_000_000_000,
        );

        let res_inv = extract_pool_mid_price(
            &pool_inverted,
            arb_constants::WSOL_MINT,
            arb_constants::USDC_MINT,
            9,
            6,
        );
        assert!(res_inv.is_some());
        let (price_inv, _) = res_inv.unwrap();
        assert!(
            (price_inv - 200.0).abs() < 1e-4,
            "inverted CPMM pool price expected ~200.0, got {price_inv}"
        );

        // 3. Direct CLMM pool (Orca Whirlpool): base = WSOL (9 dec), quote = USDC (6 dec)
        // Raw price = 0.2 (200 * 10^-3). sqrt(0.2) * 2^64
        let sqrt_0_2 = 0.4472135954999579_f64;
        let clmm_direct_q64 = (sqrt_0_2 * 18446744073709551616.0) as u128;
        pool_direct.protocol = DexProtocol::OrcaWhirlpool;
        pool_direct.clmm = Some(ClmmState {
            sqrt_price: clmm_direct_q64,
            liquidity: 1_000_000,
            tick_current: 0,
            tick_spacing: 1,
            fee_rate: 25,
            tick_arrays: Vec::new(),
        });

        let res_clmm_dir = extract_pool_mid_price(
            &pool_direct,
            arb_constants::WSOL_MINT,
            arb_constants::USDC_MINT,
            9,
            6,
        );
        assert!(res_clmm_dir.is_some());
        let (p_clmm_dir, venue) = res_clmm_dir.unwrap();
        assert_eq!(venue, "Orca Whirlpool");
        assert!(
            (p_clmm_dir - 200.0).abs() < 0.01,
            "direct CLMM price expected ~200.0, got {p_clmm_dir}"
        );

        // 4. Inverted CLMM pool (Orca Whirlpool): base = USDC (6 dec), quote = WSOL (9 dec)
        // Raw price = 5.0 (10^3 / 200). sqrt(5.0) * 2^64
        let sqrt_5_0 = 2.23606797749979_f64;
        let clmm_inv_q64 = (sqrt_5_0 * 18446744073709551616.0) as u128;
        pool_inverted.protocol = DexProtocol::OrcaWhirlpool;
        pool_inverted.clmm = Some(ClmmState {
            sqrt_price: clmm_inv_q64,
            liquidity: 1_000_000,
            tick_current: 0,
            tick_spacing: 1,
            fee_rate: 25,
            tick_arrays: Vec::new(),
        });

        let res_clmm_inv = extract_pool_mid_price(
            &pool_inverted,
            arb_constants::WSOL_MINT,
            arb_constants::USDC_MINT,
            9,
            6,
        );
        assert!(res_clmm_inv.is_some());
        let (p_clmm_inv, _) = res_clmm_inv.unwrap();
        assert!(
            (p_clmm_inv - 200.0).abs() < 0.01,
            "inverted CLMM price expected ~200.0, got {p_clmm_inv}"
        );
    }

    #[test]
    fn median_price_rejects_single_pool_outlier_and_diagnostic_pair_never_marks_eligible() {
        let mut prices = vec![73.0, 73.1, 72.9, 500.0];
        let med = Supervisor::median_price(&mut prices).unwrap();
        assert!(
            (med - 73.05).abs() < 1e-9,
            "expected median ~73.05, got {med}"
        );

        let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
        let mut p1 = test_pool(
            DexProtocol::RaydiumV4,
            wsol,
            usdc,
            9,
            6,
            1_000_000_000_000,
            73_000_000_000,
        );
        p1.pool_id = PoolId::new([1u8; 32]);
        let mut p2 = test_pool(
            DexProtocol::RaydiumV4,
            wsol,
            usdc,
            9,
            6,
            1_000_000_000_000,
            73_050_000_000,
        );
        p2.pool_id = PoolId::new([2u8; 32]);

        let cfg = crate::config::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/base.toml"
        ))
        .unwrap();
        let mode = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(
            crate::BotMode::Simulate as u8,
        ));
        let sup = Supervisor::with_mode_atomic(std::sync::Arc::clone(&mode), cfg);
        let control = std::sync::Arc::new(crate::control_service::ControlService::new(
            mode,
            sup.kill_switch(),
            std::env::temp_dir()
                .join(format!("arb_diag_test_{}.log", std::process::id()))
                .to_string_lossy()
                .into_owned(),
        ));
        sup.attach_control(std::sync::Arc::clone(&control));

        let verified = crate::verified_config::VerifiedQuoteInputs {
            flash_loan_fee_sf: crate::verified_config::VerifiedValue::new(
                11_529_215_046_068,
                "test",
                Slot(100),
                Slot(100),
            ),
            base_decimals: crate::verified_config::VerifiedValue::new(
                9,
                "test",
                Slot(100),
                Slot(100),
            ),
            lamports_per_base_raw: crate::verified_config::VerifiedValue::new(
                1,
                "test",
                Slot(100),
                Slot(100),
            ),
            lamports_per_base_denom: crate::verified_config::VerifiedValue::new(
                1,
                "test",
                Slot(100),
                Slot(100),
            ),
            tip_lamports: crate::verified_config::VerifiedValue::new(
                Lamports(10_000),
                "test",
                Slot(100),
                Slot(100),
            ),
        };
        sup.evaluate_and_record_cross_pool_routes(
            Slot(100),
            &[p1, p2],
            100_000_000, // 0.1 SOL
            &verified,
            100_000,
        );
        let recorded = control.opportunities(5);
        assert_eq!(recorded.len(), 1);
        assert!(!recorded[0].eligible);
        assert!(recorded[0].gross_profit_lamports < 0);
        assert!(recorded[0].net_profit_lamports < recorded[0].gross_profit_lamports);
        assert_eq!(control.status().profitable_total, 0);
    }
}
