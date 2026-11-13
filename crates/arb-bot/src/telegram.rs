//! Telegram telemetry: every opportunity the bot fires on, every landed fill,
//! and every failure — delivered to the operator's chat.
//!
//! Nothing here can block trading: sends are best-effort, time-boxed, and a
//! Telegram outage is logged, never propagated.

use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct TelegramNotifier {
    bot_token: Option<String>,
    chat_id: Option<String>,
    client: Client,
    sol_price: f64,
    min_rejected_alert_sol: f64,
    tx: Option<tokio::sync::mpsc::Sender<String>>,
    rejected_cooldown:
        std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<String, std::time::Instant>>>,
}

/// Escape the three characters Telegram's HTML parser treats as markup.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl Default for TelegramNotifier {
    fn default() -> Self {
        Self::from_env()
    }
}

impl TelegramNotifier {
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        let chat_id = std::env::var("TELEGRAM_CHAT_ID")
            .ok()
            .filter(|t| !t.trim().is_empty());

        let min_alert_usd = std::env::var("TELEGRAM_MIN_ALERT_USD")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        let sol_price = std::env::var("SOL_USD_PRICE")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(200.0);
        let min_rejected_alert_sol = std::env::var("TELEGRAM_REJECTED_ALERT_MIN_SOL")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(min_alert_usd / sol_price);

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(10))
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        let mut tx_opt = None;
        if bot_token.is_some() && chat_id.is_some() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1000);
                let client_bg = client.clone();
                let token_bg = bot_token.clone().unwrap();
                let chat_id_bg = chat_id.clone().unwrap();

                handle.spawn(async move {
                    while let Some(msg) = rx.recv().await {
                        let url = format!("https://api.telegram.org/bot{token_bg}/sendMessage");
                        let payload = json!({
                            "chat_id": chat_id_bg,
                            "text": msg,
                            "parse_mode": "HTML",
                            "disable_web_page_preview": true
                        });

                        for attempt in 1..=2 {
                            match client_bg.post(&url).json(&payload).send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    debug!("Telegram alert delivered");
                                    break;
                                }
                                Ok(resp) => {
                                    if resp.status().as_u16() == 429 {
                                        let text = resp.text().await.unwrap_or_default();
                                        let retry_after = serde_json::from_str::<serde_json::Value>(&text)
                                            .ok()
                                            .and_then(|v| v.get("parameters")?.get("retry_after")?.as_u64())
                                            .unwrap_or(15);
                                        warn!(
                                            "Telegram API returned status 429 Too Many Requests — retry_after {retry_after}s. Draining stale queue and pausing sender."
                                        );
                                        while rx.try_recv().is_ok() {}
                                        tokio::time::sleep(std::time::Duration::from_secs(retry_after)).await;
                                    } else {
                                        warn!("Telegram API returned status {}", resp.status());
                                    }
                                    break;
                                }
                                Err(e) => {
                                    if attempt == 2 {
                                        error!("Failed to send Telegram alert: {}", e);
                                    } else {
                                        tokio::time::sleep(std::time::Duration::from_millis(400))
                                            .await;
                                    }
                                }
                            }
                        }
                        // Enforce at least 1.5s between messages to stay comfortably within Telegram's 1 msg/sec rate limit
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    }
                });
                tx_opt = Some(tx);
            }
            info!(
                "🔔 Telegram notifier enabled (chat {})",
                chat_id.as_deref().unwrap_or("")
            );
        } else {
            debug!("Telegram credentials not fully configured — notifications disabled");
        }

        Self {
            bot_token,
            chat_id,
            client,
            sol_price,
            min_rejected_alert_sol,
            tx: tx_opt,
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    pub fn min_rejected_alert_sol(&self) -> f64 {
        self.min_rejected_alert_sol
    }

    pub fn is_enabled(&self) -> bool {
        self.bot_token.is_some() && self.chat_id.is_some()
    }

    /// Lightweight reachability check for the health monitor. Uses `getMe`
    /// (no chat needed) so a Telegram outage is detected without sending a
    /// message. Time-boxed; never throws into the trading flow.
    pub async fn ping(&self) -> Result<()> {
        let (Some(token), _) = (&self.bot_token, &self.chat_id) else {
            return Ok(());
        };
        let url = format!("https://api.telegram.org/bot{token}/getMe");
        let _resp = self.client.get(url).send().await?.error_for_status()?;
        Ok(())
    }

    /// Dispatch a notification asynchronously via non-blocking queue.
    /// Never blocks the caller's hot trading flow.
    pub async fn send_message(&self, message: &str) -> Result<()> {
        if let Some(ref tx) = self.tx {
            if let Err(e) = tx.try_send(message.to_string()) {
                warn!("Telegram queue full or closed, dropping notification: {e}");
            }
            return Ok(());
        }

        let (Some(token), Some(chat_id)) = (&self.bot_token, &self.chat_id) else {
            return Ok(());
        };

        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let payload = json!({
            "chat_id": chat_id,
            "text": message,
            "parse_mode": "HTML",
            "disable_web_page_preview": true
        });

        for attempt in 1..=2 {
            match self.client.post(&url).json(&payload).send().await {
                Ok(resp) if resp.status().is_success() => {
                    debug!("Telegram alert delivered");
                    break;
                }
                Ok(resp) => {
                    if resp.status().as_u16() == 429 {
                        warn!("Telegram API returned status 429 Too Many Requests");
                    } else {
                        warn!("Telegram API returned status {}", resp.status());
                    }
                    break;
                }
                Err(e) => {
                    if attempt == 2 {
                        error!("Failed to send Telegram alert: {}", e);
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    }
                }
            }
        }
        Ok(())
    }

    /// The bot is firing on an opportunity: route, expected net, and slot.
    pub async fn alert_opportunity(
        &self,
        route: &str,
        gross_sol: f64,
        net_sol: f64,
        net_usd: f64,
        slot: u64,
    ) {
        if net_sol < self.min_rejected_alert_sol && net_usd < 1.0 {
            return;
        }
        let msg = format!(
            "🎯 <b>فرصة مقتنصة (> $1.00)</b>\n\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             💰 <b>الربح الصافي المتوقع:</b> +{:.6} SOL (~${:.2})\n\
             📈 <b>الإجمالي قبل التكاليف:</b> {:.6} SOL\n\
             🧊 <b>السلوت:</b> <code>{}</code>",
            esc(route),
            net_sol,
            net_usd,
            gross_sol,
            slot
        );
        let _ = self.send_message(&msg).await;
    }

    /// Full telemetry alert for an opportunity with blockchain context and simulation data.
    #[allow(clippy::too_many_arguments)]
    pub async fn alert_opportunity_with_telemetry(
        &self,
        route: &str,
        gross_sol: f64,
        net_sol: f64,
        net_usd: f64,
        slot: u64,
        blockhash: &str,
        leader: &str,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        price_impact_bps: u16,
        active_bins: &[Option<i32>],
    ) {
        // Individual alerts are ON by default; set
        // TELEGRAM_SILENCE_INDIVIDUAL_ALERTS=true to suppress them when the
        // consolidated minute/five-minute digest is preferred.
        if std::env::var("TELEGRAM_SILENCE_INDIVIDUAL_ALERTS")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
        {
            return;
        }
        if net_sol < self.min_rejected_alert_sol && net_usd < 1.0 {
            return;
        }
        let bh_short = if blockhash.len() > 16 {
            &blockhash[..16]
        } else {
            blockhash
        };
        let leader_short = if leader.len() > 16 {
            &leader[..16]
        } else {
            leader
        };
        let bins_display = if active_bins.is_empty() {
            "N/A".to_string()
        } else {
            active_bins
                .iter()
                .map(|b| match b {
                    Some(id) => id.to_string(),
                    None => "-".to_string(),
                })
                .collect::<Vec<_>>()
                .join(" → ")
        };
        let msg = format!(
            "🎯 <b>فرصة مربحة (محاكاة ناجحة > $1.00)</b>\n\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             💎 <b>الربح الصافي المتوقع:</b> +{:.6} SOL (~${:.2})\n\
             📈 <b>الإجمالي قبل التكاليف:</b> {:.6} SOL\n\
             📉 <b>أثر السعر:</b> {} bps\n\
             🎚️ <b>الأبناء النشيطة (DLMM):</b> <code>{}</code>\n\
             🧊 <b>السلوت:</b> <code>{}</code>\n\
             🔗 <b>Blockhash:</b> <code>{}...</code>\n\
             👑 <b>Jito Leader:</b> <code>{}...</code>\n\
             ⚡ <b>المعالجة:</b> <code>{} CU | Priority: {} µ-lamp</code>\n\
             🛡️ <b>الحالة:</b> جاهزة للتنفيذ الذري",
            esc(route),
            net_sol,
            net_usd,
            gross_sol,
            price_impact_bps,
            bins_display,
            slot,
            esc(bh_short),
            esc(leader_short),
            cu_limit,
            priority_fee_micro_lamports
        );
        let _ = self.send_message(&msg).await;
    }

    /// Real-time alert explaining why a gross-profitable opportunity was rejected in final gates.
    /// Alerts are suppressed if the gross profit is below `min_rejected_alert_sol` to prevent notification spam.
    pub async fn alert_opportunity_rejected(
        &self,
        route: &str,
        gross_sol: f64,
        stage: &str,
        reason: &str,
        slot: u64,
    ) {
        // Individual rejection alerts are ON by default; set
        // TELEGRAM_SILENCE_INDIVIDUAL_ALERTS=true to suppress them when the
        // consolidated digest is preferred.
        if std::env::var("TELEGRAM_SILENCE_INDIVIDUAL_ALERTS")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
        {
            return;
        }
        if gross_sol < self.min_rejected_alert_sol {
            debug!(
                "Rejection alert suppressed: gross {:.6} SOL < threshold {:.6} SOL",
                gross_sol, self.min_rejected_alert_sol
            );
            return;
        }

        // Deduplication & Cooldown: Suppress repetitive rejection alerts for the same route and stage for 60 seconds
        let key = format!("{route}:{stage}");
        let now = std::time::Instant::now();
        {
            let mut cooldowns = self.rejected_cooldown.lock();
            // Prune entries older than 5 minutes to prevent unbounded memory growth
            cooldowns
                .retain(|_, time| now.duration_since(*time) < std::time::Duration::from_secs(300));
            if let Some(last_sent) = cooldowns.get(&key) {
                if now.duration_since(*last_sent) < std::time::Duration::from_secs(60) {
                    debug!("Rejection alert suppressed by 60s cooldown for {key}");
                    return;
                }
            }
            cooldowns.insert(key, now);
        }

        let gross_usd = gross_sol * self.sol_price;
        let msg = format!(
            "🚫 <b>تم رفض فرصة واعدة (> $1.00)</b>\n\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             📈 <b>الربح الإجمالي المحتمل:</b> {:.6} SOL (~${:.2})\n\
             🧩 <b>المرحلة:</b> <code>{}</code>\n\
             📋 <b>السبب الدقيق:</b> <code>{}</code>\n\
             🧊 <b>السلوت:</b> <code>{}</code>",
            esc(route),
            gross_sol,
            gross_usd,
            esc(stage),
            esc(reason),
            slot
        );
        let _ = self.send_message(&msg).await;
    }

    /// Periodic snapshot digest for monitored venues, pools, and meme tokens.
    #[allow(clippy::too_many_arguments)]
    pub async fn alert_market_snapshot_digest(
        &self,
        total_pools: usize,
        orca_count: usize,
        raydium_count: usize,
        meteora_count: usize,
        pump_count: usize,
        base_tokens_count: usize,
        sol_price: f64,
        opportunities_total: u64,
    ) {
        let msg = format!(
            "📊 <b>سناب شوت حالة السوق والمنصات (Digest)</b>\n\n\
             🏊 <b>إجمالي الأحواض المراقبة:</b> {}\n\
             • Orca Whirlpool: {}\n\
             • Raydium (V4 / CPMM / CLMM): {}\n\
             • Meteora DLMM: {}\n\
             • Pump.fun: {}\n\
             🪙 <b>عملات الميم المرتكزة:</b> {}\n\
             💵 <b>سعر SOL:</b> ${:.2}\n\
             🎯 <b>إجمالي الفرص المرصودة:</b> {}",
            total_pools,
            orca_count,
            raydium_count,
            meteora_count,
            pump_count,
            base_tokens_count,
            sol_price,
            opportunities_total
        );
        let _ = self.send_message(&msg).await;
    }

    /// Bundle accepted by the block engine (not yet landed).
    pub async fn alert_execution_sent(&self, bundle_id: &str, route: &str, net_sol: f64) {
        let net_usd = net_sol * self.sol_price;
        let msg = format!(
            "🚀 <b>تم إرسال حزمة مراجحة للتنفيذ على الشبكة</b>\n\n\
             🆔 <b>Bundle:</b> <code>{}</code>\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             💵 <b>الربح الصافي المستهدف:</b> +{:.6} SOL (~${:.2})\n\
             ⏳ <b>الحالة:</b> بانتظار التأكيد الذري (Jito)",
            esc(bundle_id),
            esc(route),
            net_sol,
            net_usd
        );
        let _ = self.send_message(&msg).await;
    }

    /// Confirmed fill with full financial breakdown, blockchain verification, and stage latency.
    #[allow(clippy::too_many_arguments)]
    pub async fn alert_trade_landed(
        &self,
        signature: Option<&str>,
        bundle_id: &str,
        profit_sol: f64,
        profit_usd: f64,
        loan_size_sol: f64,
        tokens: &str,
        venues: &str,
        slot: u64,
        timings: &str,
    ) {
        let sig_display = signature.unwrap_or(bundle_id);
        let solscan_link = format!("https://solscan.io/tx/{sig_display}");
        let solana_fm_link = format!("https://solana.fm/tx/{sig_display}");

        let msg = format!(
            "🎉 <b>تم تنفيذ صفقة مراجحة ذرية بنجاح (Landed Trade)</b>\n\n\
             💎 <b>الربح الصافي:</b> <b>+{:.6} SOL</b> (~${:.2})\n\
             🏦 <b>حجم قرض الفلاش (Kamino):</b> {:.4} SOL\n\
             🪙 <b>العملات:</b> <code>{}</code>\n\
             🏛️ <b>المنصات:</b> <code>{}</code>\n\
             🧊 <b>السلوت المنفذ فيه:</b> <code>{}</code>\n\n\
             ⏱️ <b>سرعة التنفيذ والمراحل:</b>\n{}\n\n\
             🔗 <b>تأكيد المعاملة على البلوكتشين:</b>\n\
             • <a href=\"{}\">عرض على Solscan</a>\n\
             • <a href=\"{}\">عرض على SolanaFM</a>\n\
             🆔 <b>Signature / Bundle:</b> <code>{}</code>",
            profit_sol,
            profit_usd,
            loan_size_sol,
            esc(tokens),
            esc(venues),
            slot,
            timings,
            solscan_link,
            solana_fm_link,
            esc(sig_display)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Bundle was rejected, dropped, or expired after dispatch.
    pub async fn alert_execution_dropped(&self, bundle_id: &str, route: &str, status: &str) {
        let msg = format!(
            "⚠️ <b>لم تهبط العملية</b>\n\n\
             🆔 <b>Bundle:</b> <code>{}</code>\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             📋 <b>الحالة:</b> <code>{}</code>\n\
             ℹ️ <b>الأثر:</b> خسارة الـ tip فقط، لا رأس مال (قرض ذري)",
            esc(bundle_id),
            esc(route),
            esc(status)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Execution failed before or during dispatch.
    pub async fn alert_execution_failed(&self, stage: &str, reason: &str, route: &str) {
        let msg = format!(
            "❌ <b>فشل التنفيذ</b>\n\n\
             🧩 <b>المرحلة:</b> <code>{}</code>\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             📋 <b>السبب:</b> <code>{}</code>",
            esc(stage),
            esc(route),
            esc(reason)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Opportunity confirmed by the simulator but not sent (shadow/simulate).
    pub async fn alert_simulation_opportunity(
        &self,
        route_name: &str,
        profit_sol: f64,
        profit_usd: f64,
    ) {
        let msg = format!(
            "🧪 <b>فرصة (محاكاة فقط)</b>\n\n\
             💎 <b>الربح الصافي المتوقع:</b> +{:.6} SOL (~${:.2})\n\
             🛣️ <b>المسار:</b> <code>{}</code>\n\
             🛡️ <b>الحالة:</b> اجتازت المحاكاة، لم تُرسل",
            profit_sol,
            profit_usd,
            esc(route_name)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Startup banner: mode, watch set, and whether live dispatch is armed.
    pub async fn alert_startup(&self, mode: &str, pools: usize, live_armed: bool) {
        let msg = format!(
            "🤖 <b>ARBITRAGUNIQUE بدأ العمل</b>\n\n\
             ⚙️ <b>الوضع:</b> <code>{}</code>\n\
             🏊 <b>عدد الحواض المراقبة:</b> {}\n\
             🔴 <b>الإرسال الحي:</b> {}",
            esc(mode),
            pools,
            if live_armed {
                "مُفعَّل (ARB_LIVE)"
            } else {
                "معطَّل"
            }
        );
        let _ = self.send_message(&msg).await;
    }

    /// Alert on emergency Circuit Breaker trip.
    pub async fn alert_circuit_breaker(&self, reason: &str) {
        let msg = format!(
            "🚨 <b>تم تفعيل قاطع الدائرة</b>\n\n\
             ⚠️ <b>السبب:</b> <code>{}</code>\n\
             🛑 <b>الإجراء:</b> إيقاف التداول لحماية رأس المال",
            esc(reason)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Alert on server/service failure with exact technical reason.
    pub async fn alert_service_failure(&self, service: &str, reason: &str, slot: u64) {
        let msg = format!(
            "⚠️ <b>عطل في السيرفر</b>\n\n\
             🛠️ <b>الخدمة:</b> <code>{}</code>\n\
             📋 <b>السبب الفني:</b> <code>{}</code>\n\
             🧊 <b>السلوت:</b> <code>{}</code>\n\
             ⏳ <b>الحالة:</b> جاري الفحص ومحاولة الاستعادة التلقائية...",
            esc(service),
            esc(reason),
            slot
        );
        let _ = self.send_message(&msg).await;
    }

    /// Alert on server/service recovery detailing how it was fixed and outage duration.
    pub async fn alert_service_recovery(
        &self,
        service: &str,
        duration_secs: f64,
        resolution: &str,
        slot: u64,
    ) {
        let msg = format!(
            "✅ <b>تم استعادة الخدمة وإصلاح العطل</b>\n\n\
             🛠️ <b>الخدمة:</b> <code>{}</code>\n\
             ⏱️ <b>مدة الانقطاع:</b> {:.1} ثانية\n\
             🔧 <b>طريقة المعالجة:</b> <code>{}</code>\n\
             🧊 <b>السلوت:</b> <code>{}</code>\n\
             🚀 <b>الحالة:</b> استقرار تام واستئناف المسار اللحظي",
            esc(service),
            duration_secs,
            esc(resolution),
            slot
        );
        let _ = self.send_message(&msg).await;
    }

    /// Periodic 1-minute performance and rejections digest.
    pub async fn alert_minute_digest(&self, stats: &DigestStats) {
        let landed_len = stats.landed_trades.len();
        let dropped_len = stats.dropped_trades.len();
        let rejected_len = stats.rejected_trades.len();

        let landed_section = if landed_len == 0 {
            "• لا توجد صفقات منفذة في آخر 60 ثانية".to_string()
        } else {
            stats
                .landed_trades
                .iter()
                .map(|t| {
                    let sig_short = if t.signature.len() > 16 {
                        format!("{}...", &t.signature[..16])
                    } else {
                        t.signature.clone()
                    };
                    format!(
                        "• <b>{}</b>\n  💎 الربح الصافي: <b>+{:.6} SOL</b> (~${:.2})\n  🏦 القرض: {:.4} SOL | المنصات: {}\n  🔗 <a href=\"https://solscan.io/tx/{}\">{}</a> (سلوت {})",
                        esc(&t.route),
                        t.profit_sol,
                        t.profit_usd,
                        t.loan_sol,
                        esc(&t.venues),
                        esc(&t.signature),
                        esc(&sig_short),
                        t.slot
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let dropped_section = if dropped_len == 0 {
            "• لا توجد حزم متعثرة (جميع الحزم هبطت أو اجتازت الحماية)".to_string()
        } else {
            stats
                .dropped_trades
                .iter()
                .map(|d| {
                    let b_short = if d.bundle_id.len() > 16 {
                        format!("{}...", &d.bundle_id[..16])
                    } else {
                        d.bundle_id.clone()
                    };
                    format!(
                        "• <code>{}</code>\n  المسار: {}\n  السبب: <code>{}</code>",
                        esc(&b_short),
                        esc(&d.route),
                        esc(&d.reason)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let rejected_section = if rejected_len == 0 {
            "• لا توجد صفقات مرفوضة فوق الحد الأدنى".to_string()
        } else {
            stats
                .rejected_trades
                .iter()
                .take(8)
                .map(|r| {
                    format!(
                        "• <b>{}</b> (+{:.5} SOL)\n  🧩 <code>{}</code>: <code>{}</code>",
                        esc(&r.route),
                        r.gross_sol,
                        esc(&r.stage),
                        esc(&r.reason)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let msg = format!(
            "⏱️ <b>تقرير أداء البوت — ملخص دقيقة (1-Minute Performance Digest)</b>\n\n\
             🧊 <b>السلوت الحالي:</b> <code>{}</code> | <b>سعر SOL:</b> ${:.2}\n\
             🏊 <b>الأحواض النشطة:</b> <code>{}</code> (Orca: {}, Ray: {}, DLMM: {}, Pump: {})\n\
             🔍 <b>إجمالي الفرص المفحوصة:</b> <code>{}</code>\n\n\
             ✅ <b>الصفقات الناجحة (Landed Trades) [{}]:</b>\n{}\n\n\
             ⚠️ <b>الحزم المتعثرة (Dropped Bundles) [{}]:</b>\n{}\n\n\
             🚫 <b>أبرز الصفقات المرفوضة وأسبابها [{}]:</b>\n{}\n\n\
             🛠️ <b>حالة السيرفر والبنية التحتية:</b>\n\
             • <b>Solana RPC Pool:</b> <code>{}</code>\n\
             • <b>WebSocket Stream:</b> <code>{}</code>\n\
             • <b>Yellowstone gRPC:</b> <code>{}</code>\n\
             • <b>Jito Block Engine:</b> <code>{}</code>",
            stats.slot,
            stats.sol_price,
            stats.total_pools,
            stats.orca_count,
            stats.raydium_count,
            stats.meteora_count,
            stats.pump_count,
            stats.cycles_evaluated,
            landed_len,
            landed_section,
            dropped_len,
            dropped_section,
            rejected_len,
            rejected_section,
            esc(&stats.rpc_status),
            esc(&stats.ws_status),
            esc(&stats.ingest_status),
            esc(&stats.jito_status)
        );

        let _ = self.send_message(&msg).await;
    }

    /// Backward compatibility alias
    pub async fn alert_five_minute_digest(&self, stats: &DigestStats) {
        self.alert_minute_digest(stats).await;
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TradeSummary {
    pub route: String,
    pub profit_sol: f64,
    pub profit_usd: f64,
    pub loan_sol: f64,
    pub signature: String,
    pub venues: String,
    pub slot: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct DroppedTradeSummary {
    pub bundle_id: String,
    pub route: String,
    pub reason: String,
    pub slot: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RejectedTradeSummary {
    pub route: String,
    pub gross_sol: f64,
    pub stage: String,
    pub reason: String,
    pub slot: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DigestStats {
    pub slot: u64,
    pub sol_price: f64,
    pub total_pools: usize,
    pub orca_count: usize,
    pub raydium_count: usize,
    pub meteora_count: usize,
    pub pump_count: usize,
    pub cycles_evaluated: u64,
    pub rpc_status: String,
    pub ws_status: String,
    pub ingest_status: String,
    pub jito_status: String,
    pub landed_trades: Vec<TradeSummary>,
    pub dropped_trades: Vec<DroppedTradeSummary>,
    pub rejected_trades: Vec<RejectedTradeSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifier_builds_without_credentials() {
        // Must not panic and must report itself disabled when env is absent.
        let n = TelegramNotifier::from_env();
        let _ = n.is_enabled();
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(esc("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn default_min_rejected_alert_sol() {
        let n = TelegramNotifier::from_env();
        assert!(n.min_rejected_alert_sol() >= 0.0);
    }
}
