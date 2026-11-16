//! Oxylabs Web API client: search, scrape, and the search -> scrape research loop.
//!
//! Two endpoints on `https://webapi.oxylabs.io`:
//! - `POST /v1/search`  ranked results (title, snippet/outline, url)
//! - `POST /v1/scrape`  content of one page
//!
//! Auth: `Authorization: Bearer $OXYLABS_WEB_API_KEY`. Key is never logged.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, warn};

pub const API_BASE: &str = "https://webapi.oxylabs.io";
pub const DEFAULT_TIMEOUT_SECS: u64 = 150;
pub const MAX_ATTEMPTS: u32 = 3;
pub const RETRYABLE_STATUSES: &[u16] = &[408, 429, 500, 502, 503, 504];

#[derive(Debug, Clone)]
pub struct OxylabsConfig {
    pub api_key: String,
    pub base_url: String,
    pub timeout: Duration,
}

impl OxylabsConfig {
    pub fn from_env() -> Result<Self, OxylabsError> {
        let key = std::env::var("OXYLABS_WEB_API_KEY")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if key.is_empty() {
            return Err(OxylabsError::MissingKey);
        }
        Ok(Self {
            api_key: key,
            base_url: API_BASE.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        })
    }
}

#[derive(Debug)]
pub enum OxylabsError {
    MissingKey,
    Authentication,
    Validation { field: String, detail: String },
    Http { status: u16, message: String },
    Transport(String),
    Timeout(u32),
    EmptyResult { url: String },
    Unreadable { url: String },
}

impl std::fmt::Display for OxylabsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OxylabsError::MissingKey => write!(f, "OXYLABS_WEB_API_KEY is not set"),
            OxylabsError::Authentication => {
                write!(f, "authentication failed (401): the API key was rejected")
            }
            OxylabsError::Validation { field, detail } => {
                write!(f, "validation error: {detail} (field: {field})")
            }
            OxylabsError::Http { status, message } => write!(f, "http {status}: {message}"),
            OxylabsError::Transport(m) => write!(f, "transport error: {m}"),
            OxylabsError::Timeout(n) => write!(f, "timed out after {n} attempts"),
            OxylabsError::EmptyResult { url } => write!(f, "no result payload for {url}"),
            OxylabsError::Unreadable { url } => write!(f, "page unreadable: {url}"),
        }
    }
}

impl std::error::Error for OxylabsError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchResult {
    pub title: String,
    #[serde(default)]
    pub outline: Vec<String>,
    pub url: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScrapeResult {
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default, rename = "json")]
    pub json: Option<Value>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub results: Vec<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub errors: Option<Vec<ProblemError>>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProblemError {
    #[serde(default)]
    pub pointer: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Transport abstraction so tests can mock HTTP.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn post(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<(u16, String), OxylabsError>;
}

pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn post(
        &self,
        url: &str,
        bearer: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<(u16, String), OxylabsError> {
        let resp = self
            .client
            .post(url)
            .timeout(timeout)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    OxylabsError::Transport(format!("timeout: {e}"))
                } else {
                    OxylabsError::Transport(e.to_string())
                }
            })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Ok((status, text))
    }
}

pub struct OxylabsClient<T: HttpTransport> {
    config: OxylabsConfig,
    transport: T,
}

impl OxylabsClient<ReqwestTransport> {
    pub fn new(config: OxylabsConfig) -> Self {
        let transport = ReqwestTransport::new(config.timeout);
        Self { config, transport }
    }
}

impl<T: HttpTransport> OxylabsClient<T> {
    pub fn with_transport(config: OxylabsConfig, transport: T) -> Self {
        Self { config, transport }
    }

    pub fn config(&self) -> &OxylabsConfig {
        &self.config
    }

    pub fn request_id_from(envelope: &Envelope) -> Option<String> {
        envelope
            .metadata
            .as_ref()
            .and_then(|m| m.get("request_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, OxylabsError> {
        let url = format!("{}{}", self.config.base_url, path);
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let res = self
                .transport
                .post(&url, &self.config.api_key, &body, self.config.timeout)
                .await;
            match res {
                Ok((status, text)) => {
                    if (200..300).contains(&status) {
                        let v: Value = serde_json::from_str(&text)
                            .map_err(|e| OxylabsError::Transport(format!("bad json: {e}")))?;
                        return Ok(v);
                    }
                    if status == 401 {
                        return Err(OxylabsError::Authentication);
                    }
                    if status == 400 {
                        // parse problem body
                        if let Ok(env) = serde_json::from_str::<Envelope>(&text) {
                            if let Some(errs) = env.errors {
                                if let Some(e) = errs.first() {
                                    let field = e
                                        .pointer
                                        .clone()
                                        .unwrap_or_default()
                                        .trim_start_matches('#')
                                        .to_string();
                                    let detail = e.detail.clone().unwrap_or_default();
                                    return Err(OxylabsError::Validation { field, detail });
                                }
                            }
                            let detail = env
                                .detail
                                .clone()
                                .unwrap_or_else(|| text.chars().take(300).collect());
                            return Err(OxylabsError::Validation {
                                field: String::new(),
                                detail,
                            });
                        }
                        return Err(OxylabsError::Validation {
                            field: String::new(),
                            detail: text.chars().take(300).collect(),
                        });
                    }
                    let retryable = RETRYABLE_STATUSES.contains(&status);
                    if retryable && attempt < MAX_ATTEMPTS {
                        let backoff =
                            Duration::from_millis(10 * (1u64 << attempt) + (attempt as u64 * 7));
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(OxylabsError::Http {
                        status,
                        message: text.chars().take(300).collect(),
                    });
                }
                Err(OxylabsError::Transport(_)) if attempt < MAX_ATTEMPTS => {
                    let backoff = Duration::from_millis(10 * (1u64 << attempt));
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub async fn search(
        &self,
        query: &str,
        max_results: u32,
        location: Option<&str>,
    ) -> Result<Vec<SearchResult>, OxylabsError> {
        if query.is_empty() || query.len() > 2048 {
            return Err(OxylabsError::Validation {
                field: "query".into(),
                detail: "must be 1-2048 chars".into(),
            });
        }
        if !(1..=20).contains(&max_results) {
            return Err(OxylabsError::Validation {
                field: "max_results".into(),
                detail: "must be 1-20".into(),
            });
        }
        let mut body = serde_json::json!({
            "query": query,
            "max_results": max_results,
        });
        if let Some(loc) = location {
            body["location"] = Value::String(loc.to_string());
        }
        let v = self.post("/v1/search", body).await?;
        let env: Envelope = serde_json::from_value(v)
            .map_err(|e| OxylabsError::Transport(format!("bad envelope: {e}")))?;
        // request_id logging (debug only)
        if let Some(rid) = Self::request_id_from(&env) {
            debug!("oxylabs search request_id={}", rid);
        }
        let mut out = Vec::new();
        for item in env.results {
            if let Ok(sr) = serde_json::from_value::<SearchResult>(item) {
                out.push(sr);
            }
        }
        Ok(out)
    }

    pub async fn scrape(
        &self,
        url: &str,
        fmt: &str,
        run_js: bool,
        location: Option<&str>,
    ) -> Result<ScrapeResult, OxylabsError> {
        let mut body = serde_json::json!({
            "url": url,
            "output": [fmt],
        });
        if run_js {
            body["run_js"] = Value::Bool(true);
        }
        if let Some(loc) = location {
            body["location"] = Value::String(loc.to_string());
        }
        let v = self.post("/v1/scrape", body).await?;
        let env: Envelope = serde_json::from_value(v)
            .map_err(|e| OxylabsError::Transport(format!("bad envelope: {e}")))?;
        if let Some(rid) = Self::request_id_from(&env) {
            debug!("oxylabs scrape request_id={}", rid);
        }
        let first = env
            .results
            .into_iter()
            .next()
            .ok_or_else(|| OxylabsError::EmptyResult {
                url: url.to_string(),
            })?;
        let sr: ScrapeResult = serde_json::from_value(first)
            .map_err(|e| OxylabsError::Transport(format!("bad scrape result: {e}")))?;
        // check requested format present
        let present = match fmt {
            "markdown" => sr.markdown.is_some(),
            "html" => sr.html.is_some(),
            "json" => sr.json.is_some(),
            _ => true,
        };
        if !present {
            return Err(OxylabsError::EmptyResult {
                url: url.to_string(),
            });
        }
        Ok(sr)
    }

    /// Scrape with the empty/skeletal retry policy from the docs:
    /// 1. plain scrape
    /// 2. if empty/skeletal -> retry run_js: true
    /// 3. if still empty and site on country TLD -> retry run_js + location
    pub async fn scrape_with_retry(&self, url: &str) -> Result<ScrapeResult, OxylabsError> {
        // attempt 1
        let first = self.scrape(url, "markdown", false, None).await;
        match first {
            Ok(r) if !is_skeletal(&r) => return Ok(r),
            Ok(_) => {}
            Err(OxylabsError::EmptyResult { .. }) => {}
            Err(e) => return Err(e),
        }
        // attempt 2: run_js
        let second = self.scrape(url, "markdown", true, None).await;
        match second {
            Ok(r) if !is_skeletal(&r) => return Ok(r),
            Ok(_) => {}
            Err(OxylabsError::EmptyResult { .. }) => {}
            Err(e) => return Err(e),
        }
        // attempt 3: country TLD
        if let Some(country) = country_from_tld(url) {
            let third = self.scrape(url, "markdown", true, Some(country)).await;
            match third {
                Ok(r) if !is_skeletal(&r) => return Ok(r),
                Ok(_) => {}
                Err(OxylabsError::EmptyResult { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Err(OxylabsError::Unreadable {
            url: url.to_string(),
        })
    }

    pub async fn research(
        &self,
        question: &str,
        read_top: usize,
        location: Option<&str>,
    ) -> Result<Vec<(String, String, Option<String>)>, OxylabsError> {
        let hits = self.search(question, 10, location).await?;
        let mut pages = Vec::new();
        for hit in hits.into_iter().take(read_top) {
            match self.scrape_with_retry(&hit.url).await {
                Ok(p) => pages.push((hit.url, hit.title, p.markdown)),
                Err(OxylabsError::Authentication) => return Err(OxylabsError::Authentication),
                Err(e) => {
                    warn!("scrape failed for {}: {}", hit.url, e);
                    pages.push((hit.url, hit.title, None));
                }
            }
        }
        Ok(pages)
    }
}

fn is_skeletal(r: &ScrapeResult) -> bool {
    match &r.markdown {
        None => true,
        Some(m) => m.trim().len() < 200,
    }
}

fn country_from_tld(url: &str) -> Option<&'static str> {
    // parse host
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if host.ends_with(".lt") {
        Some("LT")
    } else if host.ends_with(".es") {
        Some("ES")
    } else if host.ends_with(".co.uk") {
        Some("GB")
    } else {
        None
    }
}

// convenience re-export for tests
pub use OxylabsError as Error;
