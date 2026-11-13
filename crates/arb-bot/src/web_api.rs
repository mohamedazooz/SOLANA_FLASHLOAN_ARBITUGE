//! Web application backend — REST + WebSocket API serving the React dashboard.
//!
//! Security model:
//! - The listener binds to localhost by default (`WEB_BIND`); remote access is
//!   expected only through a reverse proxy (Caddy/Nginx) that terminates TLS.
//! - Authentication: single-operator password from `WEB_ADMIN_PASSWORD`
//!   (fail-closed when unset or a placeholder) exchanged for a random session
//!   token with a bounded TTL. Tokens travel in the `Authorization` header
//!   (never cookies), which removes the CSRF surface for token-authenticated
//!   requests; static assets are read-only.
//! - Rate limiting: per-IP fixed-window limiter on login and mutating routes.
//! - Audit: every mutating action is appended to the shared control audit log.
//! - No private keys or signing secrets are ever exposed to the browser.

use crate::control_service::{ConfigOverrides, ControlService, execute_command};
use axum::extract::DefaultBodyLimit;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::{Json as AxumJson, Router};
use parking_lot::Mutex;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_SESSIONS: usize = 64;
const DEFAULT_SESSION_TTL_SECS: u64 = 3_600;
const LOGIN_RATE_LIMIT_PER_MIN: u32 = 10;
const MUTATION_RATE_LIMIT_PER_MIN: u32 = 60;
/// Single-use WebSocket tickets: short-lived, consumed on first use, so the
/// long-lived session token never has to appear in a URL (which browsers
/// require for the WS handshake and which proxy/access logs would retain).
const WS_TICKET_TTL_SECS: u64 = 60;
const MAX_WS_TICKETS: usize = 64;

struct Session {
    expires: Instant,
}

#[derive(Clone)]
pub struct WebState {
    control: ControlService,
    sessions: Arc<Mutex<HashMap<[u8; 32], Session>>>,
    password_hash: [u8; 32],
    session_ttl: Duration,
    login_hits: Arc<Mutex<HashMap<std::net::IpAddr, (Instant, u32)>>>,
    mutation_hits: Arc<Mutex<HashMap<std::net::IpAddr, (Instant, u32)>>>,
    ws_tickets: Arc<Mutex<HashMap<[u8; 32], Instant>>>,
    ws_clients: Arc<AtomicU64>,
}

fn sha256(input: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(input);
    hasher.finalize().into()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_placeholder(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    v.is_empty()
        || matches!(
            v.as_str(),
            "changeme" | "change-me" | "placeholder" | "replace_me" | "token" | "password"
        )
}

impl WebState {
    pub fn new(control: ControlService) -> anyhow::Result<Self> {
        let password = std::env::var("WEB_ADMIN_PASSWORD").map_err(|_| {
            anyhow::anyhow!(
                "WEB_ADMIN_PASSWORD is not set — the web API refuses to start (fail-closed)"
            )
        })?;
        if is_placeholder(&password) {
            anyhow::bail!("WEB_ADMIN_PASSWORD is a placeholder — refusing to start");
        }
        let ttl = std::env::var("WEB_SESSION_TTL_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| (60..=86_400).contains(v))
            .unwrap_or(DEFAULT_SESSION_TTL_SECS);
        Ok(Self {
            control,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            password_hash: sha256(password.trim().as_bytes()),
            session_ttl: Duration::from_secs(ttl),
            login_hits: Arc::new(Mutex::new(HashMap::new())),
            mutation_hits: Arc::new(Mutex::new(HashMap::new())),
            ws_tickets: Arc::new(Mutex::new(HashMap::new())),
            ws_clients: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Issue a single-use, 60-second WebSocket upgrade ticket. Only callable
    /// from an authorized session; the ticket hash is stored, the raw value is
    /// returned once, and the entry is consumed at first use.
    fn issue_ws_ticket(&self) -> (String, u64) {
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        let ticket_hex = hex::encode(raw);
        let hash = sha256(ticket_hex.as_bytes());
        let expires = Instant::now() + Duration::from_secs(WS_TICKET_TTL_SECS);
        let mut tickets = self.ws_tickets.lock();
        tickets.retain(|_, exp| *exp > Instant::now());
        if tickets.len() >= MAX_WS_TICKETS {
            tickets.clear();
        }
        tickets.insert(hash, expires);
        (ticket_hex, now_unix() + WS_TICKET_TTL_SECS)
    }

    /// Consume a single-use WebSocket ticket. Returns false for unknown,
    /// expired, or already-used tickets (single-use by removal).
    fn consume_ws_ticket(&self, ticket: &str) -> bool {
        let hash = sha256(ticket.trim().as_bytes());
        let mut tickets = self.ws_tickets.lock();
        match tickets.remove(&hash) {
            Some(expires) => expires > Instant::now(),
            None => false,
        }
    }

    fn rate_ok(
        hits: &Mutex<HashMap<std::net::IpAddr, (Instant, u32)>>,
        ip: std::net::IpAddr,
        limit: u32,
    ) -> bool {
        let mut map = hits.lock();
        let now = Instant::now();
        let entry = map.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= Duration::from_secs(60) {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= limit
    }

    fn login(&self, ip: std::net::IpAddr, password: &str) -> Result<(String, u64), StatusCode> {
        if !Self::rate_ok(&self.login_hits, ip, LOGIN_RATE_LIMIT_PER_MIN) {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        // Constant-time comparison over the digest.
        let candidate = sha256(password.trim().as_bytes());
        let mut diff = 0u8;
        for (a, b) in candidate.iter().zip(self.password_hash.iter()) {
            diff |= a ^ b;
        }
        if diff != 0 || candidate != self.password_hash {
            self.control
                .audit_action("web_login_failed", &ip.to_string());
            return Err(StatusCode::UNAUTHORIZED);
        }
        let mut token = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut token);
        let token_hex = hex::encode(token);
        let token_hash = sha256(token_hex.as_bytes());
        {
            let mut sessions = self.sessions.lock();
            sessions.retain(|_, s| s.expires > Instant::now());
            if sessions.len() >= MAX_SESSIONS {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            sessions.insert(
                token_hash,
                Session {
                    expires: Instant::now() + self.session_ttl,
                },
            );
        }
        self.control.audit_action("web_login", &ip.to_string());
        let ttl = self.session_ttl.as_secs();
        Ok((token_hex, now_unix() + ttl))
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let provided_hash = sha256(provided.trim().as_bytes());
        let sessions = self.sessions.lock();
        match sessions.get(&provided_hash) {
            Some(session) if session.expires > Instant::now() => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }

    fn mutation_rate_ok(&self, ip: std::net::IpAddr) -> bool {
        Self::rate_ok(&self.mutation_hits, ip, MUTATION_RATE_LIMIT_PER_MIN)
    }
}

/// Client identity for rate limiting. The API binds localhost; remote clients
/// arrive through the reverse proxy which APPENDS to `X-Forwarded-For`. The
/// right-most entry is the one added by the trusted proxy; earlier entries are
/// client-controlled and spoofable, so they must never be trusted.
fn ip_of(headers: &HeaderMap) -> std::net::IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next_back())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| std::net::IpAddr::from([127, 0, 0, 1]))
}

async fn login(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: Option<AxumJson<Value>>,
) -> Response {
    let password = body
        .as_ref()
        .and_then(|b| b.0.get("password"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match state.login(ip_of(&headers), password) {
        Ok((token, expires_at)) => (
            StatusCode::OK,
            Json(json!({"token": token, "expires_at_unix": expires_at})),
        )
            .into_response(),
        Err(code) => (code, Json(json!({"error": "login rejected"}))).into_response(),
    }
}

#[allow(clippy::result_large_err)]
fn authed(state: &WebState, headers: &HeaderMap) -> Result<std::net::IpAddr, Response> {
    let ip = ip_of(headers);
    if let Err(code) = state.authorize(headers) {
        return Err((code, Json(json!({"error": "unauthorized"}))).into_response());
    }
    Ok(ip)
}

async fn status(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.status()).into_response()
}

async fn opportunities(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.opportunities(128)).into_response()
}

async fn executions(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.executions(128)).into_response()
}

async fn logs(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.logs(300)).into_response()
}

async fn pnl(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.pnl()).into_response()
}

async fn providers(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.providers()).into_response()
}

async fn routes(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.route_coverage()).into_response()
}

async fn config_get(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    Json(state.control.config_summary()).into_response()
}

#[derive(Deserialize)]
struct ControlBody {
    confirm: Option<String>,
}

async fn control_post(
    State(state): State<WebState>,
    Path(action): Path<String>,
    headers: HeaderMap,
    body: Option<AxumJson<ControlBody>>,
) -> Response {
    let ip = match authed(&state, &headers) {
        Ok(ip) => ip,
        Err(resp) => return resp,
    };
    if !state.mutation_rate_ok(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let confirm = body.and_then(|b| b.0.confirm);
    match execute_command(&state.control, &action, confirm.as_deref()) {
        Ok(message) => {
            Json(json!({"ok": true, "action": action, "message": message})).into_response()
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "action": action, "error": err.0})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ConfigBody {
    min_net_profit_usd: Option<f64>,
    min_borrow_lamports: Option<u64>,
    max_borrow_lamports: Option<u64>,
    max_slippage_bps: Option<u64>,
}

async fn config_post(
    State(state): State<WebState>,
    headers: HeaderMap,
    AxumJson(body): AxumJson<ConfigBody>,
) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    let ip = ip_of(&headers);
    if !state.mutation_rate_ok(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limited"})),
        )
            .into_response();
    }
    let update = ConfigOverrides {
        min_net_profit_usd: body.min_net_profit_usd,
        min_borrow_lamports: body.min_borrow_lamports,
        max_borrow_lamports: body.max_borrow_lamports,
        max_slippage_bps: body.max_slippage_bps,
    };
    match state.control.update_config(&update) {
        Ok(merged) => Json(json!({"ok": true, "overrides": merged})).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": err.0})),
        )
            .into_response(),
    }
}

async fn logout(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        let hash = sha256(token.trim().as_bytes());
        state.sessions.lock().remove(&hash);
    }
    state
        .control
        .audit_action("web_logout", &ip_of(&headers).to_string());
    Json(json!({"ok": true})).into_response()
}

/// Issue a single-use WebSocket upgrade ticket for an authorized session so
/// the long-lived session token never appears in a URL.
async fn ws_ticket(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(resp) = authed(&state, &headers) {
        return resp;
    }
    let (ticket, expires_at_unix) = state.issue_ws_ticket();
    Json(json!({"ticket": ticket, "expires_at_unix": expires_at_unix})).into_response()
}

/// WebSocket event stream. Authentication happens once at upgrade time via
/// the `Authorization` header, or — because browsers cannot set custom headers
/// on WebSocket handshakes — via a single-use `?ticket=` issued by
/// `/api/ws-ticket`. After upgrade the socket is receive-only for the client
/// (server pushes events).
async fn ws_events(
    State(state): State<WebState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    upgrade: WebSocketUpgrade,
) -> Response {
    let query_param = |name: &str| {
        query.as_deref().and_then(|q| {
            q.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                (k == name).then(|| v.to_string())
            })
        })
    };
    // Preferred: single-use ticket (consumed here, safe to appear in URLs).
    let ticket_ok = query_param("ticket")
        .map(|t| state.consume_ws_ticket(&t))
        .unwrap_or(false);
    // Fallback: session token in `?token=` (legacy clients).
    let query_ok = query_param("token")
        .map(|token| {
            let hash = sha256(token.as_bytes());
            let sessions = state.sessions.lock();
            sessions
                .get(&hash)
                .map(|s| s.expires > Instant::now())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if state.authorize(&headers).is_err() && !ticket_ok && !query_ok {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    upgrade.on_upgrade(move |socket| ws_loop(state, socket))
}

async fn ws_loop(state: WebState, mut socket: WebSocket) {
    let mut rx = state.control.subscribe();
    state.ws_clients.fetch_add(1, Ordering::Relaxed);
    // Send an initial snapshot so the client renders immediately.
    let snapshot = serde_json::to_string(&state.control.status()).unwrap_or_default();
    let _ = socket.send(Message::Text(snapshot.into())).await;
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        let text = serde_json::to_string(&event).unwrap_or_default();
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // client messages ignored (receive-only)
                    Some(Err(_)) => break,
                }
            }
        }
    }
    state.ws_clients.fetch_sub(1, Ordering::Relaxed);
}

async fn index() -> Html<String> {
    for candidate in ["web/dist/index.html", "dashboard/index.html"] {
        if let Ok(html) = std::fs::read_to_string(candidate) {
            return Html(html);
        }
    }
    Html(include_str!("../../../dashboard/index.html").to_string())
}

async fn health() -> &'static str {
    "OK\n"
}

/// Builds the authenticated web router. `bind_addr` must be a loopback or
/// private interface in production; public exposure is the reverse proxy's job.
pub fn router(control: ControlService) -> anyhow::Result<Router> {
    let state = WebState::new(control)?;
    Ok(Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/status", get(status))
        .route("/api/opportunities", get(opportunities))
        .route("/api/executions", get(executions))
        .route("/api/logs", get(logs))
        .route("/api/pnl", get(pnl))
        .route("/api/providers", get(providers))
        .route("/api/routes", get(routes))
        .route("/api/config", get(config_get).post(config_post))
        .route("/api/control/{action}", post(control_post))
        .route("/api/ws-ticket", post(ws_ticket))
        .route("/api/ws", get(ws_events))
        .route("/", get(index))
        .route("/health", get(health))
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state))
}

/// Baseline security headers on every response (OWASP secure-headers baseline).
/// TLS termination happens at the reverse proxy; these apply at any origin.
async fn security_headers(req: axum::extract::Request, next: middleware::Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    res
}

/// Spawns the web server bound to `bind_addr` (default `127.0.0.1:8080`).
pub async fn serve(control: ControlService, bind_addr: &str) -> anyhow::Result<()> {
    let app = router(control)?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_service::ControlService;
    use arb_risk::KillSwitch;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    const PASSWORD: &str = "test-operator-pass-123";

    // WEB_ADMIN_PASSWORD is process-global; every test that reads or mutates it
    // must hold this lock so cargo's parallel test threads cannot race.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn test_control() -> ControlService {
        let mode = Arc::new(std::sync::atomic::AtomicU8::new(0));
        ControlService::new(
            mode,
            Arc::new(KillSwitch::new()),
            std::env::temp_dir()
                .join(format!("web_audit_{}.log", std::process::id()))
                .display()
                .to_string(),
        )
    }

    fn test_state() -> WebState {
        WebState::new(test_control()).expect("state with WEB_ADMIN_PASSWORD set")
    }

    // One sequential test: WEB_ADMIN_PASSWORD is process-global and cargo runs
    // test cases in parallel threads, so per-test env manipulation would race.
    #[tokio::test]
    async fn web_api_auth_and_control_matrix() {
        let _env = ENV_LOCK.lock().await;
        unsafe { std::env::remove_var("WEB_ADMIN_PASSWORD") };

        // 1. Fail-closed: no password, no API.
        let mode = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let control =
            ControlService::new(mode, Arc::new(KillSwitch::new()), "/tmp/unused_audit.log");
        assert!(WebState::new(control).is_err());

        unsafe { std::env::set_var("WEB_ADMIN_PASSWORD", PASSWORD) };
        let state = test_state();
        let ip = std::net::IpAddr::from([127, 0, 0, 1]);

        // 2. Wrong password rejected; correct one yields a token.
        assert_eq!(state.login(ip, "wrong"), Err(StatusCode::UNAUTHORIZED));
        let (token, expires) = state.login(ip, PASSWORD).expect("login");
        assert_eq!(token.len(), 64);
        assert!(expires > now_unix());
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        assert!(state.authorize(&headers).is_ok());

        // 3. Full router: unauthenticated 401, then authenticated 200s.
        let app = router(test_control()).expect("router");
        let resp = app
            .clone()
            .oneshot(Request::get("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"test-operator-pass-123"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let token = serde_json::from_slice::<Value>(&bytes).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();
        let auth = format!("Bearer {token}");

        for path in [
            "/api/status",
            "/api/opportunities",
            "/api/executions",
            "/api/pnl",
            "/api/providers",
            "/api/config",
            "/api/logs",
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::get(path)
                        .header("authorization", &auth)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "GET {path}");
        }

        // 4. Mutating control routes work and unknown commands fail closed.
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/control/stop")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::post("/api/control/not-a-command")
                    .header("authorization", &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 5. Login rate limiting.
        let limited_ip = std::net::IpAddr::from([192, 168, 1, 50]);
        for _ in 0..LOGIN_RATE_LIMIT_PER_MIN {
            let _ = state.login(limited_ip, "wrong");
        }
        assert_eq!(
            state.login(limited_ip, PASSWORD),
            Err(StatusCode::TOO_MANY_REQUESTS)
        );

        unsafe { std::env::remove_var("WEB_ADMIN_PASSWORD") };
    }

    // End-to-end: real listener on an ephemeral port, real login, real
    // authenticated REST call, real WebSocket upgrade (101).
    #[tokio::test]
    async fn web_server_end_to_end_with_ws_upgrade() {
        let _env = ENV_LOCK.lock().await;
        unsafe { std::env::set_var("WEB_ADMIN_PASSWORD", PASSWORD) };
        let app = router(test_control()).expect("router");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");

        let resp = client
            .post(format!("{base}/api/login"))
            .json(&serde_json::json!({"password": PASSWORD}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let token = resp.json::<Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();

        let resp = client
            .get(format!("{base}/api/status"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let status = resp.json::<Value>().await.unwrap();
        assert_eq!(status["mode"], "shadow");

        // Raw WS upgrade: browsers authenticate with ?token=.
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ=="; // RFC 6455 sample key
        let req = format!(
            "GET /api/ws?token={token} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 512];
        let n = stream.read(&mut buf).await.unwrap();
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(
            head.starts_with("HTTP/1.1 101"),
            "expected 101 switching protocols, got: {head}"
        );

        unsafe { std::env::remove_var("WEB_ADMIN_PASSWORD") };
    }

    #[test]
    fn ws_ticket_is_single_use_and_unknown_tickets_fail() {
        unsafe { std::env::set_var("WEB_ADMIN_PASSWORD", PASSWORD) };
        let state = test_state();

        let (ticket, expires_at) = state.issue_ws_ticket();
        assert!(expires_at > now_unix());
        assert!(
            state.consume_ws_ticket(&ticket),
            "fresh ticket must upgrade"
        );
        assert!(
            !state.consume_ws_ticket(&ticket),
            "a consumed ticket must be rejected (single-use)"
        );
        assert!(!state.consume_ws_ticket("not-a-ticket"));

        unsafe { std::env::remove_var("WEB_ADMIN_PASSWORD") };
    }
}
