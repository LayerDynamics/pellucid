//! NASA EONET v3 (Earth Observatory Natural Event Tracker).
//!
//! EONET aggregates wildfire, volcano, severe-storm, drought,
//! flood, etc. event metadata from NASA's various EO sources.
//!
//! Endpoint:
//! ```text
//! GET https://eonet.gsfc.nasa.gov/api/v3/events?status=open&days=7
//! ```
//!
//! Free, no auth, JSON. Refreshed continuously.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "events": [
//!     {
//!       "id":     "EONET_12345",
//!       "title":  "Wildfire — Klamath National Forest",
//!       "link":   "https://eonet.gsfc.nasa.gov/api/v3/events/EONET_12345",
//!       "categories": [{ "id": "wildfires", "title": "Wildfires" }],
//!       "geometry": [
//!         {
//!           "magnitudeValue": 12500.0,
//!           "magnitudeUnit":  "acres",
//!           "date":           "2026-04-25T18:00:00Z",
//!           "type":           "Point",
//!           "coordinates":    [-122.5, 41.7]
//!         }
//!       ]
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::error::StreamsError;

/// Default base URL — NASA EONET production.
pub const DEFAULT_BASE_URL: &str = "https://eonet.gsfc.nasa.gov";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct EonetConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for EonetConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable EONET client.
#[derive(Clone, Debug)]
pub struct NasaEonetClient {
    http: reqwest::Client,
    config: EonetConfig,
}

impl NasaEonetClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: EonetConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = EonetConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch open events filtered by `category` (e.g.
    /// `"volcanoes"`, `"wildfires"`, `"severeStorms"`) over
    /// the past `days`. Pass `None` for `category` to fetch
    /// every category.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_events(
        &self,
        category: Option<&str>,
        days: u32,
    ) -> Result<Vec<NaturalEvent>, StreamsError> {
        let url = self.build_events_url(category, days)?;
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
        let body: EventsResponse = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body.events.into_iter().map(NaturalEvent::from_raw).collect())
    }

    fn build_events_url(
        &self,
        category: Option<&str>,
        days: u32,
    ) -> Result<Url, StreamsError> {
        let raw = format!("{}/api/v3/events", self.config.base_url);
        let mut url = Url::parse(&raw)
            .map_err(|e| StreamsError::Parse(format!("eonet url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("status", "open");
            q.append_pair("days", &days.to_string());
            if let Some(cat) = category {
                q.append_pair("category", cat);
            }
        }
        Ok(url)
    }
}

/// One natural event.
#[derive(Clone, Debug, PartialEq)]
pub struct NaturalEvent {
    /// EONET event id (e.g. `"EONET_12345"`).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Permalink to the EONET event detail.
    pub link: String,
    /// Comma-separated category titles (e.g. `"Wildfires"`).
    pub categories: String,
    /// Latest geometry point (most-recent observation).
    pub latest_geometry: Option<EventGeometry>,
}

/// Last reported point geometry for an event.
#[derive(Clone, Debug, PartialEq)]
pub struct EventGeometry {
    /// ISO-8601 UTC timestamp.
    pub date: String,
    /// `Point` or `Polygon` (we only retain `Point` rows;
    /// polygons don't fit a single coordinate pair).
    pub kind: String,
    /// Longitude (WGS84). 0.0 when the geometry is missing.
    pub longitude: f64,
    /// Latitude (WGS84). 0.0 when the geometry is missing.
    pub latitude: f64,
    /// Magnitude value (e.g. acres burned, ash plume height).
    /// 0.0 when the upstream omits it.
    pub magnitude_value: f64,
    /// Magnitude unit (e.g. `"acres"`, `"m"`). Empty when
    /// absent.
    pub magnitude_unit: String,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    #[serde(default)]
    events: Vec<RawEvent>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    categories: Vec<RawCategory>,
    #[serde(default)]
    geometry: Vec<RawGeometry>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCategory {
    #[serde(default)]
    title: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeometry {
    #[serde(default)]
    date: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    coordinates: Value,
    #[serde(default, rename = "magnitudeValue")]
    magnitude_value: Option<f64>,
    #[serde(default, rename = "magnitudeUnit")]
    magnitude_unit: Option<String>,
}

impl NaturalEvent {
    fn from_raw(raw: RawEvent) -> Self {
        let categories = raw
            .categories
            .iter()
            .map(|c| c.title.clone())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let latest_geometry = raw
            .geometry
            .into_iter()
            .rfind(|g| g.kind == "Point")
            .map(|g| {
                let (lon, lat) = match &g.coordinates {
                    Value::Array(arr) if arr.len() >= 2 => (
                        arr[0].as_f64().unwrap_or(0.0),
                        arr[1].as_f64().unwrap_or(0.0),
                    ),
                    _ => (0.0, 0.0),
                };
                EventGeometry {
                    date: g.date,
                    kind: g.kind,
                    longitude: lon,
                    latitude: lat,
                    magnitude_value: g.magnitude_value.unwrap_or(0.0),
                    magnitude_unit: g.magnitude_unit.unwrap_or_default(),
                }
            });
        Self {
            id: raw.id,
            title: raw.title,
            link: raw.link,
            categories,
            latest_geometry,
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
            "events": [
                {
                    "id":    "EONET_12345",
                    "title": "Wildfire — Klamath National Forest",
                    "link":  "https://eonet.gsfc.nasa.gov/api/v3/events/EONET_12345",
                    "categories": [{ "id": "wildfires", "title": "Wildfires" }],
                    "geometry": [
                        {
                            "magnitudeValue": 12500.0,
                            "magnitudeUnit":  "acres",
                            "date":           "2026-04-25T18:00:00Z",
                            "type":           "Point",
                            "coordinates":    [-122.5, 41.7]
                        },
                        {
                            "magnitudeValue": 14500.0,
                            "magnitudeUnit":  "acres",
                            "date":           "2026-04-26T18:00:00Z",
                            "type":           "Point",
                            "coordinates":    [-122.6, 41.8]
                        }
                    ]
                },
                {
                    "id":    "EONET_67890",
                    "title": "Volcano — Kilauea",
                    "link":  "https://eonet.gsfc.nasa.gov/api/v3/events/EONET_67890",
                    "categories": [{ "id": "volcanoes", "title": "Volcanoes" }],
                    "geometry": []
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> NasaEonetClient {
        NasaEonetClient::new(
            EonetConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_events_returns_two_events_with_latest_geometry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/events"))
            .and(query_param("status", "open"))
            .and(query_param("days", "7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let events = client.fetch_events(None, 7).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, "EONET_12345");
        assert_eq!(events[0].categories, "Wildfires");
        let g = events[0].latest_geometry.as_ref().expect("Some");
        // Most-recent geometry (the second one in the input array).
        assert_eq!(g.date, "2026-04-26T18:00:00Z");
        assert!((g.magnitude_value - 14500.0).abs() < 1e-9);
        assert_eq!(g.magnitude_unit, "acres");
        // Second event has no geometry — None.
        assert!(events[1].latest_geometry.is_none());
    }

    #[tokio::test]
    async fn fetch_events_passes_category_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/events"))
            .and(query_param("category", "volcanoes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let events = client.fetch_events(Some("volcanoes"), 7).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn fetch_events_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/events"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_events(None, 7).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_events_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/events"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_events(None, 7).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_events_unreachable_yields_io() {
        let client = NasaEonetClient::new(
            EonetConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_events(None, 7).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = EonetConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
