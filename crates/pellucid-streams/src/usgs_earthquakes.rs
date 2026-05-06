//! USGS Earthquake Hazards Program GeoJSON feeds.
//!
//! USGS publishes summary earthquake feeds at:
//!
//! ```text
//! GET https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_day.geojson
//! GET https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_week.geojson
//! GET https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/significant_day.geojson
//! ```
//!
//! Free, no auth, JSON. Refreshed every minute.
//!
//! Response (relevant subset):
//! ```json
//! {
//!   "type": "FeatureCollection",
//!   "features": [
//!     {
//!       "type": "Feature",
//!       "properties": {
//!         "mag":   5.4,
//!         "place": "12 km SSW of Volcano, Hawaii",
//!         "time":  1714060800000,
//!         "url":   "https://earthquake.usgs.gov/earthquakes/eventpage/hv12345",
//!         "tsunami": 0,
//!         "alert":  "green"
//!       },
//!       "geometry": { "coordinates": [-155.34, 19.27, 5.6] }
//!     }
//!   ]
//! }
//! ```

use std::time::Duration;

use serde::Deserialize;

use crate::error::StreamsError;

/// Default base URL — USGS Earthquake Hazards.
pub const DEFAULT_BASE_URL: &str = "https://earthquake.usgs.gov";

/// Default per-request timeout — 12 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Feed window — `all_day` / `all_week` / `all_month` /
/// `significant_*`. Maps 1:1 to the USGS path component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedWindow {
    /// Past 24 hours, all magnitudes.
    AllDay,
    /// Past 7 days, all magnitudes.
    AllWeek,
    /// Past 30 days, all magnitudes.
    AllMonth,
    /// Past 24 hours, significant only.
    SignificantDay,
    /// Past 7 days, significant only.
    SignificantWeek,
}

impl FeedWindow {
    /// Path slug used in the USGS URL.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::AllDay => "all_day",
            Self::AllWeek => "all_week",
            Self::AllMonth => "all_month",
            Self::SignificantDay => "significant_day",
            Self::SignificantWeek => "significant_week",
        }
    }
}

/// Configuration.
#[derive(Clone, Debug)]
pub struct UsgsConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for UsgsConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable USGS earthquake feed client.
#[derive(Clone, Debug)]
pub struct UsgsEarthquakesClient {
    http: reqwest::Client,
    config: UsgsConfig,
}

impl UsgsEarthquakesClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: UsgsConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = UsgsConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch one of the canned summary feeds.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_feed(
        &self,
        window: FeedWindow,
    ) -> Result<Vec<EarthquakeEvent>, StreamsError> {
        let url = format!(
            "{}/earthquakes/feed/v1.0/summary/{}.geojson",
            self.config.base_url,
            window.slug()
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
        let body: FeatureCollection = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        Ok(body
            .features
            .into_iter()
            .filter_map(EarthquakeEvent::from_feature)
            .collect())
    }
}

/// One earthquake event.
#[derive(Clone, Debug, PartialEq)]
pub struct EarthquakeEvent {
    /// Magnitude (Richter / moment).
    pub magnitude: f64,
    /// Human-readable place description.
    pub place: String,
    /// Wall-clock ms when the event occurred.
    pub time_ms: i64,
    /// USGS event-page URL.
    pub url: String,
    /// `1` if a tsunami warning was issued, else `0`.
    pub tsunami: i64,
    /// PAGER alert level (`"green" | "yellow" | "orange" |
    /// "red" | ""`).
    pub alert: String,
    /// Longitude (WGS84).
    pub longitude: f64,
    /// Latitude (WGS84).
    pub latitude: f64,
    /// Depth in km (positive into the Earth).
    pub depth_km: f64,
}

#[derive(Debug, Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    #[serde(default)]
    properties: FeatureProps,
    #[serde(default)]
    geometry: Geometry,
}

#[derive(Debug, Default, Deserialize)]
struct FeatureProps {
    #[serde(default)]
    mag: Option<f64>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    time: Option<i64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    tsunami: Option<i64>,
    #[serde(default)]
    alert: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Geometry {
    #[serde(default)]
    coordinates: Vec<f64>,
}

impl EarthquakeEvent {
    fn from_feature(f: Feature) -> Option<Self> {
        let coords = &f.geometry.coordinates;
        // GeoJSON convention: [lon, lat, depth].
        let (lon, lat, depth) = (
            coords.first().copied().unwrap_or(0.0),
            coords.get(1).copied().unwrap_or(0.0),
            coords.get(2).copied().unwrap_or(0.0),
        );
        Some(Self {
            magnitude: f.properties.mag?,
            place: f.properties.place.unwrap_or_default(),
            time_ms: f.properties.time?,
            url: f.properties.url.unwrap_or_default(),
            tsunami: f.properties.tsunami.unwrap_or(0),
            alert: f.properties.alert.unwrap_or_default(),
            longitude: lon,
            latitude: lat,
            depth_km: depth,
        })
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
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": {
                        "mag":   5.4,
                        "place": "12 km SSW of Volcano, Hawaii",
                        "time":  1714060800000_i64,
                        "url":   "https://earthquake.usgs.gov/earthquakes/eventpage/hv12345",
                        "tsunami": 0,
                        "alert": "green"
                    },
                    "geometry": { "type": "Point", "coordinates": [-155.34, 19.27, 5.6] }
                },
                {
                    "type": "Feature",
                    // No `mag` — must be filtered out.
                    "properties": { "place": "missing magnitude", "time": 0 },
                    "geometry": { "type": "Point", "coordinates": [0, 0, 0] }
                }
            ]
        })
    }

    fn client_pointing_at(server: &MockServer) -> UsgsEarthquakesClient {
        UsgsEarthquakesClient::new(
            UsgsConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_feed_all_day_maps_one_event() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/earthquakes/feed/v1.0/summary/all_day.geojson"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let events = client.fetch_feed(FeedWindow::AllDay).await.unwrap();
        // The mag-less event is filtered.
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert!((e.magnitude - 5.4).abs() < 1e-9);
        assert_eq!(e.place, "12 km SSW of Volcano, Hawaii");
        assert_eq!(e.time_ms, 1_714_060_800_000);
        assert!((e.longitude + 155.34).abs() < 1e-9);
        assert!((e.latitude - 19.27).abs() < 1e-9);
        assert!((e.depth_km - 5.6).abs() < 1e-9);
        assert_eq!(e.alert, "green");
    }

    #[tokio::test]
    async fn fetch_feed_significant_week_uses_correct_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/earthquakes/feed/v1.0/summary/significant_week.geojson",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let events = client
            .fetch_feed(FeedWindow::SignificantWeek)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn fetch_feed_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/earthquakes/feed/v1.0/summary/all_day.geojson"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_feed(FeedWindow::AllDay).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_feed_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/earthquakes/feed/v1.0/summary/all_day.geojson"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_feed(FeedWindow::AllDay).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_feed_unreachable_yields_io() {
        let client = UsgsEarthquakesClient::new(
            UsgsConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_feed(FeedWindow::AllDay).await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn feed_window_slug_round_trips() {
        assert_eq!(FeedWindow::AllDay.slug(), "all_day");
        assert_eq!(FeedWindow::AllWeek.slug(), "all_week");
        assert_eq!(FeedWindow::AllMonth.slug(), "all_month");
        assert_eq!(FeedWindow::SignificantDay.slug(), "significant_day");
        assert_eq!(FeedWindow::SignificantWeek.slug(), "significant_week");
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = UsgsConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
