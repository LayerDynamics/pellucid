//! `GET /api/economic/v1/national-debt` — pure reader.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "economic:national-debt:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DebtObservation {
    pub period: String,
    #[serde(rename = "totalBillionUsd", alias = "total_billion_usd")]
    pub total_billion_usd: f64,
    #[serde(rename = "debtToGdpPct", alias = "debt_to_gdp_pct")]
    pub debt_to_gdp_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NationalDebtResponse {
    pub latest: DebtObservation,
    #[serde(rename = "qoqDeltaBillionUsd")]
    pub qoq_delta_billion_usd: f64,
    pub history: Vec<DebtObservation>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    latest: DebtObservation,
    #[serde(default, alias = "qoqDeltaBillionUsd")]
    qoq_delta_billion_usd: f64,
    history: Vec<DebtObservation>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<NationalDebtResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    Ok(Json(NationalDebtResponse {
        latest: snap.latest,
        qoq_delta_billion_usd: snap.qoq_delta_billion_usd,
        history: snap.history,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::NATIONAL_DEBT_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            NATIONAL_DEBT_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_cache_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(NATIONAL_DEBT_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase_payload() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "latest": { "period": "2026-Q1", "total_billion_usd": 35400.0, "debt_to_gdp_pct": 122.5 },
            "qoq_delta_billion_usd": 400.0_f64,
            "history": [
                { "period": "2025-Q4", "total_billion_usd": 35000.0, "debt_to_gdp_pct": 121.0 },
                { "period": "2026-Q1", "total_billion_usd": 35400.0, "debt_to_gdp_pct": 122.5 },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(NATIONAL_DEBT_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/latest/totalBillionUsd").is_some());
        assert_eq!(
            parsed.pointer("/qoqDeltaBillionUsd").and_then(Value::as_f64),
            Some(400.0),
        );
    }
}
