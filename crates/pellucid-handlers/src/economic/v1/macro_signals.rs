//! `GET /api/economic/v1/macro-signals` — composer over FRED
//! latest + financial-stress slots; emits a small set of
//! up/down/flat signal rows the panel renders as alerts.

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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SignalDirection {
    Rising,
    Falling,
    Flat,
}

impl SignalDirection {
    /// Direction from a delta + a "flat band" tolerance. Pure.
    #[must_use]
    pub fn from_delta(delta: f64, flat_band: f64) -> Self {
        if !delta.is_finite() {
            return Self::Flat;
        }
        if delta > flat_band {
            Self::Rising
        } else if delta < -flat_band {
            Self::Falling
        } else {
            Self::Flat
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MacroSignal {
    pub code: String,
    pub direction: SignalDirection,
    pub headline: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroSignalsResponse {
    pub signals: Vec<MacroSignal>,
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

#[derive(Debug, Deserialize)]
struct FsiSnap {
    latest: FsiObs,
    #[serde(default)]
    prior: Option<FsiObs>,
    #[serde(default, alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct FsiObs {
    value: f64,
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

pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<MacroSignalsResponse>, HandlerError> {
    let mut signals: Vec<MacroSignal> = Vec::new();
    let mut assembled = 0_i64;
    let mut stale = false;

    // UNRATE + CPIAUCSL signals.
    for (code, slot, label, flat_band) in [
        ("UNRATE", SLOT_UNRATE, "Unemployment", 0.05_f64),
        ("CPIAUCSL", SLOT_CPIAUCSL, "CPI", 0.10_f64),
    ] {
        let raw = get_cached_json::<Value>(&state.pool, slot)
            .await
            .map_err(|e| HandlerError::Cache(e.to_string()))?;
        let (parsed, was_stale) = decode_optional::<LatestPair>(raw)?;
        if let Some(p) = parsed {
            if p.assembled_at_ms > assembled {
                assembled = p.assembled_at_ms;
            }
            if was_stale {
                stale = true;
            }
            if let Some((latest, prior)) = current_and_prior(&p) {
                let delta = match prior {
                    Some(prior_v) => latest - prior_v,
                    None => 0.0,
                };
                let direction = SignalDirection::from_delta(delta, flat_band);
                let arrow = match direction {
                    SignalDirection::Rising => "rising",
                    SignalDirection::Falling => "falling",
                    SignalDirection::Flat => "flat",
                };
                signals.push(MacroSignal {
                    code: code.to_string(),
                    direction,
                    headline: format!("{label} {arrow}"),
                    rationale: format!(
                        "Latest {latest:.2} vs prior {} ({delta:+.2})",
                        prior.map(|p| format!("{p:.2}")).unwrap_or_else(|| "—".to_string()),
                    ),
                });
            }
        }
    }

    // Financial stress.
    let raw = get_cached_json::<Value>(&state.pool, SLOT_FSI)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (fsi, was_stale) = decode_optional::<FsiSnap>(raw)?;
    if let Some(snap) = fsi {
        if snap.assembled_at_ms > assembled {
            assembled = snap.assembled_at_ms;
        }
        if was_stale {
            stale = true;
        }
        let delta = match snap.prior {
            Some(p) => snap.latest.value - p.value,
            None => 0.0,
        };
        let direction = SignalDirection::from_delta(delta, 0.05);
        let arrow = match direction {
            SignalDirection::Rising => "rising",
            SignalDirection::Falling => "falling",
            SignalDirection::Flat => "flat",
        };
        signals.push(MacroSignal {
            code: "STLFSI".into(),
            direction,
            headline: format!("Financial stress {arrow}"),
            rationale: format!("FSI {:.2} ({delta:+.2})", snap.latest.value),
        });
    }

    if signals.is_empty() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }
    Ok(Json(MacroSignalsResponse {
        signals,
        assembled_at_ms: assembled,
        stale,
    }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::economic::v1::MACRO_SIGNALS_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    #[test]
    fn signal_direction_from_delta() {
        assert_eq!(SignalDirection::from_delta(0.5, 0.1), SignalDirection::Rising);
        assert_eq!(SignalDirection::from_delta(-0.5, 0.1), SignalDirection::Falling);
        assert_eq!(SignalDirection::from_delta(0.05, 0.1), SignalDirection::Flat);
        assert_eq!(SignalDirection::from_delta(f64::NAN, 0.1), SignalDirection::Flat);
    }

    async fn migrated() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            MACRO_SIGNALS_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[tokio::test]
    async fn returns_503_when_nothing_present() {
        let (app, _) = migrated().await;
        let resp = app
            .oneshot(Request::builder().uri(MACRO_SIGNALS_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn emits_signal_when_unrate_present() {
        let (app, pool) = migrated().await;
        let payload = serde_json::json!({
            "value": 3.8_f64,
            "prior": 3.7_f64,
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, SLOT_UNRATE, &Envelope::new(payload), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(Request::builder().uri(MACRO_SIGNALS_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let parsed: MacroSignalsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.signals[0].code, "UNRATE");
        assert_eq!(parsed.signals[0].direction, SignalDirection::Rising);
    }
}
