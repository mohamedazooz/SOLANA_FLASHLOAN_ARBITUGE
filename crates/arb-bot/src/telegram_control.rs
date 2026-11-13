//! Telegram command listener — maps the unified command set onto the same
//! `ControlService` the web API uses. Authorization: only chat IDs listed in
//! `TELEGRAM_CONTROL_CHAT_IDS` (comma-separated) may issue commands; every
//! command is executed through `execute_command` so authorization, validation
//! and audit logging are identical to the web interface.
//!
//! Long-polls `getUpdates`; disables itself silently when `TELEGRAM_BOT_TOKEN`
//! is absent (alert-only operation, as before).

use crate::control_service::{ControlService, execute_command};
use std::sync::Arc;
use std::time::Duration;

fn bot_token() -> Option<String> {
    let token = std::env::var("TELEGRAM_BOT_TOKEN").ok()?;
    let token = token.trim().to_string();
    if token.is_empty() || token.starts_with("REPLACE_ME") {
        return None;
    }
    Some(token)
}

fn allowed_chats() -> Option<Vec<i64>> {
    let raw = std::env::var("TELEGRAM_CONTROL_CHAT_IDS").ok()?;
    let ids: Vec<i64> = raw
        .split(',')
        .filter_map(|s| {
            let id = s.trim().parse::<i64>().ok()?;
            // Reject chat id 0: Telegram chat IDs are non-zero; 0 is used as a
            // sentinel for malformed updates and must never be allowlisted.
            (id != 0).then_some(id)
        })
        .collect();
    if ids.is_empty() {
        return None;
    }
    Some(ids)
}

async fn call_api(method: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(token) = bot_token() else {
        return Err("no token".into());
    };
    let url = format!("https://api.telegram.org/bot{token}/{method}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(40))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(value)
}

async fn send_reply(chat_id: i64, text: &str) {
    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    if let Err(err) = call_api("sendMessage", &body).await {
        tracing::warn!("telegram sendMessage failed: {err}");
    }
}

/// Runs the long-poll loop. Returns when the token or allowlist is absent
/// (alert-only mode) or the poll loop exhausts its retries.
pub async fn run_listener(control: Arc<ControlService>) {
    let Some(_token) = bot_token() else {
        tracing::info!("Telegram control listener disabled (no TELEGRAM_BOT_TOKEN)");
        return;
    };
    let Some(allowed) = allowed_chats() else {
        tracing::info!(
            "Telegram control listener disabled (no TELEGRAM_CONTROL_CHAT_IDS allowlist)"
        );
        return;
    };
    let mut offset: i64 = 0;
    let mut consecutive_failures: u32 = 0;
    loop {
        let body = serde_json::json!({
            "offset": offset,
            "timeout": 30,
            "allowed_updates": ["message"],
        });
        match call_api("getUpdates", &body).await {
            Ok(value) => {
                consecutive_failures = 0;
                let updates = value
                    .get("result")
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default();
                for update in updates {
                    let update_id = update
                        .get("update_id")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    offset = offset.max(update_id + 1);
                    let chat_id = update
                        .pointer("/message/chat/id")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let text = update
                        .pointer("/message/text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if chat_id == 0 || text.is_empty() || !allowed.contains(&chat_id) {
                        continue;
                    }
                    let (command, arg) = match text.split_once(' ') {
                        Some((c, a)) => (c, Some(a.trim())),
                        None => (text.as_str(), None),
                    };
                    match execute_command(&control, command, arg) {
                        Ok(reply) => send_reply(chat_id, &reply).await,
                        Err(err) => send_reply(chat_id, &format!("⛔ {err}")).await,
                    }
                }
            }
            Err(err) => {
                consecutive_failures += 1;
                if consecutive_failures > 10 {
                    tracing::warn!("Telegram control listener giving up: {err}");
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
