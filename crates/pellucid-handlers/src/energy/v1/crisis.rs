//! `GET /api/energy/v1/crisis` — composer that derives a single
//! crisis-risk score from the gas-storage + SPR cache slots. Pure
//! reader (no upstreams).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

use super::complex::{GAS_STORAGE_KEY, SPR_KEY};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrisisResponse {
    /// `low` | `elevated` | `high`.
    pub level: String,
    /// 0..=100 risk score.
    pub score: u32,
    pub headline: String,
    pub rationale: String,
    #[serde(rename = "gasStoragePct", skip_serializing_if = "Option::is_none")]
    pub gas_storage_pct: Option<f64>,
    #[serde(rename = "sprMb", skip_serializing_if = "Option::is_none")]
    pub spr_mb: Option<f64>,
    #[serde(rename = "sprDeltaMb", skip_serializing_if = "Option::is_none")]
    pub spr_delta_mb: Option<f64>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct GasSnap {
    #[serde(default, alias = "averageFullPct", alias = "average_full_pct")]
    average_full_pct: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SprSnap {
    #[serde(default, alias = "currentMb", alias = "current_mb")]
    current_mb: Option<f64>,
    #[serde(default, alias = "deltaMb", alias = "delta_mb")]
    delta_mb: Option<f64>,
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<CrisisResponse>, HandlerError> {
    let raw_gas = get_cached_json::<Value>(&state.pool, GAS_STORAGE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_spr = get_cached_json::<Value>(&state.pool, SPR_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (gas, gas_stale) = decode_optional::<GasSnap>(raw_gas)?;
    let (spr, spr_stale) = decode_optional::<SprSnap>(raw_spr)?;

    let gas_pct = gas.as_ref().and_then(|g| g.average_full_pct);
    let spr_mb = spr.as_ref().and_then(|s| s.current_mb);
    let spr_delta = spr.as_ref().and_then(|s| s.delta_mb);

    if gas_pct.is_none() && spr_mb.is_none() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    let score = score_from_signals(gas_pct, spr_delta);
    let level = level_from_score(score);
    let (headline, rationale) = explain(&level, gas_pct, spr_mb, spr_delta);

    Ok(Json(CrisisResponse {
        level,
        score,
        headline,
        rationale,
        gas_storage_pct: gas_pct,
        spr_mb,
        spr_delta_mb: spr_delta,
        assembled_at_ms: pellucid_core::now_ms(),
        stale: gas_stale || spr_stale,
    }))
}

/// Combine the two real signals into a 0..=100 risk score.
/// Lower gas storage + falling SPR → higher score.
pub fn score_from_signals(gas_pct: Option<f64>, spr_delta_mb: Option<f64>) -> u32 {
    let mut score: f64 = 0.0;
    if let Some(p) = gas_pct {
        // 100 → 0, 0 → 60.
        score += ((100.0 - p) / 100.0) * 60.0;
    }
    if let Some(d) = spr_delta_mb {
        if d < 0.0 {
            score += (-d).min(20.0) * 2.0; // each mb drawn = 2 pts, capped 40
        }
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

fn explain(
    level: &str,
    gas_pct: Option<f64>,
    spr_mb: Option<f64>,
    spr_delta: Option<f64>,
) -> (String, String) {
    let headline = match level {
        "high" => "Energy crisis risk: HIGH".to_string(),
        "elevated" => "Energy crisis risk: elevated".to_string(),
        _ => "Energy crisis risk: low".to_string(),
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = gas_pct {
        parts.push(format!("EU gas storage at {p:.0}% full"));
    }
    if let (Some(mb), Some(d)) = (spr_mb, spr_delta) {
        parts.push(format!(
            "SPR {mb:.0} mb ({}{d:.1} mb wow)",
            if d >= 0.0 { "+" } else { "" }
        ));
    }
    let rationale = if parts.is_empty() {
        "No active signals".to_string()
    } else {
        parts.join(" · ")
    };
    (headline, rationale)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::energy::v1::CRISIS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            CRISIS_PATH,
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
                    .uri(CRISIS_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn high_storage_low_risk() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({ "average_full_pct": 90.0 });
        set_cached_json(&pool, GAS_STORAGE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(CRISIS_PATH)
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
        assert_eq!(score_from_signals(Some(100.0), None), 0);
        assert!(score_from_signals(Some(0.0), Some(-50.0)) >= 90);
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
