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
                    "Solana Arbitrage Bot\n\nUsage: arb-bot [OPTIONS]\n\nOptions:\n  -c, --config <PATH>  Path to configuration TOML [default: auto-detected]\n  -h, --help           Print help\n\nMode is controlled by environment only:\n  ARB_MODE=simulate|live|canary  (default: simulate)\n  live additionally requires ARB_LIVE=1"
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
        if std::path::Path::new("config.toml").exists() {
            config_path = "config.toml".to_string();
        } else if std::path::Path::new("config/config.toml").exists() {
            config_path = "config/config.toml".to_string();
        } else if std::path::Path::new("config/config.example.toml").exists() {
            config_path = "config/config.example.toml".to_string();
        } else {
            config_path = "config.toml".to_string();
        }
    }

    // Mode is environment-controlled only. Precedence:
    // 1. `shadow-only` cargo feature (Docker shadow image): simulate, always.
    // 2. `ARB_MODE` (explicit simulate|live|canary)
    // 3. Legacy arming flags: ARB_CANARY, ARB_LIVE/LIVE/EXECUTE_LIVE
    // 4. Safe fallback: "simulate" (non-broadcast)
    let mode_str = if cfg!(feature = "shadow-only") {
        eprintln!("shadow-only build: live/canary arming is compiled out; forcing simulate");
        "simulate".to_string()
    } else if let Ok(m) = std::env::var("ARB_MODE") {
        if !m.trim().is_empty() {
            m
        } else {
            "simulate".to_string()
        }
    } else if std::env::var("ARB_CANARY").is_ok() {
        "canary".to_string()
    } else if std::env::var("ARB_LIVE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        || std::env::var("LIVE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        || std::env::var("EXECUTE_LIVE").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        "live".to_string()
    } else {
        "simulate".to_string()
    };

    let mode: BotMode = mode_str.parse()?;
    Ok(Args { config_path, mode })
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("FATAL PANIC: {info}");
        // Best-effort operator alert: the runtime may already be unwinding, so
        // this uses a short-lived curl subprocess instead of the async client.
        if let (Ok(token), Ok(chat)) = (
            std::env::var("TELEGRAM_BOT_TOKEN"),
            std::env::var("TELEGRAM_CHAT_ID"),
        ) {
            let text = format!("🚨 <b>FATAL PANIC — arb-bot stopped</b>\n<code>{info}</code>");
            let _ = std::process::Command::new("curl")
                .args([
                    "-s",
                    "-m",
                    "5",
                    "-X",
                    "POST",
                    &format!("https://api.telegram.org/bot{token}/sendMessage"),
                    "-d",
                    &format!("chat_id={chat}"),
                    "--data-urlencode",
                    &format!("text={text}"),
                    "-d",
                    "parse_mode=HTML",
                ])
                .output();
        }
        default_hook(info);
    }));
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

    let stdout_layer = fmt::layer().with_target(true).with_ansi(true);

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
        Err(e) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .init();
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

#[tokio::main]
async fn main() -> Result<()> {
    install_panic_hook();
    if dotenvy::dotenv().is_ok() {
        // .env loaded
    }

    let args = parse_args()?;

    init_event_logger();

    info!("Starting arb-bot in mode: {}", args.mode);

    let config = config::load(&args.config_path)
        .with_context(|| format!("Failed to load configuration from '{}'", args.config_path))?;

    info!("Configuration loaded — RPC: {}", config.rpc.url);
    info!("Max trade size: {} USD", config.risk.max_trade_size_usd);
    info!(
        "Min net profit: {} lamports ({} bps)",
        config.strategy.min_net_profit_abs, config.strategy.min_net_profit_bps
    );

    if args.mode == BotMode::Live {
        if !live_execution_enabled() {
            anyhow::bail!("Mode live requires ARB_LIVE=1 (explicit truthy value).");
        }
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
                                ks.activate();
                                info!("admin: kill switch activated via authenticated /pause");
                                (
                                    axum::http::StatusCode::OK,
                                    "Bot trading paused via admin API\n",
                                )
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
                                ks.deactivate();
                                info!("admin: kill switch cleared via authenticated /resume");
                                (
                                    axum::http::StatusCode::OK,
                                    "Bot trading resumed via admin API\n",
                                )
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
    use super::{admin_authorized, constant_time_eq, is_placeholder_secret, is_truthy_flag};

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
