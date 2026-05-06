//! `GET /api/economic/v1/fuel-prices` — pure reader of the
//! `energy:fuel-prices:current:v1` cache slot (already seeded
//! by the T3.8 energy domain).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "energy:fuel-prices:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FuelPriceRow {
    pub region: String,
    pub product: String,
    #[serde(rename = "usdPerGallon", alias = "usd_per_gallon")]
    pub usd_per_gallon: f64,
    #[serde(rename = "weekOverWeekChangePct", alias = "wow_change_pct", default)]
    pub week_over_week_change_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FuelPricesResponse {
    pub rows: Vec<FuelPriceRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    rows: Vec<FuelPriceRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<FuelPricesResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    Ok(Json(FuelPricesResponse {
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
    use crate::economic::v1::FUEL_PRICES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            FUEL_PRICES_PATH,
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
                    .uri(FUEL_PRICES_PATH)
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
                "region": "US National",
                "product": "Regular Gasoline",
                "usd_per_gallon": 3.45,
                "wow_change_pct": -0.5,
            }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FUEL_PRICES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/usdPerGallon").is_some());
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
    }
}
