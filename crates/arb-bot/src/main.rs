//! Entry point for the Solana Arbitrage Bot.
//!
//! M0 — Foundation & Bootstrap: CLI configuration, logging, safety checks.

use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use arb_bot::*;

struct Args {
    config_path: String,
    mode: BotMode,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut config_path = std::env::var("ARB_CONFIG").unwrap_or_default();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Solana Arbitrage Bot\n\nUsage: arb-bot [OPTIONS]\n\nOptions:\n  -c, --config <PATH>  Path to configuration TOML [default: auto-detected]\n  -h, --help           Print help\n\nMode is controlled by environment only:\n  ARB_MODE=shadow|observe|analyze|simulate|paper_trade|live|canary  (default: observe)\n  shadow selects non-broadcast simulation; live additionally requires ARB_LIVE=1"
                );
                std::process::exit(0);
            }
            "--config" | "-c" => {
                config_path = args.next().context("Missing argument after --config")?;
            }
            other => anyhow::bail!(
                "Unknown argument: '{other}'. Mode is environment-controlled (ARB_MODE)."
            ),
        }
    }

    if config_path.is_empty() {
        // One canonical default only. The legacy root config.toml is no longer
        // tracked or auto-selected; this prevents stale values from shadowing
        // the operator-managed deployment file. config.example.toml is never
        // auto-selected because it contains placeholder-safe defaults.
        config_path = resolve_default_config_path(&std::env::current_dir()?)?;
    }

    // Mode is environment-controlled only. Precedence:
    // 1. `shadow-only` cargo feature (Docker shadow image): simulate, always.
    // 2. `ARB_MODE` (shadow|observe|analyze|simulate|paper_trade|live|canary)
    // 3. Legacy arming flags: ARB_CANARY, ARB_LIVE/LIVE/EXECUTE_LIVE/LIVE_TRADING
    // 4. Safe fallback: "observe" (monitoring only)
    let mode_str = if cfg!(feature = "shadow-only") {
        eprintln!("shadow-only build: live/canary arming is disabled; forcing simulate");
        "simulate".to_string()
    } else if let Ok(m) = std::env::var("ARB_MODE") {
        if !m.trim().is_empty() {
            m
        } else {
            "observe".to_string()
        }
    } else if std::env::var("ARB_CANARY").is_ok() {
        "canary".to_string()
    } else if live_execution_enabled() {
        "live".to_string()
    } else {
        "observe".to_string()
    };

    let mode: BotMode = mode_str.parse()?;
    Ok(Args { config_path, mode })
}

fn resolve_default_config_path(root: &std::path::Path) -> Result<String> {
    let canonical = root.join("config").join("config.toml");
    if canonical.is_file() {
        return Ok(canonical.to_string_lossy().into_owned());
    }
    anyhow::bail!(
        "No canonical configuration found at '{}'. Copy config/config.example.toml to config/config.toml, set ARB_CONFIG, or pass --config <PATH>. Legacy root config.toml is not auto-selected.",
        canonical.display()
    )
}

fn redact_secret(value: &str, secret: &str) -> String {
    if secret.is_empty() {
        value.to_string()
    } else {
        value.replace(secret, "[REDACTED]")
    }
}

fn install_panic_hook() {
    // Do not chain the default hook: it prints the original, unredacted panic
    // payload and could leak credentials after our sanitized log entry.
    let _default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads can contain configuration values. Redact known
        // Telegram credentials before either logging or sending the alert.
        let token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
        let chat = std::env::var("TELEGRAM_CHAT_ID").ok();
        let mut panic_message = info.to_string();
        if let Some(secret) = token.as_deref() {
            panic_message = redact_secret(&panic_message, secret);
        }
        if let Some(secret) = chat.as_deref() {
            panic_message = redact_secret(&panic_message, secret);
        }
        tracing::error!("FATAL PANIC: {}", panic_message);

        // Never spawn a shell process or place credentials in process
        // arguments. Best-effort notification is sent through reqwest only
        // when a Tokio runtime is still available during unwinding.
        if let (Some(token), Some(chat)) = (token, chat) {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let client = reqwest::Client::new();
                let url = format!("https://api.telegram.org/bot{token}/sendMessage");
                let text = format!("FATAL PANIC — arb-bot stopped\n{panic_message}");
                handle.spawn(async move {
                    let _ = client
                        .post(url)
                        .timeout(Duration::from_secs(5))
                        .form(&[("chat_id", chat.as_str()), ("text", text.as_str())])
                        .send()
                        .await;
                });
            }
        }
        if let Some(location) = info.location() {
            tracing::error!(
                file = location.file(),
                line = location.line(),
                column = location.column(),
                "panic location"
            );
        }
    }));
}

#[cfg(test)]
mod panic_hook_tests {
    use super::redact_secret;

    #[test]
    fn panic_alert_redacts_bot_token_and_chat_id() {
        let message = "panic token=123456:ABC chat=987654";
        let without_token = redact_secret(message, "123456:ABC");
        let redacted = redact_secret(&without_token, "987654");

        assert!(!redacted.contains("123456:ABC"));
        assert!(!redacted.contains("987654"));
        assert_eq!(redacted, "panic token=[REDACTED] chat=[REDACTED]");
    }

    #[test]
    fn empty_secret_does_not_change_message() {
        assert_eq!(redact_secret("panic", ""), "panic");
    }
}

#[derive(Clone)]
struct FileMakeWriter {
    file: Arc<Mutex<std::fs::File>>,
    path: Arc<std::path::PathBuf>,
}

impl FileMakeWriter {
    /// Re-open the log path after the old inode was renamed away (rotation).
    fn reopen(&self) {
        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path.as_path())
        {
            if let Ok(mut guard) = self.file.lock() {
                *guard = f;
            }
        }
    }
}

/// Log-retention policy (operator directive): runtime logs older than
/// `LOG_RETENTION_HOURS` (default 24) are deleted; the opportunity/pool/PnL
/// database is never touched. The active file is rotated by size so a
/// long-running process cannot grow it unbounded.
const LOG_ROTATE_BYTES: u64 = 64 * 1024 * 1024;

fn spawn_log_retention(path: std::path::PathBuf, writer: FileMakeWriter) {
    let hours: u64 = std::env::var("LOG_RETENTION_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|h| *h > 0)
        .unwrap_or(24);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            let cutoff = std::time::SystemTime::now()
                .checked_sub(Duration::from_secs(hours * 3600))
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if let Ok(md) = std::fs::metadata(&path)
                && md.len() > LOG_ROTATE_BYTES
            {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let rotated = path.with_extension(format!("log.{stamp}"));
                if std::fs::rename(&path, &rotated).is_ok() {
                    writer.reopen();
                }
            }
            let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                continue;
            };
            let stem = path
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Ok(entries) = std::fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with(&format!("{stem}.")) {
                        continue;
                    }
                    let Ok(md) = entry.metadata() else { continue };
                    let Ok(modified) = md.modified() else {
                        continue;
                    };
                    if modified < cutoff {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    });
}

impl std::io::Write for FileMakeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut f = self
            .file
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        f.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut f = self
            .file
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        f.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileMakeWriter {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn init_event_logger() {
    let log_file_path =
        std::env::var("ARB_EVENT_LOG").unwrap_or_else(|_| "logs/arb_events.log".to_string());

    if let Some(parent) = std::path::Path::new(&log_file_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let default_filter = "info,arb_bot=debug,arb_engine=debug,arb_ingest=info,arb_tx=debug,arb_land=debug,arb_risk=debug,arb_sim=debug";
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let json_format = std::env::var("LOG_FORMAT")
        .map(|v| v.trim().eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
    {
        Ok(file) => {
            let file_writer = FileMakeWriter {
                file: Arc::new(Mutex::new(file)),
                path: Arc::new(std::path::PathBuf::from(&log_file_path)),
            };
            spawn_log_retention(
                std::path::PathBuf::from(&log_file_path),
                file_writer.clone(),
            );
            if json_format {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt::layer().json().flatten_event(true).with_ansi(true))
                    .with(
                        fmt::layer()
                            .json()
                            .flatten_event(true)
                            .with_ansi(false)
                            .with_writer(file_writer),
                    )
                    .init();
                tracing::info!(
                    "Structured JSON event logging active: console + file '{}'",
                    log_file_path
                );
            } else {
                let stdout_layer = fmt::layer().with_target(true).with_ansi(true);
                let file_layer = fmt::layer()
                    .with_target(true)
                    .with_ansi(false)
                    .with_writer(file_writer);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(stdout_layer)
                    .with(file_layer)
                    .init();
                tracing::info!(
                    "Detailed event logging active: console + file '{}'",
                    log_file_path
                );
            }
        }
        Err(e) => {
            if json_format {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt::layer().json().flatten_event(true).with_ansi(true))
                    .init();
            } else {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt::layer().with_target(true).with_ansi(true))
                    .init();
            }
            tracing::warn!(
                "Failed to open log file '{}': {}. Logging to stdout only.",
                log_file_path,
                e
            );
        }
    }
}

/// True when a secret is absent or obviously a placeholder. Placeholders must
/// never authorize an admin action (SEC-02 / Gate E).
fn is_placeholder_secret(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "changeme" | "change-me" | "placeholder" | "your_token_here" | "token"
    )
}

/// Constant-time comparison for admin bearer tokens. Length is not secret, but
/// the byte comparison must not short-circuit on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Accepts `Authorization: Bearer <token>` or `X-Admin-Token: <token>`.
fn admin_authorized(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-admin-token").and_then(|v| v.to_str().ok()));
    provided.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
}

fn main() -> Result<()> {
    install_panic_hook();
    load_process_environment()?;

    // Load .env before creating the Tokio runtime. Shadow builds use a filtered
    // loader so wallet secrets and live/canary arming values never enter env.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to create Tokio runtime")?
        .block_on(run())
}

#[cfg(feature = "shadow-only")]
const SHADOW_EXCLUDED_ENV_KEYS: &[&str] = &[
    "SOLANA_PRIVATE_KEY",
    "ARB_MODE",
    "ARB_CANARY",
    "ARB_LIVE",
    "LIVE_TRADING",
    "LIVE",
    "EXECUTE_LIVE",
    "SOLANA_TX_V1_DIRECT_ENABLED",
    "SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED",
    "SOLANA_TX_V1_JITO_ENABLED",
    "SOLANA_TX_V1_JITO_INGRESS_VERIFIED",
];

#[cfg(feature = "shadow-only")]
fn is_shadow_excluded_environment_key(key: &str) -> bool {
    SHADOW_EXCLUDED_ENV_KEYS.contains(&key)
}

#[cfg(feature = "shadow-only")]
fn load_process_environment() -> Result<()> {
    // Remove inherited wallet and live-arming values before reading .env.
    // This runs synchronously before Tokio runtime creation or other threads.
    for key in SHADOW_EXCLUDED_ENV_KEYS {
        // SAFETY: no application threads exist during this synchronous bootstrap.
        unsafe { std::env::remove_var(key) };
    }

    match dotenvy::dotenv_iter() {
        Ok(entries) => {
            for entry in entries {
                let (key, value) = entry?;
                if is_shadow_excluded_environment_key(&key) || std::env::var_os(&key).is_some() {
                    continue;
                }
                // SAFETY: no application threads exist during this synchronous bootstrap.
                unsafe { std::env::set_var(key, value) };
            }
        }
        Err(error) if error.not_found() => {}
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

#[cfg(not(feature = "shadow-only"))]
fn load_process_environment() -> Result<()> {
    // Preserve existing non-shadow behaviour: inherited variables win over .env.
    let _ = dotenvy::dotenv();
    Ok(())
}

fn append_config_audit_to(
    audit_path: &std::path::Path,
    config_path: &str,
    mode: BotMode,
) -> Result<()> {
    use sha2::Digest;
    use std::io::Write;
    let bytes = std::fs::read(config_path)?;
    let digest = sha2::Sha256::digest(&bytes);
    let sha256 = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    if let Some(parent) = audit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)?;
    let path_json = serde_json::to_string(config_path)?;
    writeln!(
        file,
        "{{\"timestamp_unix\":{timestamp},\"config_path\":{path_json},\"config_sha256\":\"{sha256}\",\"mode\":\"{mode}\"}}"
    )?;
    Ok(())
}

fn append_config_audit(config_path: &str, mode: BotMode) -> Result<()> {
    append_config_audit_to(
        std::path::Path::new("data/config_load_audit.jsonl"),
        config_path,
        mode,
    )
}

async fn run() -> Result<()> {
    let args = parse_args()?;

    init_event_logger();

    info!("Starting arb-bot in mode: {}", args.mode);
    info!(config_path = %args.config_path, "Using selected configuration file");

    let config = config::load(&args.config_path)
        .with_context(|| format!("Failed to load configuration from '{}'", args.config_path))?;
    if let Err(error) = append_config_audit(&args.config_path, args.mode) {
        warn!("Could not append config provenance audit record: {error}");
    }

    info!("Configuration loaded - RPC endpoint configured (credentials redacted)");
    info!("Max trade size: {} USD", config.risk.max_trade_size_usd);
    info!(
        "Min net profit: {} lamports ({} bps)",
        config.strategy.min_net_profit_abs, config.strategy.min_net_profit_bps
    );

    if args.mode == BotMode::Live {
        if !live_execution_enabled() {
            anyhow::bail!("Mode live requires ARB_LIVE=1 (explicit truthy value).");
        }
        // PIPE-08: live mode cannot carry dev freshness tolerances.
        config::validate_live_freshness(&config, true)?;
        warn!("LIVE execution mode active on real network funds!");
    } else {
        info!("Running in safe mode (dry-run / simulate). No live transactions will be submitted.");
    }

    // Start Prometheus text-exporter metrics server
    let metrics_server = arb_metrics::MetricsServer::new(&config.metrics.listen);
    let metrics_handle = metrics_server.start();
    info!(
        "📈 Prometheus Metrics Server running on http://{}/metrics",
        config.metrics.listen
    );

    info!("TRACE MAIN 1: before Supervisor::new");
    let admin_listen = config.admin.listen.clone();
    let admin_token_env = config.admin.token_env.clone();
    let supervisor_mode_atomic = Arc::new(std::sync::atomic::AtomicU8::new(args.mode as u8));
    let supervisor =
        supervisor::Supervisor::with_mode_atomic(Arc::clone(&supervisor_mode_atomic), config);
    let admin_kill_switch = supervisor.kill_switch();

    // Operator authentication is mandatory for every mutating admin route.
    // A missing or placeholder token disables the admin listener entirely
    // (fail closed) instead of exposing /pause and /resume unauthenticated.
    match std::env::var(&admin_token_env) {
        Ok(token) if !token.trim().is_empty() && !is_placeholder_secret(&token) => {
            // Start Admin HTTP Server on configured port (e.g. 127.0.0.1:9101)
            tokio::spawn(async move {
                let ks_pause = Arc::clone(&admin_kill_switch);
                let ks_resume = Arc::clone(&admin_kill_switch);
                let ks_status = Arc::clone(&admin_kill_switch);
                // Each route closure owns its own copy; the token value itself is
                // never logged or moved twice.
                let pause_token = token.clone();
                let resume_token = token;

                let app = axum::Router::new()
                    .route("/health", axum::routing::get(|| async { "OK\n" }))
                    .route(
                        "/status",
                        axum::routing::get(move || {
                            let ks = Arc::clone(&ks_status);
                            async move {
                                if ks.is_blocked() {
                                    "PAUSED\n"
                                } else {
                                    "RUNNING\n"
                                }
                            }
                        }),
                    )
                    .route(
                        "/pause",
                        axum::routing::post(move |headers: axum::http::HeaderMap| {
                            let ks = Arc::clone(&ks_pause);
                            let expected = pause_token.clone();
                            async move {
                                if !admin_authorized(&headers, &expected) {
                                    warn!("admin: rejected unauthenticated /pause request");
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "unauthorized\n",
                                    );
                                }
                                match ks.activate() {
                                    Ok(true) => {
                                        info!("admin: durable kill switch activated via authenticated /pause");
                                        (axum::http::StatusCode::OK, "Bot trading paused; durable marker persisted\n")
                                    }
                                    Ok(false) => (
                                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                        "Bot blocked in memory only; durable storage is not configured\n",
                                    ),
                                    Err(error) => {
                                        tracing::error!(error = %error, "admin /pause persistence failed; in-memory block remains active");
                                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Bot blocked in memory; durable persistence failed\n")
                                    }
                                }
                            }
                        }),
                    )
                    .route(
                        "/resume",
                        axum::routing::post(move |headers: axum::http::HeaderMap| {
                            let ks = Arc::clone(&ks_resume);
                            let expected = resume_token.clone();
                            async move {
                                if !admin_authorized(&headers, &expected) {
                                    warn!("admin: rejected unauthenticated /resume request");
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "unauthorized\n",
                                    );
                                }
                                match ks.deactivate() {
                                    Ok(()) => {
                                        info!("admin: kill switch cleared via authenticated /resume");
                                        (axum::http::StatusCode::OK, "Kill switch cleared via admin API\n")
                                    }
                                    Err(error) => {
                                        tracing::error!(error = %error, "admin /resume failed; kill switch remains blocked");
                                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Bot remains blocked; marker removal failed\n")
                                    }
                                }
                            }
                        }),
                    );

                match tokio::net::TcpListener::bind(&admin_listen).await {
                    Ok(listener) => {
                        info!(
                            "🛠️ Admin HTTP Server running on http://{admin_listen} (token required)"
                        );
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        warn!("Could not bind Admin HTTP Server on {admin_listen}: {e}");
                    }
                }
            });
        }
        _ => {
            warn!(
                "Admin HTTP API disabled: env var '{}' is missing or a placeholder; /pause and /resume stay unavailable (fail closed)",
                admin_token_env
            );
        }
    }

    // Unified control service: one command layer shared by the web app and
    // Telegram. Attached to the supervisor before run so the loop publishes
    // live state into it.
    let audit_path =
        std::env::var("WEB_AUDIT_LOG").unwrap_or_else(|_| "data/web_audit.log".to_string());
    let control = Arc::new(control_service::ControlService::new(
        Arc::clone(&supervisor_mode_atomic),
        supervisor.kill_switch(),
        audit_path,
    ));
    supervisor.attach_control(Arc::clone(&control));

    // Web application server (React frontend + authenticated REST/WS API).
    // Binds localhost by default; remote access goes through a TLS reverse proxy.
    let web_bind = std::env::var("WEB_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    {
        let control = Arc::clone(&control);
        tokio::spawn(async move {
            match web_api::serve((*control).clone(), &web_bind).await {
                Ok(()) => {}
                Err(err) => warn!("Web API server stopped on {web_bind}: {err}"),
            }
        });
    }

    // Telegram command listener: same unified control service as the web app.
    {
        let control = Arc::clone(&control);
        tokio::spawn(async move {
            telegram_control::run_listener(control).await;
        });
    }

    info!("TRACE MAIN 2: after Supervisor::new, before run");
    supervisor.run().await?;
    info!("TRACE MAIN 3: after run");

    metrics_handle.shutdown().await;

    info!("arb-bot exited cleanly.");
    Ok(())
}

#[cfg(test)]
mod flag_tests {
    #[cfg(feature = "shadow-only")]
    use super::is_shadow_excluded_environment_key;
    use super::{admin_authorized, constant_time_eq, is_placeholder_secret, is_truthy_flag};

    #[test]
    fn canonical_config_path_wins_and_legacy_root_is_never_selected() {
        let root =
            std::env::temp_dir().join(format!("arb-config-path-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config.toml"), "legacy = true").unwrap();
        std::fs::write(root.join("config/config.toml"), "canonical = true").unwrap();
        let resolved = super::resolve_default_config_path(&root).unwrap();
        assert_eq!(
            std::path::PathBuf::from(resolved),
            root.join("config").join("config.toml")
        );
        std::fs::remove_file(root.join("config/config.toml")).unwrap();
        assert!(super::resolve_default_config_path(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_audit_is_append_only_and_contains_no_config_values() {
        let root =
            std::env::temp_dir().join(format!("arb-config-audit-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("runtime.toml");
        let audit_path = root.join("audit/config_load_audit.jsonl");
        std::fs::write(&config_path, "admin_token = \"never-log-this-secret\"\n").unwrap();
        let config_path_string = config_path.to_string_lossy().into_owned();
        super::append_config_audit_to(&audit_path, &config_path_string, arb_bot::BotMode::Simulate)
            .unwrap();
        super::append_config_audit_to(&audit_path, &config_path_string, arb_bot::BotMode::Simulate)
            .unwrap();
        let contents = std::fs::read_to_string(&audit_path).unwrap();
        let rows: Vec<serde_json::Value> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["mode"], "simulate");
        assert!(rows[0]["config_sha256"].as_str().unwrap().len() == 64);
        assert!(!contents.contains("never-log-this-secret"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn arb_live_requires_an_explicit_truthy_value() {
        assert!(is_truthy_flag(Some("1")));
        assert!(is_truthy_flag(Some("true")));
        assert!(is_truthy_flag(Some(" TRUE ")));
        // Presence or falsy values must never authorize live execution.
        assert!(!is_truthy_flag(Some("0")));
        assert!(!is_truthy_flag(Some("false")));
        assert!(!is_truthy_flag(Some("")));
        assert!(!is_truthy_flag(None));
    }

    #[cfg(feature = "shadow-only")]
    #[test]
    fn shadow_dotenv_loader_excludes_wallet_and_live_arming_keys() {
        for key in [
            "SOLANA_PRIVATE_KEY",
            "ARB_MODE",
            "ARB_CANARY",
            "ARB_LIVE",
            "LIVE_TRADING",
            "LIVE",
            "EXECUTE_LIVE",
            "SOLANA_TX_V1_DIRECT_ENABLED",
            "SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED",
            "SOLANA_TX_V1_JITO_ENABLED",
            "SOLANA_TX_V1_JITO_INGRESS_VERIFIED",
        ] {
            assert!(
                is_shadow_excluded_environment_key(key),
                "expected {key} to be excluded from shadow .env loading"
            );
        }

        for key in [
            "HELIUS_RPC_URL",
            "SOLANA_RPC_URL",
            "TELEGRAM_BOT_TOKEN",
            "TELEGRAM_CHAT_ID",
        ] {
            assert!(
                !is_shadow_excluded_environment_key(key),
                "expected {key} to remain available in shadow mode"
            );
        }
    }

    #[test]
    fn placeholder_secrets_never_authorize_admin() {
        for placeholder in ["", "  ", "changeme", "CHANGE-ME", "placeholder", "token"] {
            assert!(
                is_placeholder_secret(placeholder),
                "expected {placeholder:?} to be treated as a placeholder"
            );
        }
        assert!(!is_placeholder_secret("s3cret-operator-token"));
    }

    #[test]
    fn constant_time_eq_matches_exact_bytes_only() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        // Length mismatch is a mismatch, never a panic.
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn admin_authorized_accepts_bearer_and_x_admin_token() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!admin_authorized(&headers, "secret-token"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret-token"),
        );
        assert!(admin_authorized(&headers, "secret-token"));
        assert!(!admin_authorized(&headers, "other-token"));

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-admin-token",
            axum::http::HeaderValue::from_static("secret-token"),
        );
        assert!(admin_authorized(&headers, "secret-token"));
        assert!(!admin_authorized(&headers, "other-token"));

        // A non-Bearer Authorization header must not be accepted.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Basic secret-token"),
        );
        assert!(!admin_authorized(&headers, "secret-token"));
    }
}
