//! `GET /api/consumer-prices/v1/list` — composes the per-region
//! latest CPI snapshots the T3.8 economic seeders publish.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

pub const SLOT_US: &str = "consumer-prices:latest-cpi:US:v1";
pub const SLOT_EU: &str = "consumer-prices:latest-cpi:EU:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CpiRegionRow {
    pub region: String,
    pub period: String,
    /// Headline CPI value as published.
    #[serde(rename = "headlineValue", alias = "value", alias = "headline_value")]
    pub headline_value: f64,
    /// YoY %.
    #[serde(rename = "yoyPct", alias = "yoy_pct", default)]
    pub yoy_pct: f64,
    /// MoM %.
    #[serde(rename = "momPct", alias = "mom_pct", default)]
    pub mom_pct: f64,
    /// Optional component breakdown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<CpiComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CpiComponent {
    pub label: String,
    #[serde(rename = "yoyPct", alias = "yoy_pct")]
    pub yoy_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpiListResponse {
    pub rows: Vec<CpiRegionRow>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    region: String,
    period: String,
    #[serde(default, alias = "headlineValue", alias = "headline_value")]
    value: f64,
    #[serde(default, alias = "yoyPct", alias = "yoy_pct")]
    yoy_pct: f64,
    #[serde(default, alias = "momPct", alias = "mom_pct")]
    mom_pct: f64,
    #[serde(default)]
    components: Vec<CpiComponent>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

pub async fn handler(State(state): State<AppState>) -> Result<Json<CpiListResponse>, HandlerError> {
    let mut rows: Vec<CpiRegionRow> = Vec::new();
    let mut assembled = 0_i64;
    let mut stale = false;
    for slot in [SLOT_US, SLOT_EU] {
        let raw = get_cached_json::<Value>(&state.pool, slot)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (parsed, was_stale) = decode_optional::<Snap>(raw)?;
        if let Some(s) = parsed {
            if s.assembled_at_ms > assembled {
                assembled = s.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            rows.push(CpiRegionRow {
                region: s.region,
                period: s.period,
                headline_value: s.value,
                yoy_pct: s.yoy_pct,
                mom_pct: s.mom_pct,
                components: s.components,
            });
        }
    }
    if rows.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    Ok(Json(CpiListResponse {
        rows,
        assembled_at_ms: assembled,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::consumer_prices::v1::LIST_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(LIST_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_nothing_present() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn composes_when_us_only_present() {
        let (app, pool) = migrated().await;
        let payload = serde_json::json!({
            "region": "US",
            "period": "2026-04",
            "headline_value": 312.5_f64,
            "yoy_pct": 3.2_f64,
            "mom_pct": 0.3_f64,
            "components": [
                { "label": "Food", "yoy_pct": 2.1_f64 },
                { "label": "Energy", "yoy_pct": -1.5_f64 },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, SLOT_US, &Envelope::new(payload), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(LIST_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/rows/0/region").and_then(Value::as_str),
            Some("US"),
        );
        assert!(parsed.pointer("/rows/0/yoyPct").is_some());
        assert!(parsed.pointer("/rows/0/components/0/yoyPct").is_some());
    }
}
