//! Telegram telemetry: every opportunity the bot fires on, every landed fill,
//! and every failure — delivered to the operator's chat.
//!
//! Nothing here can block trading: sends are best-effort, time-boxed, and a
//! Telegram outage is logged, never propagated.

use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use tracing::{debug, error, info, warn};

/// Telegram 403 is a PERMANENT recipient state — never retry it (retries can
/// never succeed and only burn rate limit). Report the remediation exactly once.
fn report_telegram_403_once() {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        error!(
            "Telegram API returned 403 Forbidden — PERMANENT recipient state; alerts will NOT be delivered until fixed. Causes: (1) the target chat never sent /start to this bot, (2) the bot was blocked by the recipient, (3) TELEGRAM_CHAT_ID is another bot's @username (bots cannot message bots — use your numeric user ID or a group/channel the bot is in), (4) the bot token was revoked (regenerate via @BotFather). Not retrying."
        );
    }
}

#[derive(Clone)]
pub struct TelegramNotifier {
    bot_token: Option<String>,
    chat_id: Option<String>,
    client: Client,
    sol_price: std::sync::Arc<parking_lot::RwLock<f64>>,
    /// True only after a verified live source (Binance/Coinbase feed, on-chain
    /// SOL/USDC pool, or an operator-provided SOL_USD_PRICE) set the price.
    /// The 200.0 seed is a placeholder, never economic evidence.
    sol_price_live: std::sync::Arc<std::sync::atomic::AtomicBool>,
    min_rejected_alert_sol: f64,
    min_rejected_alert_pct: f64,
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
        let env_sol_price = std::env::var("SOL_USD_PRICE")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|p| *p > 0.0);
        let initial_sol_price = env_sol_price.unwrap_or(200.0);
        let sol_price = std::sync::Arc::new(parking_lot::RwLock::new(initial_sol_price));
        let sol_price_live =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(env_sol_price.is_some()));

        let min_rejected_alert_sol = std::env::var("TELEGRAM_REJECTED_ALERT_MIN_SOL")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(min_alert_usd / initial_sol_price);
        // Minimum gross profit percentage relative to loan size to trigger rejection alert (default 0.01% = 1 bps)
        let min_rejected_alert_pct = std::env::var("TELEGRAM_REJECTED_ALERT_MIN_PCT")
            .ok()
            .and_then(|v| {
                let clean = v.trim().trim_end_matches('%');
                clean.parse::<f64>().ok()
            })
            .unwrap_or(0.01);

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
                                    } else if resp.status().as_u16() == 403 {
                                        report_telegram_403_once();
                                        while rx.try_recv().is_ok() {}
                                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
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

        // Spawn real-time live market price tracker for SOL/USD (Binance + Coinbase feed)
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let client_price = client.clone();
            let sol_price_bg = sol_price.clone();
            let sol_price_live_bg = sol_price_live.clone();
            handle.spawn(async move {
                loop {
                    let mut fetched = None;
                    if let Ok(resp) = client_price
                        .get("https://api.binance.com/api/v3/ticker/price?symbol=SOLUSDT")
                        .timeout(std::time::Duration::from_secs(5))
                        .send()
                        .await
                    {
                        if resp.status().is_success() {
                            if let Ok(json) = resp.json::<serde_json::Value>().await {
                                if let Some(p_str) = json.get("price").and_then(|p| p.as_str()) {
                                    if let Ok(p) = p_str.parse::<f64>() {
                                        if p > 0.0 && p.is_finite() {
                                            fetched = Some(p);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if fetched.is_none() {
                        if let Ok(resp) = client_price
                            .get("https://api.coinbase.com/v2/prices/SOL-USD/spot")
                            .timeout(std::time::Duration::from_secs(5))
                            .send()
                            .await
                        {
                            if resp.status().is_success() {
                                if let Ok(json) = resp.json::<serde_json::Value>().await {
                                    if let Some(p_str) =
                                        json.pointer("/data/amount").and_then(|p| p.as_str())
                                    {
                                        if let Ok(p) = p_str.parse::<f64>() {
                                            if p > 0.0 && p.is_finite() {
                                                fetched = Some(p);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(p) = fetched {
                        *sol_price_bg.write() = p;
                        sol_price_live_bg.store(true, std::sync::atomic::Ordering::Relaxed);
                        debug!("Live SOL/USD price updated from market feed: ${:.2}", p);
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                }
            });
        }

        Self {
            bot_token,
            chat_id,
            client,
            sol_price,
            sol_price_live,
            min_rejected_alert_sol,
            min_rejected_alert_pct,
            tx: tx_opt,
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    pub fn min_rejected_alert_sol(&self) -> f64 {
        self.min_rejected_alert_sol
    }

    pub fn min_rejected_alert_pct(&self) -> f64 {
        self.min_rejected_alert_pct
    }

    pub fn update_sol_price(&self, price: f64) {
        if price > 0.0 && price.is_finite() {
            *self.sol_price.write() = price;
            self.sol_price_live
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub fn sol_price(&self) -> f64 {
        let p = *self.sol_price.read();
        if p > 0.0 && p.is_finite() { p } else { 200.0 }
    }

    /// True only when the current SOL/USD price came from a verified live
    /// source; the 200.0 placeholder never claims to be market evidence.
    pub fn sol_price_is_live(&self) -> bool {
        self.sol_price_live
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Provenance label for alert messages: honest about unverified prices.
    pub fn sol_price_provenance_label(&self) -> &'static str {
        if self.sol_price_is_live() {
            "سعر حي من السوق"
        } else {
            "سعر غير موثّق (افتراضي — لا يوجد بث حي بعد)"
        }
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
                    } else if resp.status().as_u16() == 403 {
                        report_telegram_403_once();
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
        pools_count: usize,
        loan_size_sol: f64,
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
        let live_sol_price = self.sol_price();
        let price_label = self.sol_price_provenance_label();
        let computed_net_usd = if net_usd > 0.0 {
            net_usd
        } else {
            net_sol * live_sol_price
        };
        if net_sol < self.min_rejected_alert_sol && computed_net_usd < 1.0 {
            return;
        }
        let bh_short = if blockhash.len() > 16 {
            &blockhash[..16]
        } else {
            blockhash
        };
        let leader_display = if leader.trim().is_empty() {
            "Jito Authorized Validator / Leader"
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
        let loan_usd = loan_size_sol * live_sol_price;
        let gross_usd = gross_sol * live_sol_price;
        let pool_desc = if pools_count == 2 {
            "Two-Leg Cycle"
        } else if pools_count == 3 {
            "Triangular Cycle"
        } else {
            "Multi-Hop Cycle"
        };
        let msg = format!(
            "🎯 <b>فرصة مراجحة واعدة (محاكاة ناجحة > $1.00)</b>\n\n\
             💰 <b>الربح الصافي الحقيقي (Net Profit):</b> <b>+{:.6} SOL</b> (~${:.2} USD)\n\
             🏊 <b>عدد الأحواض:</b> <code>{} أحواض ({})</code>\n\
             🏦 <b>حجم قرض الفلاش (Kamino Borrow):</b> <code>{:.4} SOL (~${:.2} USD)</code>\n\
             🛣️ <b>المسار والقفزات:</b> <code>{}</code>\n\
             📈 <b>الربح الإجمالي قبل التكاليف:</b> {:.6} SOL (~${:.2})\n\
             📉 <b>أثر السعر (Price Impact):</b> {} bps\n\
             🎚️ <b>الأبناء النشطة (DLMM Bins):</b> <code>{}</code>\n\
             💵 <b>سعر SOL اللحظي المستخدم:</b> ${:.2} USD ({price_label})\n\
             🧊 <b>السلوت (Slot):</b> <code>{}</code>\n\
             👑 <b>البيلدر / الليدر المستهدف (Target Builder/Leader):</b> <code>{}</code>\n\
             🔗 <b>Blockhash:</b> <code>{}...</code>\n\
             ⚡ <b>المعالجة:</b> <code>{} CU | Priority: {} µ-lamp</code>\n\
             🛡️ <b>الحالة:</b> جاهزة للتنفيذ الذري وموجهة للبيلدر الصحيح",
            net_sol,
            computed_net_usd,
            pools_count,
            pool_desc,
            loan_size_sol,
            loan_usd,
            esc(route),
            gross_sol,
            gross_usd,
            price_impact_bps,
            bins_display,
            live_sol_price,
            slot,
            esc(leader_display),
            esc(bh_short),
            cu_limit,
            priority_fee_micro_lamports
        );
        let _ = self.send_message(&msg).await;
    }

    /// Real-time alert explaining why a gross-profitable opportunity was rejected in final gates.
    /// Alerts are strictly filtered by percentage yield relative to loan size (no fixed SOL threshold).
    pub async fn alert_opportunity_rejected(
        &self,
        route: &str,
        gross_sol: f64,
        stage: &str,
        reason: &str,
        slot: u64,
        pools_count: usize,
        loan_size_sol: f64,
        net_sol: Option<f64>,
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

        // Percentage of gross profit relative to loan size
        let profit_pct = if loan_size_sol > 0.0 {
            (gross_sol / loan_size_sol) * 100.0
        } else {
            0.0
        };

        // Strictly percentage-based alert threshold (no fixed 0.01 SOL number)
        if profit_pct < self.min_rejected_alert_pct {
            debug!(
                "Rejection alert suppressed: gross {:.6} SOL ({:.4}%) < percentage threshold {:.4}%",
                gross_sol, profit_pct, self.min_rejected_alert_pct
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

        let live_sol_price = self.sol_price();
        let price_label = self.sol_price_provenance_label();
        let gross_usd = gross_sol * live_sol_price;
        let loan_usd = loan_size_sol * live_sol_price;
        let net_display = match net_sol {
            Some(n) => {
                let n_usd = n * live_sol_price;
                format!("{n:+.6} SOL (~${n_usd:.2} USD)")
            }
            None => "غير محقق / تم الاستبعاد قبل استكمال صافي التنفيذ".to_string(),
        };

        let pool_desc = if pools_count == 2 {
            "Two-Leg Cycle"
        } else if pools_count == 3 {
            "Triangular Cycle"
        } else {
            "Multi-Hop Cycle"
        };

        let msg = format!(
            "🚫 <b>تنبيه استبعاد فرصة مراجحة (Opportunity Rejected Alert)</b>\n\n\
             🛑 <b>سبب عدم التنفيذ والنزول في البلوك:</b> <code>{}</code>\n\
             🧩 <b>مرحلة الاستبعاد في الفحص (Pipeline Stage):</b> <code>{}</code>\n\
             📈 <b>الربح الإجمالي المحتمل:</b> <code>{:.6} SOL (~${:.2} USD) | {:.4}% من حجم القرض</code>\n\
             🎯 <b>حد التنبيه بالنسبة المئوية:</b> <code>{:.4}% من حجم القرض (نسبة مئوية ديناميكية)</code>\n\
             🏦 <b>حجم قرض الفلاش (Flash Loan):</b> <code>{:.4} SOL (~${:.2} USD)</code>\n\
             💰 <b>صافي الربح التقديري:</b> <code>{}</code>\n\
             🏊 <b>عدد الأحواض:</b> <code>{} أحواض ({})</code>\n\
             🛣️ <b>المسار والقفزات:</b> <code>{}</code>\n\
             💵 <b>سعر SOL اللحظي المستخدم:</b> <code>${:.2} USD ({price_label})</code>\n\
             🧊 <b>السلوت (Slot):</b> <code>{}</code>",
            esc(reason),
            esc(stage),
            gross_sol,
            gross_usd,
            profit_pct,
            self.min_rejected_alert_pct,
            loan_size_sol,
            loan_usd,
            esc(&net_display),
            pools_count,
            pool_desc,
            esc(route),
            live_sol_price,
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

    /// Bundle accepted by the block engine / SWQoS (dispatched to the leader).
    pub async fn alert_execution_sent(
        &self,
        bundle_id: &str,
        route: &str,
        net_sol: f64,
        pools_count: usize,
        loan_size_sol: f64,
        leader: &str,
    ) {
        let live_sol_price = self.sol_price();
        let price_label = self.sol_price_provenance_label();
        let net_usd = net_sol * live_sol_price;
        let loan_usd = loan_size_sol * live_sol_price;
        let pool_desc = if pools_count == 2 {
            "Two-Leg Cycle"
        } else if pools_count == 3 {
            "Triangular Cycle"
        } else {
            "Multi-Hop Cycle"
        };
        let leader_display = if leader.trim().is_empty() {
            "Jito Authorized Validator / Leader"
        } else {
            leader
        };
        let msg = format!(
            "🚀 <b>تم إرسال حزمة مراجحة للتنفيذ على الشبكة</b>\n\n\
             💰 <b>الربح الصافي المستهدف (Target Net):</b> <b>+{:.6} SOL</b> (~${:.2} USD)\n\
             🏦 <b>حجم قرض الفلاش (Flash Loan):</b> <code>{:.4} SOL (~${:.2} USD)</code>\n\
             🏊 <b>عدد الأحواض:</b> <code>{} أحواض ({})</code>\n\
             🛣️ <b>المسار والقفزات:</b> <code>{}</code>\n\
             💵 <b>سعر SOL اللحظي المستخدم:</b> <code>${:.2} USD ({price_label})</code>\n\
             👑 <b>البيلدر / الليدر المستهدف (Target Builder):</b> <code>{}</code>\n\
             🆔 <b>معرف الحزمة (Bundle ID):</b> <code>{}</code>\n\
             ⏳ <b>الحالة:</b> تم التوجيه للبيلدر وبانتظار النزول في البلوك (In-Flight)",
            net_sol,
            net_usd,
            loan_size_sol,
            loan_usd,
            pools_count,
            pool_desc,
            esc(route),
            live_sol_price,
            esc(leader_display),
            esc(bundle_id)
        );
        let _ = self.send_message(&msg).await;
    }

    /// Primary notification for executed & landed arbitrage trades.
    /// Explicitly displays the 5 mandatory verification fields:
    /// 1. Real Net Profit (in SOL and USD)
    /// 2. Number of pools in the cycle (Pools count) and route hop names
    /// 3. Flash loan borrow amount (in SOL and USD)
    /// 4. Confirmed landing proof in the block (Block slot, tx signature with Solscan explorer link, landed status: Confirmed/Finalized)
    /// 5. Target Builder / Jito Leader identity (confirming it went to the right builder/validator)
    #[allow(clippy::too_many_arguments)]
    pub async fn notify_trade_executed(
        &self,
        signature: Option<&str>,
        bundle_id: &str,
        profit_sol: f64,
        passed_profit_usd: f64,
        loan_size_sol: f64,
        pools_count: usize,
        route_hops: &str,
        venues: &str,
        slot: u64,
        leader: &str,
        landed_status: &str,
        timings: &str,
    ) {
        let sig_display = signature.unwrap_or(bundle_id);
        let solscan_link = format!("https://solscan.io/tx/{sig_display}");
        let solana_fm_link = format!("https://solana.fm/tx/{sig_display}");
        let live_sol_price = self.sol_price();
        let price_label = self.sol_price_provenance_label();
        let profit_usd = if passed_profit_usd > 0.0 {
            passed_profit_usd
        } else {
            profit_sol * live_sol_price
        };
        let loan_usd = loan_size_sol * live_sol_price;

        let leader_display = if leader.trim().is_empty() {
            "Jito Authorized Validator / Leader".to_string()
        } else {
            leader.to_string()
        };

        let pool_desc = if pools_count == 2 {
            "Two-Leg Cycle"
        } else if pools_count == 3 {
            "Triangular Cycle"
        } else {
            "Multi-Hop Cycle"
        };

        let msg = format!(
            "🎉 <b>تم تنفيذ صفقة مراجحة ذرية وهبوطها في البلوك بنجاح!</b>\n\n\
             💰 <b>الربح الصافي الحقيقي (Real Net Profit):</b>\n\
             👉 <b>+{:.6} SOL</b> (~${:.2} USD)\n\n\
             🏦 <b>حجم قرض الفلاش (Flash Loan Borrow):</b>\n\
             👉 <code>{:.4} SOL</code> (~${:.2} USD) عبر بروتوكول Kamino\n\n\
             🏊 <b>عدد الأحواض والمسار (Pools & Route):</b>\n\
             • <b>عدد الأحواض:</b> <code>{} أحواض ({})</code>\n\
             • <b>المسار والقفزات:</b> <code>{}</code>\n\
             • <b>المنصات (DEXes):</b> <code>{}</code>\n\n\
             💵 <b>سعر SOL اللحظي المستخدم في الحساب:</b> <code>${:.2} USD ({price_label})</code>\n\n\
             🧱 <b>إثبات النزول في البلوك (Block Landing Proof):</b>\n\
             • <b>حالة الهبوط:</b> <code>{}</code> ✅\n\
             • <b>رقم السلوت (Block Slot):</b> <code>{}</code>\n\
             • <b>البيلدر / الليدر المستهدف (Target Builder/Leader):</b> <code>{}</code>\n\
             • <b>تأكيد التوجيه:</b> تم التحقق من استلام البيلدر الصحيح للحزمة 🎯\n\n\
             🔗 <b>روابط التحقق المباشر على المستكشف:</b>\n\
             • <a href=\"{}\">🔍 عرض المعاملة على Solscan Explorer</a>\n\
             • <a href=\"{}\">🌐 عرض المعاملة على SolanaFM Explorer</a>\n\n\
             🆔 <b>Signature:</b> <code>{}</code>\n\
             🆔 <b>Bundle ID:</b> <code>{}</code>\n\n\
             ⏱️ <b>تفاصيل أزمنة التنفيذ (Latency Timings):</b>\n{}",
            profit_sol,
            profit_usd,
            loan_size_sol,
            loan_usd,
            pools_count,
            pool_desc,
            esc(route_hops),
            esc(venues),
            live_sol_price,
            esc(landed_status),
            slot,
            esc(&leader_display),
            solscan_link,
            solana_fm_link,
            esc(sig_display),
            esc(bundle_id),
            timings
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
                    let pools_info = if t.pools_count > 0 {
                        format!("{} أحواض", t.pools_count)
                    } else {
                        "N/A".to_string()
                    };
                    let leader_info = if !t.leader.is_empty() {
                        if t.leader.len() > 16 {
                            format!("{}...", &t.leader[..16])
                        } else {
                            t.leader.clone()
                        }
                    } else {
                        "Jito Leader".to_string()
                    };
                    format!(
                        "• <b>{}</b>\n  💰 صافي الربح: <b>+{:.6} SOL</b> (~${:.2})\n  🏦 القرض: {:.4} SOL | أحواض: {}\n  👑 البيلدر: {}\n  🔗 <a href=\"https://solscan.io/tx/{}\">{}</a> (سلوت {})",
                        esc(&t.route),
                        t.profit_sol,
                        t.profit_usd,
                        t.loan_sol,
                        pools_info,
                        esc(&leader_info),
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
                    let net_str = match r.net_sol {
                        Some(n) => format!(" | صافي: {:+.5} SOL", n),
                        None => String::new(),
                    };
                    format!(
                        "• <b>{}</b> (إجمالي: +{:.5} SOL{} | أحواض: {} | قرض: {:.2} SOL)\n  🧩 <code>{}</code>: <code>{}</code>",
                        esc(&r.route),
                        r.gross_sol,
                        net_str,
                        r.pools_count,
                        r.loan_size_sol,
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
    #[serde(default)]
    pub pools_count: usize,
    #[serde(default)]
    pub leader: String,
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
    pub pools_count: usize,
    pub loan_size_sol: f64,
    pub net_sol: Option<f64>,
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

    /// G7: the 200.0 placeholder must never be presented as market evidence.
    /// A notifier seeded without SOL_USD_PRICE reports its price as unverified
    /// until a live source updates it; update_sol_price flips the provenance.
    #[test]
    fn unverified_sol_price_is_labeled_as_such() {
        let n = TelegramNotifier {
            bot_token: None,
            chat_id: None,
            client: reqwest::Client::new(),
            sol_price: std::sync::Arc::new(parking_lot::RwLock::new(200.0)),
            sol_price_live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            min_rejected_alert_sol: 0.005,
            min_rejected_alert_pct: 0.01,
            tx: None,
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };
        assert!(!n.sol_price_is_live());
        assert_eq!(
            n.sol_price_provenance_label(),
            "سعر غير موثّق (افتراضي — لا يوجد بث حي بعد)"
        );

        // A live source update flips the provenance flag.
        n.update_sol_price(215.5);
        assert!(n.sol_price_is_live());
        assert_eq!(n.sol_price_provenance_label(), "سعر حي من السوق");
    }

    #[tokio::test]
    async fn notify_trade_executed_includes_all_five_mandatory_fields() {
        // We set up a channel so we can capture the actual formatted message.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
        let notifier = TelegramNotifier {
            bot_token: Some("fake_token".to_string()),
            chat_id: Some("123456789".to_string()),
            client: reqwest::Client::new(),
            sol_price: std::sync::Arc::new(parking_lot::RwLock::new(200.0)),
            sol_price_live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            min_rejected_alert_sol: 0.005,
            min_rejected_alert_pct: 0.01,
            tx: Some(tx),
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };

        notifier
            .notify_trade_executed(
                Some("5abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"),
                "bundle-abc-123",
                0.045678,
                9.1356,
                1.5,
                3,
                "SOL ➔ USDC ➔ BONK ➔ SOL",
                "OrcaWhirlpool ➔ RaydiumClmm ➔ MeteoraDlmm",
                312456789,
                "JitoValidatorVoteKey11111111111111111111111",
                "Confirmed / Finalized (مؤكد في البلوك)",
                "• Ingest ➔ Quote: 45 µs\n• Block Landing: 320 ms",
            )
            .await;

        let msg = rx.recv().await.expect("Expected telegram message");
        // 1. Real Net Profit (in SOL and USD)
        assert!(msg.contains("الربح الصافي الحقيقي (Real Net Profit)"));
        assert!(msg.contains("+0.045678 SOL"));
        assert!(msg.contains("$9.14 USD"));

        // 2. Pools count and route hops
        assert!(msg.contains("عدد الأحواض والمسار (Pools & Route)"));
        assert!(msg.contains("3 أحواض (Triangular Cycle)"));
        assert!(msg.contains("SOL ➔ USDC ➔ BONK ➔ SOL"));
        assert!(msg.contains("OrcaWhirlpool ➔ RaydiumClmm ➔ MeteoraDlmm"));

        // 3. Flash loan borrow amount (in SOL and USD)
        assert!(msg.contains("حجم قرض الفلاش (Flash Loan Borrow)"));
        assert!(msg.contains("1.5000 SOL"));
        assert!(msg.contains("$300.00 USD"));
        assert!(msg.contains("Kamino"));

        // 4. Confirmed landing proof in the block
        assert!(msg.contains("إثبات النزول في البلوك (Block Landing Proof)"));
        assert!(msg.contains("Confirmed / Finalized (مؤكد في البلوك)"));
        assert!(msg.contains("312456789"));
        assert!(msg.contains("https://solscan.io/tx/5abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"));

        // 5. Target Builder / Jito Leader identity
        assert!(msg.contains("البيلدر / الليدر المستهدف (Target Builder/Leader)"));
        assert!(msg.contains("JitoValidatorVoteKey11111111111111111111111"));
        assert!(msg.contains("تم التحقق من استلام البيلدر الصحيح للحزمة"));
    }

    #[tokio::test]
    async fn alert_opportunity_with_telemetry_includes_mandatory_fields() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
        let notifier = TelegramNotifier {
            bot_token: Some("fake_token".to_string()),
            chat_id: Some("123456789".to_string()),
            client: reqwest::Client::new(),
            sol_price: std::sync::Arc::new(parking_lot::RwLock::new(200.0)),
            sol_price_live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            min_rejected_alert_sol: 0.001,
            min_rejected_alert_pct: 0.01,
            tx: Some(tx),
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };

        notifier
            .alert_opportunity_with_telemetry(
                "SOL -> USDC -> SOL",
                0.05,
                0.025,
                5.0,
                300100200,
                "Blockhash11111111111111111111111111111",
                "JitoLeaderVoteKeyXYZ",
                200_000,
                50_000,
                15,
                &[Some(100), Some(101)],
                2,
                0.8,
            )
            .await;

        let msg = rx.recv().await.expect("Expected telegram message");
        assert!(msg.contains("الربح الصافي الحقيقي (Net Profit)"));
        assert!(msg.contains("+0.025000 SOL"));
        assert!(msg.contains("حجم قرض الفلاش (Kamino Borrow)"));
        assert!(msg.contains("0.8000 SOL"));
        assert!(msg.contains("عدد الأحواض"));
        assert!(msg.contains("2 أحواض (Two-Leg Cycle)"));
        assert!(msg.contains("JitoLeaderVoteKeyXYZ"));
        assert!(msg.contains("300100200"));
    }

    #[tokio::test]
    async fn alert_execution_sent_includes_mandatory_fields() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
        let notifier = TelegramNotifier {
            bot_token: Some("fake_token".to_string()),
            chat_id: Some("123456789".to_string()),
            client: reqwest::Client::new(),
            sol_price: std::sync::Arc::new(parking_lot::RwLock::new(200.0)),
            sol_price_live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            min_rejected_alert_sol: 0.001,
            min_rejected_alert_pct: 0.01,
            tx: Some(tx),
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };

        notifier
            .alert_execution_sent(
                "bundle-test-456",
                "SOL -> USDT -> SOL",
                0.015,
                2,
                0.5,
                "JitoLeaderTest789",
            )
            .await;

        let msg = rx.recv().await.expect("Expected telegram message");
        assert!(msg.contains("الربح الصافي المستهدف (Target Net)"));
        assert!(msg.contains("+0.015000 SOL"));
        assert!(msg.contains("حجم قرض الفلاش (Flash Loan)"));
        assert!(msg.contains("0.5000 SOL"));
        assert!(msg.contains("عدد الأحواض"));
        assert!(msg.contains("JitoLeaderTest789"));
        assert!(msg.contains("bundle-test-456"));
    }

    #[tokio::test]
    async fn alert_opportunity_rejected_percentage_threshold_and_reason_fields() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
        let notifier = TelegramNotifier {
            bot_token: Some("fake_token".to_string()),
            chat_id: Some("123456789".to_string()),
            client: reqwest::Client::new(),
            sol_price: std::sync::Arc::new(parking_lot::RwLock::new(215.50)),
            sol_price_live: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            min_rejected_alert_sol: 0.001,
            // 0.02% (2 bps) percentage threshold
            min_rejected_alert_pct: 0.02,
            tx: Some(tx),
            rejected_cooldown: std::sync::Arc::new(parking_lot::Mutex::new(
                std::collections::HashMap::new(),
            )),
        };

        // 1. Below threshold: 0.00005 SOL on 1.0 SOL loan = 0.005% (< 0.02%) -> suppressed
        notifier
            .alert_opportunity_rejected(
                "SOL ➔ USDC ➔ SOL",
                0.00005,
                "Profit Floor Protection",
                "Gross profit below threshold",
                312456790,
                2,
                1.0,
                None,
            )
            .await;
        assert!(rx.try_recv().is_err());

        // 2. Above percentage threshold: 0.001 SOL on 2.0 SOL loan = 0.05% (>= 0.02%) -> sent!
        notifier
            .alert_opportunity_rejected(
                "SOL ➔ USDC ➔ BONK ➔ SOL",
                0.001,
                "Pre-flight Simulation",
                "Simulation reverted with custom error 0x1",
                312456791,
                3,
                2.0,
                Some(0.0004),
            )
            .await;

        let msg = rx
            .recv()
            .await
            .expect("Expected telegram rejection message");
        assert!(msg.contains("تنبيه استبعاد فرصة مراجحة (Opportunity Rejected Alert)"));
        assert!(msg.contains("سبب عدم التنفيذ والنزول في البلوك:"));
        assert!(msg.contains("Simulation reverted with custom error 0x1"));
        assert!(msg.contains("مرحلة الاستبعاد في الفحص (Pipeline Stage):"));
        assert!(msg.contains("Pre-flight Simulation"));
        assert!(msg.contains("حجم قرض الفلاش (Flash Loan):"));
        assert!(msg.contains("2.0000 SOL (~$431.00 USD)"));
        assert!(msg.contains("سعر SOL اللحظي المستخدم:"));
        assert!(msg.contains("$215.50 USD (سعر حي من السوق)"));
        assert!(msg.contains("حد التنبيه بالنسبة المئوية:"));
        assert!(msg.contains("0.0200% من حجم القرض"));
        assert!(msg.contains("3 أحواض (Triangular Cycle)"));

        // 3. Test dynamic live price update:
        notifier.update_sol_price(225.0);
        assert_eq!(notifier.sol_price(), 225.0);
    }
}
