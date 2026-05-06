//! CISA Known Exploited Vulnerabilities (KEV) catalog client.
//!
//! CISA publishes the KEV catalog as a single JSON document
//! at:
//!
//! ```text
//! GET https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json
//! ```
//!
//! Free, no auth. Refreshed when CISA adds entries (typically
//! daily during business hours).
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "title":          "CISA Catalog of Known Exploited Vulnerabilities",
//!   "catalogVersion": "2026.04.25",
//!   "dateReleased":   "2026-04-25T20:00:00.000Z",
//!   "count":          1234,
//!   "vulnerabilities": [
//!     {
//!       "cveID":                "CVE-2024-12345",
//!       "vendorProject":        "Acme Corp",
//!       "product":              "AcmeRouter",
//!       "vulnerabilityName":    "Acme Buffer Overflow",
//!       "dateAdded":            "2026-04-20",
//!       "shortDescription":     "...",
//!       "requiredAction":       "Apply patch per Acme guidance.",
//!       "dueDate":              "2026-05-11",
//!       "knownRansomwareCampaignUse": "Known",
//!       "notes":                "..."
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;

use crate::error::StreamsError;

/// Default base URL — CISA KEV public CDN.
pub const DEFAULT_BASE_URL: &str = "https://www.cisa.gov";

/// Default per-request timeout — 15 s. The KEV file is ~2 MiB.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct CisaKevConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for CisaKevConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable CISA KEV client.
#[derive(Clone, Debug)]
pub struct CisaKevClient {
    http: reqwest::Client,
    config: CisaKevConfig,
}

impl CisaKevClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: CisaKevConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = CisaKevConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the full KEV catalog.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_catalog(&self) -> Result<KevCatalog, StreamsError> {
        let url = format!(
            "{}/sites/default/files/feeds/known_exploited_vulnerabilities.json",
            self.config.base_url
        );
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
        let body: RawCatalog = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(KevCatalog {
            catalog_version: body.catalog_version,
            date_released: body.date_released,
            vulnerabilities: body
                .vulnerabilities
                .into_iter()
                .map(KevVulnerability::from_raw)
                .collect(),
        })
    }
}

/// Distilled KEV catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct KevCatalog {
    /// Catalog version (typically `YYYY.MM.DD`).
    pub catalog_version: String,
    /// ISO-8601 release timestamp.
    pub date_released: String,
    /// Vulnerability rows.
    pub vulnerabilities: Vec<KevVulnerability>,
}

/// One KEV row.
#[derive(Clone, Debug, PartialEq)]
pub struct KevVulnerability {
    /// CVE identifier (e.g. `"CVE-2024-12345"`).
    pub cve_id: String,
    /// Vendor / project (`"Acme Corp"`).
    pub vendor_project: String,
    /// Product name.
    pub product: String,
    /// Human-readable vulnerability name.
    pub vulnerability_name: String,
    /// `YYYY-MM-DD` date CISA added the entry.
    pub date_added: String,
    /// One-paragraph description.
    pub short_description: String,
    /// Required action language from CISA.
    pub required_action: String,
    /// `YYYY-MM-DD` federal-agency due date.
    pub due_date: String,
    /// `Known | Unknown` — ransomware-campaign-use flag.
    pub known_ransomware_campaign_use: String,
}

#[derive(Debug, Deserialize)]
struct RawCatalog {
    #[serde(default, rename = "catalogVersion")]
    catalog_version: String,
    #[serde(default, rename = "dateReleased")]
    date_released: String,
    #[serde(default)]
    vulnerabilities: Vec<RawVuln>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVuln {
    #[serde(default, rename = "cveID")]
    cve_id: String,
    #[serde(default, rename = "vendorProject")]
    vendor_project: String,
    #[serde(default)]
    product: String,
    #[serde(default, rename = "vulnerabilityName")]
    vulnerability_name: String,
    #[serde(default, rename = "dateAdded")]
    date_added: String,
    #[serde(default, rename = "shortDescription")]
    short_description: String,
    #[serde(default, rename = "requiredAction")]
    required_action: String,
    #[serde(default, rename = "dueDate")]
    due_date: String,
    #[serde(default, rename = "knownRansomwareCampaignUse")]
    known_ransomware_campaign_use: String,
}

impl KevVulnerability {
    fn from_raw(raw: RawVuln) -> Self {
        Self {
            cve_id: raw.cve_id,
            vendor_project: raw.vendor_project,
            product: raw.product,
            vulnerability_name: raw.vulnerability_name,
            date_added: raw.date_added,
            short_description: raw.short_description,
            required_action: raw.required_action,
            due_date: raw.due_date,
            known_ransomware_campaign_use: raw.known_ransomware_campaign_use,
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
        serde_json::json!({
            "title":          "CISA Catalog of Known Exploited Vulnerabilities",
            "catalogVersion": "2026.04.25",
            "dateReleased":   "2026-04-25T20:00:00.000Z",
            "count":          2,
            "vulnerabilities": [
                {
                    "cveID":             "CVE-2024-12345",
                    "vendorProject":     "Acme Corp",
                    "product":           "AcmeRouter",
                    "vulnerabilityName": "Acme Buffer Overflow",
                    "dateAdded":         "2026-04-20",
                    "shortDescription":  "...",
                    "requiredAction":    "Apply patch per Acme guidance.",
                    "dueDate":           "2026-05-11",
                    "knownRansomwareCampaignUse": "Known"
                },
                {
                    "cveID":             "CVE-2024-67890",
                    "vendorProject":     "Foo",
                    "product":           "FooApp",
                    "vulnerabilityName": "Foo Auth Bypass",
                    "dateAdded":         "2026-04-22",
                    "knownRansomwareCampaignUse": "Unknown"
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> CisaKevClient {
        CisaKevClient::new(
            CisaKevConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_catalog_returns_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/sites/default/files/feeds/known_exploited_vulnerabilities.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let cat = client.fetch_catalog().await.unwrap();
        assert_eq!(cat.catalog_version, "2026.04.25");
        assert_eq!(cat.vulnerabilities.len(), 2);
        assert_eq!(cat.vulnerabilities[0].cve_id, "CVE-2024-12345");
        assert_eq!(
            cat.vulnerabilities[0].known_ransomware_campaign_use,
            "Known"
        );
        assert_eq!(
            cat.vulnerabilities[1].known_ransomware_campaign_use,
            "Unknown"
        );
    }

    #[tokio::test]
    async fn fetch_catalog_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/sites/default/files/feeds/known_exploited_vulnerabilities.json",
            ))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_catalog().await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_catalog_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/sites/default/files/feeds/known_exploited_vulnerabilities.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_catalog().await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_catalog_unreachable_yields_io() {
        let client = CisaKevClient::new(
            CisaKevConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_catalog().await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = CisaKevConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
