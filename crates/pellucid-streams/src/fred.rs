//! Federal Reserve Economic Data (FRED) v2 client.
//!
//! FRED (St. Louis Fed) publishes 800,000+ economic time
//! series at:
//!
//! ```text
//! GET https://api.stlouisfed.org/fred/series/observations
//!     ?series_id=<id>&api_key=<key>&file_type=json
//!     &sort_order=desc&limit=<n>
//! ```
//!
//! Free with API-key registration at
//! `https://fred.stlouisfed.org/docs/api/api_key.html`. The
//! API key is a 32-character alphanumeric string.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "realtime_start":   "2026-05-04",
//!   "realtime_end":     "2026-05-04",
//!   "observation_start":"1776-07-04",
//!   "observation_end":  "9999-12-31",
//!   "units":            "lin",
//!   "output_type":      1,
//!   "file_type":        "json",
//!   "order_by":         "observation_date",
//!   "sort_order":       "desc",
//!   "count":            460,
//!   "offset":           0,
//!   "limit":            10,
//!   "observations": [
//!     { "realtime_start": "2026-05-04", "realtime_end": "2026-05-04",
//!       "date": "2026-03-01", "value": "112.345" },
//!     ...
//!   ]
//! }
//! ```
//!
//! `value` is a string (FRED uses `"."` for "no observation");
//! the parser converts numeric strings to `f64` and surfaces
//! `"."` as `None`.

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — FRED production.
pub const DEFAULT_BASE_URL: &str = "https://api.stlouisfed.org";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct FredConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Registered FRED API key (32-char alphanumeric).
    pub api_key: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl FredConfig {
    /// Build from an API key, using the production base URL.
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

/// Pluggable FRED client.
#[derive(Clone, Debug)]
pub struct FredClient {
    http: reqwest::Client,
    config: FredConfig,
}

impl FredClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: FredConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(api_key: impl Into<String>) -> Result<Self, StreamsError> {
        let cfg = FredConfig::with_api_key(api_key);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the most-recent `limit` observations for `series_id`.
    /// Newest-first per FRED's `sort_order=desc`.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_observations(
        &self,
        series_id: &str,
        limit: u32,
    ) -> Result<FredSeries, StreamsError> {
        let url = self.build_url(series_id, limit)?;
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
        let body: RawResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(FredSeries {
            series_id: series_id.to_string(),
            count: body.count,
            observations: body
                .observations
                .into_iter()
                .map(FredObservation::from_raw)
                .collect(),
        })
    }

    fn build_url(&self, series_id: &str, limit: u32) -> Result<Url, StreamsError> {
        let raw = format!("{}/fred/series/observations", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("fred url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("series_id", series_id);
            q.append_pair("api_key", &self.config.api_key);
            q.append_pair("file_type", "json");
            q.append_pair("sort_order", "desc");
            q.append_pair("limit", &limit.to_string());
        }
        Ok(url)
    }
}

/// One series response.
#[derive(Clone, Debug, PartialEq)]
pub struct FredSeries {
    /// Echo of the requested series id.
    pub series_id: String,
    /// Server-side total count for the series.
    pub count: u64,
    /// Observations newest-first (per `sort_order=desc`).
    pub observations: Vec<FredObservation>,
}

/// One observation.
#[derive(Clone, Debug, PartialEq)]
pub struct FredObservation {
    /// `YYYY-MM-DD` observation date.
    pub date: String,
    /// Observation value, or `None` when FRED reports `"."`
    /// (no observation for this date).
    pub value: Option<f64>,
}

impl FredSeries {
    /// Most-recent observation with a numeric value (skips
    /// `"."` placeholders). `None` when the series has no
    /// numeric observations.
    #[must_use]
    pub fn latest(&self) -> Option<&FredObservation> {
        self.observations.iter().find(|o| o.value.is_some())
    }
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    count: u64,
    #[serde(default)]
    observations: Vec<RawObservation>,
}

#[derive(Debug, Default, Deserialize)]
struct RawObservation {
    #[serde(default)]
    date: String,
    #[serde(default)]
    value: String,
}

impl FredObservation {
    fn from_raw(raw: RawObservation) -> Self {
        let trimmed = raw.value.trim();
        let value = if trimmed == "." || trimmed.is_empty() {
            None
        } else {
            trimmed.parse::<f64>().ok()
        };
        Self {
            date: raw.date,
            value,
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
            "realtime_start":   "2026-05-04",
            "realtime_end":     "2026-05-04",
            "observation_start":"1985-12-01",
            "observation_end":  "9999-12-31",
            "units":            "lin",
            "output_type":      1,
            "file_type":        "json",
            "order_by":         "observation_date",
            "sort_order":       "desc",
            "count":            460,
            "offset":           0,
            "limit":            5,
            "observations": [
                { "realtime_start": "2026-05-04", "realtime_end": "2026-05-04",
                  "date": "2026-04-01", "value": "117.123" },
                { "realtime_start": "2026-05-04", "realtime_end": "2026-05-04",
                  "date": "2026-03-01", "value": "115.500" },
                { "realtime_start": "2026-05-04", "realtime_end": "2026-05-04",
                  "date": "2026-02-01", "value": "." },
                { "realtime_start": "2026-05-04", "realtime_end": "2026-05-04",
                  "date": "2026-01-01", "value": "112.345" }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> FredClient {
        FredClient::new(
            FredConfig {
                base_url: server.uri(),
                api_key: "test-key-32-chars".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_observations_maps_four_rows_with_value_and_dot() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/fred/series/observations"))
            .and(query_param("series_id", "PCU3344133441"))
            .and(query_param("api_key", "test-key-32-chars"))
            .and(query_param("file_type", "json"))
            .and(query_param("sort_order", "desc"))
            .and(query_param("limit", "5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let series = client
            .fetch_observations("PCU3344133441", 5)
            .await
            .unwrap();
        assert_eq!(series.series_id, "PCU3344133441");
        assert_eq!(series.count, 460);
        assert_eq!(series.observations.len(), 4);
        assert_eq!(series.observations[0].date, "2026-04-01");
        assert!((series.observations[0].value.unwrap() - 117.123).abs() < 1e-9);
        // The "." observation surfaces as None.
        assert_eq!(series.observations[2].date, "2026-02-01");
        assert!(series.observations[2].value.is_none());
        // latest() skips the dot row and returns the newest numeric one.
        let latest = series.latest().unwrap();
        assert_eq!(latest.date, "2026-04-01");
    }

    #[tokio::test]
    async fn fetch_observations_403_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/fred/series/observations"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_observations("PCU3344133441", 5)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 403 }));
    }

    #[tokio::test]
    async fn fetch_observations_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/fred/series/observations"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_observations("PCU3344133441", 5)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_observations_unreachable_yields_io() {
        let client = FredClient::new(
            FredConfig {
                base_url: "http://127.0.0.1:1".into(),
                api_key: "x".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_observations("PCU3344133441", 5)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[tokio::test]
    async fn fetch_observations_only_dot_values_yields_no_latest() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/fred/series/observations"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "count": 2,
                    "observations": [
                        { "date": "2026-04-01", "value": "." },
                        { "date": "2026-03-01", "value": "" }
                    ]
                })),
            )
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let series = client
            .fetch_observations("BOGUS_SERIES", 5)
            .await
            .unwrap();
        assert_eq!(series.observations.len(), 2);
        assert!(series.latest().is_none());
    }

    #[test]
    fn config_with_api_key_uses_production() {
        let cfg = FredConfig::with_api_key("k");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.api_key, "k");
    }
}
