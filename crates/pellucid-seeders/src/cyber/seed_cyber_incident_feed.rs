//! seed_cyber_incident_feed — FAST-tier roll-up of CVEs
//! published in the last 24 hours from NIST NVD.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::cyber::CyberSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "cyber:incident-feed:24h:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "cyber-incident-feed-nvd-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "cyber-cve-feed";

/// Default lookback — 24 hours.
pub const DEFAULT_LOOKBACK_HOURS: u32 = 24;

/// Default per-page row cap.
pub const DEFAULT_RESULTS_PER_PAGE: u32 = 100;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CyberIncidentFeedConfig {
    /// Lookback window in hours.
    pub lookback_hours: u32,
    /// Results-per-page cap (NVD max 2000).
    pub results_per_page: u32,
    /// `now()` override for tests so they can pin the date
    /// window. Production passes `None` (uses current wall
    /// clock).
    pub now_unix_secs: Option<i64>,
}

impl Default for CyberIncidentFeedConfig {
    fn default() -> Self {
        Self {
            lookback_hours: DEFAULT_LOOKBACK_HOURS,
            results_per_page: DEFAULT_RESULTS_PER_PAGE,
            now_unix_secs: None,
        }
    }
}

/// One CVE row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CveRow {
    /// CVE identifier.
    pub cve_id: String,
    /// ISO-8601 first-published timestamp.
    pub published: String,
    /// English description.
    pub description: String,
    /// CVSS v3.1 base score.
    pub cvss_base_score: f64,
    /// CVSS v3.1 severity label.
    pub cvss_severity: String,
    /// Primary reference URL.
    pub primary_reference: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CyberIncidentFeedSnapshot {
    /// CVEs in the window.
    pub rows: Vec<CveRow>,
    /// Server-side total result count for the window.
    pub total_results: u64,
    /// Echo of the lookback window in hours.
    pub lookback_hours: u32,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled CVE — mirrors `pellucid_streams::NvdVulnerability`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCve {
    /// Id.
    pub cve_id: String,
    /// Published.
    pub published: String,
    /// Last modified.
    pub last_modified: String,
    /// Description.
    pub description: String,
    /// CVSS v3.1 base score.
    pub cvss_v31_base_score: f64,
    /// CVSS v3.1 severity.
    pub cvss_v31_severity: String,
    /// Primary reference URL.
    pub primary_reference: String,
}

/// Distilled response.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedNvdResponse {
    /// Total results.
    pub total_results: u64,
    /// Vulnerabilities.
    pub vulnerabilities: Vec<FetchedCve>,
}

/// DI trait — wraps `pellucid_streams::NvdClient::fetch_recent`.
#[async_trait]
pub trait NvdFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch CVEs in the last-modified window
    /// `[start_iso, end_iso]`.
    async fn fetch_recent(
        &self,
        last_mod_start: &str,
        last_mod_end: &str,
        results_per_page: u32,
    ) -> Result<FetchedNvdResponse, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`CyberSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NvdFetcher,
    config: &CyberIncidentFeedConfig,
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
    if resp.vulnerabilities.is_empty() {
        return Err(CyberSeederError::EmptyUpstream);
    }
    let rows: Vec<CveRow> = resp
        .vulnerabilities
        .into_iter()
        .map(|v| CveRow {
            cve_id: v.cve_id,
            published: v.published,
            description: v.description,
            cvss_base_score: v.cvss_v31_base_score,
            cvss_severity: v.cvss_v31_severity,
            primary_reference: v.primary_reference,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CyberIncidentFeedSnapshot {
        rows,
        total_results: resp.total_results,
        lookback_hours: config.lookback_hours,
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

/// Format `unix_secs` as `YYYY-MM-DDTHH:MM:SS.000Z` per the
/// NVD API contract. Pure arithmetic — no chrono dep.
pub fn format_iso8601_utc(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_today = unix_secs.rem_euclid(86_400);
    let h = (secs_today / 3600) as u32;
    let m = ((secs_today % 3600) / 60) as u32;
    let s = (secs_today % 60) as u32;
    let (y, mo, d) = epoch_days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.000Z")
}

fn epoch_days_to_ymd(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 {
        days / 146_097
    } else {
        (days - 146_096) / 146_097
    };
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year_i32 = (y + i64::from(m <= 2)) as i32;
    (year_i32, m as u32, d as u32)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        response: FetchedNvdResponse,
        last_window: std::sync::Mutex<(String, String)>,
    }

    #[async_trait]
    impl NvdFetcher for StaticFetcher {
        async fn fetch_recent(
            &self,
            last_mod_start: &str,
            last_mod_end: &str,
            _results_per_page: u32,
        ) -> Result<FetchedNvdResponse, Box<dyn std::error::Error + Send + Sync>> {
            *self.last_window.lock().unwrap() =
                (last_mod_start.to_string(), last_mod_end.to_string());
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
            description: format!("Description for {id}"),
            cvss_v31_base_score: score,
            cvss_v31_severity: sev.into(),
            primary_reference: format!("https://nvd.nist.gov/vuln/detail/{id}"),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "cyber:incident-feed:24h:v1");
    }

    #[test]
    fn format_iso8601_utc_known_unix_secs() {
        // 2026-05-04T07:00:00Z = unix 1777878000 (verified via
        // `datetime(2026,5,4,7,0,0,tzinfo=utc).timestamp()`).
        assert_eq!(
            format_iso8601_utc(1_777_878_000),
            "2026-05-04T07:00:00.000Z"
        );
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 42,
                vulnerabilities: vec![
                    cve("CVE-2024-001", 9.8, "CRITICAL"),
                    cve("CVE-2024-002", 5.5, "MEDIUM"),
                ],
            },
            last_window: std::sync::Mutex::new((String::new(), String::new())),
        };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &CyberIncidentFeedConfig {
                lookback_hours: 24,
                results_per_page: 100,
                now_unix_secs: Some(1_777_878_000),
            },
        )
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
        assert!((rows[0].get("cvss_base_score").unwrap().as_f64().unwrap() - 9.8).abs() < 1e-9);
        // Date window is the lookback derived from now_unix_secs.
        let (start, end) = fetcher.last_window.lock().unwrap().clone();
        assert_eq!(end, "2026-05-04T07:00:00.000Z");
        assert_eq!(start, "2026-05-03T07:00:00.000Z");
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 0,
                vulnerabilities: vec![],
            },
            last_window: std::sync::Mutex::new((String::new(), String::new())),
        };
        let err = run_cycle(&pool, &fetcher, &CyberIncidentFeedConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, CyberSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &CyberIncidentFeedConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, CyberSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            response: FetchedNvdResponse {
                total_results: 1,
                vulnerabilities: vec![cve("CVE-2024-001", 9.8, "CRITICAL")],
            },
            last_window: std::sync::Mutex::new((String::new(), String::new())),
        };
        let _ = run_cycle(&pool, &fetcher, &CyberIncidentFeedConfig::default())
            .await
            .unwrap();
        let meta: (String, String) = sqlx::query_as(
            "SELECT source_version, cascade_group FROM seed_meta WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(meta.0, SOURCE_VERSION);
        assert_eq!(meta.1, CASCADE_GROUP);
    }
}
