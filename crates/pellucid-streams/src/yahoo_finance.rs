//! Yahoo Finance v8 `chart` endpoint client — universal price
//! source for stocks, indices, ETFs, commodity futures, FX, and
//! crypto.
//!
//! The v8 `chart` endpoint shape (production):
//! ```text
//! GET https://query1.finance.yahoo.com/v8/finance/chart/{symbol}
//!     ?interval=1d&range=5d
//! ```
//!
//! Response (relevant subset):
//! ```json
//! { "chart": { "result": [ {
//!     "meta": {
//!         "symbol":               "SPY",
//!         "regularMarketPrice":   524.31,
//!         "previousClose":        522.10,
//!         "currency":             "USD",
//!         "exchangeName":         "PCX",
//!         "regularMarketTime":    1714060800
//!     },
//!     "timestamp":  [ 1713888000, 1713974400, ... ],
//!     "indicators": { "quote": [ {
//!         "close": [ 521.4, 522.1, ... ]
//!     } ] }
//! } ], "error": null } }
//! ```
//!
//! Yahoo's v7 multi-symbol `quote` endpoint now requires a
//! crumb-cookie pair. The v8 `chart` endpoint does NOT — it
//! accepts unauthenticated GETs from any client that sends a
//! browser-shaped User-Agent. We send one `Chrome/121` UA and
//! issue one HTTP call per symbol.
//!
//! `YahooFinanceClient::fetch_quotes` fans out N parallel chart
//! calls and returns a `Vec<YahooQuote>` in input order, with
//! `None` for symbols whose upstream returned no rows.

use std::time::Duration;

use futures::future::join_all;
use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — `https://query1.finance.yahoo.com`.
pub const DEFAULT_BASE_URL: &str = "https://query1.finance.yahoo.com";

/// Default per-request timeout — 8 s. Yahoo's chart endpoint
/// usually responds in < 500 ms; 8 s is a generous ceiling that
/// still keeps a single bad fan-out from blocking the seeder.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Browser-shaped User-Agent. The chart endpoint rejects
/// unmarked Rust clients (`reqwest/0.x`) with 401, so every
/// `pellucid-streams` Yahoo call sends this UA verbatim.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36";

/// Configuration for the Yahoo Finance client.
#[derive(Clone, Debug)]
pub struct YahooFinanceConfig {
    /// Base URL — no trailing slash. Tests inject a `wiremock`
    /// server URI; production uses [`DEFAULT_BASE_URL`].
    pub base_url: String,
    /// Per-request timeout passed through to `reqwest`.
    pub timeout: Duration,
    /// User-Agent header — defaults to [`DEFAULT_USER_AGENT`].
    pub user_agent: String,
}

impl Default for YahooFinanceConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Yahoo Finance v8 chart client.
#[derive(Clone, Debug)]
pub struct YahooFinanceClient {
    http: reqwest::Client,
    config: YahooFinanceConfig,
}

impl YahooFinanceClient {
    /// Build a client. The supplied `reqwest::Client` is used
    /// for every fan-out call; pass one with connection pooling
    /// configured (the production binaries do).
    #[must_use]
    pub fn new(config: YahooFinanceConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor against the production base URL
    /// with a default `reqwest::Client` (timeout-only configured).
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if `reqwest` fails to build
    /// the underlying client (rare — only if rustls fails to
    /// initialise).
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = YahooFinanceConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the latest quote for one symbol. Returns
    /// `Ok(None)` when the upstream's `chart.result[]` is empty
    /// (Yahoo's "no such symbol" response shape).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for unparseable bodies.
    pub async fn fetch_quote(&self, symbol: &str) -> Result<Option<YahooQuote>, StreamsError> {
        let url = self.build_chart_url(symbol)?;
        let resp = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: ChartResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        if let Some(err) = body.chart.error {
            return Err(StreamsError::Parse(format!(
                "yahoo error: code={} description={}",
                err.code, err.description
            )));
        }
        let Some(first) = body.chart.result.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(YahooQuote::from_chart(first, symbol)))
    }

    /// Fan out one quote call per symbol concurrently. The
    /// returned vector has the same length as `symbols` and is
    /// ordered identically; `None` entries are propagated from
    /// `fetch_quote` (`Ok(None)`) and from per-symbol errors —
    /// callers that need to distinguish error kinds should use
    /// `fetch_quote` directly.
    pub async fn fetch_quotes(&self, symbols: &[&str]) -> Vec<Option<YahooQuote>> {
        let calls = symbols.iter().map(|s| async move {
            self.fetch_quote(s).await.ok().flatten()
        });
        join_all(calls).await
    }

    fn build_chart_url(&self, symbol: &str) -> Result<Url, StreamsError> {
        // Yahoo URL-decodes the symbol once; commodity futures
        // like `CL=F` round-trip as-is, but we percent-encode
        // belt-and-braces via `Url::parse`.
        let raw = format!(
            "{}/v8/finance/chart/{}?interval=1d&range=5d",
            self.config.base_url, symbol
        );
        Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("yahoo url: {e}")))
    }

    /// Fetch the historical OHLC bar series for `symbol` over
    /// the supplied `interval` (`1d`, `1h`, …) and `range`
    /// (`1mo`, `3mo`, `1y`, …).
    ///
    /// Returns `Ok(Vec::new())` when the upstream returns no
    /// `result[]` rows (Yahoo's "no such symbol" shape) or when
    /// the bars array is missing — the seeder treats that as
    /// "no upstream data" and surfaces it as `EmptyUpstream`.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for unparseable bodies.
    pub async fn fetch_history(
        &self,
        symbol: &str,
        interval: &str,
        range: &str,
    ) -> Result<Vec<YahooBar>, StreamsError> {
        let url = self.build_history_url(symbol, interval, range)?;
        let resp = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: HistoryResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        if let Some(err) = body.chart.error {
            return Err(StreamsError::Parse(format!(
                "yahoo error: code={} description={}",
                err.code, err.description
            )));
        }
        let Some(first) = body.chart.result.into_iter().next() else {
            return Ok(Vec::new());
        };
        Ok(YahooBar::from_history(first))
    }

    fn build_history_url(
        &self,
        symbol: &str,
        interval: &str,
        range: &str,
    ) -> Result<Url, StreamsError> {
        let raw = format!(
            "{}/v8/finance/chart/{}?interval={}&range={}",
            self.config.base_url, symbol, interval, range
        );
        Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("yahoo url: {e}")))
    }
}

/// Distilled per-symbol response — only the fields Pellucid
/// seeders consume. The full chart response carries minute-bar
/// arrays we don't currently need.
#[derive(Clone, Debug, PartialEq)]
pub struct YahooQuote {
    /// Symbol as Yahoo returned it (e.g. `SPY`, `CL=F`,
    /// `BTC-USD`).
    pub symbol: String,
    /// Most recent regular-market price, in `currency`.
    pub price: f64,
    /// Previous trading session's close.
    pub previous_close: f64,
    /// Currency the price is denominated in (`USD`, `EUR`, …).
    pub currency: String,
    /// Exchange the symbol trades on.
    pub exchange: String,
    /// Wall-clock seconds when the upstream stamped this row
    /// (`meta.regularMarketTime`). 0 when absent.
    pub regular_market_time: i64,
}

impl YahooQuote {
    /// Percent change relative to `previous_close`. Returns 0.0
    /// when `previous_close` is zero (avoids div-by-zero on
    /// freshly-listed symbols).
    #[must_use]
    pub fn percent_change(&self) -> f64 {
        if self.previous_close == 0.0 {
            0.0
        } else {
            (self.price - self.previous_close) / self.previous_close * 100.0
        }
    }

    fn from_chart(raw: ChartResult, requested: &str) -> Self {
        let meta = raw.meta;
        let symbol = if meta.symbol.is_empty() {
            requested.to_string()
        } else {
            meta.symbol
        };
        Self {
            symbol,
            price: meta.regular_market_price,
            previous_close: meta.previous_close,
            currency: meta.currency,
            exchange: meta.exchange_name,
            regular_market_time: meta.regular_market_time,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    chart: ChartEnvelope,
}

#[derive(Debug, Deserialize)]
struct ChartEnvelope {
    #[serde(default)]
    result: Vec<ChartResult>,
    #[serde(default)]
    error: Option<ChartError>,
}

#[derive(Debug, Deserialize)]
struct ChartError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct ChartResult {
    meta: ChartMeta,
}

/// One historical OHLC bar — wall-clock seconds + open / high /
/// low / close / volume.
#[derive(Clone, Debug, PartialEq)]
pub struct YahooBar {
    /// Wall-clock seconds at the bar's open. Yahoo emits these
    /// as exchange-local 9:30 ET ticks for `interval=1d`.
    pub time_secs: i64,
    /// Open price.
    pub open: f64,
    /// Highest traded price during the bar.
    pub high: f64,
    /// Lowest traded price during the bar.
    pub low: f64,
    /// Close price.
    pub close: f64,
    /// Volume (shares / contracts) — `None` when Yahoo has no
    /// reported volume (some FX / index symbols).
    pub volume: Option<i64>,
}

impl YahooBar {
    /// Project a `chart.result[0]` row's parallel arrays into
    /// `Vec<YahooBar>`. Drops any index whose `close` is `null`
    /// (Yahoo emits `null` for non-trading days inside the
    /// requested range — including them would skew the
    /// downstream backtest math).
    fn from_history(raw: HistoryResult) -> Vec<Self> {
        let timestamps = raw.timestamp.unwrap_or_default();
        let Some(quote) = raw.indicators.quote.into_iter().next() else {
            return Vec::new();
        };
        let n = timestamps.len();
        let mut out: Vec<Self> = Vec::with_capacity(n);
        for i in 0..n {
            let close = quote.close.get(i).copied().flatten();
            let open = quote.open.get(i).copied().flatten();
            let high = quote.high.get(i).copied().flatten();
            let low = quote.low.get(i).copied().flatten();
            let volume = quote.volume.get(i).copied().flatten();
            // Drop bars whose close is missing — Yahoo's
            // "non-trading day" rows surface as JSON `null`
            // entries inside the parallel arrays.
            let (Some(close), Some(open), Some(high), Some(low), Some(time)) =
                (close, open, high, low, timestamps.get(i).copied())
            else {
                continue;
            };
            out.push(Self {
                time_secs: time,
                open,
                high,
                low,
                close,
                volume,
            });
        }
        out
    }
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    chart: HistoryEnvelope,
}

#[derive(Debug, Deserialize)]
struct HistoryEnvelope {
    #[serde(default)]
    result: Vec<HistoryResult>,
    #[serde(default)]
    error: Option<ChartError>,
}

#[derive(Debug, Deserialize)]
struct HistoryResult {
    #[serde(default)]
    timestamp: Option<Vec<i64>>,
    #[serde(default)]
    indicators: HistoryIndicators,
}

#[derive(Debug, Default, Deserialize)]
struct HistoryIndicators {
    #[serde(default)]
    quote: Vec<HistoryQuoteSeries>,
}

#[derive(Debug, Default, Deserialize)]
struct HistoryQuoteSeries {
    #[serde(default)]
    open: Vec<Option<f64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<i64>>,
}

#[derive(Debug, Default, Deserialize)]
struct ChartMeta {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "regularMarketPrice")]
    regular_market_price: f64,
    #[serde(default, rename = "previousClose")]
    previous_close: f64,
    #[serde(default)]
    currency: String,
    #[serde(default, rename = "exchangeName")]
    exchange_name: String,
    #[serde(default, rename = "regularMarketTime")]
    regular_market_time: i64,
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn chart_body(symbol: &str, price: f64, prev: f64) -> serde_json::Value {
        serde_json::json!({
            "chart": {
                "result": [{
                    "meta": {
                        "symbol":             symbol,
                        "regularMarketPrice": price,
                        "previousClose":      prev,
                        "currency":           "USD",
                        "exchangeName":       "PCX",
                        "regularMarketTime":  1714060800
                    }
                }],
                "error": null
            }
        })
    }

    fn client_pointing_at(server: &MockServer) -> YahooFinanceClient {
        YahooFinanceClient::new(
            YahooFinanceConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_quote_maps_chart_to_yahoo_quote() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/SPY"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(chart_body("SPY", 524.31, 522.10)),
            )
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let quote = client.fetch_quote("SPY").await.unwrap().expect("Some");
        assert_eq!(quote.symbol, "SPY");
        assert!((quote.price - 524.31).abs() < 1e-9);
        assert!((quote.previous_close - 522.10).abs() < 1e-9);
        assert_eq!(quote.currency, "USD");
        assert_eq!(quote.exchange, "PCX");
    }

    #[tokio::test]
    async fn fetch_quote_empty_result_returns_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/BOGUS"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "chart": { "result": [], "error": null }
                })),
            )
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let res = client.fetch_quote("BOGUS").await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn fetch_quote_explicit_yahoo_error_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/INVALID"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "chart": {
                        "result": [],
                        "error": { "code": "Not Found", "description": "No data found" }
                    }
                })),
            )
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_quote("INVALID").await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_quote_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/SPY"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_quote("SPY").await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_quote_unparseable_body_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/SPY"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json {{"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_quote("SPY").await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_quote_unreachable_yields_io_error() {
        let client = YahooFinanceClient::new(
            YahooFinanceConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_quote("SPY").await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_quotes_preserves_input_order() {
        let server = MockServer::start().await;
        for (sym, price, prev) in [("SPY", 524.0, 522.0), ("QQQ", 460.0, 458.0), ("DIA", 390.0, 389.0)] {
            Mock::given(method("GET"))
                .and(path(format!("/v8/finance/chart/{sym}")))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(chart_body(sym, price, prev)),
                )
                .mount(&server)
                .await;
        }
        let client = client_pointing_at(&server);
        let quotes = client.fetch_quotes(&["SPY", "QQQ", "DIA"]).await;
        assert_eq!(quotes.len(), 3);
        assert_eq!(quotes[0].as_ref().unwrap().symbol, "SPY");
        assert_eq!(quotes[1].as_ref().unwrap().symbol, "QQQ");
        assert_eq!(quotes[2].as_ref().unwrap().symbol, "DIA");
    }

    #[tokio::test]
    async fn fetch_quotes_returns_none_for_failed_symbols() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/SPY"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(chart_body("SPY", 524.0, 522.0)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/FAIL"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let quotes = client.fetch_quotes(&["SPY", "FAIL"]).await;
        assert!(quotes[0].is_some());
        assert!(quotes[1].is_none());
    }

    #[test]
    fn percent_change_against_zero_previous_is_zero() {
        let q = YahooQuote {
            symbol: "X".into(),
            price: 100.0,
            previous_close: 0.0,
            currency: "USD".into(),
            exchange: "X".into(),
            regular_market_time: 0,
        };
        assert!((q.percent_change() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn percent_change_simple_case() {
        let q = YahooQuote {
            symbol: "X".into(),
            price: 110.0,
            previous_close: 100.0,
            currency: "USD".into(),
            exchange: "X".into(),
            regular_market_time: 0,
        };
        assert!((q.percent_change() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn config_default_uses_production_base_url() {
        let cfg = YahooFinanceConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
        assert!(cfg.user_agent.contains("Chrome/121"));
    }

    #[test]
    fn from_chart_falls_back_to_requested_symbol_when_meta_empty() {
        let meta = ChartMeta {
            symbol: String::new(),
            regular_market_price: 1.23,
            previous_close: 1.20,
            currency: "USD".into(),
            exchange_name: "X".into(),
            regular_market_time: 1,
        };
        let q = YahooQuote::from_chart(ChartResult { meta }, "FALLBACK");
        assert_eq!(q.symbol, "FALLBACK");
    }
}
