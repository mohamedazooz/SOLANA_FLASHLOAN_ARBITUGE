use arb_ingest::oxylabs::{HttpTransport, OxylabsClient, OxylabsConfig, OxylabsError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct MockTransport {
    responses: Arc<Mutex<Vec<(u16, String)>>>,
}

impl MockTransport {
    fn new(responses: Vec<(u16, String)>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
        }
    }
}

#[async_trait]
impl HttpTransport for MockTransport {
    async fn post(
        &self,
        _url: &str,
        _bearer: &str,
        _body: &Value,
        _timeout: Duration,
    ) -> Result<(u16, String), OxylabsError> {
        let mut r = self.responses.lock().unwrap();
        if r.is_empty() {
            return Ok((
                200,
                r#"{"results":[],"metadata":{"request_id":"rid-final"}}"#.to_string(),
            ));
        }
        Ok(r.remove(0))
    }
}

fn cfg() -> OxylabsConfig {
    OxylabsConfig {
        api_key: "dummy-key-never-real".into(),
        base_url: "https://example.test".into(),
        timeout: Duration::from_secs(1),
    }
}

#[tokio::test]
async fn search_maps_401_to_authentication() {
    let t = MockTransport::new(vec![(401, r#"{"title":"UNAUTHORIZED"}"#.to_string())]);
    let c = OxylabsClient::with_transport(cfg(), t);
    let err = c.search("q", 5, None).await.unwrap_err();
    assert!(matches!(err, OxylabsError::Authentication));
}

#[tokio::test]
async fn search_maps_400_to_validation_with_detail() {
    let body = json!({"title":"VALIDATION_ERROR","errors":[{"pointer":"#/max_results","detail":"too big"}]}).to_string();
    let t = MockTransport::new(vec![(400, body)]);
    let c = OxylabsClient::with_transport(cfg(), t);
    let err = c.search("q", 5, None).await.unwrap_err();
    match err {
        OxylabsError::Validation { detail, .. } => assert_eq!(detail, "too big"),
        _ => panic!("expected Validation"),
    }
}

#[tokio::test]
async fn scrape_retries_empty_with_run_js_then_country() {
    let empty = json!({"results":[{"markdown":""}],"metadata":{"request_id":"r1"}}).to_string();
    let skeletal =
        json!({"results":[{"markdown":"hi"}],"metadata":{"request_id":"r2"}}).to_string();
    let longtext = "x".repeat(250);
    let ok = json!({"results":[{"markdown": longtext}],"metadata":{"request_id":"r3"}}).to_string();
    let t = MockTransport::new(vec![(200, empty), (200, skeletal), (200, ok)]);
    let c = OxylabsClient::with_transport(cfg(), t);
    let r = c
        .scrape_with_retry("https://example.lt/page")
        .await
        .unwrap();
    assert!(r.markdown.unwrap().len() > 200);
}

#[tokio::test]
async fn scrape_reports_unreadable_after_all_fail() {
    let empty = json!({"results":[{"markdown":""}],"metadata":{"request_id":"r"}}).to_string();
    let t = MockTransport::new(vec![
        (200, empty.clone()),
        (200, empty.clone()),
        (200, empty),
    ]);
    let c = OxylabsClient::with_transport(cfg(), t);
    let err = c
        .scrape_with_retry("https://example.lt/page")
        .await
        .unwrap_err();
    assert!(matches!(err, OxylabsError::Unreadable { .. }));
}

#[tokio::test]
async fn retryable_5xx_retried() {
    let ok = json!({"results":[],"metadata":{"request_id":"r"}}).to_string();
    let t = MockTransport::new(vec![(500, "boom".to_string()), (200, ok)]);
    let c = OxylabsClient::with_transport(cfg(), t);
    let v = c.search("q", 5, None).await.unwrap();
    assert!(v.is_empty());
}
