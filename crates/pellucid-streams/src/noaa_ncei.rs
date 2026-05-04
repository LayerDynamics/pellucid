//! NOAA NCEI Climate-at-a-Glance global temperature anomalies.
//!
//! NCEI publishes the canonical global land+ocean temperature
//! anomaly series at:
//!
//! ```text
//! GET https://www.ncei.noaa.gov/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json
//! ```
//!
//! Response shape (relevant subset):
//! ```json
//! {
//!   "description": { "title": "Global Land and Ocean", "units": "degrees Celsius" },
//!   "data": {
//!     "188012": "-0.40",
//!     "188112": "-0.35",
//!     ...
//!     "202612": "1.10"
//!   }
//! }
//! ```
//!
//! Keys are `YYYYMM` strings; values are stringified °C
//! anomaly relative to the 20th-century mean. The seeder asks
//! for the full series and pulls the most-recent entry; the
//! free public endpoint streams the whole series in one
//! response (~25 KiB).

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::error::StreamsError;

/// Default base URL — NCEI's public climate-at-a-glance API.
pub const DEFAULT_BASE_URL: &str = "https://www.ncei.noaa.gov";

/// Default per-request timeout — 12 s. NCEI is usually fast
/// (<1 s); the headroom protects against the rare slow path.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent. NCEI accepts unmarked clients but a
/// branded UA helps if they ever publish per-tenant
/// throttling.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct NoaaNceiConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for NoaaNceiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable NOAA NCEI client.
#[derive(Clone, Debug)]
pub struct NoaaNceiClient {
    http: reqwest::Client,
    config: NoaaNceiConfig,
}

impl NoaaNceiClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: NoaaNceiConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = NoaaNceiConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the global land+ocean temperature anomaly series
    /// from `1880` through `end_year`. Returns the parsed
    /// series in chronological order.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_global_land_ocean(
        &self,
        end_year: u16,
    ) -> Result<TemperatureAnomalySeries, StreamsError> {
        let url = format!(
            "{}/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-{end_year}/data.json",
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
        let body: Value = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        parse_series(&body)
    }
}

/// One annual anomaly reading.
#[derive(Clone, Debug, PartialEq)]
pub struct TemperatureAnomaly {
    /// Calendar year (e.g. `2026`).
    pub year: u16,
    /// °C anomaly relative to the 20th-century mean.
    pub anomaly_c: f64,
}

/// Distilled series.
#[derive(Clone, Debug, PartialEq)]
pub struct TemperatureAnomalySeries {
    /// Series title from the upstream description (e.g.
    /// `"Global Land and Ocean Temperature Anomalies"`).
    pub title: String,
    /// Units (always `"degrees Celsius"` for this endpoint;
    /// echoed for safety).
    pub units: String,
    /// Annual readings, oldest-first.
    pub readings: Vec<TemperatureAnomaly>,
}

impl TemperatureAnomalySeries {
    /// Most-recent reading. `None` for an empty series.
    #[must_use]
    pub fn latest(&self) -> Option<&TemperatureAnomaly> {
        self.readings.last()
    }
}

#[derive(Debug, Deserialize)]
struct NceiResponse {
    description: Description,
    data: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct Description {
    #[serde(default)]
    title: String,
    #[serde(default)]
    units: String,
}

fn parse_series(body: &Value) -> Result<TemperatureAnomalySeries, StreamsError> {
    let parsed: NceiResponse = serde_json::from_value(body.clone())
        .map_err(|e| StreamsError::Parse(format!("ncei response: {e}")))?;
    let mut readings: Vec<TemperatureAnomaly> = Vec::with_capacity(parsed.data.len());
    for (key, val) in &parsed.data {
        // Key shape: "YYYYMM" — we want the year prefix.
        if key.len() < 4 {
            continue;
        }
        let year = match key[..4].parse::<u16>() {
            Ok(y) => y,
            Err(_) => continue,
        };
        let anomaly = match val {
            Value::String(s) => s.parse::<f64>().ok(),
            Value::Number(n) => n.as_f64(),
            _ => None,
        };
        let Some(anomaly_c) = anomaly else { continue };
        readings.push(TemperatureAnomaly { year, anomaly_c });
    }
    if readings.is_empty() {
        return Err(StreamsError::Parse("ncei response: empty series".into()));
    }
    readings.sort_by_key(|r| r.year);
    Ok(TemperatureAnomalySeries {
        title: parsed.description.title,
        units: parsed.description.units,
        readings,
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!({
            "description": {
                "title": "Global Land and Ocean Temperature Anomalies",
                "units": "degrees Celsius"
            },
            "data": {
                "188012": "-0.40",
                "200012":  "0.42",
                "202612":  "1.10"
            }
        })
    }

    fn client_pointing_at(server: &MockServer) -> NoaaNceiClient {
        NoaaNceiClient::new(
            NoaaNceiConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_global_returns_three_readings_sorted() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let series = client.fetch_global_land_ocean(2026).await.unwrap();
        assert_eq!(series.readings.len(), 3);
        assert_eq!(series.readings[0].year, 1880);
        assert_eq!(series.readings[2].year, 2026);
        assert!((series.readings[2].anomaly_c - 1.10).abs() < 1e-9);
        assert_eq!(series.latest().unwrap().year, 2026);
        assert!(series.title.contains("Global"));
        assert_eq!(series.units, "degrees Celsius");
    }

    #[tokio::test]
    async fn fetch_global_handles_numeric_values() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "description": { "title": "Global", "units": "degrees Celsius" },
                "data": { "202612": 1.10, "202512": 1.05 }
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let series = client.fetch_global_land_ocean(2026).await.unwrap();
        assert_eq!(series.readings.len(), 2);
        assert!((series.readings[1].anomaly_c - 1.10).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_global_skips_unparseable_keys() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "description": { "title": "Global", "units": "degrees Celsius" },
                "data": {
                    "ABC":     "1.0",
                    "12":      "2.0",
                    "202612":  "1.10"
                }
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let series = client.fetch_global_land_ocean(2026).await.unwrap();
        assert_eq!(series.readings.len(), 1);
        assert_eq!(series.readings[0].year, 2026);
    }

    #[tokio::test]
    async fn fetch_global_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_land_ocean(2026).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_global_unparseable_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_land_ocean(2026).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_global_empty_data_yields_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/12/12/1880-2026/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "description": { "title": "Global", "units": "degrees Celsius" },
                "data": {}
            })))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_land_ocean(2026).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_global_unreachable_yields_io_error() {
        let client = NoaaNceiClient::new(
            NoaaNceiConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_global_land_ocean(2026).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = NoaaNceiConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
