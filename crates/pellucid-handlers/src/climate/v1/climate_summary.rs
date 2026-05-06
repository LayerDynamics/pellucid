//! `GET /api/climate/v1/summary` — composer over latest-anomaly +
//! air-quality + wildfire active count. Powers ClimatePanel.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

use super::{air_quality, climate_anomalies, wildfire};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryTile {
    pub code: String,
    pub label: String,
    pub value: String,
    /// `positive` | `negative` | `neutral`.
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClimateSummaryResponse {
    pub tiles: Vec<SummaryTile>,
    #[serde(rename = "availableTiles")]
    pub available_tiles: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct AnomalySnap {
    #[serde(default, alias = "anomalyC", alias = "anomaly_c")]
    anomaly_c: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AqSnap {
    #[serde(default)]
    rows: Vec<AqRow>,
}

#[derive(Debug, Deserialize)]
struct AqRow {
    #[serde(default)]
    aqi: u32,
}

#[derive(Debug, Deserialize)]
struct FireSnap {
    #[serde(default)]
    rows: Vec<FireRow>,
}

#[derive(Debug, Deserialize)]
struct FireRow {
    #[serde(default, alias = "containmentPct", alias = "containment_pct")]
    containment_pct: f64,
}

fn anomaly_tone(c: f64) -> &'static str {
    if c >= 1.0 {
        "negative"
    } else if c <= -0.5 {
        "positive"
    } else {
        "neutral"
    }
}

fn aqi_tone(worst: u32) -> &'static str {
    if worst >= 150 {
        "negative"
    } else if worst >= 100 {
        "neutral"
    } else {
        "positive"
    }
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<ClimateSummaryResponse>, HandlerError> {
    let raw_anom = get_cached_json::<Value>(&state.pool, climate_anomalies::ANOMALY_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_aq = get_cached_json::<Value>(&state.pool, air_quality::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_fire = get_cached_json::<Value>(&state.pool, wildfire::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (anom, s1) = decode_optional::<AnomalySnap>(raw_anom)?;
    let (aq, s2) = decode_optional::<AqSnap>(raw_aq)?;
    let (fire, s3) = decode_optional::<FireSnap>(raw_fire)?;

    let mut tiles: Vec<SummaryTile> = Vec::new();
    if let Some(anom) = anom {
        if let Some(c) = anom.anomaly_c {
            tiles.push(SummaryTile {
                code: "ANOMALY".into(),
                label: "Global anomaly".into(),
                value: format!("{c:+.2} °C"),
                tone: anomaly_tone(c).into(),
            });
        }
    }
    if let Some(aq) = aq {
        if !aq.rows.is_empty() {
            let worst = aq.rows.iter().map(|r| r.aqi).max().unwrap_or(0);
            tiles.push(SummaryTile {
                code: "AQI".into(),
                label: "Worst AQI".into(),
                value: worst.to_string(),
                tone: aqi_tone(worst).into(),
            });
        }
    }
    if let Some(fire) = fire {
        if !fire.rows.is_empty() {
            let active = fire
                .rows
                .iter()
                .filter(|r| r.containment_pct < 100.0)
                .count();
            tiles.push(SummaryTile {
                code: "FIRES".into(),
                label: "Active wildfires".into(),
                value: active.to_string(),
                tone: if active > 0 { "negative" } else { "positive" }.into(),
            });
        }
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let available = tiles.len();
    Ok(Json(ClimateSummaryResponse {
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
    use crate::climate::v1::SUMMARY_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            SUMMARY_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_empty() {
        let (app, _) = migrated().await;
        let resp = app.oneshot(Request::builder().uri(SUMMARY_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn picks_one_seeded() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({ "anomaly_c": 1.42 });
        set_cached_json(&pool, climate_anomalies::ANOMALY_KEY, &Envelope::new(snap), 60_000).await.unwrap();
        let resp = app.oneshot(Request::builder().uri(SUMMARY_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/availableTiles").and_then(Value::as_u64), Some(1));
        assert_eq!(parsed.pointer("/tiles/0/code").and_then(Value::as_str), Some("ANOMALY"));
        assert_eq!(parsed.pointer("/tiles/0/tone").and_then(Value::as_str), Some("negative"));
    }

    #[test]
    fn tone_bands() {
        assert_eq!(anomaly_tone(1.5), "negative");
        assert_eq!(anomaly_tone(0.2), "neutral");
        assert_eq!(anomaly_tone(-1.0), "positive");
        assert_eq!(aqi_tone(200), "negative");
        assert_eq!(aqi_tone(120), "neutral");
        assert_eq!(aqi_tone(40), "positive");
    }
}
