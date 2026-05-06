//! `GET /api/energy/v1/complex` — composer over the energy
//! complex (gas storage % full, SPR status, fuel-prices average).
//! Tolerates per-slot absence: returns whatever real signals are
//! cached today and an `availableTiles` count so the UI can show
//! partial state without inventing numbers.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::get_cached_json;

use crate::economic::v1::shared::{
    decode_optional, HandlerError, DEFAULT_RETRY_AFTER_SECS,
};
use crate::state::AppState;

pub const FUEL_PRICES_KEY: &str = "energy:fuel-prices:current:v1";
pub const GAS_STORAGE_KEY: &str = "energy:gie-gas-storage:current:v1";
pub const SPR_KEY: &str = "energy:spr-status:current:v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComplexTile {
    pub code: String,
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subline: Option<String>,
    /// `positive` | `negative` | `neutral`.
    pub tone: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComplexResponse {
    pub tiles: Vec<ComplexTile>,
    #[serde(rename = "availableTiles")]
    pub available_tiles: usize,
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    pub stale: bool,
}

#[derive(Debug, Deserialize)]
struct FuelSnap {
    #[serde(default)]
    rows: Vec<FuelRow>,
}

#[derive(Debug, Deserialize)]
struct FuelRow {
    #[serde(default, alias = "usdPerGallon", alias = "usd_per_gallon")]
    usd_per_gallon: f64,
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
) -> Result<Json<ComplexResponse>, HandlerError> {
    let raw_fuel = get_cached_json::<Value>(&state.pool, FUEL_PRICES_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_gas = get_cached_json::<Value>(&state.pool, GAS_STORAGE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let raw_spr = get_cached_json::<Value>(&state.pool, SPR_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (fuel, fuel_stale) = decode_optional::<FuelSnap>(raw_fuel)?;
    let (gas, gas_stale) = decode_optional::<GasSnap>(raw_gas)?;
    let (spr, spr_stale) = decode_optional::<SprSnap>(raw_spr)?;

    let mut tiles: Vec<ComplexTile> = Vec::new();
    if let Some(fuel) = fuel {
        if !fuel.rows.is_empty() {
            let avg = fuel.rows.iter().map(|r| r.usd_per_gallon).sum::<f64>()
                / fuel.rows.len() as f64;
            tiles.push(ComplexTile {
                code: "FUEL".into(),
                label: "Avg fuel price".into(),
                value: format!("${avg:.2}/gal"),
                subline: Some(format!("{} regions", fuel.rows.len())),
                tone: "neutral".into(),
            });
        }
    }
    if let Some(gas) = gas {
        if let Some(pct) = gas.average_full_pct {
            tiles.push(ComplexTile {
                code: "GAS-STORAGE".into(),
                label: "EU gas storage".into(),
                value: format!("{pct:.1}%"),
                subline: None,
                tone: tone_storage(pct),
            });
        }
    }
    if let Some(spr) = spr {
        if let Some(cur) = spr.current_mb {
            let delta = spr.delta_mb.unwrap_or(0.0);
            tiles.push(ComplexTile {
                code: "SPR".into(),
                label: "U.S. strategic reserve".into(),
                value: format!("{cur:.0} mb"),
                subline: Some(format!("{}{delta:.1} mb", if delta >= 0.0 { "+" } else { "" })),
                tone: if delta < 0.0 { "negative".into() } else { "positive".into() },
            });
        }
    }

    if tiles.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let available = tiles.len();
    let stale = fuel_stale || gas_stale || spr_stale;
    Ok(Json(ComplexResponse {
        tiles,
        available_tiles: available,
        assembled_at_ms: pellucid_core::now_ms(),
        stale,
    }))
}

/// EU gas-storage tone band: < 30% danger, > 80% positive, else neutral.
fn tone_storage(pct: f64) -> String {
    if pct < 30.0 {
        "negative".into()
    } else if pct > 80.0 {
        "positive".into()
    } else {
        "neutral".into()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::energy::v1::COMPLEX_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            COMPLEX_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_all_slots_empty() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COMPLEX_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_partial_when_one_slot_seeded() {
        let (app, pool) = migrated().await;
        let snap = serde_json::json!({ "average_full_pct": 25.0 });
        set_cached_json(&pool, GAS_STORAGE_KEY, &Envelope::new(snap), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(COMPLEX_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.pointer("/availableTiles").and_then(Value::as_u64), Some(1));
        assert_eq!(
            parsed.pointer("/tiles/0/code").and_then(Value::as_str),
            Some("GAS-STORAGE")
        );
        assert_eq!(
            parsed.pointer("/tiles/0/tone").and_then(Value::as_str),
            Some("negative")
        );
    }

    #[test]
    fn tone_storage_bands() {
        assert_eq!(tone_storage(20.0), "negative");
        assert_eq!(tone_storage(50.0), "neutral");
        assert_eq!(tone_storage(85.0), "positive");
    }
}
