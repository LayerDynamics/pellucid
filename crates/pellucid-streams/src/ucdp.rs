//! UCDP (Uppsala Conflict Data Program) REST API client.
//!
//! UCDP publishes the Georeferenced Event Dataset (GED) via a
//! free public REST API:
//!
//! ```text
//! GET https://ucdpapi.pcr.uu.se/api/gedevents/24.1?Year=2024&pagesize=100
//! ```
//!
//! No auth required. Returns JSON pages of conflict events
//! (one row per georeferenced fatality-bearing incident).
//!
//! Response shape (relevant subset):
//! ```json
//! {
//!   "TotalCount":     15234,
//!   "TotalPages":     153,
//!   "PageCount":      100,
//!   "Result": [
//!     {
//!       "id":            "GED-12345",
//!       "year":          2024,
//!       "date_start":    "2024-04-25",
//!       "date_end":      "2024-04-25",
//!       "country":       "Syria",
//!       "country_id":    652,
//!       "side_a":        "Government of Syria",
//!       "side_b":        "ISIS",
//!       "type_of_violence": 1,
//!       "best":          12,
//!       "high":          14,
//!       "low":           10,
//!       "latitude":      35.123,
//!       "longitude":     38.456
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — UCDP production API.
pub const DEFAULT_BASE_URL: &str = "https://ucdpapi.pcr.uu.se";

/// Default API version — bumped when UCDP releases a new
/// dataset version (currently 24.1 as of mid-2024).
pub const DEFAULT_VERSION: &str = "24.1";

/// Default per-request timeout — 15 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct UcdpConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Dataset version slug (e.g. `"24.1"`).
    pub version: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for UcdpConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            version: DEFAULT_VERSION.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable UCDP client.
#[derive(Clone, Debug)]
pub struct UcdpClient {
    http: reqwest::Client,
    config: UcdpConfig,
}

impl UcdpClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: UcdpConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = UcdpConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch one page of GED events for `year`. Up to
    /// `page_size` rows.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_ged_events(
        &self,
        year: u16,
        page_size: u32,
    ) -> Result<UcdpPage, StreamsError> {
        let url = self.build_ged_url(year, page_size)?;
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
        let body: UcdpResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(UcdpPage {
            total_count: body.total_count,
            total_pages: body.total_pages,
            events: body.result.into_iter().map(UcdpEvent::from_raw).collect(),
        })
    }

    fn build_ged_url(&self, year: u16, page_size: u32) -> Result<Url, StreamsError> {
        let raw = format!(
            "{}/api/gedevents/{}",
            self.config.base_url, self.config.version
        );
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("ucdp url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("Year", &year.to_string());
            q.append_pair("pagesize", &page_size.to_string());
        }
        Ok(url)
    }
}

/// One page of GED events.
#[derive(Clone, Debug, PartialEq)]
pub struct UcdpPage {
    /// Total event count for the query (server-side).
    pub total_count: u64,
    /// Total page count for the query.
    pub total_pages: u32,
    /// Events on this page.
    pub events: Vec<UcdpEvent>,
}

/// One UCDP event.
#[derive(Clone, Debug, PartialEq)]
pub struct UcdpEvent {
    /// GED event id.
    pub id: String,
    /// Calendar year.
    pub year: u16,
    /// Event start date (`YYYY-MM-DD`).
    pub date_start: String,
    /// Event end date.
    pub date_end: String,
    /// Country name (UCDP-canonical English).
    pub country: String,
    /// Side A (typically the government / state actor).
    pub side_a: String,
    /// Side B (non-state / opposing actor).
    pub side_b: String,
    /// UCDP `type_of_violence` enum: `1` state-based,
    /// `2` non-state, `3` one-sided.
    pub type_of_violence: i32,
    /// UCDP `best` fatality estimate.
    pub best_fatalities: i32,
    /// UCDP `high` fatality estimate.
    pub high_fatalities: i32,
    /// UCDP `low` fatality estimate.
    pub low_fatalities: i32,
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
}

#[derive(Debug, Deserialize)]
struct UcdpResponse {
    #[serde(default, rename = "TotalCount")]
    total_count: u64,
    #[serde(default, rename = "TotalPages")]
    total_pages: u32,
    #[serde(default, rename = "Result")]
    result: Vec<RawEvent>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvent {
    #[serde(default)]
    id: serde_json::Value,
    #[serde(default)]
    year: i32,
    #[serde(default)]
    date_start: String,
    #[serde(default)]
    date_end: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    side_a: String,
    #[serde(default)]
    side_b: String,
    #[serde(default)]
    type_of_violence: i32,
    #[serde(default)]
    best: i32,
    #[serde(default)]
    high: i32,
    #[serde(default)]
    low: i32,
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

impl UcdpEvent {
    fn from_raw(raw: RawEvent) -> Self {
        let id = match raw.id {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        Self {
            id,
            year: u16::try_from(raw.year).unwrap_or(0),
            date_start: raw.date_start,
            date_end: raw.date_end,
            country: raw.country,
            side_a: raw.side_a,
            side_b: raw.side_b,
            type_of_violence: raw.type_of_violence,
            best_fatalities: raw.best,
            high_fatalities: raw.high,
            low_fatalities: raw.low,
            latitude: raw.latitude.unwrap_or(0.0),
            longitude: raw.longitude.unwrap_or(0.0),
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
            "TotalCount": 15234,
            "TotalPages": 153,
            "PageCount":  100,
            "Result": [
                {
                    "id":               "GED-12345",
                    "year":             2024,
                    "date_start":       "2024-04-25",
                    "date_end":         "2024-04-25",
                    "country":          "Syria",
                    "country_id":       652,
                    "side_a":           "Government of Syria",
                    "side_b":           "ISIS",
                    "type_of_violence": 1,
                    "best":             12,
                    "high":             14,
                    "low":              10,
                    "latitude":         35.123,
                    "longitude":        38.456
                },
                {
                    "id":               54321,
                    "year":             2024,
                    "date_start":       "2024-04-26",
                    "country":          "Yemen",
                    "side_a":           "Houthis",
                    "side_b":           "Saudi-led coalition",
                    "type_of_violence": 1,
                    "best":             5,
                    "latitude":         null,
                    "longitude":        null
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> UcdpClient {
        UcdpClient::new(
            UcdpConfig {
                base_url: server.uri(),
                version: "24.1".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_ged_events_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/gedevents/24.1"))
            .and(query_param("Year", "2024"))
            .and(query_param("pagesize", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let page = client.fetch_ged_events(2024, 100).await.unwrap();
        assert_eq!(page.total_count, 15234);
        assert_eq!(page.total_pages, 153);
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.events[0].id, "GED-12345");
        assert_eq!(page.events[0].country, "Syria");
        assert_eq!(page.events[0].best_fatalities, 12);
        assert!((page.events[0].latitude - 35.123).abs() < 1e-9);
        // Numeric id round-trips to string.
        assert_eq!(page.events[1].id, "54321");
        // Null coords default to 0.0.
        assert!((page.events[1].latitude - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_ged_events_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/gedevents/24.1"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_ged_events(2024, 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_ged_events_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/gedevents/24.1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_ged_events(2024, 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_ged_events_unreachable_yields_io() {
        let client = UcdpClient::new(
            UcdpConfig {
                base_url: "http://127.0.0.1:1".into(),
                version: "24.1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_ged_events(2024, 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = UcdpConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.version, DEFAULT_VERSION);
    }
}
