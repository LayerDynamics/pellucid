//! Snapshot envelope shape for the Telegram intel feed.
//!
//! Identical to the JSON shape the (now-deleted) `seed_telegram_intel_min`
//! seeder used to write so the existing handler at
//! `pellucid_handlers::telegram::v1::feed` keeps reading the same
//! cache key with the same envelope contract. Drift here is what would
//! break the `TelegramIntelPanel` panel (T4.1.5).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Cache key — pinned to the FAST-tier slot the handler reads.
pub const CACHE_KEY: &str = "telegram:recent-feed:v1";

/// FAST-tier TTL — matches SPEC-001 §17.7's 60 s expiry.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp. Distinct from the deleted scraper's
/// `telegram-intel-min-public-v1` so dashboards / tests can tell
/// which writer published the row.
pub const SOURCE_VERSION: &str = "telegram-mtproto-v1";

/// Cascade group tag — same value the legacy seeder used, so the
/// `/health` cascade and downstream dashboards keep working without
/// reconfiguration.
pub const CASCADE_GROUP: &str = "intel-telegram";

/// Default per-channel cap on retained messages.
pub const DEFAULT_PER_CHANNEL_LIMIT: usize = 5;

/// One message row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// View counter as the upstream rendered it (e.g. `"1.2K"`).
    pub views: String,
}

/// Published snapshot — top-level value under `data` in the seed
/// envelope. Same shape the legacy scraper-backed seeder published.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramIntelMinSnapshot {
    /// Per-channel message rows interleaved (newest-first per channel,
    /// channels in basket order).
    pub rows: Vec<TelegramMessageRow>,
    /// Echo of the channel basket the run task queried this cycle.
    pub channels: Vec<String>,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrips_through_serde_json_value() {
        let snap = TelegramIntelMinSnapshot {
            rows: vec![TelegramMessageRow {
                channel: "rt_intl_news".into(),
                data_post: "rt_intl_news/1".into(),
                url: "https://t.me/rt_intl_news/1".into(),
                datetime: "2026-05-04T12:00:00Z".into(),
                text: "hello".into(),
                views: "1.0K".into(),
            }],
            channels: vec!["rt_intl_news".into()],
            assembled_at_ms: 1_746_360_000_000,
        };
        let value = serde_json::to_value(&snap).unwrap();
        let back: TelegramIntelMinSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn ttl_matches_fast_tier_60s() {
        assert_eq!(TTL.as_secs(), 60);
    }
}
