//! Google Cloud Platform status / incidents JSON client.
//!
//! GCP publishes a public JSON status feed at:
//!
//! ```text
//! GET https://status.cloud.google.com/incidents.json
//! ```
//!
//! Free, no auth. Returns every recorded incident. We surface
//! the rows that are still open (no `end` timestamp) plus the
//! N most-recent closed incidents.
//!
//! Response shape (relevant subset):
//! ```json
//! [
//!   {
//!     "id":              "ABcDe12345",
//!     "external_desc":   "Compute Engine — high latency in us-central1",
//!     "begin":           "2026-04-25T08:00:00+00:00",
//!     "end":             null,
//!     "modified":        "2026-04-25T09:00:00+00:00",
//!     "severity":        "high",
//!     "status_impact":   "SERVICE_OUTAGE",
//!     "service_name":    "Google Compute Engine",
//!     "uri":             "https://status.cloud.google.com/incidents/ABcDe12345"
//!   }
//! ]
//! ```

use std::time::Duration;

use serde::Deserialize;

use crate::error::StreamsError;

/// Default base URL — GCP status production.
pub const DEFAULT_BASE_URL: &str = "https://status.cloud.google.com";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct GcpStatusConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for GcpStatusConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable GCP status client.
#[derive(Clone, Debug)]
pub struct GcpStatusClient {
    http: reqwest::Client,
    config: GcpStatusConfig,
}

impl GcpStatusClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: GcpStatusConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = GcpStatusConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the GCP incidents feed.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_incidents(&self) -> Result<Vec<GcpIncident>, StreamsError> {
        let url = format!("{}/incidents.json", self.config.base_url);
        let resp = self
            .http
            .get(&url)
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
        let body: Vec<RawIncident> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.into_iter().map(GcpIncident::from_raw).collect())
    }
}

/// One GCP incident row.
#[derive(Clone, Debug, PartialEq)]
pub struct GcpIncident {
    /// Incident id.
    pub id: String,
    /// External description.
    pub description: String,
    /// ISO-8601 begin timestamp.
    pub begin: String,
    /// ISO-8601 end timestamp (empty when ongoing).
    pub end: String,
    /// ISO-8601 last-modified timestamp.
    pub modified: String,
    /// Severity tag (`high | medium | low`).
    pub severity: String,
    /// `SERVICE_OUTAGE | SERVICE_DISRUPTION | SERVICE_INFORMATION`.
    pub status_impact: String,
    /// Affected service name.
    pub service_name: String,
    /// Permalink to the incident detail.
    pub uri: String,
    /// Pre-computed `true` iff `end` is empty.
    pub ongoing: bool,
}

#[derive(Debug, Default, Deserialize)]
struct RawIncident {
    #[serde(default)]
    id: String,
    #[serde(default)]
    external_desc: String,
    #[serde(default)]
    begin: String,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    modified: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    status_impact: String,
    #[serde(default)]
    service_name: String,
    #[serde(default)]
    uri: String,
}

impl GcpIncident {
    fn from_raw(raw: RawIncident) -> Self {
        let end = raw.end.unwrap_or_default();
        let ongoing = end.is_empty();
        Self {
            id: raw.id,
            description: raw.external_desc,
            begin: raw.begin,
            end,
            modified: raw.modified,
            severity: raw.severity,
            status_impact: raw.status_impact,
            service_name: raw.service_name,
            uri: raw.uri,
            ongoing,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!([
            {
                "id":            "abc-1",
                "external_desc": "Compute Engine — high latency in us-central1",
                "begin":         "2026-04-25T08:00:00+00:00",
                "end":           null,
                "modified":      "2026-04-25T09:00:00+00:00",
                "severity":      "high",
                "status_impact": "SERVICE_OUTAGE",
                "service_name":  "Google Compute Engine",
                "uri":           "https://status.cloud.google.com/incidents/abc-1"
            },
            {
                "id":            "def-2",
                "external_desc": "BigQuery — degraded performance",
                "begin":         "2026-04-24T08:00:00+00:00",
                "end":           "2026-04-24T18:00:00+00:00",
                "modified":      "2026-04-24T18:30:00+00:00",
                "severity":      "medium",
                "status_impact": "SERVICE_DISRUPTION",
                "service_name":  "BigQuery"
            }
        ])
    }

    fn client_pointing_at(server: &MockServer) -> GcpStatusClient {
        GcpStatusClient::new(
            GcpStatusConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_incidents_marks_ongoing_correctly() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/incidents.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_incidents().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "abc-1");
        assert!(rows[0].ongoing);
        assert_eq!(rows[0].end, "");
        assert!(!rows[1].ongoing);
        assert_eq!(rows[1].end, "2026-04-24T18:00:00+00:00");
    }

    #[tokio::test]
    async fn fetch_incidents_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/incidents.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_incidents().await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_incidents_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/incidents.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_incidents().await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_incidents_unreachable_yields_io() {
        let client = GcpStatusClient::new(
            GcpStatusConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_incidents().await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }
}
