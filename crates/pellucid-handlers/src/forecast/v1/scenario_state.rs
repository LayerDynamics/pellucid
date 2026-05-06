//! `GET /api/forecast/v1/scenario-state` — pure reader of the
//! scenario-state slot, surfaces top-N by yes-price and computes
//! a market-cap weighted aggregate probability.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_required, HandlerError};
use crate::state::AppState;

pub const CACHE_KEY: &str = "prediction:scenario-state:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateRow {
    pub id: String,
    pub question: String,
    #[serde(rename = "yesPrice", alias = "yes_price")]
    pub yes_price: f64,
    #[serde(rename = "volumeUsd", alias = "volume_usd")]
    pub volume_usd: f64,
    pub category: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioStateResponse {
    pub rows: Vec<StateRow>,
    #[serde(rename = "topRow", skip_serializing_if = "Option::is_none")]
    pub top_row: Option<StateRow>,
    #[serde(rename = "weightedAvgYes")]
    pub weighted_avg_yes: f64,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct Snap {
    #[serde(default)]
    rows: Vec<StateRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

fn weighted_avg(rows: &[StateRow]) -> f64 {
    let total_volume: f64 = rows.iter().map(|r| r.volume_usd.max(0.0)).sum();
    if total_volume <= 0.0 {
        return 0.0;
    }
    rows.iter()
        .map(|r| r.yes_price * r.volume_usd.max(0.0))
        .sum::<f64>()
        / total_volume
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<ScenarioStateResponse>, HandlerError> {
    let raw = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (snap, stale) = decode_required::<Snap>(raw)?;
    let total = snap.rows.len();
    let avg = weighted_avg(&snap.rows);
    let top_row = snap
        .rows
        .iter()
        .max_by(|a, b| {
            a.yes_price
                .partial_cmp(&b.yes_price)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned();
    Ok(Json(ScenarioStateResponse {
        rows: snap.rows,
        top_row,
        weighted_avg_yes: avg,
        total,
        assembled_at_ms: snap.assembled_at_ms,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::forecast::v1::SCENARIO_STATE_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            SCENARIO_STATE_PATH,
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
                    .uri(SCENARIO_STATE_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn computes_weighted_avg_and_top() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [
                { "id": "a", "question": "qa", "yes_price": 0.6, "volume_usd": 10000.0, "category": "geo" },
                { "id": "b", "question": "qb", "yes_price": 0.9, "volume_usd": 90000.0, "category": "geo" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(SCENARIO_STATE_PATH)
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
        let avg = parsed
            .pointer("/weightedAvgYes")
            .and_then(Value::as_f64)
            .unwrap();
        assert!((avg - 0.87).abs() < 0.001);
        assert_eq!(
            parsed.pointer("/topRow/id").and_then(Value::as_str),
            Some("b")
        );
    }

    #[test]
    fn weighted_avg_zero_volume_returns_zero() {
        let rows = vec![StateRow {
            id: "x".into(),
            question: "q".into(),
            yes_price: 0.5,
            volume_usd: 0.0,
            category: "g".into(),
        }];
        assert_eq!(weighted_avg(&rows), 0.0);
    }
}
