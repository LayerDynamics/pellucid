//! Cloudflare Radar API client (annotations endpoint).
//!
//! Cloudflare Radar publishes internet-outage annotations via:
//!
//! ```text
//! GET https://api.cloudflare.com/client/v4/radar/annotations/outages
//!     ?dateRange=7d&format=json
//! Header: Authorization: Bearer <token>
//! ```
//!
//! Requires an API token from a Cloudflare account (free tier
//! grants Radar read access).
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "result": {
//!     "annotations": [
//!       {
//!         "uuid":         "abc-123",
//!         "scope":        "country",
//!         "locations":    [{ "name": "Sudan", "code": "SD", "asnsList": [], "groupId": null }],
//!         "asnsDetails":  [],
//!         "outageType":   "POWEROUTAGE",
//!         "outageCause":  "POWER",
//!         "linkedUrl":    "https://...",
//!         "description":  "Nationwide ...",
//!         "startDate":    "2026-04-25T08:00:00Z",
//!         "endDate":      null,
//!         "eventType":    "OUTAGE"
//!       }
//!     ]
//!   },
//!   "success": true,
//!   "errors":  [],
//!   "messages": []
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — Cloudflare API production.
pub const DEFAULT_BASE_URL: &str = "https://api.cloudflare.com";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct CloudflareRadarConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Cloudflare API token (sent as `Authorization: Bearer …`).
    pub api_token: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl CloudflareRadarConfig {
    /// Build a config from a token, using the production
    /// base URL.
    #[must_use]
    pub fn with_token(api_token: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_token: api_token.into(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Cloudflare Radar client.
#[derive(Clone, Debug)]
pub struct CloudflareRadarClient {
    http: reqwest::Client,
    config: CloudflareRadarConfig,
}

impl CloudflareRadarClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: CloudflareRadarConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(api_token: impl Into<String>) -> Result<Self, StreamsError> {
        let cfg = CloudflareRadarConfig::with_token(api_token);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch internet-outage annotations over `date_range`
    /// (e.g. `"7d"` or `"24h"`).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_outages(
        &self,
        date_range: &str,
    ) -> Result<Vec<OutageAnnotation>, StreamsError> {
        let url = self.build_url(date_range)?;
        let resp = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header(
                "authorization",
                format!("Bearer {}", self.config.api_token),
            )
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: RadarResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        if !body.success {
            return Err(StreamsError::Parse("cloudflare radar: success=false".into()));
        }
        Ok(body.result.annotations.into_iter().map(OutageAnnotation::from_raw).collect())
    }

    fn build_url(&self, date_range: &str) -> Result<Url, StreamsError> {
        let raw = format!("{}/client/v4/radar/annotations/outages", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("radar url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("dateRange", date_range);
            q.append_pair("format", "json");
        }
        Ok(url)
    }
}

/// One outage annotation row.
#[derive(Clone, Debug, PartialEq)]
pub struct OutageAnnotation {
    /// UUID for the annotation.
    pub uuid: String,
    /// Scope (`country`, `asn`, `region`).
    pub scope: String,
    /// Comma-joined location names (`"Sudan"`, `"AS6697 Beltelecom"`).
    pub locations: String,
    /// Outage type (`POWEROUTAGE`, `SHUTDOWN`, etc.).
    pub outage_type: String,
    /// Outage cause (`POWER`, `GOVERNMENT`, `TECHNICAL`).
    pub outage_cause: String,
    /// Free-form description.
    pub description: String,
    /// ISO-8601 start.
    pub start_date: String,
    /// ISO-8601 end (empty when ongoing).
    pub end_date: String,
    /// External link (when set).
    pub linked_url: String,
}

#[derive(Debug, Deserialize)]
struct RadarResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    result: RadarResult,
}

#[derive(Debug, Default, Deserialize)]
struct RadarResult {
    #[serde(default)]
    annotations: Vec<RawAnnotation>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAnnotation {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    locations: Vec<RawLocation>,
    #[serde(default, rename = "outageType")]
    outage_type: String,
    #[serde(default, rename = "outageCause")]
    outage_cause: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "startDate")]
    start_date: String,
    #[serde(default, rename = "endDate")]
    end_date: Option<String>,
    #[serde(default, rename = "linkedUrl")]
    linked_url: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawLocation {
    #[serde(default)]
    name: String,
}

impl OutageAnnotation {
    fn from_raw(raw: RawAnnotation) -> Self {
        let locations = raw
            .locations
            .iter()
            .map(|l| l.name.clone())
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            uuid: raw.uuid,
            scope: raw.scope,
            locations,
            outage_type: raw.outage_type,
            outage_cause: raw.outage_cause,
            description: raw.description,
            start_date: raw.start_date,
            end_date: raw.end_date.unwrap_or_default(),
            linked_url: raw.linked_url,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "success":  true,
            "errors":   [],
            "messages": [],
            "result": {
                "annotations": [
                    {
                        "uuid":        "abc-123",
                        "scope":       "country",
                        "locations":   [{ "name": "Sudan", "code": "SD" }],
                        "outageType":  "POWEROUTAGE",
                        "outageCause": "POWER",
                        "description": "Nationwide outage",
                        "startDate":   "2026-04-25T08:00:00Z",
                        "endDate":     null,
                        "linkedUrl":   "https://example.com"
                    },
                    {
                        "uuid":        "def-456",
                        "scope":       "asn",
                        "locations":   [{ "name": "AS6697 Beltelecom" }],
                        "outageType":  "SHUTDOWN",
                        "outageCause": "GOVERNMENT",
                        "description": "Government-ordered shutdown",
                        "startDate":   "2026-04-26T12:00:00Z",
                        "endDate":     "2026-04-26T18:00:00Z"
                    }
                ]
            }
        })
    }

    fn client_pointing_at(server: &MockServer) -> CloudflareRadarClient {
        CloudflareRadarClient::new(
            CloudflareRadarConfig {
                base_url: server.uri(),
                api_token: "test-token".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_outages_maps_two_rows_with_bearer_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/client/v4/radar/annotations/outages"))
            .and(query_param("dateRange", "7d"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_outages("7d").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uuid, "abc-123");
        assert_eq!(rows[0].locations, "Sudan");
        assert_eq!(rows[0].outage_cause, "POWER");
        assert_eq!(rows[0].end_date, "");
        assert_eq!(rows[1].locations, "AS6697 Beltelecom");
        assert_eq!(rows[1].end_date, "2026-04-26T18:00:00Z");
    }

    #[tokio::test]
    async fn fetch_outages_success_false_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/client/v4/radar/annotations/outages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": false,
                "result":  { "annotations": [] }
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_outages("7d").await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_outages_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/client/v4/radar/annotations/outages"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_outages("7d").await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_outages_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/client/v4/radar/annotations/outages"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_outages("7d").await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_outages_unreachable_yields_io() {
        let client = CloudflareRadarClient::new(
            CloudflareRadarConfig {
                base_url: "http://127.0.0.1:1".into(),
                api_token: "t".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_outages("7d").await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_with_token_uses_production() {
        let cfg = CloudflareRadarConfig::with_token("t");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.api_token, "t");
    }
}
