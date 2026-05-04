//! seed_telegram_intel_min — FAST-tier roll-up of recent
//! messages from a small set of public Telegram channels.
//!
//! "Min" qualifier: SPEC-001 §17.7 calls out that full
//! Telegram MTProto integration lands in M3 week 14. Until
//! then, this seeder scrapes the public web preview at
//! `t.me/s/<channel>` for a curated channel basket.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::intel::IntelSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "telegram:recent-feed:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "telegram-intel-min-public-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "intel-telegram";

/// Default channel basket — credible news / official
/// channels with public previews.
pub const DEFAULT_CHANNELS: &[&str] = &[
    "bbcbreaking",
    "rianru",
    "tassagency_en",
    "voxgamma",
    "iaeaorg",
];

/// Default per-channel cap on retained messages.
pub const DEFAULT_PER_CHANNEL_LIMIT: usize = 5;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct TelegramIntelMinConfig {
    /// Channel basket (no `@`).
    pub channels: Vec<String>,
    /// Per-channel cap on retained messages.
    pub per_channel_limit: usize,
}

impl Default for TelegramIntelMinConfig {
    fn default() -> Self {
        Self {
            channels: DEFAULT_CHANNELS.iter().map(|s| (*s).to_string()).collect(),
            per_channel_limit: DEFAULT_PER_CHANNEL_LIMIT,
        }
    }
}

/// One message row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelegramMessageRow {
    /// Channel slug.
    pub channel: String,
    /// `<channel>/<id>` data-post identifier.
    pub data_post: String,
    /// Permalink.
    pub url: String,
    /// ISO-8601 timestamp.
    pub datetime: String,
    /// Plain-text message body.
    pub text: String,
    /// View counter as the upstream rendered it.
    pub views: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelegramIntelMinSnapshot {
    /// Per-channel message rows interleaved (newest-first per
    /// channel, channels in basket order).
    pub rows: Vec<TelegramMessageRow>,
    /// Echo of the channel basket.
    pub channels: Vec<String>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled message — mirrors `pellucid_streams::TelegramMessage`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedMessage {
    /// `<channel>/<id>`.
    pub data_post: String,
    /// Permalink.
    pub url: String,
    /// ISO-8601 timestamp.
    pub datetime: String,
    /// Plain-text body.
    pub text: String,
    /// View counter.
    pub views: String,
}

/// DI trait — wraps `pellucid_streams::TelegramPublicClient::fetch_channel`.
#[async_trait]
pub trait TelegramFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch messages for one public channel.
    async fn fetch_channel(
        &self,
        channel: &str,
    ) -> Result<Vec<FetchedMessage>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`IntelSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn TelegramFetcher,
    config: &TelegramIntelMinConfig,
) -> Result<PublishOutcome, IntelSeederError> {
    let mut rows: Vec<TelegramMessageRow> = Vec::new();
    for channel in &config.channels {
        let result = fetcher
            .fetch_channel(channel)
            .await
            .map_err(|e| IntelSeederError::Upstream(e.to_string()))?;
        // Take the most-recent N (the public preview returns
        // oldest-first), reverse so newest-first survives the
        // truncate.
        let mut chan_rows: Vec<TelegramMessageRow> = result
            .into_iter()
            .rev()
            .take(config.per_channel_limit)
            .map(|m| TelegramMessageRow {
                channel: channel.clone(),
                data_post: m.data_post,
                url: m.url,
                datetime: m.datetime,
                text: m.text,
                views: m.views,
            })
            .collect();
        rows.append(&mut chan_rows);
    }
    if rows.is_empty() {
        return Err(IntelSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = TelegramIntelMinSnapshot {
        rows,
        channels: config.channels.clone(),
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
        atomic_publish(pool, "telegram", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        responses: std::collections::HashMap<String, Vec<FetchedMessage>>,
    }

    #[async_trait]
    impl TelegramFetcher for StaticFetcher {
        async fn fetch_channel(
            &self,
            channel: &str,
        ) -> Result<Vec<FetchedMessage>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.responses.get(channel).cloned().unwrap_or_default())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl TelegramFetcher for FailingFetcher {
        async fn fetch_channel(
            &self,
            _channel: &str,
        ) -> Result<Vec<FetchedMessage>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn msg(channel: &str, id: u32) -> FetchedMessage {
        FetchedMessage {
            data_post: format!("{channel}/{id}"),
            url: format!("https://t.me/{channel}/{id}"),
            datetime: format!("2026-05-04T{:02}:00:00+00:00", id % 24),
            text: format!("Message {id} from {channel}"),
            views: "1.0K".into(),
        }
    }

    fn config_for(channels: &[&str]) -> TelegramIntelMinConfig {
        TelegramIntelMinConfig {
            channels: channels.iter().map(|s| (*s).to_string()).collect(),
            per_channel_limit: 3,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "telegram:recent-feed:v1");
    }

    #[tokio::test]
    async fn run_cycle_takes_recent_n_per_channel_in_basket_order() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        // 5 messages each (oldest-first as upstream returns)
        responses.insert(
            "a".into(),
            (1..=5).map(|i| msg("a", i)).collect(),
        );
        responses.insert(
            "b".into(),
            (1..=5).map(|i| msg("b", i)).collect(),
        );
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(&pool, &fetcher, &config_for(&["a", "b"]))
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        // 3 per channel × 2 channels = 6 total.
        assert_eq!(rows.len(), 6);
        // First three are channel a (newest first → ids 5, 4, 3).
        assert_eq!(rows[0].get("data_post").unwrap().as_str().unwrap(), "a/5");
        assert_eq!(rows[1].get("data_post").unwrap().as_str().unwrap(), "a/4");
        assert_eq!(rows[2].get("data_post").unwrap().as_str().unwrap(), "a/3");
        // Then channel b.
        assert_eq!(rows[3].get("data_post").unwrap().as_str().unwrap(), "b/5");
    }

    #[tokio::test]
    async fn run_cycle_drops_channel_with_no_messages() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("a".into(), vec![msg("a", 1)]);
        // b has no entry → empty.
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_for(&["a", "b"]))
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
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn run_cycle_no_messages_anywhere_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let err = run_cycle(&pool, &fetcher, &config_for(&["a"]))
            .await
            .unwrap_err();
        assert!(matches!(err, IntelSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &config_for(&["a"]))
            .await
            .unwrap_err();
        assert!(matches!(err, IntelSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("a".into(), vec![msg("a", 1)]);
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_for(&["a"]))
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
