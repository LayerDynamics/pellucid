//! NASA FIRMS active fire detections (VIIRS S-NPP 24h CSV).
//!
//! NASA's Fire Information for Resource Management System
//! (FIRMS) publishes the latest 24-hour global VIIRS fire
//! detections at:
//!
//! ```text
//! GET https://firms.modaps.eosdis.nasa.gov/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv
//! ```
//!
//! No API key required for this public bulk file (separate
//! from the per-area JSON endpoint that needs a `MAP_KEY`).
//! Refreshed roughly every 3 hours.
//!
//! CSV header (column 0..n):
//! ```text
//! latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight
//! ```
//!
//! - `latitude`/`longitude`: WGS84 degrees.
//! - `acq_date`/`acq_time`: UTC date (`YYYY-MM-DD`) and time
//!   (`HHMM`) of the satellite pass.
//! - `confidence`: nominal/low/high — VIIRS confidence label.
//! - `frp`: Fire Radiative Power (MW).
//! - `daynight`: `D` or `N`.

use std::time::Duration;

use crate::error::StreamsError;

/// Default base URL — NASA FIRMS public data root.
pub const DEFAULT_BASE_URL: &str = "https://firms.modaps.eosdis.nasa.gov";

/// Default per-request timeout — 30 s. The CSV is 1-5 MiB
/// during peak fire season; 30 s tolerates a slow link
/// without blocking the seeder.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct NasaFirmsConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for NasaFirmsConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable NASA FIRMS client.
#[derive(Clone, Debug)]
pub struct NasaFirmsClient {
    http: reqwest::Client,
    config: NasaFirmsConfig,
}

impl NasaFirmsClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: NasaFirmsConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = NasaFirmsConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the global 24-hour VIIRS active-fire detection
    /// CSV and parse it into typed rows.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for CSV header / row mismatch.
    pub async fn fetch_global_24h(&self) -> Result<Vec<FireDetection>, StreamsError> {
        let url = format!(
            "{}/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            self.config.base_url
        );
        let resp = self
            .http
            .get(&url)
            .header("user-agent", &self.config.user_agent)
            .header("accept", "text/csv")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let body = resp
            .text()
            .await
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        parse_csv(&body)
    }
}

/// One fire detection row.
#[derive(Clone, Debug, PartialEq)]
pub struct FireDetection {
    /// WGS84 latitude.
    pub latitude: f64,
    /// WGS84 longitude.
    pub longitude: f64,
    /// VIIRS Brightness Temperature I-4 channel (Kelvin).
    pub bright_ti4: f64,
    /// `YYYY-MM-DD` UTC date the satellite acquired the
    /// observation.
    pub acq_date: String,
    /// `HHMM` UTC time of the acquisition.
    pub acq_time: String,
    /// Satellite identifier (`N` for Suomi-NPP, etc.).
    pub satellite: String,
    /// Confidence label (`l` low / `n` nominal / `h` high).
    pub confidence: String,
    /// VIIRS Brightness Temperature I-5 channel (Kelvin).
    pub bright_ti5: f64,
    /// Fire Radiative Power (megawatts).
    pub frp: f64,
    /// `D` (daytime) or `N` (nighttime).
    pub daynight: String,
}

fn parse_csv(body: &str) -> Result<Vec<FireDetection>, StreamsError> {
    let mut lines = body.lines();
    let header = lines
        .next()
        .ok_or_else(|| StreamsError::Parse("firms csv: empty".into()))?;
    let header_lower: Vec<String> = header.split(',').map(|s| s.trim().to_lowercase()).collect();

    let idx = |name: &str| {
        header_lower
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| StreamsError::Parse(format!("firms csv: missing column {name}")))
    };
    let i_lat = idx("latitude")?;
    let i_lon = idx("longitude")?;
    let i_bti4 = idx("bright_ti4")?;
    let i_date = idx("acq_date")?;
    let i_time = idx("acq_time")?;
    let i_sat = idx("satellite")?;
    let i_conf = idx("confidence")?;
    let i_bti5 = idx("bright_ti5")?;
    let i_frp = idx("frp")?;
    let i_dn = idx("daynight")?;

    let mut out = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < header_lower.len() {
            // Tolerate truncated rows during peak-load streams.
            continue;
        }
        let lat = cols[i_lat].trim().parse::<f64>().unwrap_or(f64::NAN);
        let lon = cols[i_lon].trim().parse::<f64>().unwrap_or(f64::NAN);
        if lat.is_nan() || lon.is_nan() {
            continue;
        }
        out.push(FireDetection {
            latitude: lat,
            longitude: lon,
            bright_ti4: cols[i_bti4].trim().parse::<f64>().unwrap_or(0.0),
            acq_date: cols[i_date].trim().to_string(),
            acq_time: cols[i_time].trim().to_string(),
            satellite: cols[i_sat].trim().to_string(),
            confidence: cols[i_conf].trim().to_string(),
            bright_ti5: cols[i_bti5].trim().parse::<f64>().unwrap_or(0.0),
            frp: cols[i_frp].trim().parse::<f64>().unwrap_or(0.0),
            daynight: cols[i_dn].trim().to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SAMPLE_CSV: &str = "\
latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight
40.123,-118.456,329.10,0.41,0.39,2026-05-04,0123,N,h,2.0NRT,295.40,12.50,N
-22.500,134.250,355.20,0.50,0.45,2026-05-04,0540,N,n,2.0NRT,310.10,45.30,D
";

    fn client_pointing_at(server: &MockServer) -> NasaFirmsClient {
        NasaFirmsClient::new(
            NasaFirmsConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_global_24h_parses_two_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE_CSV))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_global_24h().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!((rows[0].latitude - 40.123).abs() < 1e-9);
        assert!((rows[0].longitude + 118.456).abs() < 1e-9);
        assert_eq!(rows[0].acq_date, "2026-05-04");
        assert_eq!(rows[0].acq_time, "0123");
        assert_eq!(rows[0].satellite, "N");
        assert_eq!(rows[0].confidence, "h");
        assert!((rows[0].frp - 12.50).abs() < 1e-9);
        assert_eq!(rows[0].daynight, "N");
        assert_eq!(rows[1].confidence, "n");
        assert_eq!(rows[1].daynight, "D");
    }

    #[tokio::test]
    async fn fetch_global_24h_skips_unparseable_rows() {
        let server = MockServer::start().await;
        let csv = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight\nbad,bad,0,0,0,2026-05-04,0123,N,h,2.0,0,0,N\n40.0,-118.0,0,0,0,2026-05-04,0123,N,h,2.0,0,0,N\n";
        Mock::given(method("GET"))
            .and(path(
                "/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(csv))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_global_24h().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].latitude - 40.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_global_24h_missing_column_yields_parse() {
        let server = MockServer::start().await;
        // CSV missing `frp`.
        let csv = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,daynight\n40.0,-118.0,300,0,0,2026-05-04,0123,N,h,2,310,N\n";
        Mock::given(method("GET"))
            .and(path(
                "/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(csv))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_24h().await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_global_24h_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            ))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_24h().await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_global_24h_unreachable_yields_io() {
        let client = NasaFirmsClient::new(
            NasaFirmsConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client.fetch_global_24h().await.unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[tokio::test]
    async fn fetch_global_24h_empty_body_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_global_24h().await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = NasaFirmsConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
