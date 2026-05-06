//! `GET /api/economic/v1/big-mac` — pure reader.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "economic:big-mac-index:v1";
pub const MAX_LIMIT: usize = 200;
pub const DEFAULT_LIMIT: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BigMacRow {
    pub iso: String,
    pub country: String,
    #[serde(rename = "localPrice", alias = "local_price")]
    pub local_price: f64,
    pub currency: String,
    #[serde(rename = "usdPrice", alias = "usd_price")]
    pub usd_price: f64,
    pub ppp: f64,
    #[serde(rename = "fxRate", alias = "fx_rate")]
    pub fx_rate: f64,
    #[serde(rename = "valuationPct", alias = "valuation_pct")]
    pub valuation_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BigMacResponse {
    #[serde(rename = "snapshotDate")]
    pub snapshot_date: String,
    pub rows: Vec<BigMacRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct BigMacQuery {
    pub limit: Option<usize>,
    /// Comma-separated allow-list of ISO-3 codes.
    pub iso: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Snap {
    snapshot_date: String,
    rows: Vec<BigMacRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<BigMacQuery>,
) -> Result<Json<BigMacResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let mut rows = snap.rows;
    if let Some(raw) = q.iso.as_deref() {
        let allow: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !allow.is_empty() {
            rows.retain(|r| allow.iter().any(|a| a == &r.iso.to_ascii_uppercase()));
        }
    }
    let total = rows.len();
    let cap = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    if rows.len() > cap {
        rows.truncate(cap);
    }
    Ok(Json(BigMacResponse {
        snapshot_date: snap.snapshot_date,
        rows,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::BIG_MAC_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(BIG_MAC_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    fn snap() -> Value {
        serde_json::json!({
            "snapshot_date": "2026-01-01",
            "rows": [
                { "iso": "USA", "country": "US", "local_price": 5.0, "currency": "USD",
                  "usd_price": 5.0, "ppp": 5.0, "fx_rate": 1.0, "valuation_pct": 0.0 },
                { "iso": "CHE", "country": "Switzerland", "local_price": 6.0, "currency": "CHF",
                  "usd_price": 6.5, "ppp": 5.0, "fx_rate": 0.92, "valuation_pct": 30.0 },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    #[tokio::test]
    async fn returns_503_when_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BIG_MAC_PATH)
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
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap()), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BIG_MAC_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed.pointer("/rows/0/usdPrice").is_some());
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(2));
    }

    #[tokio::test]
    async fn iso_filter_subsets_rows() {
        let (app, pool) = migrated().await;
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap()), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{BIG_MAC_PATH}?iso=che"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: BigMacResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0].iso, "CHE");
    }
}
