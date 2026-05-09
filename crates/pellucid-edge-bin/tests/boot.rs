#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Edge-bin boot integration tests (T2.6).
//!
//! Builds the **real** edge router via `pellucid_edge_bin::build_app`,
//! binds it to a random local port, and curls real HTTP endpoints.
//! The aviationstack upstream is wiremock'd — the only external
//! integration the edge bin has.

use std::time::Duration;

use pellucid_edge_bin::{build_app, Config, ConfigSource, HealthcheckBody, HEALTHCHECK_PATH};
use serde_json::Value;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Materialise a minimal SPA bundle (just `index.html`) into a
/// fresh temp dir so the host-routing tests can stand up an
/// edge instance with `webview_dist` pointing at a real path
/// without requiring `bun run build` to have been run first.
fn make_spa_dir() -> (TempDir, String) {
    let dir = tempfile::tempdir().expect("create temp spa dir");
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><body data-test='spa-shell'>pellucid</body></html>",
    )
    .expect("write index.html");
    let path = dir.path().to_string_lossy().to_string();
    (dir, path)
}

async fn spawn_edge(cfg: Config) -> String {
    let (router, _pool) = build_app(&cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    // Tiny settle so the server is ready before the test client
    // fires its first request. 50 ms is enough for axum on
    // localhost; if the test ever flakes here we bump it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://127.0.0.1:{port}")
}

fn aviation_response_body() -> Value {
    serde_json::json!({
        "data": [{
            "flight_status": "active",
            "departure": {
                "iata": "JFK",
                "scheduled": "2026-04-25T12:00:00Z",
                "gate": "A12"
            },
            "arrival": {
                "iata": "LAX",
                "scheduled": "2026-04-25T15:00:00Z",
                "gate": null
            },
            "flight": { "iata": "AA100" }
        }]
    })
}

#[tokio::test]
async fn healthz_returns_200_with_ok_body() {
    let cfg = Config::parse(&ConfigSource::default()).unwrap();
    let base = spawn_edge(cfg).await;
    let resp = reqwest::get(format!("{base}{HEALTHCHECK_PATH}"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: HealthcheckBody = resp.json().await.unwrap();
    assert!(body.ok);
    assert!(!body.version.is_empty());
}

#[tokio::test]
async fn aviation_v1_get_flight_status_returns_envelope_via_real_pipeline() {
    // Stand up a wiremock'd aviationstack so the request flow
    // hits a real upstream the edge bin actually fetches from.
    let aviationstack = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/flights"))
        .respond_with(ResponseTemplate::new(200).set_body_json(aviation_response_body()))
        .mount(&aviationstack)
        .await;

    let src = ConfigSource {
        aviationstack_base_url: Some(format!("{}/v1", aviationstack.uri())),
        aviationstack_api_key: Some("test-key".into()),
        ..ConfigSource::default()
    };
    let cfg = Config::parse(&src).unwrap();
    let base = spawn_edge(cfg).await;

    let url =
        format!("{base}/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK");
    let resp = reqwest::Client::new()
        .get(&url)
        .header("origin", "http://localhost:5173")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["flight"], "AA100");
    assert_eq!(body["origin"], "JFK");
    assert_eq!(body["destination"], "LAX");
    assert_eq!(body["status"], "active");
}

#[tokio::test]
async fn unknown_route_returns_404_via_gateway() {
    // The 14-stage gateway includes a route-fallback stage; an
    // unknown path should surface a deterministic 404 instead of
    // a tower default.
    let cfg = Config::parse(&ConfigSource::default()).unwrap();
    let base = spawn_edge(cfg).await;
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/does/not/exist"))
        .header("origin", "http://localhost:5173")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn spa_falls_back_to_index_html_on_apex_host_when_dist_configured() {
    // SPEC-001 §17: the apex hostname (and the four variant
    // subdomains) gets the SPA bundle for any non-API path so
    // React Router can take over client-side. With
    // `api_host_prefix` UNSET, every host receives the SPA — the
    // single-domain default this test pins.
    //
    // NOTE on status code: `tower_http::services::ServeDir::
    // not_found_service(ServeFile)` returns the SPA `index.html`
    // body with HTTP `404`, not `200`. Browsers ignore the status
    // and render the body, so React Router still boots; we assert
    // on body content rather than status so a future tower-http
    // change to default-200 wouldn't false-fail this regression.
    let (_dir, spa_path) = make_spa_dir();
    let src = ConfigSource {
        webview_dist: Some(spa_path),
        ..ConfigSource::default()
    };
    let cfg = Config::parse(&src).unwrap();
    let base = spawn_edge(cfg).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/some/spa/route"))
        .header("host", "pellucid.world")
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("data-test='spa-shell'"),
        "expected SPA shell, got: {body}"
    );
}

#[tokio::test]
async fn api_host_prefix_returns_plain_404_for_non_api_paths() {
    // SPEC-001 §17: when `api_host_prefix = "api."`, a request
    // arriving with `Host: api.<anything>` gets a plain 404 for
    // any non-API path instead of the SPA index. Pins the
    // `api.worldmonitor.app` rule so the api hostname is API-only
    // (no SPA leakage). Both branches emit `404`, but the api host
    // branch returns the literal "not found" body emitted by
    // `HostAwareSpaFallback`, NOT the SPA shell — that's the
    // observable difference we assert on.
    let (_dir, spa_path) = make_spa_dir();
    let src = ConfigSource {
        webview_dist: Some(spa_path),
        api_host_prefix: Some("api.".into()),
        ..ConfigSource::default()
    };
    let cfg = Config::parse(&src).unwrap();
    let base = spawn_edge(cfg).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/"))
        .header("host", "api.pellucid.world")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "api host root should be 404");
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("data-test='spa-shell'"),
        "api host should NOT return SPA body, got: {body}"
    );
}

#[tokio::test]
async fn api_host_prefix_still_serves_healthz() {
    // The host filter only gates the SPA fallback; gateway routes
    // (including `/healthz`) match under any hostname. The
    // operational health endpoint must answer regardless of which
    // host header the upstream load-balancer attaches — otherwise
    // Railway's healthchecker would fail when the service is
    // resolved by its api-prefixed name.
    let (_dir, spa_path) = make_spa_dir();
    let src = ConfigSource {
        webview_dist: Some(spa_path),
        api_host_prefix: Some("api.".into()),
        ..ConfigSource::default()
    };
    let cfg = Config::parse(&src).unwrap();
    let base = spawn_edge(cfg).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}{HEALTHCHECK_PATH}"))
        .header("host", "api.pellucid.world")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: HealthcheckBody = resp.json().await.unwrap();
    assert!(body.ok);
}

#[tokio::test]
async fn api_host_prefix_apex_still_gets_spa() {
    // Same config as `api_host_prefix_returns_plain_404...`, but
    // the request arrives WITHOUT the `api.` prefix. The SPA
    // fallback should still serve so the apex/variant hostnames
    // keep their existing UX. As above, status is `404` from
    // `not_found_service` — we assert by body content.
    let (_dir, spa_path) = make_spa_dir();
    let src = ConfigSource {
        webview_dist: Some(spa_path),
        api_host_prefix: Some("api.".into()),
        ..ConfigSource::default()
    };
    let cfg = Config::parse(&src).unwrap();
    let base = spawn_edge(cfg).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/dashboard"))
        .header("host", "pellucid.world")
        .send()
        .await
        .unwrap();
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("data-test='spa-shell'"),
        "apex host should still get SPA shell, got: {body}"
    );
}

#[tokio::test]
async fn malformed_aviation_query_returns_400_envelope() {
    let cfg = Config::parse(&ConfigSource::default()).unwrap();
    let base = spawn_edge(cfg).await;
    // Missing `origin` → axum's Query extractor rejects.
    let url = format!("{base}/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25");
    let resp = reqwest::Client::new()
        .get(&url)
        .header("origin", "http://localhost:5173")
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_client_error(), "got {}", resp.status());
}
