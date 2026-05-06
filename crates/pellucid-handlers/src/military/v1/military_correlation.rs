//! `GET /api/military/v1/military-correlation` — composer that
//! correlates per-theater posture readiness with co-located UCDP
//! events and active deployments. Returns one row per theater
//! that appears in BOTH the posture slot and the UCDP/deployment
//! slots (T4.5.4).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;
use std::collections::BTreeMap;

use crate::conflict::v1::ucdp_events;
use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

use super::strategic_posture;

pub const DEPLOYMENTS_KEY: &str = "military:active-deployments:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CorrelationRow {
    pub theater: String,
    pub readiness: String,
    pub headcount: u32,
    #[serde(rename = "fatalities24h")]
    pub fatalities_24h: u32,
    pub deployments: u32,
    /// 0..=100 correlation score derived from the three signals.
    pub correlation: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MilitaryCorrelationResponse {
    pub rows: Vec<CorrelationRow>,
    pub total: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct PostureSnap {
    #[serde(default)]
    rows: Vec<PostureRow>,
}

#[derive(Debug, Deserialize)]
struct PostureRow {
    #[serde(default)]
    theater: String,
    #[serde(default)]
    readiness: String,
    #[serde(default)]
    headcount: u32,
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

#[derive(Debug, Deserialize)]
struct DeploySnap {
    #[serde(default)]
    rows: Vec<DeployRow>,
}

#[derive(Debug, Deserialize)]
struct DeployRow {
    #[serde(default)]
    theater: String,
}

/// Public so tests can pin the formula independently.
pub fn correlation_score(readiness: &str, fatalities_24h: u32, deployments: u32) -> u32 {
    let mut s: f64 = 0.0;
    s += match readiness {
        "C-1" | "DEFCON-1" | "DEFCON-2" | "high" => 40.0,
        "C-2" | "DEFCON-3" | "elevated" => 20.0,
        _ => 0.0,
    };
    s += (fatalities_24h as f64 / 200.0).min(1.0) * 40.0;
    s += (deployments as f64 / 5.0).min(1.0) * 20.0;
    s.clamp(0.0, 100.0).round() as u32
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<MilitaryCorrelationResponse>, HandlerError> {
    let raw_p = get_cached_json::<Value>(&state.pool, strategic_posture::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_u = get_cached_json::<Value>(&state.pool, ucdp_events::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_d = get_cached_json::<Value>(&state.pool, DEPLOYMENTS_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (p, s1) = decode_optional::<PostureSnap>(raw_p)?;
    let (u, s2) = decode_optional::<UcdpSnap>(raw_u)?;
    let (d, s3) = decode_optional::<DeploySnap>(raw_d)?;

    let posture_rows = p.map(|p| p.rows).unwrap_or_default();
    if posture_rows.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let mut fatal_by_country: BTreeMap<String, u32> = BTreeMap::new();
    if let Some(u) = u {
        for r in u.rows {
            *fatal_by_country.entry(r.country).or_insert(0) += r.fatalities;
        }
    }
    let mut deploy_by_theater: BTreeMap<String, u32> = BTreeMap::new();
    if let Some(d) = d {
        for r in d.rows {
            *deploy_by_theater.entry(r.theater).or_insert(0) += 1;
        }
    }

    let mut rows: Vec<CorrelationRow> = posture_rows
        .into_iter()
        .map(|p| {
            let fatalities = *fatal_by_country.get(&p.theater).unwrap_or(&0);
            let deployments = *deploy_by_theater.get(&p.theater).unwrap_or(&0);
            let score = correlation_score(&p.readiness, fatalities, deployments);
            CorrelationRow {
                theater: p.theater,
                readiness: p.readiness,
                headcount: p.headcount,
                fatalities_24h: fatalities,
                deployments,
                correlation: score,
            }
        })
        .collect();
    rows.sort_by(|a, b| b.correlation.cmp(&a.correlation));
    let total = rows.len();
    Ok(Json(MilitaryCorrelationResponse {
        rows,
        total,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: s1 || s2 || s3,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::military::v1::MILITARY_CORRELATION_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            MILITARY_CORRELATION_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_posture_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(MILITARY_CORRELATION_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn correlates_when_posture_seeded() {
        let (app, pool) = migrated().await;
        let posture = serde_json::json!({
            "rows": [{ "theater": "EUCOM", "force": "USAREUR", "readiness": "C-1", "headcount": 25000 }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(
            &pool,
            strategic_posture::CACHE_KEY,
            &Envelope::new(posture),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(MILITARY_CORRELATION_PATH)
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
        assert_eq!(parsed.pointer("/total").and_then(Value::as_u64), Some(1));
        assert_eq!(
            parsed.pointer("/rows/0/theater").and_then(Value::as_str),
            Some("EUCOM")
        );
        assert!(
            parsed
                .pointer("/rows/0/correlation")
                .and_then(Value::as_u64)
                .unwrap()
                >= 40
        );
    }

    #[test]
    fn score_pins() {
        assert_eq!(correlation_score("C-1", 0, 0), 40);
        assert_eq!(correlation_score("low", 200, 5), 60);
        assert_eq!(correlation_score("normal", 0, 0), 0);
    }
}
