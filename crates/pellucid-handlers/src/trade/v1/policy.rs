//! `GET /api/trade/v1/policy` — pure reader for the tariff alerts
//! slot (T4.5.10).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "trade:tariff-alerts:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TariffRow {
    pub authority: String,
    pub origin: String,
    pub destination: String,
    #[serde(rename = "hsCode", alias = "hs_code")]
    pub hs_code: String,
    pub product: String,
    #[serde(rename = "rateDeltaPp", alias = "rate_delta_pp")]
    pub rate_delta_pp: f64,
    pub effective: String,
    pub headline: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradePolicyResponse {
    pub rows: Vec<TariffRow>,
    #[serde(rename = "totalRateDeltaPp")]
    pub total_rate_delta_pp: f64,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<TariffRow>,
    #[serde(default, alias = "totalRateDeltaPp")]
    total_rate_delta_pp: f64,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<TradePolicyResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    Ok(Json(TradePolicyResponse {
        rows: snap.rows,
        total_rate_delta_pp: snap.total_rate_delta_pp,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::trade::v1::POLICY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new()
            .route(POLICY_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(POLICY_PATH)
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
            "rows": [{ "authority": "USTR", "origin": "CN", "destination": "US", "hs_code": "8542", "product": "Semiconductors", "rate_delta_pp": 25.0, "effective": "2026-04-30", "headline": "S301 raise" }],
            "total_rate_delta_pp": 25.0,
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(POLICY_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/totalRateDeltaPp").and_then(Value::as_f64), Some(25.0));
        assert_eq!(parsed.pointer("/rows/0/hsCode").and_then(Value::as_str), Some("8542"));
    }
}
