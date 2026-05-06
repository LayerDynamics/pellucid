//! `GET /api/infra/v1/internet-outages` — pure reader of the
//! infrastructure grid-stress slot (powers InternetOutagesPanel).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "infrastructure:grid-stress:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutageRow {
    pub provider: String,
    pub region: String,
    pub status: String,
    #[serde(rename = "startedAtMs", alias = "started_at_ms")]
    pub started_at_ms: i64,
    #[serde(rename = "affectedAsCount", alias = "affected_as_count")]
    pub affected_as_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutagesResponse {
    pub rows: Vec<OutageRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<OutageRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<OutagesResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    Ok(Json(OutagesResponse {
        rows: snap.rows,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::infra::v1::INTERNET_OUTAGES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            INTERNET_OUTAGES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_cache_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(INTERNET_OUTAGES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{ "provider": "Cloudflare", "region": "global", "status": "degraded", "started_at_ms": 1_700_000_000_000_i64, "affected_as_count": 12 }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(INTERNET_OUTAGES_PATH)
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
        assert_eq!(
            parsed
                .pointer("/rows/0/affectedAsCount")
                .and_then(Value::as_u64),
            Some(12)
        );
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
    }
}
