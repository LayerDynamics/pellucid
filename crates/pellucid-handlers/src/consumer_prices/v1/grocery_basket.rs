//! `GET /api/consumer-prices/v1/grocery-basket` — pure reader.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "consumer-prices:grocery-basket:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GroceryRow {
    pub iso: String,
    pub country: String,
    #[serde(rename = "basketUsd", alias = "basket_usd")]
    pub basket_usd: f64,
    #[serde(rename = "basketYoyPct", alias = "basket_yoy_pct")]
    pub basket_yoy_pct: f64,
    pub period: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroceryResponse {
    pub rows: Vec<GroceryRow>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    rows: Vec<GroceryRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<GroceryResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    Ok(Json(GroceryResponse {
        rows: snap.rows,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::consumer_prices::v1::GROCERY_BASKET_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            GROCERY_BASKET_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GROCERY_BASKET_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase_payload() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{
                "iso": "USA", "country": "United States",
                "basket_usd": 120.0_f64, "basket_yoy_pct": 3.2_f64,
                "period": "2026-04",
            }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(GROCERY_BASKET_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/basketUsd").is_some());
        assert!(parsed.pointer("/rows/0/basketYoyPct").is_some());
    }
}
