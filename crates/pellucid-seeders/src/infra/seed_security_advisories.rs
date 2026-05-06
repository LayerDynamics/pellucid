//! seed_security_advisories — FAST-tier snapshot of CISA's
//! Known Exploited Vulnerabilities catalog (recent additions).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::infra::InfraSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "cyber:active-campaigns:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "security-advisories-cisa-kev-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "cyber-kev";

/// Default cap on how many recent KEV entries to publish.
pub const DEFAULT_RECENT_LIMIT: usize = 50;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct SecurityAdvisoriesConfig {
    /// Number of recent advisories to keep.
    pub recent_limit: usize,
}

impl Default for SecurityAdvisoriesConfig {
    fn default() -> Self {
        Self {
            recent_limit: DEFAULT_RECENT_LIMIT,
        }
    }
}

/// One KEV row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryRow {
    /// CVE identifier.
    pub cve_id: String,
    /// Vendor / project.
    pub vendor_project: String,
    /// Product.
    pub product: String,
    /// Vulnerability name.
    pub vulnerability_name: String,
    /// `YYYY-MM-DD` date CISA added.
    pub date_added: String,
    /// Short description.
    pub short_description: String,
    /// Required action.
    pub required_action: String,
    /// `YYYY-MM-DD` due date.
    pub due_date: String,
    /// Ransomware-campaign-use flag.
    pub known_ransomware_campaign_use: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SecurityAdvisoriesSnapshot {
    /// Most-recent KEV entries (capped at `recent_limit`).
    pub rows: Vec<AdvisoryRow>,
    /// Catalog version (`YYYY.MM.DD`).
    pub catalog_version: String,
    /// Total catalog size (before truncation).
    pub total_catalog_size: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled vulnerability — mirrors `pellucid_streams::KevVulnerability`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedAdvisory {
    /// CVE id.
    pub cve_id: String,
    /// Vendor.
    pub vendor_project: String,
    /// Product.
    pub product: String,
    /// Name.
    pub vulnerability_name: String,
    /// Date added.
    pub date_added: String,
    /// Description.
    pub short_description: String,
    /// Required action.
    pub required_action: String,
    /// Due date.
    pub due_date: String,
    /// Ransomware flag.
    pub known_ransomware_campaign_use: String,
}

/// Distilled catalog — mirrors `pellucid_streams::KevCatalog`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCatalog {
    /// Catalog version.
    pub catalog_version: String,
    /// Vulnerabilities.
    pub vulnerabilities: Vec<FetchedAdvisory>,
}

/// DI trait — wraps `pellucid_streams::CisaKevClient::fetch_catalog`.
#[async_trait]
pub trait SecurityAdvisoriesFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the full KEV catalog.
    async fn fetch_catalog(
        &self,
    ) -> Result<FetchedCatalog, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`InfraSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn SecurityAdvisoriesFetcher,
    config: &SecurityAdvisoriesConfig,
) -> Result<PublishOutcome, InfraSeederError> {
    let catalog = fetcher
        .fetch_catalog()
        .await
        .map_err(|e| InfraSeederError::Upstream(e.to_string()))?;
    if catalog.vulnerabilities.is_empty() {
        return Err(InfraSeederError::EmptyUpstream);
    }
    let total_catalog_size = catalog.vulnerabilities.len();
    let mut rows: Vec<AdvisoryRow> = catalog
        .vulnerabilities
        .into_iter()
        .map(|v| AdvisoryRow {
            cve_id: v.cve_id,
            vendor_project: v.vendor_project,
            product: v.product,
            vulnerability_name: v.vulnerability_name,
            date_added: v.date_added,
            short_description: v.short_description,
            required_action: v.required_action,
            due_date: v.due_date,
            known_ransomware_campaign_use: v.known_ransomware_campaign_use,
        })
        .collect();
    rows.sort_by(|a, b| b.date_added.cmp(&a.date_added));
    rows.truncate(config.recent_limit);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = SecurityAdvisoriesSnapshot {
        rows,
        catalog_version: catalog.catalog_version,
        total_catalog_size,
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        catalog: FetchedCatalog,
    }

    #[async_trait]
    impl SecurityAdvisoriesFetcher for StaticFetcher {
        async fn fetch_catalog(
            &self,
        ) -> Result<FetchedCatalog, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.catalog.clone())
        }
    }

    fn vuln(cve: &str, date: &str) -> FetchedAdvisory {
        FetchedAdvisory {
            cve_id: cve.into(),
            vendor_project: "Acme".into(),
            product: "Router".into(),
            vulnerability_name: format!("{cve} name"),
            date_added: date.into(),
            short_description: "desc".into(),
            required_action: "patch".into(),
            due_date: "2026-05-11".into(),
            known_ransomware_campaign_use: "Known".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "cyber:active-campaigns:v1");
    }

    #[tokio::test]
    async fn run_cycle_keeps_most_recent_advisories() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            catalog: FetchedCatalog {
                catalog_version: "2026.04.25".into(),
                vulnerabilities: vec![
                    vuln("CVE-2024-001", "2026-04-20"),
                    vuln("CVE-2024-002", "2026-04-24"),
                    vuln("CVE-2024-003", "2026-04-22"),
                ],
            },
        };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &SecurityAdvisoriesConfig { recent_limit: 2 },
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
        assert_eq!(
            rows[0].get("cve_id").unwrap().as_str().unwrap(),
            "CVE-2024-002"
        );
        assert_eq!(
            rows[1].get("cve_id").unwrap().as_str().unwrap(),
            "CVE-2024-003"
        );
        assert_eq!(
            parsed.pointer("/data/total_catalog_size").unwrap().as_u64(),
            Some(3)
        );
        assert_eq!(
            parsed
                .pointer("/data/catalog_version")
                .unwrap()
                .as_str()
                .unwrap(),
            "2026.04.25"
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            catalog: FetchedCatalog {
                catalog_version: "x".into(),
                vulnerabilities: vec![],
            },
        };
        let err = run_cycle(&pool, &fetcher, &SecurityAdvisoriesConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, InfraSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            catalog: FetchedCatalog {
                catalog_version: "2026.04.25".into(),
                vulnerabilities: vec![vuln("CVE-2024-001", "2026-04-20")],
            },
        };
        let _ = run_cycle(&pool, &fetcher, &SecurityAdvisoriesConfig::default())
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
