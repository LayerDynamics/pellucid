//! `GET /api/market/v1/backtest-stock` — tier-2 stock backtest.
//!
//! Pure cache reader. Given the FAST-tier
//! `market:stocks-bootstrap:v1` snapshot the handler synthesises
//! a deterministic backtest of one allocation strategy against
//! the cached basket as the universe.
//!
//! ## Why "synthesised"
//!
//! The FAST snapshot only carries today's price + previous
//! close per symbol. A real backtest needs a price history; the
//! plan's M4 follow-up wires `pellucid-correlation` + the OHLC
//! seed loop and replaces this handler's body with a true walk-
//! forward simulation. The wire shape stays the same so the
//! `StockBacktestPanel` (T4.2.3) doesn't re-render its chrome
//! when the M4 upgrade lands.
//!
//! Until then the synthesis is honest about its inputs:
//!
//!   - **Two-bar window** — every "bar" pair is the previous
//!     close → current price for one symbol.
//!   - **Three strategies** — `equal-weight`, `momentum`,
//!     `meanreversion`. Each picks weights from the same
//!     two-bar window; the panel can side-by-side compare them.
//!   - **Result columns** — total return %, win rate %, max
//!     drawdown %, plus per-symbol picks the strategy made.
//!
//! ## Tier
//!
//! Tier 2 (Pro). Same gate as `analyze-stock`; the gateway
//! middleware enforces, and the handler exposes
//! [`REQUIRED_TIER`] so a regression test pins the contract.
//!
//! ## M4 outage path
//!
//! Identical envelope to every other family handler.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;
use pellucid_gateway::traits::Tier;

use crate::market::v1::list_market_quotes::CACHE_KEY;
use crate::state::AppState;

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Tier required to call this route. The binary's
/// `RouteEntitlementRules::require()` call wires this to the
/// `/api/market/v1/backtest-stock` path.
pub const REQUIRED_TIER: Tier = Tier::Tier2;

/// Hard cap on `?limit_universe`. The default basket is 8
/// symbols; the cap leaves headroom for a custom basket without
/// letting a misconfigured panel drag the response weight up.
pub const MAX_UNIVERSE: usize = 50;

/// Default universe size — the seeder's basket is 8 symbols.
pub const DEFAULT_UNIVERSE: usize = 50;

/// Strategy selector — picks how `weights` is computed from the
/// two-bar window the seeder snapshot exposes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// Equal weight across every symbol.
    EqualWeight,
    /// Long the top half of percent-change movers; short the
    /// bottom half. Pure cross-sectional momentum.
    Momentum,
    /// Inverse momentum — long the bottom half, short the top.
    MeanReversion,
}

impl Strategy {
    /// Parse the wire-format string. Reject anything not in the
    /// fixed set so a typoed query param surfaces as 400 rather
    /// than silently fall through to the default.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "equal-weight" => Some(Self::EqualWeight),
            "momentum" => Some(Self::Momentum),
            "mean-reversion" => Some(Self::MeanReversion),
            _ => None,
        }
    }
}

/// Per-symbol pick — the weight the strategy assigned and the
/// per-symbol return contribution (weight * percent_change).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pick {
    /// Ticker.
    pub symbol: String,
    /// Strategy weight in `[-1, 1]`. Positive is long, negative
    /// is short. The full set sums to ~1 in absolute terms for
    /// equal-weight + ~0 for the long/short strategies.
    pub weight: f64,
    /// Symbol's percent change over the two-bar window.
    #[serde(rename = "percentChange")]
    pub percent_change: f64,
    /// Contribution to total return = `weight * percent_change`.
    pub contribution: f64,
}

/// Aggregated metrics for one strategy run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyMetrics {
    /// Sum of `pick.contribution` across the universe.
    #[serde(rename = "totalReturnPct")]
    pub total_return_pct: f64,
    /// Fraction of picks with `weight * percent_change > 0`,
    /// expressed as a percent in `[0, 100]`.
    #[serde(rename = "winRatePct")]
    pub win_rate_pct: f64,
    /// Worst single-symbol contribution (most negative). 0.0
    /// when no pick lost money.
    #[serde(rename = "maxDrawdownPct")]
    pub max_drawdown_pct: f64,
}

/// One strategy's result row — picks + metrics.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StrategyResult {
    /// Strategy id.
    pub strategy: Strategy,
    /// Picks the strategy made.
    pub picks: Vec<Pick>,
    /// Aggregated metrics.
    pub metrics: StrategyMetrics,
}

/// Wire-format response — three strategies side-by-side.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestStockResponse {
    /// Strategy results — `equal-weight`, `momentum`,
    /// `mean-reversion` in that order.
    pub strategies: Vec<StrategyResult>,
    /// Universe symbols the strategies operated on. Echoed so
    /// the panel can render the universe header without re-
    /// fetching `list-market-quotes`.
    pub universe: Vec<String>,
    /// Whether the underlying snapshot was stale.
    pub stale: bool,
    /// Wall-clock ms from the underlying snapshot's
    /// `assembledAtMs` field. The panel renders this as "as of
    /// 14:23 UTC" so users know how fresh the run is.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct BacktestStockQuery {
    /// Cap on universe size. Defaults to [`DEFAULT_UNIVERSE`],
    /// clamped to [`MAX_UNIVERSE`].
    #[serde(default, rename = "limitUniverse")]
    pub limit_universe: Option<usize>,
    /// Optional comma-separated allow-list of ticker symbols.
    /// Filters the universe before the strategies run.
    #[serde(default)]
    pub symbols: Option<String>,
}

/// Errors the handler can surface.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failed.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but the body did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// `?symbols` filter resulted in an empty universe.
    #[error("symbols filter excluded every cached row")]
    EmptyUniverse,
    /// M4 outage path: empty cache slot.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value emitted with the 503.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::EmptyUniverse => "empty_universe",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::EmptyUniverse => StatusCode::NOT_FOUND,
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

/// One row used by the strategies — symbol + percent change.
/// Decoupled from the cache wire shape so the strategies don't
/// depend on the snake_case / camelCase fields.
#[derive(Clone, Debug, PartialEq)]
pub struct UniverseRow {
    /// Ticker.
    pub symbol: String,
    /// Percent change over the two-bar window.
    pub percent_change: f64,
}

/// Cache payload shape — same reader the `list-market-quotes`
/// handler uses, expressed locally so we don't take a circular
/// dep on the sibling module's private struct.
#[derive(Debug, Deserialize)]
struct SnapshotPayload {
    rows: Vec<SeederQuoteRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct SeederQuoteRow {
    symbol: String,
    #[serde(default)]
    percent_change: f64,
}

/// Build the universe from the cached snapshot, applying the
/// `?symbols` allow-list + `?limit_universe` clamp. Pure —
/// exported for tests.
#[must_use]
pub fn build_universe(
    rows: Vec<UniverseRow>,
    q: &BacktestStockQuery,
) -> Vec<UniverseRow> {
    let mut filtered: Vec<UniverseRow> = if let Some(raw) = q.symbols.as_deref() {
        let allow: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if allow.is_empty() {
            rows
        } else {
            rows.into_iter()
                .filter(|r| allow.iter().any(|a| a == &r.symbol.to_ascii_uppercase()))
                .collect()
        }
    } else {
        rows
    };
    let limit = q
        .limit_universe
        .unwrap_or(DEFAULT_UNIVERSE)
        .clamp(1, MAX_UNIVERSE);
    if filtered.len() > limit {
        filtered.truncate(limit);
    }
    filtered
}

/// Run one strategy. Pure; deterministic given the same input.
#[must_use]
pub fn run_strategy(strategy: Strategy, universe: &[UniverseRow]) -> StrategyResult {
    let weights = compute_weights(strategy, universe);
    let picks: Vec<Pick> = universe
        .iter()
        .zip(weights.iter())
        .map(|(row, &w)| Pick {
            symbol: row.symbol.clone(),
            weight: w,
            percent_change: row.percent_change,
            contribution: w * row.percent_change,
        })
        .collect();
    let metrics = compute_metrics(&picks);
    StrategyResult {
        strategy,
        picks,
        metrics,
    }
}

/// Compute weights for one strategy.
fn compute_weights(strategy: Strategy, universe: &[UniverseRow]) -> Vec<f64> {
    let n = universe.len();
    if n == 0 {
        return Vec::new();
    }
    match strategy {
        Strategy::EqualWeight => {
            let w = 1.0 / (n as f64);
            vec![w; n]
        }
        Strategy::Momentum => long_short_by_percent(universe, /*long_top=*/ true),
        Strategy::MeanReversion => long_short_by_percent(universe, /*long_top=*/ false),
    }
}

/// Long/short bisection by `percent_change`. When `long_top` is
/// true we go long the top half; otherwise we go long the
/// bottom half. Symmetric weights so the absolute exposure is 1
/// across both legs.
fn long_short_by_percent(universe: &[UniverseRow], long_top: bool) -> Vec<f64> {
    let n = universe.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        // One symbol — no two-sided strategy is possible. Express
        // the strategy intent: long if it's "the top" (long_top
        // or not — there's only one row; pick long for momentum,
        // short for mean-reversion to keep the directionality).
        return vec![if long_top { 1.0 } else { -1.0 }];
    }
    let mut sorted: Vec<usize> = (0..n).collect();
    sorted.sort_by(|&a, &b| {
        universe[b]
            .percent_change
            .partial_cmp(&universe[a].percent_change)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mid = n / 2;
    let half = mid as f64;
    let other = (n - mid) as f64;
    // Top weight: +1/half (long_top) or -1/half (mean-reversion).
    // Bottom weight: -1/other (long_top) or +1/other.
    let top_w = if long_top { 1.0 / half } else { -1.0 / half };
    let bot_w = if long_top { -1.0 / other } else { 1.0 / other };
    let mut weights = vec![0.0_f64; n];
    for (rank, &idx) in sorted.iter().enumerate() {
        weights[idx] = if rank < mid { top_w } else { bot_w };
    }
    weights
}

/// Aggregate per-pick contributions into the strategy metrics.
fn compute_metrics(picks: &[Pick]) -> StrategyMetrics {
    if picks.is_empty() {
        return StrategyMetrics {
            total_return_pct: 0.0,
            win_rate_pct: 0.0,
            max_drawdown_pct: 0.0,
        };
    }
    let total: f64 = picks.iter().map(|p| p.contribution).sum();
    let wins = picks.iter().filter(|p| p.contribution > 0.0).count();
    let win_rate = (wins as f64) / (picks.len() as f64) * 100.0;
    let drawdown = picks
        .iter()
        .map(|p| p.contribution)
        .fold(0.0_f64, |acc, c| if c < acc { c } else { acc });
    StrategyMetrics {
        total_return_pct: total,
        win_rate_pct: win_rate,
        max_drawdown_pct: drawdown,
    }
}

/// Run all three strategies. Pure.
#[must_use]
pub fn backtest(universe: &[UniverseRow]) -> Vec<StrategyResult> {
    vec![
        run_strategy(Strategy::EqualWeight, universe),
        run_strategy(Strategy::Momentum, universe),
        run_strategy(Strategy::MeanReversion, universe),
    ]
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<BacktestStockQuery>,
) -> Result<Json<BacktestStockResponse>, HandlerError> {
    let raw: CacheHit<Value> = get_cached_json::<Value>(&state.pool, CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => {
            return Err(HandlerError::Outage {
                retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
            });
        }
    };
    let inner = unwrap_envelope_data(value);
    let payload: SnapshotPayload =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    let rows: Vec<UniverseRow> = payload
        .rows
        .into_iter()
        .map(|r| UniverseRow {
            symbol: r.symbol,
            percent_change: r.percent_change,
        })
        .collect();
    let universe = build_universe(rows, &q);
    if universe.is_empty() {
        return Err(HandlerError::EmptyUniverse);
    }
    let universe_symbols: Vec<String> =
        universe.iter().map(|r| r.symbol.clone()).collect();
    let strategies = backtest(&universe);
    Ok(Json(BacktestStockResponse {
        strategies,
        universe: universe_symbols,
        stale,
        assembled_at_ms: payload.assembled_at_ms,
    }))
}

/// Same envelope-unwrap helper used by every cache reader.
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    use crate::market::v1::BACKTEST_STOCK_PATH;

    fn row(symbol: &str, pct: f64) -> UniverseRow {
        UniverseRow {
            symbol: symbol.into(),
            percent_change: pct,
        }
    }

    fn snapshot(rows: Vec<(&str, f64)>) -> serde_json::Value {
        serde_json::json!({
            "rows": rows.into_iter().map(|(s, p)| serde_json::json!({
                "symbol": s,
                "price": 100.0,
                "previous_close": 100.0,
                "percent_change": p,
                "currency": "USD",
                "exchange": "NMS",
                "regular_market_time": 1_700_000_000,
            })).collect::<Vec<_>>(),
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app = axum::Router::new().route(
            BACKTEST_STOCK_PATH,
            axum::routing::get(handler).with_state(state),
        );
        (app, pool)
    }

    #[test]
    fn strategy_parse_accepts_only_known_strings() {
        assert_eq!(Strategy::parse("equal-weight"), Some(Strategy::EqualWeight));
        assert_eq!(Strategy::parse("momentum"), Some(Strategy::Momentum));
        assert_eq!(
            Strategy::parse("mean-reversion"),
            Some(Strategy::MeanReversion),
        );
        assert_eq!(Strategy::parse("EqualWeight"), None);
        assert_eq!(Strategy::parse(""), None);
    }

    #[test]
    fn equal_weight_assigns_one_over_n_to_every_pick() {
        let universe = vec![row("A", 1.0), row("B", -2.0), row("C", 0.5), row("D", 0.0)];
        let result = run_strategy(Strategy::EqualWeight, &universe);
        for pick in &result.picks {
            assert!((pick.weight - 0.25).abs() < 1e-9);
        }
        // Sum of weights == 1 (long-only).
        let sum: f64 = result.picks.iter().map(|p| p.weight).sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn momentum_longs_top_half_shorts_bottom_half() {
        let universe = vec![
            row("WIN", 3.0),
            row("LOSE", -3.0),
            row("MID_UP", 1.0),
            row("MID_DN", -1.0),
        ];
        let result = run_strategy(Strategy::Momentum, &universe);
        let by_symbol = |s: &str| -> &Pick {
            result.picks.iter().find(|p| p.symbol == s).unwrap()
        };
        assert!(by_symbol("WIN").weight > 0.0);
        assert!(by_symbol("MID_UP").weight > 0.0);
        assert!(by_symbol("MID_DN").weight < 0.0);
        assert!(by_symbol("LOSE").weight < 0.0);
    }

    #[test]
    fn mean_reversion_inverts_momentum_weights() {
        let universe = vec![row("WIN", 3.0), row("LOSE", -3.0)];
        let mom = run_strategy(Strategy::Momentum, &universe);
        let mr = run_strategy(Strategy::MeanReversion, &universe);
        let mom_win = mom.picks.iter().find(|p| p.symbol == "WIN").unwrap();
        let mr_win = mr.picks.iter().find(|p| p.symbol == "WIN").unwrap();
        // Same magnitudes, flipped signs.
        assert!((mom_win.weight + mr_win.weight).abs() < 1e-9);
    }

    #[test]
    fn metrics_total_return_is_sum_of_contributions() {
        let universe = vec![row("A", 2.0), row("B", -1.0)];
        let result = run_strategy(Strategy::EqualWeight, &universe);
        let expected: f64 = result.picks.iter().map(|p| p.contribution).sum();
        assert!((result.metrics.total_return_pct - expected).abs() < 1e-9);
    }

    #[test]
    fn metrics_win_rate_counts_positive_contributions() {
        let universe = vec![row("A", 2.0), row("B", -1.0), row("C", 4.0)];
        // Equal-weight + percent_change > 0 → contribution > 0.
        let result = run_strategy(Strategy::EqualWeight, &universe);
        // 2 of 3 picks have positive contribution.
        assert!((result.metrics.win_rate_pct - (200.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn metrics_max_drawdown_is_most_negative_contribution() {
        let universe = vec![row("A", 2.0), row("B", -10.0), row("C", -1.0)];
        let result = run_strategy(Strategy::EqualWeight, &universe);
        // Equal-weight = 1/3; worst contribution is -10/3 ≈ -3.333.
        assert!((result.metrics.max_drawdown_pct + 10.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn metrics_zero_drawdown_when_no_losers() {
        let universe = vec![row("A", 2.0), row("B", 1.0)];
        let result = run_strategy(Strategy::EqualWeight, &universe);
        assert_eq!(result.metrics.max_drawdown_pct, 0.0);
    }

    #[test]
    fn build_universe_filters_by_symbols_csv_case_insensitive() {
        let rows = vec![
            row("SPY", 0.5),
            row("QQQ", -0.2),
            row("DIA", 0.1),
        ];
        let q = BacktestStockQuery {
            limit_universe: None,
            symbols: Some("spy, dia".into()),
        };
        let out = build_universe(rows, &q);
        assert_eq!(out.iter().map(|r| r.symbol.as_str()).collect::<Vec<_>>(), vec!["SPY", "DIA"]);
    }

    #[test]
    fn build_universe_clamps_limit_universe() {
        let rows: Vec<UniverseRow> = (0..MAX_UNIVERSE + 5)
            .map(|i| row(&format!("S{i}"), 0.0))
            .collect();
        let q = BacktestStockQuery {
            limit_universe: Some(MAX_UNIVERSE * 10),
            symbols: None,
        };
        let out = build_universe(rows, &q);
        assert_eq!(out.len(), MAX_UNIVERSE);
    }

    #[test]
    fn handler_error_status_codes() {
        assert_eq!(
            HandlerError::Cache("x".into()).status(),
            Code::BAD_GATEWAY,
        );
        assert_eq!(
            HandlerError::Shape("x".into()).status(),
            Code::BAD_GATEWAY,
        );
        assert_eq!(HandlerError::EmptyUniverse.status(), Code::NOT_FOUND);
        assert_eq!(
            HandlerError::Outage { retry_after_secs: 30 }.status(),
            Code::SERVICE_UNAVAILABLE,
        );
    }

    #[test]
    fn required_tier_is_two_per_plan() {
        assert_eq!(REQUIRED_TIER, Tier::Tier2);
    }

    #[tokio::test]
    async fn handler_returns_503_when_cache_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BACKTEST_STOCK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_returns_404_when_symbols_filter_excludes_everything() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![("SPY", 0.5)]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{}?symbols=NVDA", BACKTEST_STOCK_PATH))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed.pointer("/error/code").and_then(serde_json::Value::as_str),
            Some("empty_universe"),
        );
    }

    #[tokio::test]
    async fn handler_returns_three_strategies_for_populated_cache() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(snapshot(vec![
            ("SPY", 0.5),
            ("QQQ", -0.2),
            ("DIA", 0.1),
            ("IWM", -0.4),
        ]));
        set_cached_json(&pool, CACHE_KEY, &env, 60_000).await.unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(BACKTEST_STOCK_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: BacktestStockResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.strategies.len(), 3);
        assert_eq!(parsed.universe.len(), 4);
        assert_eq!(parsed.strategies[0].strategy, Strategy::EqualWeight);
        assert_eq!(parsed.strategies[1].strategy, Strategy::Momentum);
        assert_eq!(parsed.strategies[2].strategy, Strategy::MeanReversion);
        assert!(!parsed.stale);
        assert_eq!(parsed.assembled_at_ms, 1_700_000_000_000);
    }
}
