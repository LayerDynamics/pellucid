//! `GET /api/geo/v1/ucdp-events` — pure reader for UCDP GED feed.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "conflict:events-24h:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UcdpEventRow {
    pub id: String,
    pub country: String,
    #[serde(rename = "actor1")]
    pub actor1: String,
    #[serde(rename = "actor2")]
    pub actor2: String,
    pub fatalities: u32,
    pub lat: f64,
    pub lon: f64,
    #[serde(rename = "occurredAt", alias = "occurred_at")]
    pub occurred_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UcdpEventsResponse {
    pub rows: Vec<UcdpEventRow>,
    #[serde(rename = "totalFatalities")]
    pub total_fatalities: u32,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<UcdpEventRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<UcdpEventsResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let total_fatalities: u32 = snap.rows.iter().map(|r| r.fatalities).sum();
    Ok(Json(UcdpEventsResponse {
        rows: snap.rows,
        total_fatalities,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geo::v1::UCDP_EVENTS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(UCDP_EVENTS_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app.oneshot(Request::builder().uri(UCDP_EVENTS_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn sums_fatalities() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "id": "u1", "country": "Ukraine", "actor1": "GovOfRus", "actor2": "GovOfUkr", "fatalities": 12, "lat": 50.4, "lon": 30.5, "occurred_at": "2026-04-29T01:00:00Z" },
                { "id": "u2", "country": "Sudan",   "actor1": "RSF",      "actor2": "SAF",      "fatalities": 7,  "lat": 15.5, "lon": 32.5, "occurred_at": "2026-04-29T02:00:00Z" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000).await.unwrap();
        let resp = app.oneshot(Request::builder().uri(UCDP_EVENTS_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/totalFatalities").and_then(Value::as_u64), Some(19));
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(2));
    }
}
