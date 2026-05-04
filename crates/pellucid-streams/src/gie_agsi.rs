//! GIE AGSI (Aggregated Gas Storage Inventory) JSON client.
//!
//! Gas Infrastructure Europe (GIE) publishes the AGSI+ daily
//! storage data via a free REST API that requires registration
//! for an API key:
//!
//! ```text
//! GET https://agsi.gie.eu/api?country=DE&from=2026-04-25&to=2026-05-04
//! Header: x-key: <api_key>
//! ```
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "data": [
//!     {
//!       "gasDayStart":   "2026-04-25",
//!       "name":          "Germany",
//!       "code":          "DE",
//!       "url":           "https://agsi.gie.eu/#/historical/DE",
//!       "gasInStorage":  214.55,
//!       "consumption":   53.24,
//!       "consumptionFull": null,
//!       "injection":     85.12,
//!       "withdrawal":    12.34,
//!       "workingGasVolume": 234.0,
//!       "injectionCapacity": 5.6,
//!       "withdrawalCapacity": 4.5,
//!       "status":        "E",
//!       "trend":         0.42,
//!       "full":          91.7
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — GIE AGSI production.
pub const DEFAULT_BASE_URL: &str = "https://agsi.gie.eu";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct GieAgsiConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Registered AGSI API key (sent as `x-key` header).
    pub api_key: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl GieAgsiConfig {
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

/// Pluggable GIE AGSI client.
#[derive(Clone, Debug)]
pub struct GieAgsiClient {
    http: reqwest::Client,
    config: GieAgsiConfig,
}

impl GieAgsiClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: GieAgsiConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production(api_key: impl Into<String>) -> Result<Self, StreamsError> {
        let cfg = GieAgsiConfig::with_api_key(api_key);
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch storage data for a country (ISO 2-letter code) over
    /// `[from_date, to_date]` (`YYYY-MM-DD`).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_country(
        &self,
        country: &str,
        from_date: &str,
        to_date: &str,
    ) -> Result<Vec<GasStorageRow>, StreamsError> {
        let url = self.build_url(country, from_date, to_date)?;
        let resp = self
            .http
            .get(url)
            .header("user-agent", &self.config.user_agent)
            .header("x-key", &self.config.api_key)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body: GieResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.data.into_iter().map(GasStorageRow::from_raw).collect())
    }

    fn build_url(
        &self,
        country: &str,
        from_date: &str,
        to_date: &str,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/api", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("agsi url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("country", country);
            q.append_pair("from", from_date);
            q.append_pair("to", to_date);
        }
        Ok(url)
    }
}

/// One day's storage reading for one country.
#[derive(Clone, Debug, PartialEq)]
pub struct GasStorageRow {
    /// Gas-day start (`YYYY-MM-DD`).
    pub gas_day_start: String,
    /// Country name (e.g. `"Germany"`).
    pub name: String,
    /// ISO 2-letter country code.
    pub code: String,
    /// Gas in storage (TWh).
    pub gas_in_storage_twh: f64,
    /// Working gas volume (TWh).
    pub working_gas_volume_twh: f64,
    /// Storage % full.
    pub full_pct: f64,
    /// Daily injection (TWh).
    pub injection_twh: f64,
    /// Daily withdrawal (TWh).
    pub withdrawal_twh: f64,
    /// Daily consumption (TWh) — `null` from upstream surfaces as 0.
    pub consumption_twh: f64,
    /// Day-over-day trend.
    pub trend: f64,
    /// Status flag (`E` for estimated, `C` for confirmed, …).
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct GieResponse {
    #[serde(default)]
    data: Vec<RawRow>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRow {
    #[serde(default, rename = "gasDayStart")]
    gas_day_start: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default, rename = "gasInStorage")]
    gas_in_storage: Value,
    #[serde(default, rename = "workingGasVolume")]
    working_gas_volume: Value,
    #[serde(default)]
    full: Value,
    #[serde(default)]
    injection: Value,
    #[serde(default)]
    withdrawal: Value,
    #[serde(default)]
    consumption: Value,
    #[serde(default)]
    trend: Value,
    #[serde(default)]
    status: String,
}

impl GasStorageRow {
    fn from_raw(raw: RawRow) -> Self {
        Self {
            gas_day_start: raw.gas_day_start,
            name: raw.name,
            code: raw.code,
            gas_in_storage_twh: parse_value_f64(&raw.gas_in_storage),
            working_gas_volume_twh: parse_value_f64(&raw.working_gas_volume),
            full_pct: parse_value_f64(&raw.full),
            injection_twh: parse_value_f64(&raw.injection),
            withdrawal_twh: parse_value_f64(&raw.withdrawal),
            consumption_twh: parse_value_f64(&raw.consumption),
            trend: parse_value_f64(&raw.trend),
            status: raw.status,
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "data": [
                {
                    "gasDayStart":          "2026-04-25",
                    "name":                 "Germany",
                    "code":                 "DE",
                    "url":                  "https://agsi.gie.eu/#/historical/DE",
                    "gasInStorage":         214.55,
                    "consumption":          53.24,
                    "consumptionFull":      null,
                    "injection":            85.12,
                    "withdrawal":           12.34,
                    "workingGasVolume":     234.0,
                    "injectionCapacity":    5.6,
                    "withdrawalCapacity":   4.5,
                    "status":               "E",
                    "trend":                0.42,
                    "full":                 91.7
                },
                {
                    "gasDayStart":  "2026-04-26",
                    "name":         "Germany",
                    "code":         "DE",
                    "gasInStorage": "215.10",
                    "full":         "91.9",
                    "status":       "C"
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> GieAgsiClient {
        GieAgsiClient::new(
            GieAgsiConfig {
                base_url: server.uri(),
                api_key: "test-key".into(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_country_maps_two_rows_with_api_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(query_param("country", "DE"))
            .and(query_param("from", "2026-04-25"))
            .and(query_param("to", "2026-05-04"))
            .and(header("x-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_country("DE", "2026-04-25", "2026-05-04")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].code, "DE");
        assert_eq!(rows[0].gas_day_start, "2026-04-25");
        assert!((rows[0].gas_in_storage_twh - 214.55).abs() < 1e-6);
        assert!((rows[0].full_pct - 91.7).abs() < 1e-9);
        assert_eq!(rows[0].status, "E");
        // Second row uses stringified values.
        assert!((rows[1].gas_in_storage_twh - 215.10).abs() < 1e-6);
        assert!((rows[1].full_pct - 91.9).abs() < 1e-6);
        assert_eq!(rows[1].status, "C");
    }

    #[tokio::test]
    async fn fetch_country_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_country("DE", "2026-04-25", "2026-05-04")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_country_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_country("DE", "2026-04-25", "2026-05-04")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_country_unreachable_yields_io() {
        let client = GieAgsiClient::new(
            GieAgsiConfig {
                base_url: "http://127.0.0.1:1".into(),
                api_key: "k".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_country("DE", "2026-04-25", "2026-05-04")
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn parse_value_f64_handles_string_number_null() {
        assert!((parse_value_f64(&Value::String("214.55".into())) - 214.55).abs() < 1e-6);
        assert!(
            (parse_value_f64(&Value::Number(serde_json::Number::from_f64(91.7).unwrap()))
                - 91.7)
                .abs()
                < 1e-9
        );
        assert!((parse_value_f64(&Value::Null) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn config_with_api_key_uses_production() {
        let cfg = GieAgsiConfig::with_api_key("k");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.api_key, "k");
    }
}
