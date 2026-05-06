//! `GET /api/climate/v1/natural-events` — composer over volcano +
//! wildfire + earthquake feeds. Returns a unified event tile list.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

use super::{earthquakes, volcano_activity, wildfire};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventTile {
    pub kind: String,
    pub label: String,
    pub region: String,
    pub severity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NaturalEventsResponse {
    pub tiles: Vec<EventTile>,
    #[serde(rename = "totalEvents")]
    pub total_events: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct VolcanoSnap {
    #[serde(default)]
    rows: Vec<volcano_activity::VolcanoRow>,
}

#[derive(Debug, Deserialize)]
struct WildfireSnap {
    #[serde(default)]
    rows: Vec<wildfire::WildfireRow>,
}

#[derive(Debug, Deserialize)]
struct QuakeSnap {
    #[serde(default)]
    rows: Vec<earthquakes::QuakeRow>,
}

fn quake_severity(mag: f64) -> &'static str {
    if mag >= 7.0 {
        "extreme"
    } else if mag >= 6.0 {
        "severe"
    } else if mag >= 5.0 {
        "moderate"
    } else {
        "minor"
    }
}

fn fire_severity(acres: f64) -> &'static str {
    if acres >= 100_000.0 {
        "extreme"
    } else if acres >= 25_000.0 {
        "severe"
    } else if acres >= 1_000.0 {
        "moderate"
    } else {
        "minor"
    }
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<NaturalEventsResponse>, HandlerError> {
    let raw_v = get_cached_json::<Value>(&state.pool, volcano_activity::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_w = get_cached_json::<Value>(&state.pool, wildfire::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_q = get_cached_json::<Value>(&state.pool, earthquakes::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (v, vs) = decode_optional::<VolcanoSnap>(raw_v)?;
    let (w, ws) = decode_optional::<WildfireSnap>(raw_w)?;
    let (q, qs) = decode_optional::<QuakeSnap>(raw_q)?;

    let mut tiles: Vec<EventTile> = Vec::new();
    if let Some(v) = v {
        for row in v.rows {
            tiles.push(EventTile {
                kind: "volcano".into(),
                label: row.name,
                region: row.country,
                severity: row.status,
            });
        }
    }
    if let Some(w) = w {
        for row in w.rows {
            tiles.push(EventTile {
                kind: "wildfire".into(),
                label: row.label,
                region: row.region,
                severity: fire_severity(row.acres_burned).into(),
            });
        }
    }
    if let Some(q) = q {
        for row in q.rows {
            tiles.push(EventTile {
                kind: "earthquake".into(),
                label: format!("M{:.1} {}", row.mag, row.place),
                region: row.place,
                severity: quake_severity(row.mag).into(),
            });
        }
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let total = tiles.len();
    Ok(Json(NaturalEventsResponse {
        tiles,
        total_events: total,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: vs || ws || qs,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::climate::v1::NATURAL_EVENTS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            NATURAL_EVENTS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(NATURAL_EVENTS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn merges_one_seeded_slot() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({
            "rows": [{ "id": "us2", "place": "Japan", "mag": 6.1, "depth": 50.0, "lat": 35.0, "lon": 140.0, "occurred_at_ms": 1_700_000_000_000_i64 }],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, earthquakes::CACHE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(NATURAL_EVENTS_PATH)
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
            parsed.pointer("/totalEvents").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            parsed.pointer("/tiles/0/kind").and_then(Value::as_str),
            Some("earthquake")
        );
        assert_eq!(
            parsed.pointer("/tiles/0/severity").and_then(Value::as_str),
            Some("severe")
        );
    }

    #[test]
    fn severity_thresholds() {
        assert_eq!(quake_severity(7.5), "extreme");
        assert_eq!(quake_severity(6.5), "severe");
        assert_eq!(quake_severity(5.0), "moderate");
        assert_eq!(quake_severity(2.0), "minor");
        assert_eq!(fire_severity(150_000.0), "extreme");
        assert_eq!(fire_severity(50_000.0), "severe");
        assert_eq!(fire_severity(5_000.0), "moderate");
        assert_eq!(fire_severity(50.0), "minor");
    }
}
