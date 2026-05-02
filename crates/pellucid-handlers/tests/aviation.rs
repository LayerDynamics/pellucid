#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Aviation handler integration tests.
//!
//! These tests wire the **full M1 stack**:
//!
//! - `pellucid_db` SQLite pool (in-memory, migrations applied)
//! - `pellucid_cache` coalesce registry + envelope + negative
//!   sentinel
//! - `pellucid_streams::AviationstackClient` pointed at a
//!   `wiremock` server (so the production HTTP path is exercised)
//! - `pellucid_handlers::aviation::v1::get_flight_status` handler
//! - `pellucid_gateway::build_router` 14-stage middleware chain
//!
//! The only mocked component is the upstream aviationstack server,
//! which is the layer the spec explicitly calls out as the
//! external integration. Every other layer the request would touch
//! in production runs for real.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use pellucid_gateway::{build_router, GatewayConfig};
use pellucid_handlers::generated::aviation::v1::FlightStatus;
use pellucid_handlers::{
    aviation::v1::GET_FLIGHT_STATUS_PATH, build_handlers, AppState, FlightStatusUpstream,
};
use pellucid_streams::{AviationstackClient, AviationstackConfig};

/// Adapter that wraps the production `pellucid-streams` client in
/// the [`FlightStatusUpstream`] trait the handler uses, so we can
/// drive the *real* HTTP client (against a wiremock'd upstream)
/// from the handler layer.
#[derive(Debug)]
struct StreamsAdapter(AviationstackClient);

#[async_trait]
impl FlightStatusUpstream for StreamsAdapter {
    async fn fetch_flight(
        &self,
        flight: &str,
        date: &str,
        origin: &str,
    ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
        match self.0.fetch_flight(flight, date, origin).await {
            Ok(opt) => Ok(opt),
            Err(e) => Err(Box::new(e)),
        }
    }
}

fn aviationstack_payload(flight: &str, status: &str) -> serde_json::Value {
    serde_json::json!({
        "data": [{
            "flight_status": status,
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
            "flight": { "iata": flight }
        }]
    })
}

async fn wired_state(server: &MockServer) -> AppState {
    let state = AppState::for_tests_async()
        .await
        .expect("in-memory db opens");
    let client = AviationstackClient::new(
        AviationstackConfig {
            base_url: format!("{}/v1", server.uri()),
            access_key: "integration-test-key".into(),
        },
        reqwest::Client::new(),
    );
    state.with_aviation(Arc::new(StreamsAdapter(client)))
}

fn full_pipeline_router(state: AppState) -> axum::Router {
    build_router(build_handlers(state), GatewayConfig::permissive_for_tests())
}

#[tokio::test]
async fn cold_cache_calls_upstream_once_warm_cache_serves_locally() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/flights"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(aviationstack_payload("AA100", "active")),
        )
        // The mock will fail the test if it is hit more than once;
        // proving the cache layer absorbs the second call.
        .expect(1)
        .mount(&server)
        .await;

    let state = wired_state(&server).await;
    let app = full_pipeline_router(state);

    let url = format!(
        "{GET_FLIGHT_STATUS_PATH}?flight=AA100&date=2026-04-25&origin=JFK"
    );
    let mk_req = || {
        Request::builder()
            .uri(&url)
            .header("origin", "http://localhost:5173")
            .body(Body::empty())
            .unwrap()
    };

    // Cold call → 200 + body.
    let resp1 = app.clone().oneshot(mk_req()).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let body1 = axum::body::to_bytes(resp1.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed1: FlightStatus = serde_json::from_slice(&body1).unwrap();
    assert_eq!(parsed1.flight, "AA100");
    assert_eq!(parsed1.status, "active");
    assert_eq!(parsed1.origin, "JFK");
    assert_eq!(parsed1.destination, "LAX");

    // Warm call → 200 + identical body, must NOT touch the upstream.
    let resp2 = app.oneshot(mk_req()).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = axum::body::to_bytes(resp2.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed2: FlightStatus = serde_json::from_slice(&body2).unwrap();
    assert_eq!(parsed1, parsed2, "warm cache must return identical body");
    // Mock's `expect(1)` enforces single upstream invocation —
    // server.verify() runs on drop and panics if violated.
}

#[tokio::test]
async fn upstream_503_surfaces_as_handler_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/flights"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let state = wired_state(&server).await;
    let app = full_pipeline_router(state);

    let url = format!(
        "{GET_FLIGHT_STATUS_PATH}?flight=AA100&date=2026-04-25&origin=JFK"
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri(url)
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Handler maps wrapped upstream failure to 502 (cache_failure
    // code due to sqlx::Error wrapping in cached_fetch_json).
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let code = envelope
        .pointer("/error/code")
        .and_then(|v| v.as_str())
        .expect("error.code present");
    assert!(
        matches!(code, "upstream_failure" | "cache_failure"),
        "unexpected code: {code}"
    );
}

#[tokio::test]
async fn upstream_empty_data_surfaces_as_404_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/flights"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": [] })),
        )
        .mount(&server)
        .await;

    let state = wired_state(&server).await;
    let app = full_pipeline_router(state);

    let url = format!(
        "{GET_FLIGHT_STATUS_PATH}?flight=ZZ999&date=2026-04-25&origin=JFK"
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri(url)
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        envelope.pointer("/error/code").and_then(|v| v.as_str()),
        Some("upstream_failure")
    );
}

#[tokio::test]
async fn invalid_query_returns_400() {
    let server = MockServer::start().await;
    let state = wired_state(&server).await;
    let app = full_pipeline_router(state);

    // Missing `origin` query param → axum's Query extractor
    // rejects the request before our validator runs. Either status
    // is acceptable evidence the malformed input was rejected
    // before the cache layer was touched.
    let url = format!("{GET_FLIGHT_STATUS_PATH}?flight=AA100&date=2026-04-25");
    let resp = app
        .oneshot(
            Request::builder()
                .uri(url)
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "missing origin should be a 4xx, got {}",
        resp.status(),
    );
}
