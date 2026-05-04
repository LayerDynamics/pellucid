//! seed_huggingface_trending — FAST-tier roll-up of trending
//! HuggingFace models (covers the user's "AI sources" ask).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::technology::TechnologySeederError;

/// Cache key — NEW FAST tier slot added by T3.8 expansion.
pub const CACHE_KEY: &str = "technology:ai-models-trending:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "huggingface-trending-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "technology-ai";

/// Sort dimension forwarded to the upstream client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HfSort {
    /// Sort by all-time downloads desc.
    Downloads,
    /// Sort by likes desc.
    Likes,
    /// Sort by `lastModified` desc (the closest signal to
    /// "trending" without a paid HF API tier).
    Trending,
}

impl HfSort {
    /// Slug forwarded to the streams client.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Downloads => "downloads",
            Self::Likes => "likes",
            Self::Trending => "lastModified",
        }
    }
}

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct HuggingFaceTrendingConfig {
    /// Sort order.
    pub sort_by: HfSort,
    /// Result cap per cycle.
    pub limit: u32,
}

impl Default for HuggingFaceTrendingConfig {
    fn default() -> Self {
        Self {
            sort_by: HfSort::Trending,
            limit: 30,
        }
    }
}

/// One model row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRow {
    /// `<owner>/<name>` model id.
    pub id: String,
    /// All-time downloads.
    pub downloads: i64,
    /// Like count.
    pub likes: i64,
    /// ISO-8601 last-modified timestamp.
    pub last_modified: String,
    /// Pipeline tag (`text-generation`, `text-to-image`, …).
    pub pipeline_tag: String,
    /// Library (`transformers`, `diffusers`, …).
    pub library_name: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HuggingFaceTrendingSnapshot {
    /// Models in upstream order.
    pub rows: Vec<ModelRow>,
    /// Echo of the sort slug.
    pub sort_by: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled model — mirrors `pellucid_streams::HfModel`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedModel {
    /// Id.
    pub id: String,
    /// Downloads.
    pub downloads: i64,
    /// Likes.
    pub likes: i64,
    /// Last modified.
    pub last_modified: String,
    /// Pipeline tag.
    pub pipeline_tag: String,
    /// Library name.
    pub library_name: String,
}

/// DI trait — wraps `pellucid_streams::HuggingFaceClient::fetch_models`.
#[async_trait]
pub trait HuggingFaceFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch models sorted by `sort_slug` desc.
    async fn fetch_models(
        &self,
        sort_slug: &str,
        limit: u32,
    ) -> Result<Vec<FetchedModel>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`TechnologySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn HuggingFaceFetcher,
    config: &HuggingFaceTrendingConfig,
) -> Result<PublishOutcome, TechnologySeederError> {
    let fetched = fetcher
        .fetch_models(config.sort_by.slug(), config.limit)
        .await
        .map_err(|e| TechnologySeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(TechnologySeederError::EmptyUpstream);
    }
    let rows: Vec<ModelRow> = fetched
        .into_iter()
        .map(|m| ModelRow {
            id: m.id,
            downloads: m.downloads,
            likes: m.likes,
            last_modified: m.last_modified,
            pipeline_tag: m.pipeline_tag,
            library_name: m.library_name,
        })
        .collect();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = HuggingFaceTrendingSnapshot {
        rows,
        sort_by: config.sort_by.slug().to_string(),
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
        rows: Vec<FetchedModel>,
    }

    #[async_trait]
    impl HuggingFaceFetcher for StaticFetcher {
        async fn fetch_models(
            &self,
            _sort_slug: &str,
            _limit: u32,
        ) -> Result<Vec<FetchedModel>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl HuggingFaceFetcher for FailingFetcher {
        async fn fetch_models(
            &self,
            _sort_slug: &str,
            _limit: u32,
        ) -> Result<Vec<FetchedModel>, Box<dyn std::error::Error + Send + Sync>> {
            Err("upstream down".into())
        }
    }

    fn model(id: &str, downloads: i64) -> FetchedModel {
        FetchedModel {
            id: id.into(),
            downloads,
            likes: 100,
            last_modified: "2026-04-25T08:00:00.000Z".into(),
            pipeline_tag: "text-generation".into(),
            library_name: "transformers".into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "technology:ai-models-trending:v1");
    }

    #[test]
    fn sort_slug_round_trips() {
        assert_eq!(HfSort::Downloads.slug(), "downloads");
        assert_eq!(HfSort::Likes.slug(), "likes");
        assert_eq!(HfSort::Trending.slug(), "lastModified");
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                model("meta-llama/Llama-3.1-8B-Instruct", 12_345_678),
                model("stabilityai/stable-diffusion-3.5", 8_000_000),
            ],
        };
        let _ = run_cycle(&pool, &fetcher, &HuggingFaceTrendingConfig::default())
            .await
            .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            parsed.pointer("/data/sort_by").unwrap().as_str().unwrap(),
            "lastModified"
        );
    }

    #[tokio::test]
    async fn run_cycle_empty_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher, &HuggingFaceTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &HuggingFaceTrendingConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, TechnologySeederError::Upstream(_)));
    }
}
