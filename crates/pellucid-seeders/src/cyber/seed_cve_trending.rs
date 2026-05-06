//! seed_cve_trending — FAST-tier roll-up of the highest-CVSS
//! CVEs from the last 7 days (the "trending" list the cyber
//! panel renders alongside the last-24h incident feed).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::cyber::seed_cyber_incident_feed::{format_iso8601_utc, FetchedCve, NvdFetcher};
use crate::cyber::CyberSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "cyber:cve-trending:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "cve-trending-nvd-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "cyber-cve-trending";

/// Default lookback window — 7 days.
pub const DEFAULT_LOOKBACK_HOURS: u32 = 7 * 24;

/// Default top-N cap.
pub const DEFAULT_TOP_N: usize = 25;

/// Default minimum CVSS to qualify as "trending" (HIGH+).
pub const DEFAULT_MIN_CVSS: f64 = 7.0;

/// Default per-page row cap (NVD max 2000).
pub const DEFAULT_RESULTS_PER_PAGE: u32 = 500;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CveTrendingConfig {
    /// Lookback window in hours.
    pub lookback_hours: u32,
    /// Top-N cap on the published snapshot.
    pub top_n: usize,
    /// Minimum CVSS v3.1 base score to keep.
    pub min_cvss: f64,
    /// Results-per-page cap.
    pub results_per_page: u32,
    /// `now()` override for tests.
    pub now_unix_secs: Option<i64>,
}

impl Default for CveTrendingConfig {
    fn default() -> Self {
        Self {
            lookback_hours: DEFAULT_LOOKBACK_HOURS,
            top_n: DEFAULT_TOP_N,
            min_cvss: DEFAULT_MIN_CVSS,
            results_per_page: DEFAULT_RESULTS_PER_PAGE,
            now_unix_secs: None,
        }
    }
}

/// One trending CVE row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendingCveRow {
    /// CVE id.
    pub cve_id: String,
    /// First-published timestamp.
    pub published: String,
    /// Description.
    pub description: String,
    /// CVSS v3.1 base score.
    pub cvss_base_score: f64,
    /// CVSS v3.1 severity.
    pub cvss_severity: String,
    /// Primary reference URL.
    pub primary_reference: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CveTrendingSnapshot {
    /// Top-N CVEs by CVSS desc.
    pub rows: Vec<TrendingCveRow>,
    /// Echo of the lookback window in hours.
    pub lookback_hours: u32,
    /// Minimum CVSS used.
    pub min_cvss: f64,
    /// Server-side total result count for the window.
    pub total_results: u64,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`CyberSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NvdFetcher,
    config: &CveTrendingConfig,
) -> Result<PublishOutcome, CyberSeederError> {
    let now = config
        .now_unix_secs
        .unwrap_or_else(|| pellucid_core::now_ms() / 1000);
    let start = now - i64::from(config.lookback_hours) * 3600;
    let start_iso = format_iso8601_utc(start);
    let end_iso = format_iso8601_utc(now);
    let resp = fetcher
        .fetch_recent(&start_iso, &end_iso, config.results_per_page)
        .await
        .map_err(|e| CyberSeederError::Upstream(e.to_string()))?;
    let total_results = resp.total_results;
    let mut rows: Vec<TrendingCveRow> = resp
        .vulnerabilities
        .into_iter()
        .filter(|v| v.cvss_v31_base_score >= config.min_cvss)
        .map(map_row)
        .collect();
    if rows.is_empty() {
        return Err(CyberSeederError::EmptyUpstream);
    }
    rows.sort_by(|a, b| {
        b.cvss_base_score
            .partial_cmp(&a.cvss_base_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(config.top_n);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CveTrendingSnapshot {
        rows,
        lookback_hours: config.lookback_hours,
        min_cvss: config.min_cvss,
        total_results,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "cyber", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(v: FetchedCve) -> TrendingCveRow {
    TrendingCveRow {
        cve_id: v.cve_id,
        published: v.published,
        description: v.description,
        cvss_base_score: v.cvss_v31_base_score,
        cvss_severity: v.cvss_v31_severity,
        primary_reference: v.primary_reference,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    use crate::cyber::seed_cyber_incident_feed::FetchedNvdResponse;

    #[derive(Debug)]
    struct StaticFetcher {
        response: FetchedNvdResponse,
    }

    #[async_trait]
    impl NvdFetcher for StaticFetcher {
        async fn fetch_recent(
            &self,
            _last_mod_start: &str,
            _last_mod_end: &str,
            _results_per_page: u32,
        ) -> Result<FetchedNvdResponse, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.response.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl NvdFetcher for FailingFetcher {
        async fn fetch_recent(
            &self,
            _last_mod_start: &str,
            _last_mod_end: &str,
            _results_per_page: u32,
        ) -> Result<FetchedNvdResponse, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn cve(id: &str, score: f64, sev: &str) -> FetchedCve {
        FetchedCve {
            cve_id: id.into(),
            published: "2026-04-25T08:00:00.000".into(),
            last_modified: "2026-04-25T20:00:00.000".into(),
            description: format!("Description {id}"),
            cvss_v31_base_score: score,
            cvss_v31_severity: sev.into(),
            primary_reference: format!("https://nvd.nist.gov/vuln/detail/{id}"),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "cyber:cve-trending:v1");
    }

    #[tokio::test]
    async fn run_cycle_filters_by_min_cvss_and_sorts_desc() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 4,
                vulnerabilities: vec![
                    cve("CVE-2024-001", 5.5, "MEDIUM"), // dropped
                    cve("CVE-2024-002", 9.8, "CRITICAL"),
                    cve("CVE-2024-003", 7.2, "HIGH"),
                    cve("CVE-2024-004", 3.1, "LOW"), // dropped
                ],
            },
        };
        let outcome = run_cycle(&pool, &fetcher, &CveTrendingConfig::default())
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].get("cve_id").unwrap().as_str().unwrap(),
            "CVE-2024-002"
        );
        assert_eq!(
            rows[1].get("cve_id").unwrap().as_str().unwrap(),
            "CVE-2024-003"
        );
    }

    #[tokio::test]
    async fn run_cycle_truncates_to_top_n() {
        let pool = open_in_memory().await.unwrap();
        let mut vulns: Vec<FetchedCve> = (0..30)
            .map(|i| cve(&format!("CVE-{i:04}"), 7.0 + (i as f64) * 0.1, "HIGH"))
            .collect();
        vulns.reverse();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 30,
                vulnerabilities: vulns,
            },
        };
        let cfg = CveTrendingConfig {
            top_n: 5,
            ..CveTrendingConfig::default()
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 5);
        // Top entry has the highest CVSS — CVE-0029 (7.0 + 29*0.1 = 9.9).
        assert_eq!(rows[0].get("cve_id").unwrap().as_str().unwrap(), "CVE-0029");
    }

    #[tokio::test]
    async fn run_cycle_no_vulns_above_threshold_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 2,
                vulnerabilities: vec![cve("low-1", 1.0, "LOW"), cve("low-2", 2.0, "LOW")],
            },
        };
        let err = run_cycle(&pool, &fetcher, &CveTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, CyberSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &CveTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, CyberSeederError::Upstream(_)));
    }
}
