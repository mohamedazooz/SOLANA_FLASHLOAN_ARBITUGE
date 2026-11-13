//! Opportunity processing (R9 split from strategy.rs): the per-candidate gate
//! ladder — risk gate, tip ladder, decoupled profit protection, implausibility
//! gate, EV gate, economic invariants — followed by transaction assembly,
//! V1 compilation and dispatch. Extracted verbatim from the strategy cycle.
use super::*;

/// Everything a per-tick opportunity pass needs from the enclosing cycle.
pub(super) struct OpportunityPass<'a> {
    pub current_slot: Slot,
    pub expected_stream_generation: u64,
    pub verified: VerifiedQuoteInputs,
    pub payer: Pubkey,
    pub kamino_params: &'a KaminoFlashParams,
    pub t_ingest_to_quote_us: u64,
    pub pools: &'a [PoolState],
}

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

impl Supervisor {
    pub(super) async fn process_opportunities(
        &self,
        opportunities: &[ExecutionOpportunity],
        pass: OpportunityPass<'_>,
    ) {
        let OpportunityPass {
            current_slot,
            expected_stream_generation,
            verified,
            payer,
            kamino_params,
            t_ingest_to_quote_us,
            pools,
        } = pass;
        let quote_ingest_time =
            std::time::Instant::now() - Duration::from_micros(t_ingest_to_quote_us);
        for opp in opportunities {
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
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Jito Leader Gate".to_string(),
                            reason: reason.clone(),
                            slot: current_slot.0,
                        });
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
                    "Opportunity rejected: implied ROI {implied_roi_bps} bps exceeds implausibility ceiling {max_implausible_roi_bps} bps (stale-quote artifact); route={route_label} size_in={} slot={}",
                    opp.size_in.as_u64(),
                    current_slot.0
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
                        let reason = format!("تعذر تحويل حد الربح الأدنى إلى الوحدات الأساسية: {e}");
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
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Chronos EV Gate".to_string(),
                            reason: reason_msg.clone(),
                            slot: current_slot.0,
                        });
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
            let state_age_slots = max_state_age_slots(&opp.hops, pools, current_slot);

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
            let bypass_risk_gate = !matches!(self.mode(), BotMode::Live)
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
                    self.mode()
                );
            }

            let risk_check = if bypass_risk_gate {
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
                                Lamports(
                                    arb_types::dimensional::base_raw_to_lamports_checked(
                                        real_net_base_raw as u128,
                                        verified.lamports_per_base_raw.value,
                                        verified.lamports_per_base_denom.value,
                                    )
                                    .unwrap_or(0),
                                ),
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
                                Lamports(
                                    arb_types::dimensional::base_raw_to_lamports_checked(
                                        real_net_base_raw as u128,
                                        verified.lamports_per_base_raw.value,
                                        verified.lamports_per_base_denom.value,
                                    )
                                    .unwrap_or(0),
                                ),
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
                    match arb_tx::build_swap_hop_instruction(return_hop, payer, total_out, min_out)
                    {
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
                    kamino_params,
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
                        kamino_params,
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
            let mut last_valid_block_height: Option<i64> = None;
            let tx_bytes = if let Some(ref signer) = self.keypair {
                let (blockhash, lvbh) = self.fetch_recent_blockhash().await;
                last_valid_block_height = lvbh;
                blockhash_b58 = bs58::encode(blockhash).into_string();
                // Fail fast: a zero blockhash must never be sent.
                if blockhash == [0u8; 32] {
                    let reason_msg = "فشل جلب أحدث Blockhash صالح من شبكة سولانا (Zero Blockhash)";
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
                    matches!(self.mode(), BotMode::Shadow | BotMode::Simulate) || direct_v1_enabled;
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
                let (blockhash, _lvbh) = self.fetch_recent_blockhash().await;
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
            static ATTEMPT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
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
                decision: format!("{}", self.mode()),
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
                if self.mode() == BotMode::Live {
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
                        self.record_rejected_opportunity(crate::telegram::RejectedTradeSummary {
                            route: route_label.clone(),
                            gross_sol,
                            stage: "Pre-flight Simulation".to_string(),
                            reason,
                            slot: current_slot.0,
                        });
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
                last_valid_block_height,
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
    }
}
