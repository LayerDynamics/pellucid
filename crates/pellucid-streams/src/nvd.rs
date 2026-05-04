//! NIST National Vulnerability Database (NVD) v2.0 client.
//!
//! NVD's public REST API:
//!
//! ```text
//! GET https://services.nvd.nist.gov/rest/json/cves/2.0
//!     ?lastModStartDate=2026-04-25T00:00:00.000Z
//!     &lastModEndDate=2026-05-04T23:59:59.999Z
//!     &resultsPerPage=100
//! ```
//!
//! No auth required for the rate-limited free tier (5 req per
//! 30 s window). Higher rate limits are available via a free
//! API key as the `apiKey` header — pass via [`NvdConfig::api_key`].
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "vulnerabilities": [
//!     {
//!       "cve": {
//!         "id":           "CVE-2024-12345",
//!         "published":    "2026-04-25T08:00:00.000",
//!         "lastModified": "2026-04-25T20:00:00.000",
//!         "descriptions": [
//!           { "lang": "en", "value": "Buffer overflow in ..." }
//!         ],
//!         "metrics": {
//!           "cvssMetricV31": [
//!             {
//!               "cvssData": {
//!                 "baseScore":    9.8,
//!                 "baseSeverity": "CRITICAL",
//!                 "vectorString": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
//!               }
//!             }
//!           ]
//!         },
//!         "references": [
//!           { "url": "https://nvd.nist.gov/vuln/detail/CVE-2024-12345" }
//!         ]
//!       }
//!     }
//!   ],
//!   "totalResults": 42
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — NIST NVD production.
pub const DEFAULT_BASE_URL: &str = "https://services.nvd.nist.gov";

/// Default per-request timeout — 20 s. The free tier is slow
/// (often 2-5 s per call); 20 s tolerates the 95th percentile.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct NvdConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Optional API key. Sent as `apiKey` header. With a key
    /// the rate limit jumps from 5/30s to 50/30s.
    pub api_key: Option<String>,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for NvdConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: None,
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable NVD client.
#[derive(Clone, Debug)]
pub struct NvdClient {
    http: reqwest::Client,
    config: NvdConfig,
}

impl NvdClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: NvdConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(api_key: Option<String>) -> Result<Self, StreamsError> {
        let cfg = NvdConfig {
            api_key,
            ..NvdConfig::default()
        };
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch CVEs modified in `[last_mod_start, last_mod_end]`
    /// up to `results_per_page` rows.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_recent(
        &self,
        last_mod_start: &str,
        last_mod_end: &str,
        results_per_page: u32,
    ) -> Result<NvdResponse, StreamsError> {
        let url = self.build_url(last_mod_start, last_mod_end, results_per_page)?;
        let mut req = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json");
        if let Some(key) = self.config.api_key.as_deref() {
            req = req.header("apiKey", key);
        }
        let resp = req.send().await?;
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
        Ok(NvdResponse {
            total_results: body.total_results,
            vulnerabilities: body
                .vulnerabilities
                .into_iter()
                .filter_map(NvdVulnerability::from_raw)
                .collect(),
        })
    }

    fn build_url(
        &self,
        last_mod_start: &str,
        last_mod_end: &str,
        results_per_page: u32,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/rest/json/cves/2.0", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("nvd url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("lastModStartDate", last_mod_start);
            q.append_pair("lastModEndDate", last_mod_end);
            q.append_pair("resultsPerPage", &results_per_page.to_string());
        }
        Ok(url)
    }
}

/// Distilled NVD response.
#[derive(Clone, Debug, PartialEq)]
pub struct NvdResponse {
    /// Server-side total result count for the query window.
    pub total_results: u64,
    /// Returned vulnerability rows.
    pub vulnerabilities: Vec<NvdVulnerability>,
}

/// One NVD vulnerability row.
#[derive(Clone, Debug, PartialEq)]
pub struct NvdVulnerability {
    /// CVE id.
    pub cve_id: String,
    /// ISO-8601 first-published timestamp.
    pub published: String,
    /// ISO-8601 last-modified timestamp.
    pub last_modified: String,
    /// English description (when present).
    pub description: String,
    /// CVSS v3.1 base score (0.0 when no v3.1 metric).
    pub cvss_v31_base_score: f64,
    /// CVSS v3.1 severity label (`CRITICAL | HIGH | MEDIUM | LOW`).
    /// Empty when not scored.
    pub cvss_v31_severity: String,
    /// First reference URL (when present).
    pub primary_reference: String,
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    #[serde(default, rename = "totalResults")]
    total_results: u64,
    #[serde(default)]
    vulnerabilities: Vec<RawWrapper>,
}

#[derive(Debug, Deserialize)]
struct RawWrapper {
    #[serde(default)]
    cve: Value,
}

impl NvdVulnerability {
    fn from_raw(wrapper: RawWrapper) -> Option<Self> {
        let cve = wrapper.cve.as_object()?;
        let id = cve.get("id")?.as_str()?.to_string();
        let published = cve
            .get("published")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let last_modified = cve
            .get("lastModified")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let description = cve
            .get("descriptions")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter().find_map(|d| {
                    let lang = d.get("lang")?.as_str()?;
                    if lang == "en" {
                        d.get("value")?.as_str().map(str::to_string)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let (cvss_v31_base_score, cvss_v31_severity) = cve
            .get("metrics")
            .and_then(|m| m.get("cvssMetricV31"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|first| first.get("cvssData"))
            .map(|d| {
                let base = d.get("baseScore").and_then(Value::as_f64).unwrap_or(0.0);
                let sev = d
                    .get("baseSeverity")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                (base, sev)
            })
            .unwrap_or((0.0, String::new()));
        let primary_reference = cve
            .get("references")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|r| r.get("url"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Some(Self {
            cve_id: id,
            published,
            last_modified,
            description,
            cvss_v31_base_score,
            cvss_v31_severity,
            primary_reference,
        })
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
            "totalResults": 42,
            "vulnerabilities": [
                {
                    "cve": {
                        "id":           "CVE-2024-12345",
                        "published":    "2026-04-25T08:00:00.000",
                        "lastModified": "2026-04-25T20:00:00.000",
                        "descriptions": [
                            { "lang": "es", "value": "Spanish desc" },
                            { "lang": "en", "value": "Buffer overflow" }
                        ],
                        "metrics": {
                            "cvssMetricV31": [
                                {
                                    "cvssData": {
                                        "baseScore":    9.8,
                                        "baseSeverity": "CRITICAL"
                                    }
                                }
                            ]
                        },
                        "references": [
                            { "url": "https://nvd.nist.gov/vuln/detail/CVE-2024-12345" }
                        ]
                    }
                },
                {
                    "cve": {
                        "id":           "CVE-2024-67890",
                        "published":    "2026-04-26T08:00:00.000",
                        "lastModified": "2026-04-26T20:00:00.000",
                        "descriptions": [],
                        "metrics":      {}
                    }
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> NvdClient {
        NvdClient::new(
            NvdConfig {
                base_url: server.uri(),
                api_key: None,
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_recent_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/json/cves/2.0"))
            .and(query_param("lastModStartDate", "2026-04-25T00:00:00.000Z"))
            .and(query_param("lastModEndDate", "2026-05-04T23:59:59.999Z"))
            .and(query_param("resultsPerPage", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let resp = client
            .fetch_recent(
                "2026-04-25T00:00:00.000Z",
                "2026-05-04T23:59:59.999Z",
                100,
            )
            .await
            .unwrap();
        assert_eq!(resp.total_results, 42);
        assert_eq!(resp.vulnerabilities.len(), 2);
        assert_eq!(resp.vulnerabilities[0].cve_id, "CVE-2024-12345");
        assert_eq!(resp.vulnerabilities[0].description, "Buffer overflow");
        assert!((resp.vulnerabilities[0].cvss_v31_base_score - 9.8).abs() < 1e-9);
        assert_eq!(resp.vulnerabilities[0].cvss_v31_severity, "CRITICAL");
        // Second row has no metrics.
        assert!((resp.vulnerabilities[1].cvss_v31_base_score - 0.0).abs() < 1e-9);
        assert_eq!(resp.vulnerabilities[1].description, "");
    }

    #[tokio::test]
    async fn fetch_recent_sends_api_key_header_when_configured() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/json/cves/2.0"))
            .and(header("apiKey", "abc-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = NvdClient::new(
            NvdConfig {
                base_url: server.uri(),
                api_key: Some("abc-123".into()),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        );
        let _ = client
            .fetch_recent("2026-04-25T00:00:00.000Z", "2026-05-04T23:59:59.999Z", 100)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fetch_recent_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/json/cves/2.0"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_recent("2026-04-25T00:00:00.000Z", "2026-05-04T23:59:59.999Z", 100)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_recent_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/json/cves/2.0"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_recent("2026-04-25T00:00:00.000Z", "2026-05-04T23:59:59.999Z", 100)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_recent_unreachable_yields_io() {
        let client = NvdClient::new(
            NvdConfig {
                base_url: "http://127.0.0.1:1".into(),
                api_key: None,
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_recent("2026-04-25T00:00:00.000Z", "2026-05-04T23:59:59.999Z", 100)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = NvdConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert!(cfg.api_key.is_none());
    }
}
