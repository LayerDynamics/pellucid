//! `GET /api/forecast/v1/summary` — composer over now-cast +
//! scenario-state + extended slots. Powers ForecastPanel.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

use super::{extended, now_cast, scenario_state};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForecastTile {
    pub code: String,
    pub label: String,
    pub value: String,
    /// `positive` | `negative` | `neutral`.
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForecastSummaryResponse {
    pub tiles: Vec<ForecastTile>,
    #[serde(rename = "availableTiles")]
    pub available_tiles: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct NowSnap {
    #[serde(default)]
    rows: Vec<NowRow>,
}

#[derive(Debug, Deserialize)]
struct NowRow {
    #[serde(default)]
    probability: f64,
}

#[derive(Debug, Deserialize)]
struct StateSnap {
    #[serde(default)]
    rows: Vec<StateRow>,
}

#[derive(Debug, Deserialize)]
struct StateRow {
    #[serde(default, alias = "yesPrice", alias = "yes_price")]
    yes_price: f64,
    #[serde(default, alias = "volumeUsd", alias = "volume_usd")]
    volume_usd: f64,
}

#[derive(Debug, Deserialize)]
struct ExtendedSnap {
    #[serde(default)]
    rows: Vec<ExtendedRow>,
}

#[derive(Debug, Deserialize)]
struct ExtendedRow {
    #[serde(default, alias = "delta7d", alias = "delta_7d")]
    delta_7d: f64,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<ForecastSummaryResponse>, HandlerError> {
    let raw_now = get_cached_json::<Value>(&state.pool, now_cast::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_state = get_cached_json::<Value>(&state.pool, scenario_state::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_ext = get_cached_json::<Value>(&state.pool, extended::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (nc, s1) = decode_optional::<NowSnap>(raw_now)?;
    let (st, s2) = decode_optional::<StateSnap>(raw_state)?;
    let (ext, s3) = decode_optional::<ExtendedSnap>(raw_ext)?;

    let mut tiles: Vec<ForecastTile> = Vec::new();
    if let Some(nc) = nc {
        if !nc.rows.is_empty() {
            let avg: f64 =
                nc.rows.iter().map(|r| r.probability).sum::<f64>() / nc.rows.len() as f64;
            tiles.push(ForecastTile {
                code: "NOW".into(),
                label: "Now-cast avg".into(),
                value: format!("{:.0}%", avg * 100.0),
                tone: "neutral".into(),
            });
        }
    }
    if let Some(st) = st {
        let total_v: f64 = st.rows.iter().map(|r| r.volume_usd.max(0.0)).sum();
        if total_v > 0.0 {
            let weighted: f64 = st
                .rows
                .iter()
                .map(|r| r.yes_price * r.volume_usd.max(0.0))
                .sum::<f64>()
                / total_v;
            tiles.push(ForecastTile {
                code: "MARKETS".into(),
                label: "Vol-weighted YES".into(),
                value: format!("{:.0}%", weighted * 100.0),
                tone: "neutral".into(),
            });
        }
    }
    if let Some(ext) = ext {
        if !ext.rows.is_empty() {
            let max_abs = ext
                .rows
                .iter()
                .map(|r| r.delta_7d.abs())
                .fold(0.0_f64, f64::max);
            tiles.push(ForecastTile {
                code: "MOMENTUM".into(),
                label: "Max |Δ7d|".into(),
                value: format!("{:+.0}pp", max_abs * 100.0),
                tone: if max_abs > 0.10 {
                    "negative".into()
                } else {
                    "neutral".into()
                },
            });
        }
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let available = tiles.len();
    Ok(Json(ForecastSummaryResponse {
        tiles,
        available_tiles: available,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: s1 || s2 || s3,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::forecast::v1::SUMMARY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(SUMMARY_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(SUMMARY_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn picks_one_seeded() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{ "id": "q1", "question": "Q", "probability": 0.4, "trend": "flat" }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, now_cast::CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(SUMMARY_PATH)
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
            parsed.pointer("/availableTiles").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            parsed.pointer("/tiles/0/code").and_then(Value::as_str),
            Some("NOW")
        );
    }
}
