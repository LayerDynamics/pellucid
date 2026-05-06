//! `GET /api/economic/v1/fao-food-price-index` — pure reader.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "economic:fao-food-price-index:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FaoObservation {
    pub period: String,
    pub composite: f64,
    pub subindices: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaoResponse {
    pub latest: FaoObservation,
    pub history: Vec<FaoObservation>,
    #[serde(rename = "yoyPct", alias = "yoy_pct")]
    pub yoy_pct: f64,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    latest: FaoObservation,
    history: Vec<FaoObservation>,
    #[serde(default, alias = "yoyPct")]
    yoy_pct: f64,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<FaoResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    Ok(Json(FaoResponse {
        latest: snap.latest,
        history: snap.history,
        yoy_pct: snap.yoy_pct,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::FAO_FOOD_PRICE_INDEX_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            FAO_FOOD_PRICE_INDEX_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(FAO_FOOD_PRICE_INDEX_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_payload() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "latest": { "period": "2026-04", "composite": 120.0,
                        "subindices": [["meat", 110.0_f64], ["dairy", 105.0_f64]] },
            "history": [
                { "period": "2025-04", "composite": 100.0,
                  "subindices": [["meat", 95.0_f64], ["dairy", 90.0_f64]] },
                { "period": "2026-04", "composite": 120.0,
                  "subindices": [["meat", 110.0_f64], ["dairy", 105.0_f64]] },
            ],
            "yoy_pct": 20.0_f64,
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(FAO_FOOD_PRICE_INDEX_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: FaoResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.latest.period, "2026-04");
        assert!((parsed.yoy_pct - 20.0).abs() < 1e-9);
    }
}
