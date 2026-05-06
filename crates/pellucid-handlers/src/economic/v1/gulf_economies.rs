//! `GET /api/economic/v1/gulf-economies` — pure reader.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "economic:gulf-economies:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GulfCountryRow {
    pub iso: String,
    pub country: String,
    #[serde(rename = "gdpUsdBillion", alias = "gdp_usd_billion")]
    pub gdp_usd_billion: f64,
    #[serde(rename = "gdpYoyPct", alias = "gdp_yoy_pct")]
    pub gdp_yoy_pct: f64,
    #[serde(rename = "inflationYoyPct", alias = "inflation_yoy_pct")]
    pub inflation_yoy_pct: f64,
    #[serde(rename = "unemploymentPct", alias = "unemployment_pct")]
    pub unemployment_pct: f64,
    #[serde(rename = "policyRatePct", alias = "policy_rate_pct")]
    pub policy_rate_pct: f64,
    pub period: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GulfResponse {
    pub rows: Vec<GulfCountryRow>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    rows: Vec<GulfCountryRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<GulfResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    Ok(Json(GulfResponse {
        rows: snap.rows,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::GULF_ECONOMIES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            GULF_ECONOMIES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(GULF_ECONOMIES_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_camelcase_payload() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{
                "iso": "SAU", "country": "Saudi Arabia",
                "gdp_usd_billion": 1100.0_f64, "gdp_yoy_pct": 2.5_f64,
                "inflation_yoy_pct": 1.8_f64, "unemployment_pct": 5.0_f64,
                "policy_rate_pct": 5.5_f64, "period": "2026-Q1",
            }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(GULF_ECONOMIES_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/gdpUsdBillion").is_some());
        assert!(parsed.pointer("/rows/0/policyRatePct").is_some());
    }
}
