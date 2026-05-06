//! `GET /api/economic/v1/snapshot` — composer over the existing
//! FRED-latest cache slots that the T3.8 economic domain populates.
//!
//! Returns whatever indicators are present; outputs them as a
//! list of `{ code, value, period }` rows. Tolerates per-slot
//! absence (omits the row); only 503s when EVERY indicator slot
//! is empty.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

/// FAST-tier cache slots the snapshot composes from.
pub const SLOT_UNRATE: &str = "economic:fred-latest:UNRATE:v1";
pub const SLOT_CPIAUCSL: &str = "economic:fred-latest:CPIAUCSL:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct IndicatorRow {
    pub code: String,
    pub value: f64,
    /// Optional period or observation date, when the seeder
    /// wrote one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub indicators: Vec<IndicatorRow>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct LatestObs {
    /// Some seeders write `value`, others nest under `latest.value`.
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    latest: Option<LatestInner>,
    #[serde(default, alias = "assembledAtMs", alias = "assembled_at_ms")]
    assembled_at_ms: i64,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LatestInner {
    value: f64,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

fn extract(obs: &LatestObs) -> Option<(f64, Option<String>)> {
    if let Some(v) = obs.value {
        return Some((v, obs.period.clone().or(obs.date.clone())));
    }
    if let Some(l) = &obs.latest {
        return Some((l.value, l.period.clone().or(l.date.clone())));
    }
    None
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<SnapshotResponse>, HandlerError> {
    let mut indicators: Vec<IndicatorRow> = Vec::new();
    let mut assembled = 0_i64;
    let mut stale = false;

    for (code, key) in [("UNRATE", SLOT_UNRATE), ("CPIAUCSL", SLOT_CPIAUCSL)] {
        let raw = get_cached_json::<Value>(&state.pool, key)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (parsed, was_stale) = decode_optional::<LatestObs>(raw)?;
        if let Some(obs) = parsed {
            if obs.assembled_at_ms > assembled {
                assembled = obs.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            if let Some((value, period)) = extract(&obs) {
                indicators.push(IndicatorRow {
                    code: code.to_string(),
                    value,
                    period,
                });
            }
        }
    }

    if indicators.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    Ok(Json(SnapshotResponse {
        indicators,
        assembled_at_ms: assembled,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::SNAPSHOT_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            SNAPSHOT_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_no_indicators_present() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(SNAPSHOT_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn composes_when_one_slot_present() {
        let (app, pool) = migrated().await;
        let payload = serde_json::json!({
            "value": 3.8_f64,
            "period": "2026-04",
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, SLOT_UNRATE, &Envelope::new(payload), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(SNAPSHOT_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: SnapshotResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.indicators.len(), 1);
        assert_eq!(parsed.indicators[0].code, "UNRATE");
        assert!((parsed.indicators[0].value - 3.8).abs() < 1e-9);
    }
}
