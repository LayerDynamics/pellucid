#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Bootstrap handler integration test (T2.7 / M4 fix).
//!
//! Wires:
//! - `pellucid_db` SQLite pool (in-memory, migrations applied)
//! - `pellucid_cache` envelope writes via `set_cached_json`
//! - `pellucid_handlers::bootstrap::v1::get` handler
//! - `pellucid_gateway::build_router` 14-stage middleware
//!
//! Validates the M4 invariant end-to-end: the handler returns
//! 503 + `Retry-After` only when the cache holds **no** signal at
//! all for the requested keys. Partial hits → 200 with `missing[]`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use pellucid_cache::set_cached_json;
use pellucid_core::Envelope;
use pellucid_gateway::{build_router, GatewayConfig};
use pellucid_handlers::bootstrap::keys::{FAST_KEYS, SLOW_KEYS, TOTAL_KEYS};
use pellucid_handlers::bootstrap::v1::BOOTSTRAP_PATH;
use pellucid_handlers::{build_handlers, AppState};

const TTL_MS: i64 = 60_000;

async fn wired_state() -> AppState {
    AppState::for_tests_async()
        .await
        .expect("in-memory db opens")
}

fn full_pipeline_router(state: AppState) -> axum::Router {
    build_router(build_handlers(state), GatewayConfig::permissive_for_tests())
}

async fn populate_first_n_fast_keys(state: &AppState, n: usize) {
    for key in FAST_KEYS.iter().take(n) {
        let envelope = Envelope::new(serde_json::json!({ "k": key }));
        set_cached_json(&state.pool, key, &envelope, TTL_MS)
            .await
            .expect("populate cache");
    }
}

fn fast_request() -> Request<Body> {
    Request::builder()
        .uri(format!("{BOOTSTRAP_PATH}?tier=fast"))
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap()
}

fn both_request() -> Request<Body> {
    Request::builder()
        .uri(format!("{BOOTSTRAP_PATH}?tier=both"))
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn populate_50_request_full_fast_tier_returns_partial_hydration() {
    // Mirrors the plan's spec almost verbatim — the test the M4 fix
    // is sized for. Requesting the FAST tier when only 50 keys are
    // populated must surface partial hydration with `missing[]`
    // populated, NOT trigger the 503 outage path.
    let state = wired_state().await;
    populate_first_n_fast_keys(&state, 50).await;
    let app = full_pipeline_router(state);

    let resp = app.oneshot(fast_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();

    let data = parsed
        .get("data")
        .and_then(Value::as_object)
        .expect("data is an object");
    assert_eq!(data.len(), 50, "expected 50 cache hits, got {}", data.len());

    let missing = parsed
        .get("missing")
        .and_then(Value::as_array)
        .expect("missing is an array");
    let expected_missing = FAST_KEYS.len() - 50;
    assert_eq!(
        missing.len(),
        expected_missing,
        "expected {expected_missing} missing, got {}",
        missing.len()
    );

    // Negative sentinels should be empty (we wrote no negatives).
    let negative = parsed
        .get("negative")
        .and_then(Value::as_array)
        .expect("negative is an array");
    assert!(negative.is_empty());
}

#[tokio::test]
async fn empty_cache_full_fast_tier_returns_503_with_retry_after_m4_fix() {
    // The M4 invariant: requesting the FAST tier against an empty
    // cache surfaces the outage banner, not an empty 200 page.
    let state = wired_state().await;
    let app = full_pipeline_router(state);

    let resp = app.oneshot(fast_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    assert_eq!(
        resp.headers()
            .get("x-pellucid-error")
            .map(|v| v.to_str().unwrap()),
        Some("bootstrap_upstream_empty"),
    );

    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        parsed.pointer("/error/code").and_then(Value::as_str),
        Some("bootstrap_upstream_empty"),
    );
    assert_eq!(
        parsed
            .pointer("/error/retry_after_secs")
            .and_then(Value::as_u64),
        Some(30),
    );
    assert_eq!(
        parsed
            .pointer("/error/requested")
            .and_then(Value::as_u64),
        Some(FAST_KEYS.len() as u64),
    );
}

#[tokio::test]
async fn fully_populated_request_67_returns_200_no_missing() {
    let state = wired_state().await;
    populate_first_n_fast_keys(&state, FAST_KEYS.len()).await;
    let app = full_pipeline_router(state);

    let resp = app.oneshot(fast_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        parsed.pointer("/data").unwrap().as_object().unwrap().len(),
        FAST_KEYS.len()
    );
    assert!(
        parsed.pointer("/missing").unwrap().as_array().unwrap().is_empty()
    );
}

#[tokio::test]
async fn tier_both_yields_total_keys() {
    // Populate every fast key + every slow key, request `tier=both`.
    let state = wired_state().await;
    populate_first_n_fast_keys(&state, FAST_KEYS.len()).await;
    for key in SLOW_KEYS {
        let env = Envelope::new(serde_json::json!({ "k": key }));
        set_cached_json(&state.pool, key, &env, TTL_MS).await.unwrap();
    }
    let app = full_pipeline_router(state);

    let resp = app.oneshot(both_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        parsed.pointer("/data").unwrap().as_object().unwrap().len(),
        TOTAL_KEYS,
        "tier=both must hydrate every FAST + SLOW key"
    );
}

#[tokio::test]
async fn keys_override_takes_precedence_over_tier() {
    let state = wired_state().await;
    let env = Envelope::new(serde_json::json!({ "x": 1 }));
    set_cached_json(&state.pool, "explicit:key:v1", &env, TTL_MS)
        .await
        .unwrap();
    let app = full_pipeline_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "{BOOTSTRAP_PATH}?tier=fast&keys=explicit:key:v1"
                ))
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    let data = parsed.pointer("/data").unwrap().as_object().unwrap();
    assert_eq!(data.len(), 1);
    assert!(data.contains_key("explicit:key:v1"));
}

#[tokio::test]
async fn invalid_tier_returns_400() {
    let state = wired_state().await;
    let app = full_pipeline_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("{BOOTSTRAP_PATH}?tier=BOGUS"))
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
        .await
        .unwrap();
    let parsed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        parsed.pointer("/error/code").and_then(Value::as_str),
        Some("invalid_request"),
    );
}
