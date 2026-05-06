//! CFTC Commitments of Traders (COT) weekly TXT report client.
//!
//! Endpoint shape (production — short-format Disaggregated COT
//! futures-only file, refreshed Fridays ~15:30 ET):
//! ```text
//! GET https://www.cftc.gov/dea/futures/deacmesf.txt
//! ```
//!
//! The file is a fixed-width plain-text dump grouped by report
//! contract. Each contract block is delimited by an
//! `------ <Contract Name> ------` style header, followed by
//! key:value lines like:
//!
//! ```text
//! GOLD - COMMODITY EXCHANGE INC.
//! Code-088691
//! ...
//! Open Interest is    503,482
//! :  Producer/Merchant/Processor/User                                     :          Swap Dealers              :
//! :        Long       :        Short       :  Spreading                   :   Long    :   Short    :  Spreading :
//! :       62,034      :       73,481       :        0                     :  152,883  :   31,272   :    9,221   :
//! ```
//!
//! Parsing the full fixed-width grid is brittle; CFTC also
//! publishes structured JSON via the Socrata Open Data API at
//! `https://publicreporting.cftc.gov/resource/jun7-fc8e.json`.
//! The Socrata path is far more reliable for headless ingest, so
//! [`CftcCotClient::fetch_disaggregated`] uses it. The text
//! report is kept as a documented fallback (see [`fetch_text`]).

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::StreamsError;

/// Default Socrata JSON endpoint — Disaggregated COT
/// futures-only, weekly cadence.
pub const DEFAULT_SOCRATA_BASE_URL: &str = "https://publicreporting.cftc.gov";

/// Default text-fallback endpoint.
pub const DEFAULT_TEXT_URL: &str = "https://www.cftc.gov/dea/futures/deacmesf.txt";

/// Default per-request timeout — 10 s. The Socrata endpoint is
/// usually fast (<1 s) but occasionally slow under load.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration for the CFTC COT client.
#[derive(Clone, Debug)]
pub struct CftcCotConfig {
    /// Socrata base URL — no trailing slash.
    pub socrata_base_url: String,
    /// Plain-text fallback URL.
    pub text_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
}

impl Default for CftcCotConfig {
    fn default() -> Self {
        Self {
            socrata_base_url: DEFAULT_SOCRATA_BASE_URL.to_string(),
            text_url: DEFAULT_TEXT_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Pluggable CFTC COT client.
#[derive(Clone, Debug)]
pub struct CftcCotClient {
    http: reqwest::Client,
    config: CftcCotConfig,
}

impl CftcCotClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: CftcCotConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// Returns [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = CftcCotConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch the latest Disaggregated futures-only COT rows for
    /// the supplied `contract_codes` (the `cftc_contract_market_code`
    /// column — e.g. `088691` for gold). Returns one
    /// [`CotRow`] per code that the upstream returned data for.
    ///
    /// The Socrata Open Data API is used for stability — the raw
    /// CFTC text dump is brittle to parse and changes layout
    /// occasionally.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_disaggregated(
        &self,
        contract_codes: &[&str],
    ) -> Result<Vec<CotRow>, StreamsError> {
        if contract_codes.is_empty() {
            return Ok(Vec::new());
        }
        let url = self.build_socrata_url(contract_codes)?;
        let resp = self
            .http
            .get(url)
            .header("accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        let rows: Vec<SocrataCotRow> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        // Socrata returns rows for every report week. We pick the
        // most recent row per contract by `report_date_as_yyyy_mm_dd`.
        let mut by_contract: std::collections::BTreeMap<String, SocrataCotRow> =
            std::collections::BTreeMap::new();
        for row in rows {
            let key = row.cftc_contract_market_code.clone();
            by_contract
                .entry(key)
                .and_modify(|existing| {
                    if row.report_date_as_yyyy_mm_dd > existing.report_date_as_yyyy_mm_dd {
                        *existing = row.clone();
                    }
                })
                .or_insert(row);
        }
        Ok(by_contract.into_values().map(CotRow::from).collect())
    }

    /// Fetch the raw fixed-width text dump. Exposed for callers
    /// that need a known-shape archive snapshot; production
    /// seeders should prefer [`Self::fetch_disaggregated`].
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    pub async fn fetch_text(&self) -> Result<String, StreamsError> {
        let resp = self.http.get(&self.config.text_url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StreamsError::Status {
                status: status.as_u16(),
            });
        }
        resp.text()
            .await
            .map_err(|e| StreamsError::Io(e.to_string()))
    }

    fn build_socrata_url(&self, codes: &[&str]) -> Result<Url, StreamsError> {
        // Socrata's SoQL `where` clause: `cftc_contract_market_code in ('088691', '067651')`.
        let in_clause = codes
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(",");
        let raw = format!("{}/resource/jun7-fc8e.json", self.config.socrata_base_url);
        let mut url =
            Url::parse(&raw).map_err(|e| StreamsError::Parse(format!("cftc socrata url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair(
                "$where",
                &format!("cftc_contract_market_code in ({in_clause})"),
            );
            // Limit per code is generous; one row per weekly report
            // and we only need the most recent. 16 weeks is plenty.
            q.append_pair("$limit", &(codes.len() * 16).to_string());
            q.append_pair("$order", "report_date_as_yyyy_mm_dd DESC");
        }
        Ok(url)
    }
}

/// Distilled COT row — only the long/short positions per trader
/// category Pellucid surfaces. The raw Socrata schema has 100+
/// columns; we keep the ones the panel renders.
#[derive(Clone, Debug, PartialEq)]
pub struct CotRow {
    /// CFTC contract market code (e.g. `"088691"`).
    pub contract_code: String,
    /// Human-readable contract name (e.g. `"GOLD"`).
    pub contract_name: String,
    /// `YYYY-MM-DD` date of the report week.
    pub report_date: String,
    /// Aggregate open interest across all trader categories.
    pub open_interest_all: i64,
    /// Producer/merchant/processor/user long positions.
    pub producer_long: i64,
    /// Producer/merchant/processor/user short positions.
    pub producer_short: i64,
    /// Swap dealer long positions.
    pub swap_long: i64,
    /// Swap dealer short positions.
    pub swap_short: i64,
    /// Managed-money long positions.
    pub managed_money_long: i64,
    /// Managed-money short positions.
    pub managed_money_short: i64,
}

impl CotRow {
    /// Net position for managed money (long − short). Positive
    /// indicates net-long (bullish), negative net-short.
    #[must_use]
    pub const fn managed_money_net(&self) -> i64 {
        self.managed_money_long - self.managed_money_short
    }
}

impl From<SocrataCotRow> for CotRow {
    fn from(raw: SocrataCotRow) -> Self {
        Self {
            contract_code: raw.cftc_contract_market_code,
            contract_name: raw.market_and_exchange_names,
            report_date: raw.report_date_as_yyyy_mm_dd,
            open_interest_all: parse_socrata_int(&raw.open_interest_all),
            producer_long: parse_socrata_int(&raw.prod_merc_positions_long_all),
            producer_short: parse_socrata_int(&raw.prod_merc_positions_short),
            swap_long: parse_socrata_int(&raw.swap_positions_long_all),
            swap_short: parse_socrata_int(&raw.swap__positions_short_all),
            managed_money_long: parse_socrata_int(&raw.m_money_positions_long_all),
            managed_money_short: parse_socrata_int(&raw.m_money_positions_short_all),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[allow(non_snake_case)]
struct SocrataCotRow {
    #[serde(default)]
    cftc_contract_market_code: String,
    #[serde(default)]
    market_and_exchange_names: String,
    #[serde(default)]
    report_date_as_yyyy_mm_dd: String,
    #[serde(default)]
    open_interest_all: String,
    #[serde(default)]
    prod_merc_positions_long_all: String,
    #[serde(default)]
    prod_merc_positions_short: String,
    #[serde(default)]
    swap_positions_long_all: String,
    // Yes — Socrata's column genuinely has a double underscore.
    #[serde(default)]
    swap__positions_short_all: String,
    #[serde(default)]
    m_money_positions_long_all: String,
    #[serde(default)]
    m_money_positions_short_all: String,
}

fn parse_socrata_int(s: &str) -> i64 {
    // Socrata returns numerics as strings — sometimes signed,
    // sometimes with thousands separators (legacy text export
    // mode). Strip commas + parse.
    s.replace(',', "").trim().parse::<i64>().unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn socrata_row(
        code: &str,
        name: &str,
        date: &str,
        oi: &str,
        mm_long: &str,
        mm_short: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "cftc_contract_market_code":      code,
            "market_and_exchange_names":      name,
            "report_date_as_yyyy_mm_dd":      date,
            "open_interest_all":              oi,
            "prod_merc_positions_long_all":   "62034",
            "prod_merc_positions_short":      "73481",
            "swap_positions_long_all":        "152883",
            "swap__positions_short_all":      "31272",
            "m_money_positions_long_all":     mm_long,
            "m_money_positions_short_all":    mm_short
        })
    }

    fn client_pointing_at(server: &MockServer) -> CftcCotClient {
        CftcCotClient::new(
            CftcCotConfig {
                socrata_base_url: server.uri(),
                text_url: format!("{}/dea/futures/deacmesf.txt", server.uri()),
                timeout: Duration::from_secs(2),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_disaggregated_returns_one_row_per_contract() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                socrata_row("088691", "GOLD", "2026-04-25", "503482", "120000", "30000"),
                socrata_row("067651", "SILVER", "2026-04-25", "120000", "40000", "10000"),
            ])))
            .mount(&server)
            .await;

        let client = client_pointing_at(&server);
        let rows = client
            .fetch_disaggregated(&["088691", "067651"])
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        // Index by contract code so the test isn't coupled to the
        // BTreeMap iteration order (silver code 067651 < gold 088691).
        let by_code: std::collections::HashMap<&str, &CotRow> =
            rows.iter().map(|r| (r.contract_code.as_str(), r)).collect();
        let gold = by_code.get("088691").expect("gold row");
        assert_eq!(gold.contract_name, "GOLD");
        assert_eq!(gold.report_date, "2026-04-25");
        assert_eq!(gold.open_interest_all, 503_482);
        assert_eq!(gold.managed_money_long, 120_000);
        assert_eq!(gold.managed_money_short, 30_000);
        assert_eq!(gold.managed_money_net(), 90_000);
        let silver = by_code.get("067651").expect("silver row");
        assert_eq!(silver.contract_name, "SILVER");
    }

    #[tokio::test]
    async fn fetch_disaggregated_picks_most_recent_per_contract() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                // Two weeks for the same gold contract — newer must win.
                socrata_row("088691", "GOLD", "2026-04-18", "490000", "100000", "30000"),
                socrata_row("088691", "GOLD", "2026-04-25", "503482", "120000", "30000"),
            ])))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_disaggregated(&["088691"]).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].report_date, "2026-04-25");
        assert_eq!(rows[0].managed_money_long, 120_000);
    }

    #[tokio::test]
    async fn fetch_disaggregated_empty_input_returns_empty() {
        let server = MockServer::start().await;
        // Server should not be called.
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        assert!(client.fetch_disaggregated(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_disaggregated_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_disaggregated(&["088691"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_disaggregated_unparseable_yields_parse() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_disaggregated(&["088691"]).await.unwrap_err();
        assert!(matches!(err, StreamsError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_disaggregated_handles_thousands_separators() {
        // Socrata occasionally emits formatted strings with
        // commas (legacy text-export rows). Parser must strip.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/resource/jun7-fc8e.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "cftc_contract_market_code":      "088691",
                    "market_and_exchange_names":      "GOLD",
                    "report_date_as_yyyy_mm_dd":      "2026-04-25",
                    "open_interest_all":              "503,482",
                    "prod_merc_positions_long_all":   "62,034",
                    "prod_merc_positions_short":      "73,481",
                    "swap_positions_long_all":        "152,883",
                    "swap__positions_short_all":      "31,272",
                    "m_money_positions_long_all":     "120,000",
                    "m_money_positions_short_all":    "30,000"
                }
            ])))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client.fetch_disaggregated(&["088691"]).await.unwrap();
        assert_eq!(rows[0].open_interest_all, 503_482);
        assert_eq!(rows[0].managed_money_long, 120_000);
    }

    #[tokio::test]
    async fn fetch_text_returns_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dea/futures/deacmesf.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("GOLD\nCode-088691\n"))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let body = client.fetch_text().await.unwrap();
        assert!(body.contains("GOLD"));
        assert!(body.contains("088691"));
    }

    #[tokio::test]
    async fn fetch_text_5xx_yields_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/dea/futures/deacmesf.txt"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client.fetch_text().await.unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 502 }));
    }

    #[test]
    fn parse_socrata_int_strips_commas_and_whitespace() {
        assert_eq!(parse_socrata_int("503,482"), 503_482);
        assert_eq!(parse_socrata_int("  120000 "), 120_000);
        assert_eq!(parse_socrata_int("-7,500"), -7_500);
        assert_eq!(parse_socrata_int(""), 0);
        assert_eq!(parse_socrata_int("abc"), 0);
    }

    #[test]
    fn config_default_uses_production_endpoints() {
        let cfg = CftcCotConfig::default();
        assert_eq!(cfg.socrata_base_url, DEFAULT_SOCRATA_BASE_URL);
        assert_eq!(cfg.text_url, DEFAULT_TEXT_URL);
    }

    #[test]
    fn cot_row_managed_money_net_signs_correctly() {
        let mut row = CotRow {
            contract_code: "X".into(),
            contract_name: "X".into(),
            report_date: "2026-04-25".into(),
            open_interest_all: 0,
            producer_long: 0,
            producer_short: 0,
            swap_long: 0,
            swap_short: 0,
            managed_money_long: 100,
            managed_money_short: 30,
        };
        assert_eq!(row.managed_money_net(), 70);
        row.managed_money_long = 30;
        row.managed_money_short = 100;
        assert_eq!(row.managed_money_net(), -70);
    }
}
