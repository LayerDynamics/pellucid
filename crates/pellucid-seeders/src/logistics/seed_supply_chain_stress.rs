//! seed_supply_chain_stress — FAST-tier composite "stress
//! index" derived from GDELT supply-chain theme coverage.
//!
//! Paid datasets (FBX freight indices, Drewry WCI, Project44)
//! hold the canonical numeric stress signals. The free signal
//! Pellucid can publish is the volume of news coverage on
//! supply-chain disruption themes — when shipping problems
//! become newsworthy, GDELT theme tagging spikes.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::conflict::seed_gdelt_intel::{FetchedGdeltArticle, GdeltFetcher};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::logistics::LogisticsSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "supply-chain:stress-index:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "supply-chain-stress-gdelt-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "supply-chain";

/// Default GDELT query — supply-chain disruption themes.
pub const DEFAULT_QUERY: &str =
    "(theme:SUPPLY_CHAIN OR theme:SHIPPING_DELAY OR theme:LOGISTICS OR (theme:SHIPPING AND (theme:DELAY OR theme:DISRUPT OR theme:STRIKE)))";

/// Default lookback.
pub const DEFAULT_TIMESPAN: &str = "24h";

/// Default record cap.
pub const DEFAULT_MAX_RECORDS: u32 = 100;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct SupplyChainStressConfig {
    /// GDELT query.
    pub query: String,
    /// Lookback window.
    pub timespan: String,
    /// Max records.
    pub max_records: u32,
}

impl Default for SupplyChainStressConfig {
    fn default() -> Self {
        Self {
            query: DEFAULT_QUERY.to_string(),
            timespan: DEFAULT_TIMESPAN.to_string(),
            max_records: DEFAULT_MAX_RECORDS,
        }
    }
}

/// One article row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StressArticleRow {
    /// URL.
    pub url: String,
    /// Title.
    pub title: String,
    /// Seen-date timestamp.
    pub seen_date: String,
    /// Source domain.
    pub domain: String,
    /// Source country.
    pub source_country: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupplyChainStressSnapshot {
    /// Per-domain article counts (descending). The headline
    /// figure: total articles in the window. Top sources
    /// indicate where the "stress" coverage is originating.
    pub rows: Vec<StressArticleRow>,
    /// Total articles (= the headline stress proxy).
    pub article_count: usize,
    /// Documented proxy note.
    pub source_note: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`LogisticsSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GdeltFetcher,
    config: &SupplyChainStressConfig,
) -> Result<PublishOutcome, LogisticsSeederError> {
    let fetched = fetcher
        .search_articles(&config.query, &config.timespan, config.max_records)
        .await
        .map_err(|e| LogisticsSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(LogisticsSeederError::EmptyUpstream);
    }
    let article_count = fetched.len();
    let rows: Vec<StressArticleRow> = fetched.into_iter().map(map_row).collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = SupplyChainStressSnapshot {
        rows,
        article_count,
        source_note: "Article-volume proxy via GDELT supply-chain theme \
            tagging. Quantitative stress indices (FBX / Drewry WCI / \
            Project44) require paid subscriptions; no free public REST \
            API publishes their figures."
            .into(),
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
        atomic_publish(pool, "supply-chain", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn map_row(a: FetchedGdeltArticle) -> StressArticleRow {
    StressArticleRow {
        url: a.url,
        title: a.title,
        seen_date: a.seen_date,
        domain: a.domain,
        source_country: a.source_country,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedGdeltArticle>,
    }

    #[async_trait]
    impl GdeltFetcher for StaticFetcher {
        async fn search_articles(
            &self,
            _q: &str,
            _t: &str,
            _m: u32,
        ) -> Result<Vec<FetchedGdeltArticle>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.rows.clone())
        }
    }

    fn article(url: &str) -> FetchedGdeltArticle {
        FetchedGdeltArticle {
            url: url.into(),
            title: format!("Disruption — {url}"),
            seen_date: "20260504T120000Z".into(),
            social_image: String::new(),
            domain: "freightwaves.com".into(),
            language: "English".into(),
            source_country: "US".into(),
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "supply-chain:stress-index:current:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_with_count_and_note() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![article("a"), article("b"), article("c")],
        };
        let _ = run_cycle(&pool, &fetcher, &SupplyChainStressConfig::default())
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
            parsed.pointer("/data/article_count").unwrap().as_u64(),
            Some(3)
        );
        let note = parsed
            .pointer("/data/source_note")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(note.contains("paid subscription"));
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &SupplyChainStressConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, LogisticsSeederError::EmptyUpstream));
    }
}
