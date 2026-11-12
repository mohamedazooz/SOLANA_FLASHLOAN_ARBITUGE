use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct AppState {
    signing_key: Option<Arc<str>>,
}

#[derive(Deserialize)]
struct WebhookEnvelope {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init()
        .map_err(|error| anyhow::anyhow!("Failed to initialize logging: {error}"))?;

    let listen = optional_env("ALCHEMY_WEBHOOK_LISTEN")?
        .unwrap_or_else(|| "127.0.0.1:8081".to_string())
        .parse::<SocketAddr>()
        .context("ALCHEMY_WEBHOOK_LISTEN must be a socket address")?;
    let signing_key = optional_env("ALCHEMY_WEBHOOK_SIGNING_KEY")?
        .filter(|value| !value.trim().is_empty())
        .map(Arc::<str>::from);
    if signing_key.is_none() {
        warn!(
            "ALCHEMY_WEBHOOK_SIGNING_KEY is unset; webhook POSTs will be rejected until configured"
        );
    }

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/webhooks/alchemy", post(receive_webhook))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(AppState { signing_key });

    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("Failed to bind Alchemy webhook listener at {listen}"))?;
    info!(%listen, "Alchemy webhook listener is ready");
    axum::serve(listener, app)
        .await
        .context("Alchemy webhook server stopped unexpectedly")?;
    Ok(())
}

fn optional_env(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("{name} is not valid Unicode")),
    }
}

async fn receive_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(signing_key) = state.signing_key.as_deref() else {
        error!("Rejected Alchemy webhook because its signing key is not configured");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook signing key is not configured",
        )
            .into_response();
    };

    let Some(signature) = headers
        .get("x-alchemy-signature")
        .and_then(|value| value.to_str().ok())
    else {
        warn!("Rejected Alchemy webhook without a valid signature header");
        return (StatusCode::UNAUTHORIZED, "invalid webhook signature").into_response();
    };

    if !signature_is_valid(signing_key, &body, signature) {
        warn!("Rejected Alchemy webhook with an invalid signature");
        return (StatusCode::UNAUTHORIZED, "invalid webhook signature").into_response();
    }

    let event: WebhookEnvelope = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(error) => {
            warn!(%error, "Rejected signed Alchemy webhook with an invalid JSON envelope");
            return (StatusCode::BAD_REQUEST, "invalid webhook payload").into_response();
        }
    };

    info!(
        event_id = %event.id,
        event_type = %event.event_type,
        "Received Alchemy webhook"
    );
    (StatusCode::OK, "ok").into_response()
}

fn signature_is_valid(signing_key: &str, body: &[u8], signature: &str) -> bool {
    let Ok(signature_bytes) = hex::decode(signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(signing_key.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&signature_bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_exact_body_signature() {
        let key = "test-signing-key";
        let body = br#"{"id":"event-1","type":"ADDRESS_ACTIVITY"}"#;
        let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("valid HMAC key");
        mac.update(body);
        let signature = hex::encode(mac.finalize().into_bytes());

        assert!(signature_is_valid(key, body, &signature));
        assert!(!signature_is_valid(
            key,
            br#"{"id":"event-2","type":"ADDRESS_ACTIVITY"}"#,
            &signature
        ));
        assert!(!signature_is_valid(key, body, "not-hex"));
    }
}
