#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! T3.11 — Bootstrap returns 30 hydrated keys end-to-end.
//!
//! Wires the **real** publish path used by every shipped seeder:
//!
//! ```text
//! pellucid_seeders::atomic_publish::atomic_publish
//!   → kv_envelope (staging row)
//!   → kv_envelope (canonical row) + seed_meta (per SPEC-001 §7.4)
//!   → cache layer reads canonical via get_cached_json_batch
//!   → bootstrap handler unwraps `_seed` envelope, returns inner data
//! ```
//!
//! The existing `bootstrap.rs` integration test exercises the
//! cache-read side via `set_cached_json` (a direct write).
//! T3.11's deliverable is to prove the **publish** side: that the
//! atomic_publish flow (lock → stage → promote → meta → release)
//! produces rows the bootstrap handler can read for at least 30
//! cache keys without per-seeder upstream calls.
//!
//! No live HTTP. We construct envelopes directly + call
//! atomic_publish — every shipped seeder's last step is exactly
//! this call, so this test isolates the publish→read contract
//! from per-seeder upstream variance.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use std::time::Duration;
use tower::ServiceExt;

use pellucid_core::now_ms;
use pellucid_gateway::{build_router, GatewayConfig};
use pellucid_handlers::bootstrap::keys::FAST_KEYS;
use pellucid_handlers::bootstrap::v1::BOOTSTRAP_PATH;
use pellucid_handlers::{build_handlers, AppState};
use pellucid_seeders::atomic_publish::atomic_publish;
use pellucid_seeders::envelope::{SeedEnvelope, SeedMeta};

const TTL: Duration = Duration::from_secs(60 * 60);

/// SPEC-001 §11 cache-key shape is `<domain>:<resource>:v1` —
/// the prefix before the first `:` is the seed-lock domain.
fn domain_for(cache_key: &str) -> &str {
    cache_key.split(':').next().unwrap_or("unknown")
}

/// Build a deterministic envelope for one cache key. The `data`
/// payload is intentionally small + key-tagged so the assertion
/// can prove "this envelope came back unchanged".
fn build_envelope(cache_key: &str) -> SeedEnvelope<Value> {
    SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: now_ms(),
            ttl_ms: TTL.as_millis() as i64,
            source_version: "t3-11-hydration-test-v1".into(),
            record_count: 1,
            cascade_group: Some(domain_for(cache_key).to_string()),
            run_id: String::new(), // atomic_publish stamps this.
        },
        data: serde_json::json!({
            "key": cache_key,
            "domain": domain_for(cache_key),
            "test": "T3.11 hydration",
        }),
    }
}

async fn wired_state() -> AppState {
    AppState::for_tests_async()
        .await
        .expect("in-memory db opens")
}

fn full_pipeline_router(state: AppState) -> axum::Router {
    build_router(build_handlers(state), GatewayConfig::permissive_for_tests())
}

fn fast_request() -> Request<Body> {
    Request::builder()
        .uri(format!("{BOOTSTRAP_PATH}?tier=fast"))
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn atomic_publish_30_keys_round_trip_through_bootstrap() {
    let state = wired_state().await;
    // Take the first 30 FAST_KEYS (we ship 78 — well over 30 —
    // so this exercises real domain diversity: aviation, climate,
    // conflict, consumer-prices, cyber, discord, displacement,
    // economic, eia, energy, forecast, etc.).
    let keys: Vec<&str> = FAST_KEYS.iter().take(30).copied().collect();
    assert_eq!(keys.len(), 30, "fast tier must hold ≥ 30 keys");

    for key in &keys {
        let envelope = build_envelope(key);
        let outcome = atomic_publish(&state.pool, domain_for(key), key, &envelope, TTL)
            .await
            .unwrap_or_else(|e| panic!("atomic_publish({key}): {e}"));
        // Every publish stamps a fresh run_id and writes ≥ one byte.
        assert!(!outcome.run_id.is_empty());
        assert!(outcome.bytes_written > 0);
    }

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
    assert_eq!(
        data.len(),
        30,
        "expected 30 hydrated keys after atomic_publish, got {}",
        data.len()
    );

    // Every published key must round-trip with the unwrapped `data`
    // payload (the `_seed` wrapper must be stripped by the handler).
    for key in &keys {
        let payload = data
            .get(*key)
            .unwrap_or_else(|| panic!("missing hydrated key {key}"));
        assert_eq!(
            payload.get("key").and_then(Value::as_str),
            Some(*key),
            "envelope unwrap returned wrong payload for {key}: {payload}",
        );
        assert_eq!(
            payload.get("test").and_then(Value::as_str),
            Some("T3.11 hydration"),
        );
        // The `_seed` block must NOT leak through to the webview.
        assert!(
            payload.get("_seed").is_none(),
            "_seed envelope leaked through bootstrap unwrap for {key}",
        );
    }

    // The remaining FAST keys must show up in `missing[]` — the
    // tier returns the full slice, partial hydration is reported.
    let missing = parsed
        .get("missing")
        .and_then(Value::as_array)
        .expect("missing is an array");
    let expected_missing = FAST_KEYS.len() - 30;
    assert_eq!(
        missing.len(),
        expected_missing,
        "expected {expected_missing} missing after hydrating 30, got {}",
        missing.len()
    );
}

#[tokio::test]
async fn atomic_publish_30_keys_writes_seed_meta() {
    // SPEC-001 §7.4 step 4: every successful publish writes a
    // `seed_meta` row. The relay's L3 health cascade reads from
    // this table — if a publish completes without seeding meta,
    // health regresses to "unknown" for that cascade group.
    let state = wired_state().await;
    let keys: Vec<&str> = FAST_KEYS.iter().take(30).copied().collect();
    for key in &keys {
        let env = build_envelope(key);
        atomic_publish(&state.pool, domain_for(key), key, &env, TTL)
            .await
            .expect("publish");
    }

    // Count seed_meta rows directly. The exact column set is owned
    // by pellucid-db migrations; we only assert row count == publishes.
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM seed_meta")
        .fetch_one(&state.pool)
        .await
        .expect("count seed_meta");
    assert_eq!(row.0, 30, "expected 30 seed_meta rows after 30 publishes");
}
