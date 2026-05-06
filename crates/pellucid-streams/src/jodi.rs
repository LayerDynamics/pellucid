//! JODI (Joint Organisations Data Initiative) world oil/gas
//! data CSV client.
//!
//! JODI publishes monthly oil + gas demand/supply CSVs at:
//!
//! ```text
//! GET https://www.jodidata.org/_resources/files/downloads/world-data/world_oil.csv
//! GET https://www.jodidata.org/_resources/files/downloads/world-data/world_gas.csv
//! ```
//!
//! Free, no auth. The CSVs are wide (many columns) and large
//! (~5-15 MiB) — ~50 columns of monthly per-country data.
//!
//! CSV header (relevant subset):
//! ```text
//! REF_AREA,ENERGY_PRODUCT,FLOW_BREAKDOWN,UNIT_MEASURE,TIME_PERIOD,OBS_VALUE,ASSESSMENT_CODE
//! ```

use std::time::Duration;

use crate::error::StreamsError;

/// Default base URL — JODI public CDN.
pub const DEFAULT_BASE_URL: &str = "https://www.jodidata.org";

/// Default per-request timeout — 60 s. The CSV is large.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// JODI dataset selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JodiDataset {
    /// `world_oil.csv`.
    Oil,
    /// `world_gas.csv`.
    Gas,
}

impl JodiDataset {
    /// Path slug for the dataset.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Oil => "world_oil.csv",
            Self::Gas => "world_gas.csv",
        }
    }
}

/// Configuration.
#[derive(Clone, Debug)]
pub struct JodiConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for JodiConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable JODI client.
#[derive(Clone, Debug)]
pub struct JodiClient {
    http: reqwest::Client,
    config: JodiConfig,
}

impl JodiClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: JodiConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = JodiConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch + parse the world dataset CSV. Optionally filters
    /// to `country` (REF_AREA, ISO 3-letter code) and
    /// `flow_breakdown` (e.g. `"STOCKCH"` for stock change,
    /// `"INDPROD"` for indigenous production).
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for CSV header / row mismatch.
    pub async fn fetch_world(
        &self,
        dataset: JodiDataset,
        country: Option<&str>,
        flow_breakdown: Option<&str>,
    ) -> Result<Vec<JodiRow>, StreamsError> {
        let url = format!(
            "{}/_resources/files/downloads/world-data/{}",
            self.config.base_url,
            dataset.slug()
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
        parse_csv(&body, country, flow_breakdown)
    }
}

/// One JODI row.
#[derive(Clone, Debug, PartialEq)]
pub struct JodiRow {
    /// REF_AREA — ISO 3-letter country code (`USA`, `SAU`, …).
    pub country: String,
    /// ENERGY_PRODUCT — `CRUDEOIL`, `NATGAS`, etc.
    pub energy_product: String,
    /// FLOW_BREAKDOWN — `INDPROD`, `STOCKCH`, `TOTDEMO`, …
    pub flow_breakdown: String,
    /// UNIT_MEASURE — `KBD` (kilobarrels/day) for oil,
    /// `MCM` (million cubic metres) for gas.
    pub unit_measure: String,
    /// TIME_PERIOD — `YYYY-MM`.
    pub time_period: String,
    /// OBS_VALUE.
    pub obs_value: f64,
    /// ASSESSMENT_CODE — confidence indicator (1-5).
    pub assessment_code: String,
}

fn parse_csv(
    body: &str,
    country_filter: Option<&str>,
    flow_filter: Option<&str>,
) -> Result<Vec<JodiRow>, StreamsError> {
    let mut lines = body.lines();
    let header = lines
        .next()
        .ok_or_else(|| StreamsError::Parse("jodi csv: empty".into()))?;
    let header_lower: Vec<String> = header.split(',').map(|s| s.trim().to_lowercase()).collect();
    let idx = |name: &str| {
        header_lower
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| StreamsError::Parse(format!("jodi csv: missing column {name}")))
    };
    let i_area = idx("ref_area")?;
    let i_product = idx("energy_product")?;
    let i_flow = idx("flow_breakdown")?;
    let i_unit = idx("unit_measure")?;
    let i_period = idx("time_period")?;
    let i_value = idx("obs_value")?;
    let i_assess = idx("assessment_code")?;

    let mut out = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < header_lower.len() {
            continue;
        }
        let country = cols[i_area].trim();
        if let Some(c) = country_filter {
            if !country.eq_ignore_ascii_case(c) {
                continue;
            }
        }
        let flow = cols[i_flow].trim();
        if let Some(f) = flow_filter {
            if !flow.eq_ignore_ascii_case(f) {
                continue;
            }
        }
        let value = cols[i_value].trim().parse::<f64>().unwrap_or(0.0);
        out.push(JodiRow {
            country: country.to_string(),
            energy_product: cols[i_product].trim().to_string(),
            flow_breakdown: flow.to_string(),
            unit_measure: cols[i_unit].trim().to_string(),
            time_period: cols[i_period].trim().to_string(),
            obs_value: value,
            assessment_code: cols[i_assess].trim().to_string(),
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

    const SAMPLE: &str = "\
REF_AREA,ENERGY_PRODUCT,FLOW_BREAKDOWN,UNIT_MEASURE,TIME_PERIOD,OBS_VALUE,ASSESSMENT_CODE
USA,CRUDEOIL,INDPROD,KBD,2026-04,12500.5,1
USA,CRUDEOIL,STOCKCH,KBD,2026-04,-100.2,2
SAU,CRUDEOIL,INDPROD,KBD,2026-04,9000.0,1
RUS,CRUDEOIL,INDPROD,KBD,2026-04,9500.0,1
";

    fn client_pointing_at(server: &MockServer) -> JodiClient {
        JodiClient::new(
            JodiConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_world_oil_returns_all_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_world(JodiDataset::Oil, None, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].country, "USA");
        assert!((rows[0].obs_value - 12_500.5).abs() < 1e-6);
        assert_eq!(rows[0].unit_measure, "KBD");
    }

    #[tokio::test]
    async fn fetch_world_oil_filters_by_country() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_world(JodiDataset::Oil, Some("USA"), None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.country == "USA"));
    }

    #[tokio::test]
    async fn fetch_world_oil_filters_by_flow_breakdown() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_world(JodiDataset::Oil, None, Some("INDPROD"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.flow_breakdown == "INDPROD"));
    }

    #[tokio::test]
    async fn fetch_world_oil_combined_filters() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_world(JodiDataset::Oil, Some("USA"), Some("STOCKCH"))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].obs_value - -100.2).abs() < 1e-6);
    }

    #[tokio::test]
    async fn fetch_world_gas_uses_correct_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_gas.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SAMPLE))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_world(JodiDataset::Gas, None, None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 4);
    }

    #[tokio::test]
    async fn fetch_world_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_world(JodiDataset::Oil, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_world_missing_column_yields_parse() {
        let server = MockServer::start().await;
        let csv = "REF_AREA,ENERGY_PRODUCT\nUSA,CRUDEOIL\n";
        Mock::given(method("GET"))
            .and(path("/_resources/files/downloads/world-data/world_oil.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(csv))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_world(JodiDataset::Oil, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_world_unreachable_yields_io() {
        let client = JodiClient::new(
            JodiConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_world(JodiDataset::Oil, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }

    #[test]
    fn dataset_slug_round_trips() {
        assert_eq!(JodiDataset::Oil.slug(), "world_oil.csv");
        assert_eq!(JodiDataset::Gas.slug(), "world_gas.csv");
    }

    #[test]
    fn config_default_uses_production_endpoint() {
        let cfg = JodiConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.timeout, DEFAULT_TIMEOUT);
    }
}
