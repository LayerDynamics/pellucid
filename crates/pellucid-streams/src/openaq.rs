//! OpenAQ v2 latest-measurements client.
//!
//! OpenAQ aggregates air-quality readings from government and
//! research stations worldwide. The `v2/latest` endpoint
//! returns the most-recent reading per station and per
//! parameter:
//!
//! ```text
//! GET https://api.openaq.org/v2/latest?country=US&limit=100
//! ```
//!
//! Free, no auth on v2 (v3 requires an API key as of mid-2024
//! and is not used here).
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "results": [
//!     {
//!       "location":  "Downtown LA",
//!       "city":      "Los Angeles",
//!       "country":   "US",
//!       "coordinates": { "latitude": 34.05, "longitude": -118.24 },
//!       "measurements": [
//!         { "parameter": "pm25", "value": 18.4, "unit": "µg/m³",
//!           "lastUpdated": "2026-05-04T16:00:00+00:00" },
//!         { "parameter": "no2",  "value":  0.038, "unit": "ppm",
//!           "lastUpdated": "2026-05-04T16:00:00+00:00" }
//!       ]
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — OpenAQ v2 production.
pub const DEFAULT_BASE_URL: &str = "https://api.openaq.org";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct OpenAqConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for OpenAqConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable OpenAQ v2 client.
#[derive(Clone, Debug)]
pub struct OpenAqClient {
    http: reqwest::Client,
    config: OpenAqConfig,
}

impl OpenAqClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: OpenAqConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = OpenAqConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the latest measurements for `countries` (ISO
    /// 3166-1 alpha-2 codes). Up to `limit` station rows.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_latest(
        &self,
        countries: &[&str],
        limit: u32,
    ) -> Result<Vec<AirQualityStation>, StreamsError> {
        let url = self.build_latest_url(countries, limit)?;
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
        let body: LatestResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body
            .results
            .into_iter()
            .map(AirQualityStation::from_raw)
            .collect())
    }

    fn build_latest_url(&self, countries: &[&str], limit: u32) -> Result<Url, StreamsError> {
        let raw = format!("{}/v2/latest", self.config.base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("openaq url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("limit", &limit.to_string());
            for c in countries {
                q.append_pair("country", c);
            }
        }
        Ok(url)
    }
}

/// One station with its latest measurements.
#[derive(Clone, Debug, PartialEq)]
pub struct AirQualityStation {
    /// Station name (e.g. `"Downtown LA"`).
    pub location: String,
    /// City (when reported).
    pub city: String,
    /// ISO 3166-1 alpha-2 country code.
    pub country: String,
    /// WGS84 latitude. 0.0 when absent.
    pub latitude: f64,
    /// WGS84 longitude. 0.0 when absent.
    pub longitude: f64,
    /// Per-parameter latest measurements.
    pub measurements: Vec<AirMeasurement>,
}

/// One latest measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct AirMeasurement {
    /// Pollutant code (e.g. `"pm25"`, `"no2"`, `"o3"`).
    pub parameter: String,
    /// Concentration.
    pub value: f64,
    /// Unit string (`"µg/m³"`, `"ppm"`, etc.).
    pub unit: String,
    /// ISO-8601 timestamp.
    pub last_updated: String,
}

#[derive(Debug, Deserialize)]
struct LatestResponse {
    #[serde(default)]
    results: Vec<RawStation>,
}

#[derive(Debug, Default, Deserialize)]
struct RawStation {
    #[serde(default)]
    location: String,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    coordinates: Option<RawCoordinates>,
    #[serde(default)]
    measurements: Vec<RawMeasurement>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCoordinates {
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawMeasurement {
    #[serde(default)]
    parameter: String,
    #[serde(default)]
    value: f64,
    #[serde(default)]
    unit: String,
    #[serde(default, rename = "lastUpdated")]
    last_updated: String,
}

impl AirQualityStation {
    fn from_raw(raw: RawStation) -> Self {
        let (lat, lon) = raw
            .coordinates
            .as_ref()
            .map(|c| (c.latitude.unwrap_or(0.0), c.longitude.unwrap_or(0.0)))
            .unwrap_or((0.0, 0.0));
        Self {
            location: raw.location,
            city: raw.city.unwrap_or_default(),
            country: raw.country.unwrap_or_default(),
            latitude: lat,
            longitude: lon,
            measurements: raw
                .measurements
                .into_iter()
                .map(|m| AirMeasurement {
                    parameter: m.parameter,
                    value: m.value,
                    unit: m.unit,
                    last_updated: m.last_updated,
                })
                .collect(),
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
            "results": [
                {
                    "location": "Downtown LA",
                    "city":     "Los Angeles",
                    "country":  "US",
                    "coordinates": { "latitude": 34.05, "longitude": -118.24 },
                    "measurements": [
                        { "parameter": "pm25", "value": 18.4, "unit": "µg/m³",
                          "lastUpdated": "2026-05-04T16:00:00+00:00" },
                        { "parameter": "no2",  "value":  0.038, "unit": "ppm",
                          "lastUpdated": "2026-05-04T16:00:00+00:00" }
                    ]
                },
                {
                    "location": "Brooklyn — IS 314",
                    "country":  "US",
                    "measurements": []
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> OpenAqClient {
        OpenAqClient::new(
            OpenAqConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_latest_maps_two_stations() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/latest"))
            .and(query_param("limit", "100"))
            .and(query_param("country", "US"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let stations = client.fetch_latest(&["US"], 100).await.unwrap();
        assert_eq!(stations.len(), 2);
        let la = &stations[0];
        assert_eq!(la.location, "Downtown LA");
        assert_eq!(la.city, "Los Angeles");
        assert_eq!(la.country, "US");
        assert!((la.latitude - 34.05).abs() < 1e-9);
        assert!((la.longitude + 118.24).abs() < 1e-9);
        assert_eq!(la.measurements.len(), 2);
        assert_eq!(la.measurements[0].parameter, "pm25");
        assert!((la.measurements[0].value - 18.4).abs() < 1e-9);
        assert_eq!(la.measurements[0].unit, "µg/m³");
        // Brooklyn station has no coords nor measurements.
        let bk = &stations[1];
        assert_eq!(bk.location, "Brooklyn — IS 314");
        assert!((bk.latitude - 0.0).abs() < 1e-9);
        assert!(bk.measurements.is_empty());
    }

    #[tokio::test]
    async fn fetch_latest_supports_multiple_country_filters() {
        let server = MockServer::start().await;
        // OpenAQ accepts repeated `country=` query params.
        Mock::given(method("GET"))
            .and(path("/v2/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let stations = client.fetch_latest(&["US", "CA"], 50).await.unwrap();
        assert_eq!(stations.len(), 2);
    }

    #[tokio::test]
    async fn fetch_latest_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/latest"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_latest(&["US"], 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_latest_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_latest(&["US"], 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_latest_unreachable_yields_io() {
        let client = OpenAqClient::new(
            OpenAqConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_latest(&["US"], 100).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = OpenAqConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
