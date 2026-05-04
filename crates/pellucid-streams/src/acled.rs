//! ACLED (Armed Conflict Location & Event Data) API client.
//!
//! ACLED's free-tier REST API requires registration to obtain
//! an `email` + `key` pair. The endpoint shape:
//!
//! ```text
//! GET https://api.acleddata.com/acled/read?email=<email>&key=<key>
//!     &iso=686&event_date=2024-04-25:2024-05-04
//!     &limit=500&fields=event_id_cnty|event_date|event_type|...
//! ```
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "status":  200,
//!   "success": true,
//!   "count":   42,
//!   "data": [
//!     {
//!       "event_id_cnty":  "SYR12345",
//!       "event_date":     "2024-04-25",
//!       "event_type":     "Battles",
//!       "sub_event_type": "Armed clash",
//!       "actor1":         "Government of Syria",
//!       "actor2":         "ISIS",
//!       "country":        "Syria",
//!       "admin1":         "Aleppo",
//!       "location":       "Manbij",
//!       "latitude":       "36.523",
//!       "longitude":      "37.952",
//!       "fatalities":     "5",
//!       "notes":          "..."
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — ACLED production API.
pub const DEFAULT_BASE_URL: &str = "https://api.acleddata.com";

/// Default per-request timeout — 20 s. ACLED can be slow on
/// large date-range queries.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct AcledConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Registered ACLED email.
    pub email: String,
    /// Registered ACLED API key.
    pub key: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl AcledConfig {
    /// Build a config from email + key, using the production
    /// base URL.
    #[must_use]
    pub fn with_credentials(email: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            email: email.into(),
            key: key.into(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable ACLED client.
#[derive(Clone, Debug)]
pub struct AcledClient {
    http: reqwest::Client,
    config: AcledConfig,
}

impl AcledClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: AcledConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor with default reqwest client.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(email: impl Into<String>, key: impl Into<String>) -> Result<Self, StreamsError> {
        let cfg = AcledConfig::with_credentials(email, key);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Read events for the supplied ISO 3166-1 numeric country
    /// codes (ACLED uses numeric, not alpha-2; e.g. Syria=760,
    /// Iraq=368, Yemen=887, Iran=364, Ukraine=804). `start_date`
    /// + `end_date` are `YYYY-MM-DD`. Up to `limit` rows.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_events(
        &self,
        iso_codes: &[u16],
        start_date: &str,
        end_date: &str,
        limit: u32,
    ) -> Result<Vec<AcledEvent>, StreamsError> {
        if iso_codes.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.build_read_url(iso_codes, start_date, end_date, limit)?;
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
        let body: AcledResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        if !body.success {
            return Err(StreamsError::Parse(format!(
                "acled: success=false, status={}",
                body.status
            )));
        }
        Ok(body.data.into_iter().map(AcledEvent::from_raw).collect())
    }

    fn build_read_url(
        &self,
        iso_codes: &[u16],
        start_date: &str,
        end_date: &str,
        limit: u32,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/acled/read", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("acled url: {e}")))?;
        let iso_csv = iso_codes
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join("|");
        let date_range = format!("{start_date}|{end_date}");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("email", &self.config.email);
            q.append_pair("key", &self.config.key);
            q.append_pair("iso", &iso_csv);
            q.append_pair("event_date", &date_range);
            q.append_pair("event_date_where", "BETWEEN");
            q.append_pair("limit", &limit.to_string());
        }
        Ok(url)
    }
}

/// One ACLED event.
#[derive(Clone, Debug, PartialEq)]
pub struct AcledEvent {
    /// ACLED country-prefixed event id.
    pub event_id_cnty: String,
    /// `YYYY-MM-DD`.
    pub event_date: String,
    /// Top-level event type (`Battles`, `Protests`, etc.).
    pub event_type: String,
    /// Sub-type (`Armed clash`, `Peaceful protest`, etc.).
    pub sub_event_type: String,
    /// Primary actor.
    pub actor1: String,
    /// Secondary actor (when present).
    pub actor2: String,
    /// Country name.
    pub country: String,
    /// Admin level 1 (province/state).
    pub admin1: String,
    /// Location name.
    pub location: String,
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
    /// Reported fatalities.
    pub fatalities: i64,
    /// Free-form notes.
    pub notes: String,
}

#[derive(Debug, Deserialize)]
struct AcledResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    data: Vec<RawEvent>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvent {
    #[serde(default)]
    event_id_cnty: String,
    #[serde(default)]
    event_date: String,
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    sub_event_type: String,
    #[serde(default)]
    actor1: String,
    #[serde(default)]
    actor2: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    admin1: String,
    #[serde(default)]
    location: String,
    // ACLED returns numeric fields as strings.
    #[serde(default)]
    latitude: Value,
    #[serde(default)]
    longitude: Value,
    #[serde(default)]
    fatalities: Value,
    #[serde(default)]
    notes: String,
}

impl AcledEvent {
    fn from_raw(raw: RawEvent) -> Self {
        Self {
            event_id_cnty: raw.event_id_cnty,
            event_date: raw.event_date,
            event_type: raw.event_type,
            sub_event_type: raw.sub_event_type,
            actor1: raw.actor1,
            actor2: raw.actor2,
            country: raw.country,
            admin1: raw.admin1,
            location: raw.location,
            latitude: parse_value_f64(&raw.latitude),
            longitude: parse_value_f64(&raw.longitude),
            fatalities: parse_value_i64(&raw.fatalities),
            notes: raw.notes,
        }
    }
}

fn parse_value_f64(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn parse_value_i64(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.trim().parse::<i64>().unwrap_or(0),
        _ => 0,
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
            "status":  200,
            "success": true,
            "count":   2,
            "data": [
                {
                    "event_id_cnty":  "SYR12345",
                    "event_date":     "2024-04-25",
                    "event_type":     "Battles",
                    "sub_event_type": "Armed clash",
                    "actor1":         "Government of Syria",
                    "actor2":         "ISIS",
                    "country":        "Syria",
                    "admin1":         "Aleppo",
                    "location":       "Manbij",
                    "latitude":       "36.523",
                    "longitude":      "37.952",
                    "fatalities":     "5",
                    "notes":          "Test note"
                },
                {
                    "event_id_cnty":  "IRQ54321",
                    "event_date":     "2024-04-25",
                    "event_type":     "Protests",
                    "sub_event_type": "Peaceful protest",
                    "actor1":         "Protesters",
                    "country":        "Iraq",
                    "location":       "Baghdad",
                    "latitude":       33.31,
                    "longitude":      44.36,
                    "fatalities":     0
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> AcledClient {
        AcledClient::new(
            AcledConfig {
                base_url: server.uri(),
                email: "test@example.com".into(),
                key: "test-key".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_events_maps_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acled/read"))
            .and(query_param("email", "test@example.com"))
            .and(query_param("key", "test-key"))
            .and(query_param("iso", "760|368"))
            .and(query_param("event_date", "2024-04-25|2024-05-04"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let events = client
            .fetch_events(&[760, 368], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_id_cnty, "SYR12345");
        assert_eq!(events[0].country, "Syria");
        assert!((events[0].latitude - 36.523).abs() < 1e-9);
        assert_eq!(events[0].fatalities, 5);
        // Second event uses numeric (not stringified) coords.
        assert!((events[1].latitude - 33.31).abs() < 1e-9);
        assert_eq!(events[1].fatalities, 0);
    }

    #[tokio::test]
    async fn fetch_events_success_false_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acled/read"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status":  401,
                "success": false,
                "data":    []
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_events(&[760], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_events_empty_iso_returns_empty_no_call() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acled/read"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        assert!(client
            .fetch_events(&[], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn fetch_events_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acled/read"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_events(&[760], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_events_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/acled/read"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_events(&[760], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_events_unreachable_yields_io() {
        let client = AcledClient::new(
            AcledConfig {
                base_url: "http://127.0.0.1:1".into(),
                email: "x@x".into(),
                key: "k".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_events(&[760], "2024-04-25", "2024-05-04", 500)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn parse_value_f64_handles_numeric_and_string() {
        assert!((parse_value_f64(&Value::String("36.523".into())) - 36.523).abs() < 1e-9);
        assert!(
            (parse_value_f64(&Value::Number(serde_json::Number::from_f64(33.31).unwrap()))
                - 33.31)
                .abs()
                < 1e-9
        );
        assert!((parse_value_f64(&Value::Null) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn parse_value_i64_handles_numeric_and_string() {
        assert_eq!(parse_value_i64(&Value::String("5".into())), 5);
        assert_eq!(parse_value_i64(&Value::Number(serde_json::Number::from(0))), 0);
        assert_eq!(parse_value_i64(&Value::Null), 0);
    }

    #[test]
    fn config_with_credentials_uses_production() {
        let cfg = AcledConfig::with_credentials("e@x", "k");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.email, "e@x");
        assert_eq!(cfg.key, "k");
    }
}
