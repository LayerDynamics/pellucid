#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! OREF client integration tests (T3.4).
//!
//! Drives the production `OrefClient` against a wiremock server.
//! Verifies:
//! - Chrome HTTP-shape headers reach the upstream (User-Agent,
//!   Accept-Language, x-requested-with, sec-fetch-* — wiremock
//!   asserts on each).
//! - Empty body → `Ok(None)` (no active alerts).
//! - Object body → `Ok(Some(vec![alert]))`.
//! - Array body → `Ok(Some(vec![...]))`.
//! - 5xx surfaces as `Status` error (when no proxy).
//! - With a proxy configured + direct returns 5xx, the proxy
//!   path is consulted on retry.
//! - History persists to `kv_envelope` under `relay:oref:history:v1`
//!   and round-trips through `read_history`.

use pellucid_db::open_in_memory;
use pellucid_streams::oref::{read_history, OrefClient, OrefConfig, HISTORY_CACHE_KEY};
use pellucid_streams::StreamsError;
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ALERT_OBJECT: &str =
    r#"{"id":"100","cat":"1","title":"רקטות","data":"שדרות","desc":"היכנסו למרחב מוגן"}"#;
const ALERT_ARRAY: &str = r#"[
    {"id":"200","cat":"1","title":"a","data":"x","desc":""},
    {"id":"201","cat":"13","title":"b","data":"y","desc":""}
]"#;

async fn client_for(server: &MockServer) -> OrefClient {
    let cfg = OrefConfig {
        url: format!("{}/alerts.json", server.uri()),
        proxy_url: None,
    };
    OrefClient::new(cfg).expect("client builds")
}

#[tokio::test]
async fn fetch_alerts_empty_body_returns_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let result = client.fetch_alerts().await.unwrap();
    assert!(result.is_none(), "empty body must surface as Ok(None)");
}

#[tokio::test]
async fn fetch_alerts_single_object_body_returns_singleton() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_OBJECT))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let alerts = client.fetch_alerts().await.unwrap().unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].id, "100");
    assert!(alerts[0].title.contains("רקטות"));
}

#[tokio::test]
async fn fetch_alerts_array_body_returns_full_vec() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_ARRAY))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let alerts = client.fetch_alerts().await.unwrap().unwrap();
    assert_eq!(alerts.len(), 2);
    assert_eq!(alerts[0].id, "200");
    assert_eq!(alerts[1].id, "201");
}

#[tokio::test]
async fn fetch_alerts_5xx_propagates_as_status_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let err = client.fetch_alerts().await.unwrap_err();
    assert!(matches!(err, StreamsError::Status { status: 503 }));
}

#[tokio::test]
async fn fetch_sends_chrome_http_shape_headers() {
    // Pin the load-bearing identity headers — `User-Agent` is
    // what most fingerprint-based bot filters key on, and
    // `Accept-Language` betrays a non-browser caller. Other
    // Chrome-shape headers (referer, sec-fetch-*, x-requested-
    // with) are exercised by the broader `fetch_alerts_*` tests
    // which also pass; if any of those were missing we would
    // see 404s there too.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .and(header_exists("user-agent"))
        .and(header_exists("accept-language"))
        .and(header_exists("referer"))
        .and(header_exists("x-requested-with"))
        .and(header_exists("sec-fetch-site"))
        .and(header_exists("sec-fetch-mode"))
        .and(header_exists("sec-fetch-dest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_OBJECT))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let _ = client.fetch_alerts().await.unwrap();
}

#[tokio::test]
async fn user_agent_string_contains_chrome_token() {
    // Stricter check: the UA the upstream sees MUST identify
    // as Chrome — `oref.org.il`'s filter rejects anything else.
    // Done via a dedicated test so we can assert on the value
    // (rather than just existence).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .and(wiremock::matchers::header_regex(
            "user-agent",
            r"Chrome/\d+\.0\.0\.0 Safari/537\.36",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_OBJECT))
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let _ = client.fetch_alerts().await.unwrap();
}

#[tokio::test]
async fn proxy_fallback_engages_when_direct_5xx() {
    // Direct server: 5xx. Proxy server: 200 + alert. The OREF
    // client should retry through the proxy and surface the
    // alert.
    let direct = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&direct)
        .await;

    let proxy = MockServer::start().await;
    // The proxy gets the FULL absolute URL as the request path
    // (because reqwest sends `GET http://direct/alerts.json` to
    // the proxy). wiremock's matchers operate on the path as
    // requested; since the proxy is HTTP-only, reqwest forwards
    // with absolute-form. We assert any GET hits.
    Mock::given(method("GET"))
        .and(header_exists("user-agent"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_OBJECT))
        .expect(1)
        .mount(&proxy)
        .await;

    let cfg = OrefConfig {
        url: format!("{}/alerts.json", direct.uri()),
        proxy_url: Some(proxy.uri()),
    };
    let client = OrefClient::new(cfg).unwrap();
    let alerts = client.fetch_alerts().await.unwrap().unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].id, "100");
}

#[tokio::test]
async fn proxy_path_is_skipped_when_direct_succeeds() {
    let direct = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_OBJECT))
        .mount(&direct)
        .await;

    // Set up a proxy server that asserts NO requests reach it.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("BAD"))
        .expect(0)
        .mount(&proxy)
        .await;

    let cfg = OrefConfig {
        url: format!("{}/alerts.json", direct.uri()),
        proxy_url: Some(proxy.uri()),
    };
    let client = OrefClient::new(cfg).unwrap();
    let alerts = client.fetch_alerts().await.unwrap().unwrap();
    assert_eq!(alerts[0].id, "100");
}

#[tokio::test]
async fn refresh_history_persists_and_reads_back() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ALERT_ARRAY))
        .mount(&server)
        .await;
    let pool = open_in_memory().await.unwrap();
    let client = client_for(&server).await;
    let history = client.refresh_history(&pool).await.unwrap();
    assert_eq!(history.alerts.len(), 2);
    assert!(history.updated_at_ms > 0);

    // Read back via the public reader.
    let read = read_history(&pool).await.unwrap();
    assert_eq!(read.alerts.len(), 2);
    assert_eq!(read.alerts[0].id, "200");
    assert_eq!(read.updated_at_ms, history.updated_at_ms);
}

#[tokio::test]
async fn refresh_history_dedupes_across_polls() {
    let server = MockServer::start().await;
    // First poll returns alert id 200 + 201; second poll returns
    // 201 + 202. After two polls we expect 202, 201, 200 — newest
    // first, dedup'd.
    let first_body = r#"[
        {"id":"200","cat":"1","title":"a","data":"x","desc":""},
        {"id":"201","cat":"1","title":"b","data":"y","desc":""}
    ]"#;
    let second_body = r#"[
        {"id":"202","cat":"1","title":"c","data":"z","desc":""},
        {"id":"201","cat":"1","title":"b-newer","data":"y","desc":""}
    ]"#;
    let pool = open_in_memory().await.unwrap();

    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(first_body))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let client = client_for(&server).await;
    let _ = client.refresh_history(&pool).await.unwrap();

    Mock::given(method("GET"))
        .and(path("/alerts.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(second_body))
        .mount(&server)
        .await;
    let merged = client.refresh_history(&pool).await.unwrap();

    let ids: Vec<&str> = merged.alerts.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["202", "201", "200"]);
    // Dedup picks the NEW version of "201".
    let two_oh_one = merged.alerts.iter().find(|a| a.id == "201").unwrap();
    assert_eq!(two_oh_one.title, "b-newer");
}

#[tokio::test]
async fn read_history_on_empty_db_returns_default() {
    let pool = open_in_memory().await.unwrap();
    let history = read_history(&pool).await.unwrap();
    assert!(history.alerts.is_empty());
    assert_eq!(history.updated_at_ms, 0);
}

#[test]
fn history_cache_key_matches_spec() {
    // SPEC-001 §17.6 pins `relay:oref:history:v1`.
    assert_eq!(HISTORY_CACHE_KEY, "relay:oref:history:v1");
}
