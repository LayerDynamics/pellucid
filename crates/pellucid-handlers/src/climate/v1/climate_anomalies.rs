//! `GET /api/climate/v1/anomalies` — composer over latest-anomaly +
//! station-records (powers ClimateAnomaliesPanel). Tolerates per-
//! slot absence: returns whatever real signal is cached today.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

pub const ANOMALY_KEY: &str = "climate:latest-anomaly:global:v1";
pub const STATION_RECORDS_KEY: &str = "climate:station-records:monthly:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StationRecordRow {
    #[serde(rename = "stationId", alias = "station_id")]
    pub station_id: String,
    pub label: String,
    #[serde(rename = "recordClass", alias = "record_class")]
    pub record_class: String,
    pub value: f64,
    #[serde(rename = "setOn", alias = "set_on")]
    pub set_on: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnomaliesResponse {
    #[serde(rename = "globalAnomalyC", skip_serializing_if = "Option::is_none")]
    pub global_anomaly_c: Option<f64>,
    pub period: Option<String>,
    pub records: Vec<StationRecordRow>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct AnomalySnap {
    #[serde(default, alias = "anomalyC", alias = "anomaly_c")]
    anomaly_c: Option<f64>,
    #[serde(default)]
    period: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StationSnap {
    #[serde(default)]
    rows: Vec<StationRecordRow>,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<AnomaliesResponse>, HandlerError> {
    let raw_anom = get_cached_json::<Value>(&state.pool, ANOMALY_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_rec = get_cached_json::<Value>(&state.pool, STATION_RECORDS_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (anom, anom_stale) = decode_optional::<AnomalySnap>(raw_anom)?;
    let (recs, rec_stale) = decode_optional::<StationSnap>(raw_rec)?;

    let global_anomaly_c = anom.as_ref().and_then(|a| a.anomaly_c);
    let period = anom.as_ref().and_then(|a| a.period.clone());
    let records = recs.map(|s| s.rows).unwrap_or_default();

    if global_anomaly_c.is_none() && records.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    Ok(Json(AnomaliesResponse {
        global_anomaly_c,
        period,
        records,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: anom_stale || rec_stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::climate::v1::ANOMALIES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            ANOMALIES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_both_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ANOMALIES_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_partial_anomaly_only() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({ "anomaly_c": 1.42, "period": "2026-04" });
        set_cached_json(&pool, ANOMALY_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ANOMALIES_PATH)
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
            parsed.pointer("/globalAnomalyC").and_then(Value::as_f64),
            Some(1.42)
        );
    }
}
