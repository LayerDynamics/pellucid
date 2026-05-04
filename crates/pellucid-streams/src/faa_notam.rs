//! FAA NOTAM Search REST client.
//!
//! The FAA's public NOTAM Search application exposes an
//! unauthenticated JSON API at:
//!
//! ```text
//! POST https://notams.aim.faa.gov/notamSearch/nfsapp/getNotams
//! Content-Type: application/x-www-form-urlencoded
//! ```
//!
//! Required POST fields (the application's built-in search
//! form posts the same shape):
//! - `searchType=0` — point-of-interest search.
//! - `designatorsForLocation=KJFK` — comma-separated airport
//!   ICAO designators.
//! - `latitude=` / `longitude=` — point search; we use the
//!   designator path for stability.
//! - `radius=10` — nautical miles around each designator.
//! - `sortColumns=4` / `sortDirection=1` — newest-first.
//!
//! Response shape (relevant subset):
//! ```json
//! { "totalNotamCount": 42, "notamList": [
//!     {
//!         "notamNumber":   "A1234/26",
//!         "icaoLocation":  "KJFK",
//!         "issueDate":     "1714060800",
//!         "startDate":     "1714060800",
//!         "endDate":       "1714233600",
//!         "icaoMessage":   "RWY 04L/22R CLSD"
//!     }, …
//! ] }
//! ```

use std::time::Duration;

use serde::Deserialize;

use crate::error::StreamsError;

/// Default base URL — the FAA's public NOTAM Search application.
pub const DEFAULT_BASE_URL: &str = "https://notams.aim.faa.gov";

/// Default per-request timeout — 12 s. The FAA endpoint is
/// occasionally slow (1-3 s typical); 12 s tolerates a peak
/// without blocking the seeder cycle.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// User-Agent. The FAA endpoint accepts unmarked clients but
/// some IP-level filters look for browser-shaped UAs; we ship
/// a polite app-name UA per IETF best practice.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration for the FAA NOTAM client.
#[derive(Clone, Debug)]
pub struct FaaNotamConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
    /// Search radius around each designator in nautical miles.
    pub radius_nm: u32,
}

impl Default for FaaNotamConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            radius_nm: 10,
        }
    }
}

/// Pluggable FAA NOTAM client.
#[derive(Clone, Debug)]
pub struct FaaNotamClient {
    http: reqwest::Client,
    config: FaaNotamConfig,
}

impl FaaNotamClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: FaaNotamConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = FaaNotamConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch active NOTAMs around a list of ICAO designators.
    /// Returns one [`Notam`] per row the upstream returned.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_notams(
        &self,
        designators: &[&str],
    ) -> Result<Vec<Notam>, StreamsError> {
        if designators.is_empty() {
            return Ok(Vec::new());
        }
        let endpoint = format!("{}/notamSearch/nfsapp/getNotams", self.config.base_url);
        let radius = self.config.radius_nm.to_string();
        let designators_csv = designators.join(",");
        let form: [(&str, &str); 5] = [
            ("searchType", "0"),
            ("designatorsForLocation", &designators_csv),
            ("radius", &radius),
            ("sortColumns", "4"),
            ("sortDirection", "1"),
        ];
        let resp = self
            .http
            .post(&endpoint)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "application/json")
            .form(&form)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: NotamResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.notam_list.into_iter().map(Notam::from).collect())
    }
}

/// One distilled NOTAM row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notam {
    /// FAA NOTAM number (e.g. `"A1234/26"`).
    pub number: String,
    /// ICAO location (e.g. `"KJFK"`).
    pub icao_location: String,
    /// Wall-clock seconds the NOTAM was issued. 0 when absent.
    pub issue_date_unix: i64,
    /// Wall-clock seconds the NOTAM becomes effective.
    pub start_date_unix: i64,
    /// Wall-clock seconds the NOTAM expires.
    pub end_date_unix: i64,
    /// ICAO-format message body (may be multi-line).
    pub message: String,
}

impl Notam {
    /// True iff `now_unix` falls inside `[start, end]`.
    #[must_use]
    pub const fn is_active_at(&self, now_unix: i64) -> bool {
        self.start_date_unix <= now_unix && now_unix <= self.end_date_unix
    }
}

#[derive(Debug, Deserialize)]
struct NotamResponse {
    #[serde(default, rename = "notamList")]
    notam_list: Vec<NotamRow>,
}

#[derive(Debug, Default, Deserialize)]
struct NotamRow {
    #[serde(default, rename = "notamNumber")]
    notam_number: String,
    #[serde(default, rename = "icaoLocation")]
    icao_location: String,
    #[serde(default, rename = "issueDate")]
    issue_date: String,
    #[serde(default, rename = "startDate")]
    start_date: String,
    #[serde(default, rename = "endDate")]
    end_date: String,
    #[serde(default, rename = "icaoMessage")]
    icao_message: String,
}

impl From<NotamRow> for Notam {
    fn from(raw: NotamRow) -> Self {
        Self {
            number: raw.notam_number,
            icao_location: raw.icao_location,
            issue_date_unix: parse_unix(&raw.issue_date),
            start_date_unix: parse_unix(&raw.start_date),
            end_date_unix: parse_unix(&raw.end_date),
            message: raw.icao_message,
        }
    }
}

fn parse_unix(s: &str) -> i64 {
    s.trim().parse::<i64>().unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "totalNotamCount": 2,
            "notamList": [
                {
                    "notamNumber":  "A1234/26",
                    "icaoLocation": "KJFK",
                    "issueDate":    "1714060800",
                    "startDate":    "1714060800",
                    "endDate":      "1714233600",
                    "icaoMessage":  "RWY 04L/22R CLSD WEF 2026 APR 25"
                },
                {
                    "notamNumber":  "A5678/26",
                    "icaoLocation": "KLAX",
                    "issueDate":    "1714060900",
                    "startDate":    "1714060900",
                    "endDate":      "1714238400",
                    "icaoMessage":  "TWY B CLSD"
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> FaaNotamClient {
        FaaNotamClient::new(
            FaaNotamConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
                radius_nm: 10,
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_notams_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/notamSearch/nfsapp/getNotams"))
            .and(body_string_contains("designatorsForLocation=KJFK%2CKLAX"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let notams = client.fetch_notams(&["KJFK", "KLAX"]).await.unwrap();
        assert_eq!(notams.len(), 2);
        assert_eq!(notams[0].number, "A1234/26");
        assert_eq!(notams[0].icao_location, "KJFK");
        assert_eq!(notams[0].start_date_unix, 1_714_060_800);
        assert_eq!(notams[0].end_date_unix, 1_714_233_600);
        assert!(notams[0].message.contains("RWY 04L/22R"));
    }

    #[tokio::test]
    async fn fetch_notams_empty_designators_returns_empty_no_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/notamSearch/nfsapp/getNotams"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        assert!(client.fetch_notams(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_notams_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/notamSearch/nfsapp/getNotams"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_notams(&["KJFK"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_notams_unparseable_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/notamSearch/nfsapp/getNotams"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json {{"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_notams(&["KJFK"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_notams_unreachable_yields_io_error() {
        let client = FaaNotamClient::new(
            FaaNotamConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
                radius_nm: 10,
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_notams(&["KJFK"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn notam_is_active_at_inside_window() {
        let n = Notam {
            number: "A1/26".into(),
            icao_location: "KJFK".into(),
            issue_date_unix: 100,
            start_date_unix: 200,
            end_date_unix: 300,
            message: String::new(),
        };
        assert!(n.is_active_at(200));
        assert!(n.is_active_at(250));
        assert!(n.is_active_at(300));
        assert!(!n.is_active_at(199));
        assert!(!n.is_active_at(301));
    }

    #[test]
    fn parse_unix_handles_garbage() {
        assert_eq!(parse_unix("1714060800"), 1_714_060_800);
        assert_eq!(parse_unix("  1714060800 "), 1_714_060_800);
        assert_eq!(parse_unix(""), 0);
        assert_eq!(parse_unix("not a number"), 0);
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = FaaNotamConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
        assert_eq!(cfg.radius_nm, 10);
    }
}
