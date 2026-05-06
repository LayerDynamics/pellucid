//! `GET /api/market/v1/daily-brief` handler.
//!
//! Composer over existing FAST market cache slots — synthesises
//! a deterministic, human-readable daily-brief payload the
//! webview's `DailyMarketBriefPanel` (T4.2.12) renders. Per
//! SPEC-001 §24's H3 fix the handler is a pure cache reader;
//! the brief is real prose composed from the cached numbers,
//! not an ML inference.
//!
//! ## Inputs
//!
//! - `market:stocks-bootstrap:v1` — basket avg % change, top
//!   gainer / top loser, VIX level.
//! - `market:crypto-snapshot:v1` — crypto-basket avg %.
//! - `market:commodities-snapshot:v1` — commodities-basket
//!   avg %.
//!
//! Missing inputs are simply elided from the brief lines (the
//! response always emits an array — empty when nothing is
//! present, which 503s with the same M4 envelope as every
//! other handler).

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key — stocks snapshot.
pub const STOCKS_CACHE_KEY: &str = "market:stocks-bootstrap:v1";
/// Cache key — crypto snapshot.
pub const CRYPTO_CACHE_KEY: &str = "market:crypto-snapshot:v1";
/// Cache key — commodities snapshot.
pub const COMMODITIES_CACHE_KEY: &str = "market:commodities-snapshot:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Sentiment label — same set as the fear-greed handler.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DayTone {
    /// Avg % change ≥ +0.50.
    StrongUp,
    /// Avg % change in `[+0.10, +0.50)`.
    Up,
    /// Avg % change in `(-0.10, +0.10)`.
    Mixed,
    /// Avg % change in `(-0.50, -0.10]`.
    Down,
    /// Avg % change ≤ -0.50.
    StrongDown,
}

impl DayTone {
    /// Bucket an average % change into a tone label. Pure.
    #[must_use]
    pub fn from_avg_pct(avg: f64) -> Self {
        if !avg.is_finite() {
            return Self::Mixed;
        }
        if avg >= 0.50 {
            Self::StrongUp
        } else if avg >= 0.10 {
            Self::Up
        } else if avg > -0.10 {
            Self::Mixed
        } else if avg > -0.50 {
            Self::Down
        } else {
            Self::StrongDown
        }
    }
}

/// One brief line — short rendered string + the structured
/// inputs that drove it (so the panel can hover / cite).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BriefLine {
    /// Section id (`stocks` / `crypto` / `commodities` / `vix`).
    pub section: String,
    /// Tone bucket.
    pub tone: DayTone,
    /// Rendered headline.
    pub headline: String,
    /// Optional rationale containing the input numbers.
    pub rationale: String,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DailyBriefResponse {
    /// One line per available section.
    pub lines: Vec<BriefLine>,
    /// Maximum of all contributing snapshots' assembledAtMs.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when ANY contributing slot was stale.
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
    /// All input cache slots empty.
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

/// Stocks-snapshot row — the only fields we consume.
#[derive(Debug, Deserialize, Clone)]
struct StocksRow {
    symbol: String,
    price: f64,
    percent_change: f64,
}

#[derive(Debug, Deserialize)]
struct StocksPayload {
    rows: Vec<StocksRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Crypto-snapshot row.
#[derive(Debug, Deserialize)]
struct CryptoRow {
    usd_24h_change: f64,
}

#[derive(Debug, Deserialize)]
struct CryptoPayload {
    rows: Vec<CryptoRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Commodities-snapshot row — uses the same Yahoo-quote shape
/// as the stocks snapshot.
#[derive(Debug, Deserialize)]
struct CommodityRow {
    percent_change: f64,
}

#[derive(Debug, Deserialize)]
struct CommodityPayload {
    rows: Vec<CommodityRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Public mirror of the stocks rollup.
#[derive(Clone, Debug)]
pub struct StocksRollupOwned {
    /// Symbol-level rows.
    pub rows: Vec<StocksRowOwned>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
    /// Stale flag.
    pub stale: bool,
}

/// One stocks row.
#[derive(Clone, Debug)]
pub struct StocksRowOwned {
    /// Ticker.
    pub symbol: String,
    /// Last price.
    pub price: f64,
    /// Pre-computed percent change.
    pub percent_change: f64,
}

/// Public mirror of a generic basket rollup (crypto / commodities).
#[derive(Clone, Debug)]
pub struct BasketRollupOwned {
    /// Per-row percent changes.
    pub percent_changes: Vec<f64>,
    /// Wall-clock ms.
    pub assembled_at_ms: i64,
    /// Stale flag.
    pub stale: bool,
}

/// Compose the brief from optional inputs. Pure — exported so
/// unit tests pin the line shapes.
#[must_use]
pub fn compose(
    stocks: Option<StocksRollupOwned>,
    crypto: Option<BasketRollupOwned>,
    commodities: Option<BasketRollupOwned>,
) -> Option<DailyBriefResponse> {
    let mut lines: Vec<BriefLine> = Vec::new();
    let mut assembled = 0_i64;
    let mut stale = false;

    if let Some(s) = stocks.as_ref() {
        if s.assembled_at_ms > assembled {
            assembled = s.assembled_at_ms;
        }
        if s.stale {
            stale = true;
        }
        let basket: Vec<&StocksRowOwned> = s
            .rows
            .iter()
            .filter(|r| !r.symbol.eq_ignore_ascii_case("^VIX"))
            .collect();
        if !basket.is_empty() {
            let avg = basket.iter().map(|r| r.percent_change).sum::<f64>() / basket.len() as f64;
            let mut sorted = basket.clone();
            sorted.sort_by(|a, b| {
                b.percent_change
                    .partial_cmp(&a.percent_change)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let gainer = sorted.first().copied();
            let loser = sorted.last().copied();
            let tone = DayTone::from_avg_pct(avg);
            let headline = match tone {
                DayTone::StrongUp => "Stocks broadly up",
                DayTone::Up => "Stocks edging up",
                DayTone::Mixed => "Stocks mixed",
                DayTone::Down => "Stocks edging down",
                DayTone::StrongDown => "Stocks broadly down",
            };
            let mut rationale = format!("Basket avg {avg:+.2}% across {} symbols", basket.len());
            if let (Some(g), Some(l)) = (gainer, loser) {
                rationale.push_str(&format!(
                    " (top gainer {} {:+.2}%, top loser {} {:+.2}%)",
                    g.symbol, g.percent_change, l.symbol, l.percent_change,
                ));
            }
            lines.push(BriefLine {
                section: "stocks".into(),
                tone,
                headline: headline.into(),
                rationale,
            });
        }
        if let Some(vix) = s
            .rows
            .iter()
            .find(|r| r.symbol.eq_ignore_ascii_case("^VIX"))
        {
            let level = vix.price;
            let (tone, headline) = match level {
                _ if level <= 14.0 => (DayTone::StrongUp, "Volatility very low"),
                _ if level <= 18.0 => (DayTone::Up, "Volatility low"),
                _ if level <= 22.0 => (DayTone::Mixed, "Volatility neutral"),
                _ if level <= 30.0 => (DayTone::Down, "Volatility elevated"),
                _ => (DayTone::StrongDown, "Volatility severe"),
            };
            lines.push(BriefLine {
                section: "vix".into(),
                tone,
                headline: headline.into(),
                rationale: format!("VIX at {level:.1}"),
            });
        }
    }

    if let Some(c) = crypto.as_ref() {
        if c.assembled_at_ms > assembled {
            assembled = c.assembled_at_ms;
        }
        if c.stale {
            stale = true;
        }
        if !c.percent_changes.is_empty() {
            let avg = c.percent_changes.iter().sum::<f64>() / c.percent_changes.len() as f64;
            let tone = DayTone::from_avg_pct(avg);
            let headline = match tone {
                DayTone::StrongUp => "Crypto rallying",
                DayTone::Up => "Crypto edging up",
                DayTone::Mixed => "Crypto mixed",
                DayTone::Down => "Crypto edging down",
                DayTone::StrongDown => "Crypto selling off",
            };
            lines.push(BriefLine {
                section: "crypto".into(),
                tone,
                headline: headline.into(),
                rationale: format!(
                    "Crypto basket 24h avg {avg:+.2}% across {} tokens",
                    c.percent_changes.len()
                ),
            });
        }
    }

    if let Some(co) = commodities.as_ref() {
        if co.assembled_at_ms > assembled {
            assembled = co.assembled_at_ms;
        }
        if co.stale {
            stale = true;
        }
        if !co.percent_changes.is_empty() {
            let avg = co.percent_changes.iter().sum::<f64>() / co.percent_changes.len() as f64;
            let tone = DayTone::from_avg_pct(avg);
            let headline = match tone {
                DayTone::StrongUp => "Commodities firming",
                DayTone::Up => "Commodities edging up",
                DayTone::Mixed => "Commodities mixed",
                DayTone::Down => "Commodities edging down",
                DayTone::StrongDown => "Commodities sliding",
            };
            lines.push(BriefLine {
                section: "commodities".into(),
                tone,
                headline: headline.into(),
                rationale: format!(
                    "Commodities basket avg {avg:+.2}% across {} contracts",
                    co.percent_changes.len()
                ),
            });
        }
    }

    if lines.is_empty() {
        return None;
    }
    Some(DailyBriefResponse {
        lines,
        assembled_at_ms: assembled,
        stale,
    })
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
) -> Result<Json<DailyBriefResponse>, HandlerError> {
    let stocks_raw = get_cached_json::<Value>(&state.pool, STOCKS_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let crypto_raw = get_cached_json::<Value>(&state.pool, CRYPTO_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let commodities_raw = get_cached_json::<Value>(&state.pool, COMMODITIES_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (stocks_p, stocks_stale) = decode_optional::<StocksPayload>(stocks_raw)?;
    let (crypto_p, crypto_stale) = decode_optional::<CryptoPayload>(crypto_raw)?;
    let (commod_p, commod_stale) = decode_optional::<CommodityPayload>(commodities_raw)?;

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
    let crypto_owned = crypto_p.map(|p| BasketRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: crypto_stale,
        percent_changes: p.rows.into_iter().map(|r| r.usd_24h_change).collect(),
    });
    let commod_owned = commod_p.map(|p| BasketRollupOwned {
        assembled_at_ms: p.assembled_at_ms,
        stale: commod_stale,
        percent_changes: p.rows.into_iter().map(|r| r.percent_change).collect(),
    });

    match compose(stocks_owned, crypto_owned, commod_owned) {
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
    use crate::market::v1::DAILY_BRIEF_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            DAILY_BRIEF_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn day_tone_buckets() {
        assert_eq!(DayTone::from_avg_pct(1.0), DayTone::StrongUp);
        assert_eq!(DayTone::from_avg_pct(0.5), DayTone::StrongUp);
        assert_eq!(DayTone::from_avg_pct(0.3), DayTone::Up);
        assert_eq!(DayTone::from_avg_pct(0.0), DayTone::Mixed);
        assert_eq!(DayTone::from_avg_pct(-0.3), DayTone::Down);
        assert_eq!(DayTone::from_avg_pct(-1.0), DayTone::StrongDown);
        assert_eq!(DayTone::from_avg_pct(f64::NAN), DayTone::Mixed);
    }

    #[test]
    fn compose_returns_none_when_nothing_present() {
        assert!(compose(None, None, None).is_none());
    }

    #[test]
    fn compose_emits_stocks_and_vix_lines_when_present() {
        let stocks = StocksRollupOwned {
            assembled_at_ms: 1,
            stale: false,
            rows: vec![
                StocksRowOwned {
                    symbol: "SPY".into(),
                    price: 524.0,
                    percent_change: 0.6,
                },
                StocksRowOwned {
                    symbol: "QQQ".into(),
                    price: 460.0,
                    percent_change: 0.4,
                },
                StocksRowOwned {
                    symbol: "^VIX".into(),
                    price: 13.0,
                    percent_change: -2.0,
                },
            ],
        };
        let resp = compose(Some(stocks), None, None).expect("brief");
        let sections: Vec<&str> = resp.lines.iter().map(|l| l.section.as_str()).collect();
        assert!(sections.contains(&"stocks"));
        assert!(sections.contains(&"vix"));
        let stocks_line = resp.lines.iter().find(|l| l.section == "stocks").unwrap();
        assert_eq!(stocks_line.tone, DayTone::StrongUp);
    }

    #[test]
    fn compose_emits_crypto_line_when_present() {
        let crypto = BasketRollupOwned {
            assembled_at_ms: 5,
            stale: false,
            percent_changes: vec![3.0, -1.0, 2.0],
        };
        let resp = compose(None, Some(crypto), None).unwrap();
        assert_eq!(resp.lines.len(), 1);
        assert_eq!(resp.lines[0].section, "crypto");
        assert_eq!(resp.assembled_at_ms, 5);
    }

    #[test]
    fn compose_marks_stale_when_any_input_stale() {
        let crypto = BasketRollupOwned {
            assembled_at_ms: 5,
            stale: true,
            percent_changes: vec![1.0],
        };
        assert!(compose(None, Some(crypto), None).unwrap().stale);
    }

    #[tokio::test]
    async fn handler_returns_503_when_all_caches_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(DAILY_BRIEF_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_serves_when_only_stocks_present() {
        let (app, pool) = migrated_router().await;
        let stocks = serde_json::json!({
            "rows": [
                {
                    "symbol": "SPY", "price": 524.0,
                    "previous_close": 522.0, "percent_change": 0.4,
                    "currency": "USD", "exchange": "PCX",
                    "regular_market_time": 1_714_060_800_i64,
                },
                {
                    "symbol": "^VIX", "price": 13.5,
                    "previous_close": 14.0, "percent_change": -3.5,
                    "currency": "USD", "exchange": "CCY",
                    "regular_market_time": 1_714_060_800_i64,
                },
            ],
            "assembled_at_ms": 1_700_000_000_000_i64,
        });
        set_cached_json(&pool, STOCKS_CACHE_KEY, &Envelope::new(stocks), 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(DAILY_BRIEF_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: DailyBriefResponse = serde_json::from_slice(&body).unwrap();
        let sections: Vec<&str> = parsed.lines.iter().map(|l| l.section.as_str()).collect();
        assert!(sections.contains(&"stocks"));
        assert!(sections.contains(&"vix"));
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
                    .uri(DAILY_BRIEF_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}
