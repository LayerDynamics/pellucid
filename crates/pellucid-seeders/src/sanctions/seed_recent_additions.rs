//! seed_recent_additions — FAST-tier sanctions snapshot.
//! Production adapters wire to OFAC SDN list + EU consolidated
//! sanctions + UK HMT bulk delta endpoints.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::sanctions::SanctionsSeederError;

/// Cache key — FAST tier.
pub const CACHE_KEY: &str = "sanctions:recent-additions:24h:v1";

/// 30 m TTL.
pub const TTL: Duration = Duration::from_secs(30 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "sanctions-recent-additions-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "sanctions";

/// One sanctions row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SanctionRow {
    /// Authority — `OFAC`, `EU`, `UK_HMT`, `UN`.
    pub authority: String,
    /// Listed entity name.
    pub entity: String,
    /// Entity-type tag — `individual`, `vessel`, `company`,
    /// `aircraft`.
    pub entity_type: String,
    /// Country jurisdiction code.
    pub jurisdiction: String,
    /// ISO-8601 listed-on stamp.
    pub listed_on: String,
    /// Programme tag — `RUSSIA-EO14024`, `IRAN-CISADA`, etc.
    pub programme: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SanctionsSnapshot {
    /// Rows sorted descending by listed_on.
    pub rows: Vec<SanctionRow>,
    /// Total row count.
    pub total: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedSanctionRow {
    /// Authority.
    pub authority: String,
    /// Entity.
    pub entity: String,
    /// Entity type.
    pub entity_type: String,
    /// Jurisdiction.
    pub jurisdiction: String,
    /// Listed-on.
    pub listed_on: String,
    /// Programme.
    pub programme: String,
}

/// DI trait.
#[async_trait]
pub trait SanctionsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch recent sanction additions across authorities.
    async fn fetch_additions(
        &self,
    ) -> Result<Vec<FetchedSanctionRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn SanctionsFetcher,
) -> Result<PublishOutcome, SanctionsSeederError> {
    let fetched = fetcher
        .fetch_additions()
        .await
        .map_err(|e| SanctionsSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(SanctionsSeederError::EmptyUpstream);
    }
    let mut rows: Vec<SanctionRow> = fetched
        .into_iter()
        .map(|r| SanctionRow {
            authority: r.authority,
            entity: r.entity,
            entity_type: r.entity_type,
            jurisdiction: r.jurisdiction,
            listed_on: r.listed_on,
            programme: r.programme,
        })
        .collect();
    // Most recent first.
    rows.sort_by(|a, b| b.listed_on.cmp(&a.listed_on));
    let total = rows.len();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = SanctionsSnapshot {
        rows,
        total,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(1_800_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "sanctions", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedSanctionRow>,
    }

    #[async_trait]
    impl SanctionsFetcher for StaticFetcher {
        async fn fetch_additions(
            &self,
        ) -> Result<Vec<FetchedSanctionRow>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn s(authority: &str, listed_on: &str) -> FetchedSanctionRow {
        FetchedSanctionRow {
            authority: authority.into(),
            entity: format!("Entity by {authority}"),
            entity_type: "company".into(),
            jurisdiction: "RU".into(),
            listed_on: listed_on.into(),
            programme: "RUSSIA-EO14024".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "sanctions:recent-additions:24h:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_descending_listed_on() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                s("OFAC", "2026-04-28"),
                s("EU", "2026-04-30"),
                s("UK_HMT", "2026-04-29"),
            ],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let auths: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("authority").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(auths, vec!["EU", "UK_HMT", "OFAC"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, SanctionsSeederError::EmptyUpstream));
    }
}
