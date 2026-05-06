//! `GET /api/conflict/v1/escalation-correlation` — composer that
//! correlates thermal anomalies (`thermal/v1/escalation`) with
//! UCDP events by zone (joining `zone` against `country`). Returns
//! one row per zone that appears in EITHER feed; sorted by
//! descending escalation score (T4.5.5).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;
use std::collections::BTreeMap;

use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;
use crate::thermal::v1::escalation as thermal_escalation;

use super::ucdp_events;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscalationRow {
    pub zone: String,
    #[serde(rename = "thermalAnomalies")]
    pub thermal_anomalies: u32,
    #[serde(rename = "fatalities24h")]
    pub fatalities_24h: u32,
    /// 0..=100 escalation score.
    pub escalation: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscalationCorrelationResponse {
    pub rows: Vec<EscalationRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct ThermalSnap {
    #[serde(default)]
    rows: Vec<ThermalRow>,
}

#[derive(Debug, Deserialize)]
struct ThermalRow {
    #[serde(default)]
    zone: String,
}

#[derive(Debug, Deserialize)]
struct UcdpSnap {
    #[serde(default)]
    rows: Vec<UcdpRow>,
}

#[derive(Debug, Deserialize)]
struct UcdpRow {
    #[serde(default)]
    country: String,
    #[serde(default)]
    fatalities: u32,
}

pub fn escalation_score(thermal: u32, fatalities: u32) -> u32 {
    let mut s: f64 = 0.0;
    s += (thermal as f64 / 25.0).min(1.0) * 50.0;
    s += (fatalities as f64 / 100.0).min(1.0) * 50.0;
    s.clamp(0.0, 100.0).round() as u32
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<EscalationCorrelationResponse>, HandlerError> {
    let raw_t = get_cached_json::<Value>(&state.pool, thermal_escalation::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_u = get_cached_json::<Value>(&state.pool, ucdp_events::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (t, s1) = decode_optional::<ThermalSnap>(raw_t)?;
    let (u, s2) = decode_optional::<UcdpSnap>(raw_u)?;

    if t.is_none() && u.is_none() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let mut by_zone: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    if let Some(t) = t {
        for r in t.rows {
            by_zone.entry(r.zone).or_insert((0, 0)).0 += 1;
        }
    }
    if let Some(u) = u {
        for r in u.rows {
            by_zone.entry(r.country).or_insert((0, 0)).1 += r.fatalities;
        }
    }
    let mut rows: Vec<EscalationRow> = by_zone
        .into_iter()
        .map(|(zone, (thermal, fatal))| EscalationRow {
            zone,
            thermal_anomalies: thermal,
            fatalities_24h: fatal,
            escalation: escalation_score(thermal, fatal),
        })
        .collect();
    rows.sort_by(|a, b| b.escalation.cmp(&a.escalation));
    if rows.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let total = rows.len();
    Ok(Json(EscalationCorrelationResponse {
        rows,
        total,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: s1 || s2,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::conflict::v1::ESCALATION_CORRELATION_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            ESCALATION_CORRELATION_PATH,
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
                    .uri(ESCALATION_CORRELATION_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn correlates_zone_overlap() {
        let (app, pool) = migrated().await;
        let thermal = serde_json::json!({
            "rows": [
                { "id": "a", "lat": 50.0, "lon": 30.0, "brightness_k": 410.0, "confidence": 90, "zone": "Ukraine", "acquired_at": "2026-04-29T12:00:00Z" },
                { "id": "b", "lat": 50.0, "lon": 30.0, "brightness_k": 405.0, "confidence": 88, "zone": "Ukraine", "acquired_at": "2026-04-29T12:30:00Z" },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        let ucdp = serde_json::json!({
            "rows": [{ "id":"u1","country":"Ukraine","actor1":"a","actor2":"b","fatalities":50,"lat":50.0,"lon":30.0,"occurred_at":"2026-04-29T01:00:00Z" }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, thermal_escalation::CACHE_KEY, &Envelope::new(thermal), 60_000).await.unwrap();
        set_cached_json(&pool, ucdp_events::CACHE_KEY, &Envelope::new(ucdp), 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(ESCALATION_CORRELATION_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/rows/0/zone").and_then(Value::as_str), Some("Ukraine"));
        let escalation = parsed.pointer("/rows/0/escalation").and_then(Value::as_u64).unwrap();
        assert!(escalation >= 25, "{escalation}");
    }

    #[test]
    fn score_pins() {
        assert_eq!(escalation_score(0, 0), 0);
        assert_eq!(escalation_score(25, 0), 50);
        assert_eq!(escalation_score(0, 100), 50);
        assert!(escalation_score(50, 200) >= 99);
    }
}
