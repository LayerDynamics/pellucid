//! `GET /api/military/v1/strategic-risk` — composer that blends
//! UCDP events (`conflict/v1/ucdp_events`) + theater posture +
//! sanctions pressure into a 0..=100 strategic-risk score
//! (T4.5.3).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::conflict::v1::ucdp_events;
use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

use super::strategic_posture;

pub const SANCTIONS_KEY: &str = "sanctions:recent-additions:24h:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrategicRiskResponse {
    /// `low` | `elevated` | `high`.
    pub level: String,
    pub score: u32,
    pub headline: String,
    pub rationale: String,
    #[serde(rename = "ucdpFatalities24h", skip_serializing_if = "Option::is_none")]
    pub ucdp_fatalities_24h: Option<u32>,
    #[serde(rename = "highReadinessTheaters", skip_serializing_if = "Option::is_none")]
    pub high_readiness_theaters: Option<usize>,
    #[serde(rename = "sanctions24h", skip_serializing_if = "Option::is_none")]
    pub sanctions_24h: Option<usize>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct UcdpSnap {
    #[serde(default)]
    rows: Vec<UcdpRow>,
}

#[derive(Debug, Deserialize)]
struct UcdpRow {
    #[serde(default)]
    fatalities: u32,
}

#[derive(Debug, Deserialize)]
struct PostureSnap {
    #[serde(default)]
    rows: Vec<PostureRow>,
}

#[derive(Debug, Deserialize)]
struct PostureRow {
    #[serde(default)]
    readiness: String,
}

#[derive(Debug, Deserialize)]
struct SanctionsSnap {
    #[serde(default)]
    rows: Vec<Value>,
}

/// Combine the three real signals into a 0..=100 risk score.
pub fn score_from_signals(
    fatalities_24h: Option<u32>,
    high_readiness_theaters: Option<usize>,
    sanctions_24h: Option<usize>,
) -> u32 {
    let mut score: f64 = 0.0;
    if let Some(f) = fatalities_24h {
        score += (f as f64 / 200.0).min(1.0) * 50.0;
    }
    if let Some(n) = high_readiness_theaters {
        score += (n.min(3)) as f64 * 10.0;
    }
    if let Some(s) = sanctions_24h {
        score += (s as f64 / 10.0).min(1.0) * 20.0;
    }
    score.clamp(0.0, 100.0).round() as u32
}

pub fn level_from_score(score: u32) -> String {
    if score >= 60 {
        "high".into()
    } else if score >= 30 {
        "elevated".into()
    } else {
        "low".into()
    }
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<StrategicRiskResponse>, HandlerError> {
    let raw_u = get_cached_json::<Value>(&state.pool, ucdp_events::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_p = get_cached_json::<Value>(&state.pool, strategic_posture::CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_s = get_cached_json::<Value>(&state.pool, SANCTIONS_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (u, s1) = decode_optional::<UcdpSnap>(raw_u)?;
    let (p, s2) = decode_optional::<PostureSnap>(raw_p)?;
    let (s, s3) = decode_optional::<SanctionsSnap>(raw_s)?;

    let fatalities = u.as_ref().map(|u| u.rows.iter().map(|r| r.fatalities).sum::<u32>());
    let high_readiness = p.as_ref().map(|p| {
        p.rows
            .iter()
            .filter(|r| matches!(r.readiness.as_str(), "C-1" | "DEFCON-1" | "DEFCON-2" | "high"))
            .count()
    });
    let sanctions = s.as_ref().map(|s| s.rows.len());

    if fatalities.is_none() && high_readiness.is_none() && sanctions.is_none() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let score = score_from_signals(fatalities, high_readiness, sanctions);
    let level = level_from_score(score);
    let mut parts: Vec<String> = Vec::new();
    if let Some(f) = fatalities {
        parts.push(format!("UCDP 24h: {f} fatalities"));
    }
    if let Some(n) = high_readiness {
        parts.push(format!("{n} high-readiness theaters"));
    }
    if let Some(n) = sanctions {
        parts.push(format!("{n} sanctions in 24h"));
    }
    let rationale = parts.join(" · ");
    let headline = match level.as_str() {
        "high" => "Strategic risk: HIGH".into(),
        "elevated" => "Strategic risk: elevated".into(),
        _ => "Strategic risk: low".into(),
    };

    Ok(Json(StrategicRiskResponse {
        level,
        score,
        headline,
        rationale,
        ucdp_fatalities_24h: fatalities,
        high_readiness_theaters: high_readiness,
        sanctions_24h: sanctions,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: s1 || s2 || s3,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::military::v1::STRATEGIC_RISK_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            STRATEGIC_RISK_PATH,
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
                    .uri(STRATEGIC_RISK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn computes_low_score_when_quiet() {
        let (app, pool) = migrated().await;
        set_cached_json(
            &pool,
            ucdp_events::CACHE_KEY,
            &Envelope::new(serde_json::json!({"rows":[],"assembled_at_ms":0})),
            60_000,
        )
        .await
        .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(STRATEGIC_RISK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/level").and_then(Value::as_str), Some("low"));
    }

    #[test]
    fn score_floor_and_ceiling() {
        assert_eq!(score_from_signals(Some(0), Some(0), Some(0)), 0);
        assert!(score_from_signals(Some(500), Some(5), Some(50)) >= 99);
    }

    #[test]
    fn level_thresholds() {
        assert_eq!(level_from_score(0), "low");
        assert_eq!(level_from_score(29), "low");
        assert_eq!(level_from_score(30), "elevated");
        assert_eq!(level_from_score(59), "elevated");
        assert_eq!(level_from_score(60), "high");
    }
}
