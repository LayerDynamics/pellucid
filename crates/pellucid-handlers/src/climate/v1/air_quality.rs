//! `GET /api/climate/v1/air-quality` — pure reader for OpenAQ AQ
//! snapshot.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "climate:air-quality:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AirQualityRow {
    pub city: String,
    pub country: String,
    pub aqi: u32,
    pub pollutant: String,
    pub value: f64,
    pub unit: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AirQualityResponse {
    pub rows: Vec<AirQualityRow>,
    #[serde(rename = "worstAqi")]
    pub worst_aqi: u32,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<AirQualityRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<AirQualityResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let worst_aqi = snap.rows.iter().map(|r| r.aqi).max().unwrap_or(0);
    Ok(Json(AirQualityResponse {
        rows: snap.rows,
        worst_aqi,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::climate::v1::AIR_QUALITY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            AIR_QUALITY_PATH,
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
                    .uri(AIR_QUALITY_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn computes_worst_aqi() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "city": "Delhi", "country": "IN", "aqi": 250, "pollutant": "pm25", "value": 145.0, "unit": "µg/m³" },
                { "city": "LA", "country": "US", "aqi": 90, "pollutant": "o3", "value": 0.075, "unit": "ppm" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(AIR_QUALITY_PATH)
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
            parsed.pointer("/worstAqi").and_then(Value::as_u64),
            Some(250)
        );
    }
}
