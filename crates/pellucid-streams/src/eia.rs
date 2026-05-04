//! US Energy Information Administration (EIA) v2 API client.
//!
//! EIA's v2 API exposes a uniform query shape across every
//! dataset:
//!
//! ```text
//! GET https://api.eia.gov/v2/{path}/data/
//!     ?api_key=<key>&frequency=weekly&data[0]=value
//!     &facets[product][]=EPC0&facets[duoarea][]=NUS
//!     &start=2024-01-01&sort[0][column]=period&sort[0][direction]=desc
//!     &length=10
//! ```
//!
//! The free tier requires registration to obtain an API key.
//! Pellucid stores it as `EIA_API_KEY` and configures one
//! [`EiaClient`] per process.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "response": {
//!     "data": [
//!       { "period": "2026-04-25", "value": "457321", "units": "Thousand Barrels", ... },
//!       ...
//!     ],
//!     "total":     1234
//!   }
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — EIA v2 production.
pub const DEFAULT_BASE_URL: &str = "https://api.eia.gov";

/// Default per-request timeout — 15 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct EiaConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Registered EIA API key.
    pub api_key: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl EiaConfig {
    /// Build a config from an API key, using the production
    /// base URL.
    #[must_use]
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// One EIA v2 query — built up via the typed builder, sent
/// through [`EiaClient::fetch`].
#[derive(Clone, Debug, Default)]
pub struct EiaQuery {
    /// Dataset path (e.g.
    /// `"petroleum/stoc/wstk"` for weekly stocks).
    pub path: String,
    /// `frequency` — `weekly`, `monthly`, `daily`, etc.
    pub frequency: String,
    /// `data[]=` columns to return (e.g. `["value"]`).
    pub data_columns: Vec<String>,
    /// `facets[<facet_name>][]=value` filters.
    pub facets: Vec<(String, String)>,
    /// `start=YYYY-MM-DD` lower bound (when supplied).
    pub start: Option<String>,
    /// `length` — max rows per page.
    pub length: u32,
    /// Sort direction (`desc` newest-first by default).
    pub sort_direction: String,
}

impl EiaQuery {
    /// Build a basic query.
    #[must_use]
    pub fn new(path: impl Into<String>, frequency: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            frequency: frequency.into(),
            data_columns: vec!["value".to_string()],
            facets: Vec::new(),
            start: None,
            length: 50,
            sort_direction: "desc".to_string(),
        }
    }

    /// Add a facet filter.
    #[must_use]
    pub fn facet(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.facets.push((name.into(), value.into()));
        self
    }

    /// Set the start date.
    #[must_use]
    pub fn start(mut self, start: impl Into<String>) -> Self {
        self.start = Some(start.into());
        self
    }

    /// Set the page length.
    #[must_use]
    pub const fn length(mut self, length: u32) -> Self {
        self.length = length;
        self
    }
}

/// Pluggable EIA v2 client.
#[derive(Clone, Debug)]
pub struct EiaClient {
    http: reqwest::Client,
    config: EiaConfig,
}

impl EiaClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: EiaConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(api_key: impl Into<String>) -> Result<Self, StreamsError> {
        let cfg = EiaConfig::with_api_key(api_key);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Run the supplied query.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch(&self, query: &EiaQuery) -> Result<EiaResponse, StreamsError> {
        let url = self.build_url(query)?;
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
        let body: RawEnvelope = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(EiaResponse {
            total: body.response.total,
            rows: body.response.data.into_iter().map(EiaRow::from_raw).collect(),
        })
    }

    fn build_url(&self, query: &EiaQuery) -> Result<Url, StreamsError> {
        let raw = format!("{}/v2/{}/data/", self.config.base_url, query.path);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("eia url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("api_key", &self.config.api_key);
            q.append_pair("frequency", &query.frequency);
            for (i, col) in query.data_columns.iter().enumerate() {
                q.append_pair(&format!("data[{i}]"), col);
            }
            for (name, value) in &query.facets {
                q.append_pair(&format!("facets[{name}][]"), value);
            }
            if let Some(start) = &query.start {
                q.append_pair("start", start);
            }
            q.append_pair("sort[0][column]", "period");
            q.append_pair("sort[0][direction]", &query.sort_direction);
            q.append_pair("length", &query.length.to_string());
        }
        Ok(url)
    }
}

/// Distilled response.
#[derive(Clone, Debug, PartialEq)]
pub struct EiaResponse {
    /// Server-side total row count for the query.
    pub total: u64,
    /// Returned rows (length capped by `query.length`).
    pub rows: Vec<EiaRow>,
}

/// One EIA row.
#[derive(Clone, Debug, PartialEq)]
pub struct EiaRow {
    /// `period` field — date or year-month string.
    pub period: String,
    /// `value` field as a `f64` (EIA returns numerics as
    /// strings or numbers).
    pub value: f64,
    /// `units` (e.g. `"Thousand Barrels"`).
    pub units: String,
    /// `series` / `seriesDescription` (when present).
    pub series_description: String,
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    response: RawResponse,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    data: Vec<RawRow>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRow {
    #[serde(default)]
    period: String,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    units: String,
    #[serde(default, rename = "series-description")]
    series_description: String,
}

impl EiaRow {
    fn from_raw(raw: RawRow) -> Self {
        let value = match raw.value {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
            _ => 0.0,
        };
        Self {
            period: raw.period,
            value,
            units: raw.units,
            series_description: raw.series_description,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "response": {
                "total": 1234,
                "data": [
                    {
                        "period":             "2026-04-25",
                        "value":              "457321",
                        "units":              "Thousand Barrels",
                        "series-description": "Weekly U.S. Ending Stocks of Crude Oil"
                    },
                    {
                        "period":             "2026-04-18",
                        "value":              456_900.0,
                        "units":              "Thousand Barrels",
                        "series-description": "Weekly U.S. Ending Stocks of Crude Oil"
                    }
                ]
            }
        })
    }

    fn client_pointing_at(server: &MockServer) -> EiaClient {
        EiaClient::new(
            EiaConfig {
                base_url: server.uri(),
                api_key: "test-key".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_petroleum_stocks_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/petroleum/stoc/wstk/data/"))
            .and(query_param("api_key", "test-key"))
            .and(query_param("frequency", "weekly"))
            .and(query_param("data[0]", "value"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let query = EiaQuery::new("petroleum/stoc/wstk", "weekly").length(10);
        let resp = client.fetch(&query).await.unwrap();
        assert_eq!(resp.total, 1234);
        assert_eq!(resp.rows.len(), 2);
        assert_eq!(resp.rows[0].period, "2026-04-25");
        assert!((resp.rows[0].value - 457_321.0).abs() < 1e-3);
        assert_eq!(resp.rows[0].units, "Thousand Barrels");
        // Numeric-typed value also round-trips.
        assert!((resp.rows[1].value - 456_900.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn fetch_passes_facet_query_params() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/petroleum/sum/snd/data/"))
            .and(query_param("facets[product][]", "EPOOXEX0"))
            .and(query_param("facets[duoarea][]", "NUS"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let query = EiaQuery::new("petroleum/sum/snd", "weekly")
            .facet("product", "EPOOXEX0")
            .facet("duoarea", "NUS");
        let _ = client.fetch(&query).await.unwrap();
    }

    #[tokio::test]
    async fn fetch_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/petroleum/stoc/wstk/data/"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch(&EiaQuery::new("petroleum/stoc/wstk", "weekly"))
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/x/data/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch(&EiaQuery::new("x", "weekly"))
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_unreachable_yields_io() {
        let client = EiaClient::new(
            EiaConfig {
                base_url: "http://127.0.0.1:1".into(),
                api_key: "k".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch(&EiaQuery::new("x", "weekly"))
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_with_api_key_uses_production() {
        let cfg = EiaConfig::with_api_key("k");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.api_key, "k");
    }
}
