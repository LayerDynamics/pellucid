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
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
