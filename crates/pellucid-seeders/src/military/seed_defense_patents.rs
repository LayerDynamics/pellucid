//! seed_defense_patents — SLOW-tier defense-patent trend snapshot.
//! Production adapters wire to USPTO PatentsView + WIPO bulk
//! filings filtered by defense-related CPC classes (F41/F42 small-
//! arms, B64G spacecraft, F02K/F02C jet engines, G01S radar, H04K
//! signal jamming). Tests inject deterministic rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::military::MilitarySeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "defense:patent-trends:v1";

/// 7 d TTL — patent grants publish weekly upstream.
pub const TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "defense-patents-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "defense";

/// One per-class patent-trend row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatentClassRow {
    /// CPC subclass id (e.g. `F41A`).
    pub cpc_class: String,
    /// Human label.
    pub label: String,
    /// Filings in the trailing 30 days.
    pub filings_30d: u32,
    /// YoY % change.
    pub yoy_pct: f64,
    /// Top filer name.
    pub top_filer: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatentTrendsSnapshot {
    /// Per-class rows sorted by descending filings.
    pub rows: Vec<PatentClassRow>,
    /// Trailing-30-day total filings across all classes.
    pub total_filings_30d: u32,
    /// Reference period (e.g. `2026-W18`).
    pub period: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched row.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedPatentClass {
    /// CPC subclass.
    pub cpc_class: String,
    /// Human label.
    pub label: String,
    /// Filings (30d).
    pub filings_30d: u32,
    /// YoY %.
    pub yoy_pct: f64,
    /// Top filer.
    pub top_filer: String,
}

/// DI trait.
#[async_trait]
pub trait DefensePatentsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest CPC-class trend rows + period stamp.
    async fn fetch_patents(
        &self,
    ) -> Result<(Vec<FetchedPatentClass>, String), Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn DefensePatentsFetcher,
) -> Result<PublishOutcome, MilitarySeederError> {
    let (fetched, period) = fetcher
        .fetch_patents()
        .await
        .map_err(|e| MilitarySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(MilitarySeederError::EmptyUpstream);
    }
    let total: u32 = fetched.iter().map(|r| r.filings_30d).sum();
    let mut rows: Vec<PatentClassRow> = fetched
        .into_iter()
        .map(|r| PatentClassRow {
            cpc_class: r.cpc_class,
            label: r.label,
            filings_30d: r.filings_30d,
            yoy_pct: r.yoy_pct,
            top_filer: r.top_filer,
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.filings_30d));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = PatentTrendsSnapshot {
        rows,
        total_filings_30d: total,
        period,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(604_800_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "defense", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedPatentClass>,
        period: String,
    }

    #[async_trait]
    impl DefensePatentsFetcher for StaticFetcher {
        async fn fetch_patents(
            &self,
        ) -> Result<(Vec<FetchedPatentClass>, String), Box<dyn std::error::Error + Send + Sync>>
        {
            Ok((self.rows.clone(), self.period.clone()))
        }
    }

    fn pat(cpc: &str, n: u32, yoy: f64) -> FetchedPatentClass {
        FetchedPatentClass {
            cpc_class: cpc.into(),
            label: format!("{cpc} class"),
            filings_30d: n,
            yoy_pct: yoy,
            top_filer: "Lockheed Martin".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "defense:patent-trends:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_descending() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                pat("F41A", 80, 12.0),
                pat("B64G", 220, 41.0),
                pat("G01S", 140, -3.0),
            ],
            period: "2026-W18".into(),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let cpcs: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("cpc_class").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(cpcs, vec!["B64G", "G01S", "F41A"]);
        assert_eq!(
            parsed.pointer("/data/total_filings_30d").unwrap().as_u64(),
            Some(440)
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            period: "2026-W18".into(),
        };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, MilitarySeederError::EmptyUpstream));
    }
}
