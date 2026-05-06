//! Polymarket gamma-api client for active prediction markets.
//!
//! Polymarket exposes a public REST API at:
//!
//! ```text
//! GET https://gamma-api.polymarket.com/markets
//!     ?active=true&closed=false&limit=100&order=volume24hr&ascending=false
//! ```
//!
//! Free, no auth. Returns active prediction-market metadata
//! including current prices, volume, and resolution criteria.
//!
//! Response shape (relevant subset):
//! ```json
//! [
//!   {
//!     "id":               "12345",
//!     "question":         "Will X happen by Y?",
//!     "slug":             "will-x-happen-by-y",
//!     "outcomes":         "[\"Yes\", \"No\"]",
//!     "outcomePrices":    "[\"0.62\", \"0.38\"]",
//!     "volume":           "1234567.89",
//!     "volume24hr":       12345.67,
//!     "liquidity":        "98765.43",
//!     "active":           true,
//!     "closed":           false,
//!     "endDate":          "2026-12-31T00:00:00Z",
//!     "category":         "Politics"
//!   }
//! ]
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — Polymarket gamma-api production.
pub const DEFAULT_BASE_URL: &str = "https://gamma-api.polymarket.com";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct PolymarketConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for PolymarketConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Polymarket client.
#[derive(Clone, Debug)]
pub struct PolymarketClient {
    http: reqwest::Client,
    config: PolymarketConfig,
}

impl PolymarketClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: PolymarketConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = PolymarketConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch up to `limit` active markets sorted by 24-hour
    /// volume descending.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_active_markets(
        &self,
        limit: u32,
    ) -> Result<Vec<PredictionMarket>, StreamsError> {
        let url = self.build_url(limit)?;
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
        let body: Vec<RawMarket> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.into_iter().map(PredictionMarket::from_raw).collect())
    }

    fn build_url(&self, limit: u32) -> Result<Url, StreamsError> {
        let raw = format!("{}/markets", self.config.base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("polymarket url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("active", "true");
            q.append_pair("closed", "false");
            q.append_pair("limit", &limit.to_string());
            q.append_pair("order", "volume24hr");
            q.append_pair("ascending", "false");
        }
        Ok(url)
    }
}

/// One prediction market.
#[derive(Clone, Debug, PartialEq)]
pub struct PredictionMarket {
    /// Polymarket id.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Polymarket slug (stable URL component).
    pub slug: String,
    /// Outcome labels (parsed from JSON-string).
    pub outcomes: Vec<String>,
    /// Outcome prices (parsed from JSON-string), aligned with
    /// `outcomes`.
    pub outcome_prices: Vec<f64>,
    /// Lifetime traded volume (USDC).
    pub volume: f64,
    /// 24-hour traded volume (USDC).
    pub volume_24hr: f64,
    /// Current AMM liquidity (USDC).
    pub liquidity: f64,
    /// Resolution / market-end timestamp.
    pub end_date: String,
    /// Topic category (`Politics`, `Sports`, etc.).
    pub category: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawMarket {
    #[serde(default)]
    id: Value,
    #[serde(default)]
    question: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    outcomes: Value,
    #[serde(default, rename = "outcomePrices")]
    outcome_prices: Value,
    #[serde(default)]
    volume: Value,
    #[serde(default, rename = "volume24hr")]
    volume_24hr: Value,
    #[serde(default)]
    liquidity: Value,
    #[serde(default, rename = "endDate")]
    end_date: String,
    #[serde(default)]
    category: String,
}

impl PredictionMarket {
    fn from_raw(raw: RawMarket) -> Self {
        Self {
            id: value_to_string(&raw.id),
            question: raw.question,
            slug: raw.slug,
            outcomes: parse_string_array(&raw.outcomes),
            outcome_prices: parse_f64_array(&raw.outcome_prices),
            volume: parse_f64(&raw.volume),
            volume_24hr: parse_f64(&raw.volume_24hr),
            liquidity: parse_f64(&raw.liquidity),
            end_date: raw.end_date,
            category: raw.category,
        }
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn parse_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn parse_string_array(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => serde_json::from_str::<Vec<String>>(s).unwrap_or_default(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_f64_array(v: &Value) -> Vec<f64> {
    match v {
        Value::String(s) => {
            // Inner items are strings — parse twice.
            serde_json::from_str::<Vec<String>>(s)
                .unwrap_or_default()
                .into_iter()
                .map(|x| x.trim().parse::<f64>().unwrap_or(0.0))
                .collect()
        }
        Value::Array(arr) => arr.iter().map(parse_f64).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!([
            {
                "id":            "12345",
                "question":      "Will X happen by Y?",
                "slug":          "will-x-happen-by-y",
                "outcomes":      "[\"Yes\", \"No\"]",
                "outcomePrices": "[\"0.62\", \"0.38\"]",
                "volume":        "1234567.89",
                "volume24hr":    12345.67,
                "liquidity":     "98765.43",
                "endDate":       "2026-12-31T00:00:00Z",
                "category":      "Politics"
            },
            {
                "id":            54321,
                "question":      "Will Z happen?",
                "slug":          "will-z-happen",
                "outcomes":      ["Yes", "No"],
                "outcomePrices": [0.45, 0.55],
                "volume":        500_000.0,
                "volume24hr":    "1000.0",
                "liquidity":     50_000.0,
                "endDate":       "2026-06-30T00:00:00Z",
                "category":      "Sports"
            }
        ])
    }

    fn client_pointing_at(server: &MockServer) -> PolymarketClient {
        PolymarketClient::new(
            PolymarketConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_active_markets_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/markets"))
            .and(query_param("active", "true"))
            .and(query_param("closed", "false"))
            .and(query_param("limit", "100"))
            .and(query_param("order", "volume24hr"))
            .and(query_param("ascending", "false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let markets = client.fetch_active_markets(100).await.unwrap();
        assert_eq!(markets.len(), 2);
        assert_eq!(markets[0].id, "12345");
        assert_eq!(
            markets[0].outcomes,
            vec!["Yes".to_string(), "No".to_string()]
        );
        assert_eq!(markets[0].outcome_prices, vec![0.62, 0.38]);
        assert!((markets[0].volume_24hr - 12345.67).abs() < 1e-3);
        // Second row: id is numeric, prices are array of numbers, volume is numeric, volume24hr is string.
        assert_eq!(markets[1].id, "54321");
        assert_eq!(markets[1].outcome_prices, vec![0.45, 0.55]);
        assert!((markets[1].volume_24hr - 1000.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn fetch_active_markets_empty_array_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/markets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let markets = client.fetch_active_markets(100).await.unwrap();
        assert!(markets.is_empty());
    }

    #[tokio::test]
    async fn fetch_active_markets_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/markets"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_active_markets(100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_active_markets_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/markets"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_active_markets(100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_active_markets_unreachable_yields_io() {
        let client = PolymarketClient::new(
            PolymarketConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_active_markets(100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn parse_helpers_handle_strings_and_numbers() {
        assert_eq!(
            parse_string_array(&Value::String("[\"a\", \"b\"]".into())),
            vec!["a", "b"]
        );
        assert_eq!(
            parse_f64_array(&Value::String("[\"0.62\", \"0.38\"]".into())),
            vec![0.62, 0.38]
        );
        assert!((parse_f64(&Value::String("1.5".into())) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = PolymarketConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
