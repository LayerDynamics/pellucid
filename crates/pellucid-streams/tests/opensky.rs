#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! OpenSky client integration tests (T3.2).
//!
//! Drives the production `OpenSkyClient` against a wiremock
//! server playing both the OAuth2 token endpoint and the data
//! API. The headline assertion: **100 concurrent fetches trigger
//! exactly one token refresh**.

use std::sync::Arc;
use std::time::Duration;

use pellucid_streams::opensky::OpenSkyConfig;
use pellucid_streams::{OpenSkyClient, StreamsError};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn token_body(expires_in: u64) -> serde_json::Value {
    serde_json::json!({
        "access_token": "tok-test-xyz",
        "token_type": "bearer",
        "expires_in": expires_in
    })
}

fn states_body() -> serde_json::Value {
    serde_json::json!({
        "time": 1_746_226_800,
        "states": [
            ["abc123", "AAL100  ", "United States", 1_746_226_790,
             1_746_226_795, -73.7781, 40.6413, 100.0, false,
             125.0, 245.0, 0.0, null, 110.0, "1234", false, 0]
        ]
    })
}

async fn wired_client(server: &MockServer) -> OpenSkyClient {
    let config = OpenSkyConfig {
        token_url: format!("{}/token", server.uri()),
        api_base: server.uri(),
        client_id: "ci".into(),
        client_secret: "cs".into(),
    };
    OpenSkyClient::new(config, reqwest::Client::new())
}

#[tokio::test]
async fn one_hundred_concurrent_fetches_refresh_token_exactly_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        // The contract: a thundering herd of 100 fetches MUST
        // refresh the token exactly once. mock asserts on drop.
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/states/all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(states_body()))
        .mount(&server)
        .await;

    let client = Arc::new(wired_client(&server).await);

    let mut joins = Vec::with_capacity(100);
    for _ in 0..100 {
        let c = client.clone();
        joins.push(tokio::spawn(async move {
            c.fetch_path("/states/all").await
        }));
    }
    for j in joins {
        let result = j.await.unwrap();
        assert!(result.is_ok(), "all 100 fetches must succeed: {result:?}");
    }
}

#[tokio::test]
async fn fetch_caches_positive_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/states/all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(states_body()))
        // Exactly-once: second fetch hits the LRU cache.
        .expect(1)
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let _ = client.fetch_path("/states/all").await.unwrap();
    let _ = client.fetch_path("/states/all").await.unwrap();
}

#[tokio::test]
async fn empty_states_response_records_negative_sentinel() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/states/all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "time": 1_746_226_800,
            "states": null
        })))
        // Exactly-once: the second fetch should hit the negative
        // sentinel and return Ok(None) without touching upstream.
        .expect(1)
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let r1 = client.fetch_path("/states/all").await.unwrap();
    assert!(r1.is_none(), "empty states must surface as Ok(None)");
    let r2 = client.fetch_path("/states/all").await.unwrap();
    assert!(r2.is_none(), "second call must hit negative sentinel");
}

#[tokio::test]
async fn upstream_429_triggers_cooldown() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rate-limited"))
        .respond_with(ResponseTemplate::new(429))
        // First call hits upstream; second call should NOT (cooldown).
        .expect(1)
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let r1 = client.fetch_path("/rate-limited").await.unwrap();
    assert!(r1.is_none(), "429 surfaces as Ok(None)");
    assert!(client.is_cooling_down(), "429 must engage cooldown");
    // Second call must short-circuit on cooldown — wiremock would
    // see another GET if the gate is broken.
    let r2 = client.fetch_path("/rate-limited").await.unwrap();
    assert!(r2.is_none(), "cooldown short-circuit returns None");
}

#[tokio::test]
async fn upstream_404_records_negative_sentinel() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let r1 = client.fetch_path("/missing").await.unwrap();
    assert!(r1.is_none());
    let r2 = client.fetch_path("/missing").await.unwrap();
    assert!(r2.is_none(), "404 negative sentinel applies");
}

#[tokio::test]
async fn upstream_5xx_propagates_as_status_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(3600)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/broken"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let err = client.fetch_path("/broken").await.unwrap_err();
    assert!(matches!(err, StreamsError::Status { status: 503 }));
    // 5xx does NOT engage the cooldown — that's reserved for 429.
    assert!(!client.is_cooling_down());
}

#[tokio::test]
async fn token_refresh_failure_propagates() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let client = wired_client(&server).await;
    let err = client.fetch_path("/states/all").await.unwrap_err();
    assert!(matches!(err, StreamsError::Status { status: 401 }), "got {err:?}");
}

#[tokio::test]
async fn short_lived_token_re_refreshes() {
    // expires_in=2 → buffer becomes 1s (half lifetime), refresh_at
    // = now + 1s. After sleeping 2s the second call refreshes.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body(2)))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(states_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/p2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(states_body()))
        .mount(&server)
        .await;

    let client = wired_client(&server).await;
    let _ = client.fetch_path("/p1").await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _ = client.fetch_path("/p2").await.unwrap();
    // expect(2) on the token mock verifies the refresh.
}
