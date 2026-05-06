//! Vantage.sh EC2 instances pricing client.
//!
//! AWS's official Bulk Pricing API requires authentication
//! and returns 100MB+ JSON. Vantage.sh maintains a public
//! mirror with on-demand + spot pricing for every active EC2
//! instance type:
//!
//! ```text
//! GET https://instances.vantage.sh/api/instances.json
//! ```
//!
//! Free, no auth. Single-document JSON; ~5 MiB.
//!
//! Response shape (relevant subset):
//! ```json
//! [
//!   {
//!     "instance_type":   "m7i.large",
//!     "memory":          8.0,
//!     "vCPU":            2,
//!     "pricing": {
//!       "us-east-1": {
//!         "linux": {
//!           "ondemand":     "0.1008",
//!           "reserved":     { "yrTerm1Standard.allUpfront": "0.0635" },
//!           "spot_min":     "0.030",
//!           "spot_max":     "0.082",
//!           "spot_avg":     "0.045"
//!         }
//!       }
//!     }
//!   }
//! ]
//! ```

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::error::StreamsError;

/// Default base URL — Vantage.sh production.
pub const DEFAULT_BASE_URL: &str = "https://instances.vantage.sh";

/// Default per-request timeout — 30 s. The bulk JSON is large.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default user-agent.
pub const DEFAULT_USER_AGENT: &str = "pellucid-streams/0 (+https://pellucid.dev)";

/// Configuration.
#[derive(Clone, Debug)]
pub struct VantageComputeConfig {
    /// Base URL — no trailing slash.
    pub base_url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for VantageComputeConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }
}

/// Pluggable Vantage compute-pricing client.
#[derive(Clone, Debug)]
pub struct VantageComputeClient {
    http: reqwest::Client,
    config: VantageComputeConfig,
}

impl VantageComputeClient {
    /// Build a client.
    #[must_use]
    pub fn new(config: VantageComputeConfig, http: reqwest::Client) -> Self {
        Self { http, config }
    }

    /// Convenience constructor.
    ///
    /// # Errors
    /// [`StreamsError::Io`] if `reqwest` fails to build.
    pub fn production() -> Result<Self, StreamsError> {
        let cfg = VantageComputeConfig::default();
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(&cfg.user_agent)
            .build()
            .map_err(|e| StreamsError::Io(e.to_string()))?;
        Ok(Self::new(cfg, http))
    }

    /// Fetch on-demand + spot pricing for the supplied
    /// `(region, instance_types[])` pair. Returns one row per
    /// instance type the upstream had pricing for in the
    /// requested region's Linux platform.
    ///
    /// # Errors
    /// - [`StreamsError::Io`] for transport failures.
    /// - [`StreamsError::Status`] for non-2xx responses.
    /// - [`StreamsError::Parse`] for body shape mismatches.
    pub async fn fetch_pricing(
        &self,
        region: &str,
        instance_types: &[&str],
    ) -> Result<Vec<InstancePricing>, StreamsError> {
        let url = format!("{}/api/instances.json", self.config.base_url);
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
        let body: Vec<RawInstance> = resp
            .json()
            .await
            .map_err(|e| StreamsError::Parse(e.to_string()))?;
        let wanted: std::collections::HashSet<&str> = instance_types.iter().copied().collect();
        Ok(body
            .into_iter()
            .filter(|raw| wanted.contains(raw.instance_type.as_str()))
            .filter_map(|raw| InstancePricing::from_raw(raw, region))
            .collect())
    }
}

/// One pricing row.
#[derive(Clone, Debug, PartialEq)]
pub struct InstancePricing {
    /// Instance type (`m7i.large`, `g5.xlarge`, …).
    pub instance_type: String,
    /// AWS region.
    pub region: String,
    /// Memory in GiB.
    pub memory_gib: f64,
    /// vCPU count.
    pub vcpu: u32,
    /// On-demand $/hr (Linux).
    pub ondemand_usd_hr: f64,
    /// Spot $/hr — minimum across the region's AZs.
    pub spot_min_usd_hr: f64,
    /// Spot $/hr — maximum.
    pub spot_max_usd_hr: f64,
    /// Spot $/hr — region average.
    pub spot_avg_usd_hr: f64,
}

#[derive(Debug, Default, Deserialize)]
struct RawInstance {
    #[serde(default)]
    instance_type: String,
    #[serde(default)]
    memory: f64,
    #[serde(default, rename = "vCPU")]
    vcpu: u32,
    #[serde(default)]
    pricing: serde_json::Map<String, Value>,
}

impl InstancePricing {
    fn from_raw(raw: RawInstance, region: &str) -> Option<Self> {
        let region_pricing = raw.pricing.get(region)?;
        let linux = region_pricing.get("linux")?.as_object()?;
        let ondemand = parse_value_f64(linux.get("ondemand")?);
        let spot_min = linux.get("spot_min").map(parse_value_f64).unwrap_or(0.0);
        let spot_max = linux.get("spot_max").map(parse_value_f64).unwrap_or(0.0);
        let spot_avg = linux.get("spot_avg").map(parse_value_f64).unwrap_or(0.0);
        Some(Self {
            instance_type: raw.instance_type,
            region: region.to_string(),
            memory_gib: raw.memory,
            vcpu: raw.vcpu,
            ondemand_usd_hr: ondemand,
            spot_min_usd_hr: spot_min,
            spot_max_usd_hr: spot_max,
            spot_avg_usd_hr: spot_avg,
        })
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        serde_json::json!([
            {
                "instance_type": "m7i.large",
                "memory":        8.0,
                "vCPU":          2,
                "pricing": {
                    "us-east-1": {
                        "linux": {
                            "ondemand": "0.1008",
                            "spot_min": "0.030",
                            "spot_max": "0.082",
                            "spot_avg": "0.045"
                        }
                    }
                }
            },
            {
                "instance_type": "g5.xlarge",
                "memory":        16.0,
                "vCPU":          4,
                "pricing": {
                    "us-east-1": {
                        "linux": {
                            "ondemand": 1.006,
                            "spot_min": 0.30,
                            "spot_max": 0.92,
                            "spot_avg": 0.50
                        }
                    }
                }
            },
            {
                "instance_type": "p4d.24xlarge",
                "memory":        1152.0,
                "vCPU":          96,
                "pricing": {
                    "us-east-1": {
                        "linux": { "ondemand": "32.7726" }
                    }
                }
            }
        ])
    }

    fn client_pointing_at(server: &MockServer) -> VantageComputeClient {
        VantageComputeClient::new(
            VantageComputeConfig {
                base_url: server.uri(),
                timeout: Duration::from_secs(2),
                user_agent: "pellucid-test".into(),
            },
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn fetch_pricing_filters_to_basket() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/instances.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_pricing("us-east-1", &["m7i.large", "g5.xlarge"])
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        let m7 = rows
            .iter()
            .find(|r| r.instance_type == "m7i.large")
            .unwrap();
        assert!((m7.ondemand_usd_hr - 0.1008).abs() < 1e-6);
        assert!((m7.spot_avg_usd_hr - 0.045).abs() < 1e-9);
        let g5 = rows
            .iter()
            .find(|r| r.instance_type == "g5.xlarge")
            .unwrap();
        // Numeric values also round-trip.
        assert!((g5.ondemand_usd_hr - 1.006).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_pricing_drops_instances_without_spot() {
        // p4d.24xlarge has no spot fields — it still maps (with
        // zero spot values), provided ondemand exists.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/instances.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_pricing("us-east-1", &["p4d.24xlarge"])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].spot_avg_usd_hr - 0.0).abs() < 1e-9);
        assert!((rows[0].ondemand_usd_hr - 32.7726).abs() < 1e-6);
    }

    #[tokio::test]
    async fn fetch_pricing_skips_instances_without_region() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/instances.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let rows = client
            .fetch_pricing("eu-central-1", &["m7i.large"])
            .await
            .unwrap();
        // No row had eu-central-1 pricing in fixture.
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn fetch_pricing_5xx_yields_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/instances.json"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let client = client_pointing_at(&server);
        let err = client
            .fetch_pricing("us-east-1", &["m7i.large"])
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Status { status: 503 }));
    }

    #[tokio::test]
    async fn fetch_pricing_unreachable_yields_io() {
        let client = VantageComputeClient::new(
            VantageComputeConfig {
                base_url: "http://127.0.0.1:1".into(),
                timeout: Duration::from_secs(1),
                user_agent: "x".into(),
            },
            reqwest::Client::new(),
        );
        let err = client
            .fetch_pricing("us-east-1", &["m7i.large"])
            .await
            .unwrap_err();
        assert!(matches!(err, StreamsError::Io(_)));
    }
}
