//! seed_chokepoint_status — FAST-tier vessel-density snapshot
//! for the world's strategic shipping chokepoints.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::maritime::seed_ais_snapshot::{AisStateReader, BBox, NamedBBox, NamedBBoxOwned};
use crate::maritime::MaritimeSeederError;

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "maritime:chokepoint-status:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "chokepoint-status-state-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "maritime-chokepoint";

/// Default chokepoints — Suez, Hormuz, Malacca, Bosporus,
/// Panama, Bab el-Mandeb, Dover, Gibraltar.
pub const DEFAULT_CHOKEPOINTS: &[NamedBBox] = &[
    ("suez", (27.5, 32.0, 32.5, 34.0)),
    ("hormuz", (25.0, 55.5, 27.5, 57.5)),
    ("malacca", (1.0, 98.0, 6.0, 105.0)),
    ("bosporus", (40.5, 28.5, 41.5, 29.5)),
    ("panama", (8.5, -80.5, 9.5, -79.0)),
    ("bab-el-mandeb", (12.0, 43.0, 14.0, 44.0)),
    ("dover", (50.5, 1.0, 51.5, 1.5)),
    ("gibraltar", (35.5, -5.5, 36.5, -5.0)),
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct ChokepointStatusConfig {
    /// `(name, (lamin, lomin, lamax, lomax))` pairs.
    pub chokepoints: Vec<NamedBBoxOwned>,
}

impl Default for ChokepointStatusConfig {
    fn default() -> Self {
        Self {
            chokepoints: DEFAULT_CHOKEPOINTS
                .iter()
                .map(|(n, b)| ((*n).to_string(), *b))
                .collect(),
        }
    }
}

/// One per-chokepoint row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChokepointRow {
    /// Chokepoint name.
    pub name: String,
    /// Bounding box.
    pub bbox: BBox,
    /// Vessel count in the bbox.
    pub vessel_count: usize,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChokepointStatusSnapshot {
    /// Per-chokepoint rows in input order.
    pub rows: Vec<ChokepointRow>,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Run one cycle.
///
/// # Errors
/// See [`MaritimeSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    reader: &dyn AisStateReader,
    config: &ChokepointStatusConfig,
) -> Result<PublishOutcome, MaritimeSeederError> {
    let bboxes: Vec<NamedBBox> = config
        .chokepoints
        .iter()
        .map(|(n, b)| (n.as_str(), *b))
        .collect();
    let counts = reader.vessels_by_region(&bboxes);
    let rows: Vec<ChokepointRow> = config
        .chokepoints
        .iter()
        .zip(counts)
        .map(|((name, bbox), (_returned_name, count))| ChokepointRow {
            name: name.clone(),
            bbox: *bbox,
            vessel_count: count,
        })
        .collect();
    if reader.vessel_count() == 0 {
        return Err(MaritimeSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = ChokepointStatusSnapshot {
        rows,
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
    let outcome = atomic_publish(pool, "maritime", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticReader {
        total: usize,
        counts: Vec<(String, usize)>,
    }

    #[async_trait]
    impl AisStateReader for StaticReader {
        fn vessel_count(&self) -> usize {
            self.total
        }
        fn vessels_by_region(&self, _regions: &[NamedBBox]) -> Vec<(String, usize)> {
            self.counts.clone()
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "maritime:chokepoint-status:current:v1");
    }

    #[test]
    fn default_chokepoints_include_eight_strategic_lanes() {
        let cfg = ChokepointStatusConfig::default();
        assert_eq!(cfg.chokepoints.len(), 8);
        for name in ["suez", "hormuz", "malacca", "bosporus", "panama"] {
            assert!(cfg.chokepoints.iter().any(|(n, _)| n == name));
        }
    }

    #[tokio::test]
    async fn run_cycle_writes_per_chokepoint_counts() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 1000,
            counts: vec![
                ("suez".into(), 120),
                ("hormuz".into(), 80),
                ("malacca".into(), 200),
                ("bosporus".into(), 50),
                ("panama".into(), 75),
                ("bab-el-mandeb".into(), 90),
                ("dover".into(), 110),
                ("gibraltar".into(), 60),
            ],
        };
        let _ = run_cycle(&pool, &reader, &ChokepointStatusConfig::default())
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0].get("name").unwrap().as_str().unwrap(), "suez");
        assert_eq!(rows[2].get("vessel_count").unwrap().as_u64(), Some(200));
    }

    #[tokio::test]
    async fn run_cycle_zero_total_errors() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 0,
            counts: vec![],
        };
        let err = run_cycle(&pool, &reader, &ChokepointStatusConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MaritimeSeederError::EmptyUpstream));
    }
}
