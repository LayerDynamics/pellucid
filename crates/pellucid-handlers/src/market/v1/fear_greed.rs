//! `GET /api/market/v1/fear-greed` handler.
//!
//! Composite fear/greed index over FAST-tier market cache slots
//! the seeders already populate. Anonymous-tier — the response
//! is a small derived snapshot the webview's `FearGreedPanel`
//! (T4.2.6) renders as a 0–100 dial.
//!
//! ## Components
//!
//! Inspired by CNN's classic seven-input Fear & Greed Index but
//! restricted to inputs we already have a relay-side seeder for
//! (per SPEC-001 §24's H3 fix the handler MUST NOT call
//! upstreams synchronously). The four sub-indices:
//!
//! 1. **Volatility** — derived from `^VIX` price in
//!    `market:stocks-bootstrap:v1`. Low VIX = greedy, high
//!    VIX = fearful. Linearly mapped from VIX 12 → 100 and
//!    VIX 40 → 0, clamped at the ends.
//! 2. **Momentum** — fraction of basket symbols whose
//!    `percent_change > 0` over the same snapshot. 50 % is
//!    neutral.
//! 3. **Strength** — basket-weighted average percent change,
//!    centered at 0 % and scaled so ±2 % maps to 0/100.
//! 4. **Volume** — average ETF activity ratio in
//!    `market:etf-flows:current:v1`. Centered at 1.0 with
//!    ±0.5× mapping to 0/100.
//!
//! Composite is the unweighted mean of the available sub-
//! indices (we drop any whose source cache slot is missing).
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "score": 62,
//!   "label": "greed",
//!   "components": [
//!     { "name": "volatility", "score": 75, "label": "greed",
//!       "rationale": "VIX at 14.2" },
//!     …
//!   ],
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::market::v1::etf_flows::CACHE_KEY as ETF_FLOWS_CACHE_KEY;
use crate::market::v1::list_market_quotes::CACHE_KEY as STOCKS_CACHE_KEY;
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// VIX price below which the volatility sub-index is fully
/// greedy (score = 100).
pub const VIX_GREED_FLOOR: f64 = 12.0;

/// VIX price above which the volatility sub-index is fully
/// fearful (score = 0).
pub const VIX_FEAR_CEILING: f64 = 40.0;

/// Basket-average % change at which the strength sub-index hits
/// the 0 / 100 endpoints. ±this value clamps to fear / greed.
pub const STRENGTH_PCT_BAND: f64 = 2.0;

/// Distance from 1.0 at which the volume sub-index hits the 0 /
/// 100 endpoints. ±this value clamps to fear / greed.
pub const VOLUME_RATIO_BAND: f64 = 0.5;

/// Composite-label thresholds (inclusive on the upper bound of
/// each band, exclusive on the lower except for `Fear` which
/// catches everything ≤ 25).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SentimentLabel {
    /// score ≤ 25.
    ExtremeFear,
    /// 25 < score ≤ 45.
    Fear,
    /// 45 < score ≤ 55.
    Neutral,
    /// 55 < score ≤ 75.
    Greed,
    /// score > 75.
    ExtremeGreed,
}

impl SentimentLabel {
    /// Bucket a 0–100 score into a label. Pure.
    #[must_use]
    pub fn from_score(score: u8) -> Self {
        match score {
            0..=25 => Self::ExtremeFear,
            26..=45 => Self::Fear,
            46..=55 => Self::Neutral,
            56..=75 => Self::Greed,
            _ => Self::ExtremeGreed,
        }
    }
}

/// One sub-index in the response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Component {
    /// Stable component id (`volatility` / `momentum` /
    /// `strength` / `volume`).
    pub name: String,
    /// 0–100 sub-score.
    pub score: u8,
    /// Bucket label for this component alone.
    pub label: SentimentLabel,
    /// Short, human-readable explanation the panel renders next
    /// to the dial. Includes the input value the score was
    /// derived from so the user can sanity-check.
    pub rationale: String,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FearGreedResponse {
    /// Composite 0–100 score (unweighted mean of present
    /// components).
    pub score: u8,
    /// Bucket label for the composite score.
    pub label: SentimentLabel,
    /// Sub-index components present in this response. Always
    /// emitted in the order [volatility, momentum, strength,
    /// volume]; missing components are simply absent.
    pub components: Vec<Component>,
    /// Maximum of the upstream snapshots' `assembledAtMs` values.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when ANY contributing cache slot was stale.
    pub stale: bool,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// Both upstream cache slots are empty (we can't compute
    /// any component → the composite is undefined).
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    /// Stable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::Outage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl axum::response::IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let mut body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        if let Self::Outage { retry_after_secs } = &self {
            body["error"]["retry_after_secs"] = serde_json::Value::from(*retry_after_secs);
        }
        let status = self.status();
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        if let Self::Outage { retry_after_secs } = &self {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// Internal — minimal shape we need from the stocks snapshot.
#[derive(Debug, Deserialize)]
struct StocksRow {
    symbol: String,
    price: f64,
    percent_change: f64,
}

#[derive(Debug, Deserialize)]
struct StocksSnapshot {
    rows: Vec<StocksRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Internal — minimal shape we need from the ETF-flows snapshot.
#[derive(Debug, Deserialize)]
struct EtfRow {
    activity_ratio: f64,
}

#[derive(Debug, Deserialize)]
struct EtfSnapshot {
    rows: Vec<EtfRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Linearly project a value into the 0–100 score range, clamped
/// at the endpoints. `lo` maps to 0; `hi` maps to 100. Pure.
#[must_use]
pub fn linear_score(value: f64, lo: f64, hi: f64) -> u8 {
    if !value.is_finite() || (hi - lo).abs() < f64::EPSILON {
        return 50;
    }
    let raw = (value - lo) / (hi - lo) * 100.0;
    raw.clamp(0.0, 100.0).round() as u8
}

/// Score the volatility component from a VIX price. Lower VIX =
/// higher score. Pure.
#[must_use]
pub fn score_volatility(vix: f64) -> u8 {
    // Higher VIX → more fear → lower score.
    // VIX_GREED_FLOOR (12) → 100; VIX_FEAR_CEILING (40) → 0.
    linear_score(vix, VIX_FEAR_CEILING, VIX_GREED_FLOOR)
}

/// Score the momentum component from the % of advancers. Pure.
#[must_use]
pub fn score_momentum(advancers: usize, total: usize) -> u8 {
    if total == 0 {
        return 50;
    }
    let pct = (advancers as f64) / (total as f64) * 100.0;
    pct.clamp(0.0, 100.0).round() as u8
}

/// Score the strength component from a basket-average %
/// change. ±[`STRENGTH_PCT_BAND`] saturates the score. Pure.
#[must_use]
pub fn score_strength(avg_pct_change: f64) -> u8 {
    linear_score(avg_pct_change, -STRENGTH_PCT_BAND, STRENGTH_PCT_BAND)
}

/// Score the volume component from an average activity ratio.
/// 1.0 is neutral; ±[`VOLUME_RATIO_BAND`] saturates. Pure.
#[must_use]
pub fn score_volume(avg_activity_ratio: f64) -> u8 {
    linear_score(
        avg_activity_ratio,
        1.0 - VOLUME_RATIO_BAND,
        1.0 + VOLUME_RATIO_BAND,
    )
}

/// Compose the response from optional upstream snapshots. Pure
/// — exported so unit tests pin every aggregation boundary.
#[must_use]
pub fn compose(
    stocks: Option<StocksRollupOwned>,
    etf: Option<EtfRollupOwned>,
) -> Option<FearGreedResponse> {
    let mut components: Vec<Component> = Vec::new();
    let mut assembled_at_ms: i64 = 0;
    let mut stale = false;

    if let Some(s) = stocks.as_ref() {
        if s.assembled_at_ms > assembled_at_ms {
            assembled_at_ms = s.assembled_at_ms;
        }
        if s.stale {
            stale = true;
        }
        if let Some(vix) = s
            .rows
            .iter()
            .find(|r| r.symbol.eq_ignore_ascii_case("^VIX"))
        {
            let score = score_volatility(vix.price);
            components.push(Component {
                name: "volatility".into(),
                score,
                label: SentimentLabel::from_score(score),
                rationale: format!("VIX at {:.1}", vix.price),
            });
        }
        if !s.rows.is_empty() {
            let advancers = s.rows.iter().filter(|r| r.percent_change > 0.0).count();
            let total = s.rows.len();
            let m_score = score_momentum(advancers, total);
            components.push(Component {
                name: "momentum".into(),
                score: m_score,
                label: SentimentLabel::from_score(m_score),
                rationale: format!("{}/{} symbols advancing", advancers, total),
            });
            let avg_pct: f64 =
                s.rows.iter().map(|r| r.percent_change).sum::<f64>() / (total as f64);
            let st_score = score_strength(avg_pct);
            components.push(Component {
                name: "strength".into(),
                score: st_score,
                label: SentimentLabel::from_score(st_score),
                rationale: format!("Basket avg {:+.2}%", avg_pct),
            });
        }
    }

    if let Some(e) = etf.as_ref() {
        if e.assembled_at_ms > assembled_at_ms {
            assembled_at_ms = e.assembled_at_ms;
        }
        if e.stale {
            stale = true;
        }
        if !e.rows.is_empty() {
            let avg_ratio: f64 =
                e.rows.iter().map(|r| r.activity_ratio).sum::<f64>() / (e.rows.len() as f64);
            let v_score = score_volume(avg_ratio);
            components.push(Component {
                name: "volume".into(),
                score: v_score,
                label: SentimentLabel::from_score(v_score),
                rationale: format!("ETF activity {:.2}×", avg_ratio),
            });
        }
    }

    if components.is_empty() {
        return None;
    }
    let composite = (components.iter().map(|c| u32::from(c.score)).sum::<u32>() as f64
        / components.len() as f64)
        .round() as u8;

    Some(FearGreedResponse {
        score: composite,
        label: SentimentLabel::from_score(composite),
        components,
        assembled_at_ms,
        stale,
    })
}

/// Public mirror of the stocks rollup.
#[derive(Clone, Debug)]
pub struct StocksRollupOwned {
    /// Per-symbol rows.
    pub rows: Vec<StocksRowOwned>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
    /// True when the cache hit was a stale row.
    pub stale: bool,
}

/// Public mirror of one stocks-snapshot row.
#[derive(Clone, Debug)]
pub struct StocksRowOwned {
    /// Ticker.
    pub symbol: String,
    /// Last price.
    pub price: f64,
    /// Pre-computed percent change vs previous close.
    pub percent_change: f64,
}

/// Public mirror of the ETF-flows rollup.
#[derive(Clone, Debug)]
pub struct EtfRollupOwned {
    /// Per-row activity ratios.
    pub rows: Vec<EtfRowOwned>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
    /// True when the cache hit was a stale row.
    pub stale: bool,
}

/// Public mirror of one ETF-flows row.
#[derive(Clone, Debug)]
pub struct EtfRowOwned {
    /// Activity ratio (latest dollar volume / trailing average).
    pub activity_ratio: f64,
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<FearGreedResponse>, HandlerError> {
    let stocks_raw = get_cached_json::<Value>(&state.pool, STOCKS_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let etf_raw = get_cached_json::<Value>(&state.pool, ETF_FLOWS_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (stocks_p, stocks_stale) = decode_optional::<StocksSnapshot>(stocks_raw)?;
    let (etf_p, etf_stale) = decode_optional::<EtfSnapshot>(etf_raw)?;

    let stocks_owned = stocks_p.map(|p| StocksRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: stocks_stale,
        rows: p
            .rows
            .into_iter()
            .map(|r| StocksRowOwned {
                symbol: r.symbol,
                price: r.price,
                percent_change: r.percent_change,
            })
            .collect(),
    });
    let etf_owned = etf_p.map(|p| EtfRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: etf_stale,
        rows: p
            .rows
            .into_iter()
            .map(|r| EtfRowOwned {
                activity_ratio: r.activity_ratio,
            })
            .collect(),
    });

    match compose(stocks_owned, etf_owned) {
        Some(resp) => Ok(Json(resp)),
        None => Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        }),
    }
}

fn decode_optional<T>(raw: CacheHit<Value>) -> Result<(Option<T>, bool), HandlerError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => return Ok((None, false)),
    };
    let inner = unwrap_envelope_data(value);
    let parsed: T =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    Ok((Some(parsed), stale))
}

fn unwrap_envelope_data(v: Value) -> Value {
    if let Value::Object(map) = &v {
        if map.contains_key("_seed") {
            if let Some(inner) = map.get("data") {
                return inner.clone();
            }
        }
    }
    v
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::market::v1::FEAR_GREED_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            FEAR_GREED_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    fn stocks_snapshot(rows: &[(&str, f64, f64)]) -> Value {
        serde_json::json!({
            "rows": rows.iter().map(|(s, p, pct)| serde_json::json!({
                "symbol": s,
                "price": p,
                "previous_close": p / (1.0 + pct / 100.0),
                "percent_change": pct,
                "currency": "USD",
                "exchange": "PCX",
                "regular_market_time": 1_714_060_800_i64,
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    fn etf_snapshot(ratios: &[f64]) -> Value {
        serde_json::json!({
            "rows": ratios.iter().map(|r| serde_json::json!({
                "symbol": "X",
                "latest_dollar_volume": 1_000_000.0,
                "avg_dollar_volume": 1_000_000.0,
                "activity_ratio": r,
                "latest_session_ts": 1_714_060_800_i64,
            })).collect::<Vec<_>>(),
            "lookback_days": 20,
            "assembled_at_ms": 1_700_000_001_000_i64,
        })
    }

    #[test]
    fn sentiment_label_buckets() {
        assert_eq!(SentimentLabel::from_score(0), SentimentLabel::ExtremeFear);
        assert_eq!(SentimentLabel::from_score(25), SentimentLabel::ExtremeFear);
        assert_eq!(SentimentLabel::from_score(26), SentimentLabel::Fear);
        assert_eq!(SentimentLabel::from_score(45), SentimentLabel::Fear);
        assert_eq!(SentimentLabel::from_score(50), SentimentLabel::Neutral);
        assert_eq!(SentimentLabel::from_score(56), SentimentLabel::Greed);
        assert_eq!(SentimentLabel::from_score(75), SentimentLabel::Greed);
        assert_eq!(SentimentLabel::from_score(76), SentimentLabel::ExtremeGreed);
        assert_eq!(
            SentimentLabel::from_score(100),
            SentimentLabel::ExtremeGreed
        );
    }

    #[test]
    fn linear_score_clamps_at_endpoints_and_handles_zero_band() {
        assert_eq!(linear_score(0.0, 0.0, 100.0), 0);
        assert_eq!(linear_score(50.0, 0.0, 100.0), 50);
        assert_eq!(linear_score(100.0, 0.0, 100.0), 100);
        assert_eq!(linear_score(-50.0, 0.0, 100.0), 0);
        assert_eq!(linear_score(150.0, 0.0, 100.0), 100);
        // Reversed range — high lo / low hi means inputs above lo
        // are scored toward 0.
        assert_eq!(linear_score(40.0, 40.0, 12.0), 0);
        assert_eq!(linear_score(12.0, 40.0, 12.0), 100);
        // Zero-width band returns the neutral midpoint.
        assert_eq!(linear_score(0.0, 5.0, 5.0), 50);
        // Non-finite returns neutral midpoint.
        assert_eq!(linear_score(f64::NAN, 0.0, 100.0), 50);
    }

    #[test]
    fn score_volatility_is_inverted_to_vix_level() {
        assert_eq!(score_volatility(VIX_GREED_FLOOR), 100);
        assert_eq!(score_volatility(VIX_FEAR_CEILING), 0);
        assert!(score_volatility(20.0) > 0 && score_volatility(20.0) < 100);
        // Out-of-range clamps.
        assert_eq!(score_volatility(5.0), 100);
        assert_eq!(score_volatility(60.0), 0);
    }

    #[test]
    fn score_momentum_handles_zero_total_and_normal_cases() {
        assert_eq!(score_momentum(0, 0), 50);
        assert_eq!(score_momentum(0, 4), 0);
        assert_eq!(score_momentum(2, 4), 50);
        assert_eq!(score_momentum(4, 4), 100);
    }

    #[test]
    fn score_strength_centers_on_zero_pct() {
        assert_eq!(score_strength(0.0), 50);
        assert_eq!(score_strength(STRENGTH_PCT_BAND), 100);
        assert_eq!(score_strength(-STRENGTH_PCT_BAND), 0);
        assert_eq!(score_strength(STRENGTH_PCT_BAND * 5.0), 100);
    }

    #[test]
    fn score_volume_centers_on_one_x() {
        assert_eq!(score_volume(1.0), 50);
        assert_eq!(score_volume(1.0 + VOLUME_RATIO_BAND), 100);
        assert_eq!(score_volume(1.0 - VOLUME_RATIO_BAND), 0);
    }

    #[test]
    fn compose_returns_none_when_nothing_present() {
        assert!(compose(None, None).is_none());
    }

    #[test]
    fn compose_emits_volatility_when_vix_present() {
        let stocks = StocksRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![StocksRowOwned {
                symbol: "^VIX".into(),
                price: 12.0,
                percent_change: 0.0,
            }],
        };
        let resp = compose(Some(stocks), None).expect("composite");
        let vol = resp
            .components
            .iter()
            .find(|c| c.name == "volatility")
            .expect("volatility component");
        assert_eq!(vol.score, 100);
        assert!(vol.rationale.contains("12"));
    }

    #[test]
    fn compose_emits_momentum_and_strength_from_basket() {
        let stocks = StocksRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                StocksRowOwned {
                    symbol: "SPY".into(),
                    price: 100.0,
                    percent_change: 1.0,
                },
                StocksRowOwned {
                    symbol: "QQQ".into(),
                    price: 100.0,
                    percent_change: -1.0,
                },
            ],
        };
        let resp = compose(Some(stocks), None).expect("composite");
        let mom = resp
            .components
            .iter()
            .find(|c| c.name == "momentum")
            .unwrap();
        // 1/2 advancers → 50.
        assert_eq!(mom.score, 50);
        let st = resp
            .components
            .iter()
            .find(|c| c.name == "strength")
            .unwrap();
        // Avg 0% → 50.
        assert_eq!(st.score, 50);
    }

    #[test]
    fn compose_emits_volume_when_etf_present() {
        let etf = EtfRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                EtfRowOwned {
                    activity_ratio: 1.5,
                },
                EtfRowOwned {
                    activity_ratio: 1.5,
                },
            ],
        };
        let resp = compose(None, Some(etf)).expect("composite");
        let v = resp.components.iter().find(|c| c.name == "volume").unwrap();
        assert_eq!(v.score, 100);
    }

    #[test]
    fn compose_marks_stale_when_any_input_is_stale() {
        let stocks = StocksRollupOwned {
            assembled_at_ms: 5,
            stale: true,
            rows: vec![StocksRowOwned {
                symbol: "SPY".into(),
                price: 100.0,
                percent_change: 0.0,
            }],
        };
        let resp = compose(Some(stocks), None).expect("composite");
        assert!(resp.stale);
        assert_eq!(resp.assembled_at_ms, 5);
    }

    #[test]
    fn compose_assembled_at_is_max_of_inputs() {
        let stocks = StocksRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![StocksRowOwned {
                symbol: "SPY".into(),
                price: 100.0,
                percent_change: 0.0,
            }],
        };
        let etf = EtfRollupOwned {
            assembled_at_ms: 9,
            stale: false,
            rows: vec![EtfRowOwned {
                activity_ratio: 1.0,
            }],
        };
        let resp = compose(Some(stocks), Some(etf)).unwrap();
        assert_eq!(resp.assembled_at_ms, 9);
    }

    #[tokio::test]
    async fn handler_returns_503_when_both_caches_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEAR_GREED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_serves_when_only_stocks_present() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(stocks_snapshot(&[("^VIX", 12.0, 0.0), ("SPY", 100.0, 1.5)]));
        set_cached_json(&pool, STOCKS_CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEAR_GREED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: FearGreedResponse = serde_json::from_slice(&body).unwrap();
        // Three components present: volatility, momentum, strength.
        let names: Vec<&str> = parsed.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"volatility"));
        assert!(names.contains(&"momentum"));
        assert!(names.contains(&"strength"));
        assert!(!names.contains(&"volume"));
    }

    #[tokio::test]
    async fn handler_combines_both_caches_into_four_components() {
        let (app, pool) = migrated_router().await;
        let s = Envelope::new(stocks_snapshot(&[
            ("^VIX", 14.0, 0.0),
            ("SPY", 100.0, 1.0),
            ("QQQ", 100.0, 0.5),
        ]));
        let e = Envelope::new(etf_snapshot(&[1.2, 1.3, 1.4]));
        set_cached_json(&pool, STOCKS_CACHE_KEY, &s, 60_000)
            .await
            .unwrap();
        set_cached_json(&pool, ETF_FLOWS_CACHE_KEY, &e, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEAR_GREED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: FearGreedResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.components.len(), 4);
        // assembled_at_ms is the max of the two upstream stamps.
        assert_eq!(parsed.assembled_at_ms, 1_700_000_001_000);
    }

    #[tokio::test]
    async fn handler_marks_stale_when_either_cache_is_stale() {
        let (app, pool) = migrated_router().await;
        let s = Envelope::new(stocks_snapshot(&[("^VIX", 14.0, 0.0)]));
        set_cached_json(&pool, STOCKS_CACHE_KEY, &s, 0)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEAR_GREED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: FearGreedResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "rows": "not-an-array" }));
        set_cached_json(&pool, STOCKS_CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(FEAR_GREED_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}
