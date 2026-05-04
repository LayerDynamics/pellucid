//! seed_cloud_status — FAST-tier roll-up of GCP incident
//! status. AWS / Azure can be added by extending the
//! [`CloudStatusFetcher`] trait; the panel renders a unified
//! "datacenter / cloud status" tile.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:cloud-status:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "cloud-status-gcp-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-cloud";

/// Default cap on retained closed incidents (open ones are
/// always retained).
pub const DEFAULT_CLOSED_LIMIT: usize = 20;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct CloudStatusConfig {
    /// How many of the most-recent closed incidents to keep
    /// alongside the open ones.
    pub closed_limit: usize,
}

impl Default for CloudStatusConfig {
    fn default() -> Self {
        Self {
            closed_limit: DEFAULT_CLOSED_LIMIT,
        }
    }
}

/// One incident row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncidentRow {
    /// Cloud provider (`gcp | aws | azure`).
    pub provider: String,
    /// Incident id.
    pub id: String,
    /// Description.
    pub description: String,
    /// Begin timestamp.
    pub begin: String,
    /// End timestamp (empty when ongoing).
    pub end: String,
    /// Severity tag.
    pub severity: String,
    /// Affected service.
    pub service_name: String,
    /// Permalink.
    pub uri: String,
    /// `true` when the incident is open.
    pub ongoing: bool,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudStatusSnapshot {
    /// All open incidents + the most-recent N closed incidents.
    pub rows: Vec<IncidentRow>,
    /// Count of currently-open incidents.
    pub open_incidents: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled incident — mirrors `pellucid_streams::GcpIncident`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedIncident {
    /// Id.
    pub id: String,
    /// Description.
    pub description: String,
    /// Begin.
    pub begin: String,
    /// End (empty when ongoing).
    pub end: String,
    /// Severity.
    pub severity: String,
    /// Service.
    pub service_name: String,
    /// Permalink.
    pub uri: String,
    /// Ongoing flag.
    pub ongoing: bool,
}

/// DI trait — wraps `pellucid_streams::GcpStatusClient::fetch_incidents`.
#[async_trait]
pub trait CloudStatusFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch incidents (one provider at a time).
    async fn fetch_incidents(
        &self,
    ) -> Result<Vec<FetchedIncident>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn CloudStatusFetcher,
    config: &CloudStatusConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let fetched = fetcher
        .fetch_incidents()
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }

    let mut open: Vec<IncidentRow> = Vec::new();
    let mut closed: Vec<IncidentRow> = Vec::new();
    for inc in fetched {
        let row = IncidentRow {
            provider: "gcp".into(),
            id: inc.id,
            description: inc.description,
            begin: inc.begin,
            end: inc.end,
            severity: inc.severity,
            service_name: inc.service_name,
            uri: inc.uri,
            ongoing: inc.ongoing,
        };
        if row.ongoing {
            open.push(row);
        } else {
            closed.push(row);
        }
    }
    closed.sort_by(|a, b| b.begin.cmp(&a.begin));
    closed.truncate(config.closed_limit);
    let open_count = open.len();
    let mut rows = open;
    rows.append(&mut closed);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = CloudStatusSnapshot {
        rows,
        open_incidents: open_count,
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
    let outcome =
        atomic_publish(pool, "technology", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedIncident>,
    }

    #[async_trait]
    impl CloudStatusFetcher for StaticFetcher {
        async fn fetch_incidents(
            &self,
        ) -> Result<Vec<FetchedIncident>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl CloudStatusFetcher for FailingFetcher {
        async fn fetch_incidents(
            &self,
        ) -> Result<Vec<FetchedIncident>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn inc(id: &str, begin: &str, end: Option<&str>) -> FetchedIncident {
        FetchedIncident {
            id: id.into(),
            description: format!("Incident {id}"),
            begin: begin.into(),
            end: end.unwrap_or("").into(),
            severity: "high".into(),
            service_name: "Compute Engine".into(),
            uri: format!("https://status.cloud.google.com/incidents/{id}"),
            ongoing: end.is_none(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:cloud-status:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_groups_open_then_recent_closed() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                inc("a", "2026-04-25T08:00:00Z", Some("2026-04-25T18:00:00Z")),
                inc("b", "2026-04-26T08:00:00Z", None), // open
                inc("c", "2026-04-24T08:00:00Z", Some("2026-04-24T20:00:00Z")),
                inc("d", "2026-04-26T10:00:00Z", None), // open
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &CloudStatusConfig::default())
            .await
            .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        assert_eq!(
            parsed.pointer("/data/open_incidents").unwrap().as_u64(),
            Some(2)
        );
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        // Open first (b, d in input order), then closed newest-first (a, c).
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].get("ongoing").unwrap().as_bool(), Some(true));
        assert_eq!(rows[1].get("ongoing").unwrap().as_bool(), Some(true));
        assert_eq!(rows[2].get("id").unwrap().as_str().unwrap(), "a");
        assert_eq!(rows[3].get("id").unwrap().as_str().unwrap(), "c");
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &CloudStatusConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &CloudStatusConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::Upstream(_)));
    }
}
