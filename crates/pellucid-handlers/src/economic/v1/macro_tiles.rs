//! `GET /api/economic/v1/macro-tiles` — emits a tile-grid
//! envelope spanning the FAST-tier macro indicators. Composer.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS};
use crate::state::AppState;

pub const SLOT_UNRATE: &str = "economic:fred-latest:UNRATE:v1";
pub const SLOT_CPIAUCSL: &str = "economic:fred-latest:CPIAUCSL:v1";
pub const SLOT_FSI: &str = "economic:financial-stress:v1";
pub const SLOT_FUEL: &str = "energy:fuel-prices:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MacroTile {
    pub code: String,
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subline: Option<String>,
    /// kebab-case tone — "positive" / "negative" / "neutral".
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroTilesResponse {
    pub tiles: Vec<MacroTile>,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct LatestPair {
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    prior: Option<f64>,
    #[serde(default)]
    latest: Option<LatestInner>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct LatestInner {
    value: f64,
    #[serde(default)]
    prior_value: Option<f64>,
}

fn current_and_prior(p: &LatestPair) -> Option<(f64, Option<f64>)> {
    if let Some(v) = p.value {
        return Some((v, p.prior));
    }
    if let Some(l) = &p.latest {
        return Some((l.value, l.prior_value));
    }
    None
}

#[derive(Debug, Deserialize)]
struct FsiSnap {
    latest: FsiObs,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct FsiObs {
    value: f64,
}

#[derive(Debug, Deserialize)]
struct FuelSnap {
    rows: Vec<FuelRow>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct FuelRow {
    #[serde(alias = "usdPerGallon")]
    usd_per_gallon: f64,
    #[serde(default, alias = "wow_change_pct", alias = "weekOverWeekChangePct")]
    wow_change_pct: f64,
}

fn tone_for_delta(delta: f64, neutral_above_when_neg: bool) -> &'static str {
    if !delta.is_finite() {
        return "neutral";
    }
    if delta > 0.0 {
        return if neutral_above_when_neg { "positive" } else { "negative" };
    }
    if delta < 0.0 {
        return if neutral_above_when_neg { "negative" } else { "positive" };
    }
    "neutral"
}

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<MacroTilesResponse>, HandlerError> {
    let mut tiles: Vec<MacroTile> = Vec::new();
    let mut assembled = 0_i64;
    let mut stale = false;

    // UNRATE: lower is better.
    {
        let raw = get_cached_json::<Value>(&state.pool, SLOT_UNRATE)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (p, was_stale) = decode_optional::<LatestPair>(raw)?;
        if let Some(p) = p {
            if p.assembled_at_ms > assembled {
                assembled = p.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            if let Some((v, prior)) = current_and_prior(&p) {
                let delta = prior.map(|pr| v - pr).unwrap_or(0.0);
                tiles.push(MacroTile {
                    code: "UNRATE".into(),
                    label: "Unemployment".into(),
                    value: format!("{v:.1}%"),
                    subline: prior.map(|_| format!("Δ {delta:+.2}")),
                    tone: tone_for_delta(delta, false).to_string(),
                });
            }
        }
    }

    // CPI: lower is better.
    {
        let raw = get_cached_json::<Value>(&state.pool, SLOT_CPIAUCSL)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (p, was_stale) = decode_optional::<LatestPair>(raw)?;
        if let Some(p) = p {
            if p.assembled_at_ms > assembled {
                assembled = p.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            if let Some((v, prior)) = current_and_prior(&p) {
                let delta = prior.map(|pr| v - pr).unwrap_or(0.0);
                tiles.push(MacroTile {
                    code: "CPIAUCSL".into(),
                    label: "CPI".into(),
                    value: format!("{v:.2}"),
                    subline: prior.map(|_| format!("Δ {delta:+.2}")),
                    tone: tone_for_delta(delta, false).to_string(),
                });
            }
        }
    }

    // FSI: lower is better.
    {
        let raw = get_cached_json::<Value>(&state.pool, SLOT_FSI)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (snap, was_stale) = decode_optional::<FsiSnap>(raw)?;
        if let Some(snap) = snap {
            if snap.assembled_at_ms > assembled {
                assembled = snap.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            tiles.push(MacroTile {
                code: "STLFSI".into(),
                label: "Financial stress".into(),
                value: format!("{:.2}", snap.latest.value),
                subline: None,
                tone: if snap.latest.value > 1.0 { "negative" } else { "neutral" }.to_string(),
            });
        }
    }

    // Fuel: lower is better.
    {
        let raw = get_cached_json::<Value>(&state.pool, SLOT_FUEL)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (snap, was_stale) = decode_optional::<FuelSnap>(raw)?;
        if let Some(snap) = snap {
            if snap.assembled_at_ms > assembled {
                assembled = snap.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            if let Some(first) = snap.rows.first() {
                tiles.push(MacroTile {
                    code: "FUEL".into(),
                    label: "Gas $/gal".into(),
                    value: format!("${:.2}", first.usd_per_gallon),
                    subline: Some(format!("WoW {:+.2}%", first.wow_change_pct)),
                    tone: tone_for_delta(first.wow_change_pct, false).to_string(),
                });
            }
        }
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    Ok(Json(MacroTilesResponse {
        tiles,
        assembled_at_ms: assembled,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::MACRO_TILES_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            MACRO_TILES_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_nothing_present() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(MACRO_TILES_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn composes_tile_when_one_slot_present() {
        let (app, pool) = migrated().await;
        let payload = serde_json::json!({
            "value": 3.8_f64,
            "prior": 3.6_f64,
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, SLOT_UNRATE, &Envelope::new(payload), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(MACRO_TILES_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: MacroTilesResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.tiles.len(), 1);
        assert_eq!(parsed.tiles[0].code, "UNRATE");
        assert_eq!(parsed.tiles[0].tone, "negative");
    }
}
