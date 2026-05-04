//! seed_ais_snapshot — FAST-tier per-region vessel-density
//! snapshot from the in-process AIS state accumulator.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::maritime::MaritimeSeederError;

/// `(lamin, lomin, lamax, lomax)` bounding box.
pub type BBox = (f64, f64, f64, f64);

/// `(name, BBox)` named region.
pub type NamedBBox<'a> = (&'a str, BBox);

/// `(name_owned, BBox)` config row.
pub type NamedBBoxOwned = (String, BBox);

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "maritime:ais-snapshot:summary:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "ais-snapshot-state-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "maritime-ais";

/// Default region basket — covers the major ocean basins.
pub const DEFAULT_REGIONS: &[NamedBBox] = &[
    ("north-atlantic",  ( 25.0, -80.0,  60.0, -10.0)),
    ("south-atlantic",  (-50.0, -60.0,   0.0,  20.0)),
    ("north-pacific",   ( 10.0, 120.0,  60.0, -120.0)),
    ("south-pacific",   (-50.0, 130.0, -10.0, -70.0)),
    ("indian-ocean",    (-50.0,  20.0,  30.0, 110.0)),
    ("mediterranean",   ( 30.0,  -5.0,  46.0,  37.0)),
    ("caribbean",       (  9.0, -90.0,  27.0, -60.0)),
    ("baltic",          ( 53.0,  10.0,  66.0,  30.0)),
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct AisSnapshotConfig {
    /// `(name, (lamin, lomin, lamax, lomax))` pairs.
    pub regions: Vec<NamedBBoxOwned>,
}

impl Default for AisSnapshotConfig {
    fn default() -> Self {
        Self {
            regions: DEFAULT_REGIONS
                .iter()
                .map(|(n, b)| ((*n).to_string(), *b))
                .collect(),
        }
    }
}

/// One per-region row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionRow {
    /// Region name.
    pub name: String,
    /// Vessel count seen in the bbox over the accumulator's
    /// sliding window.
    pub vessel_count: usize,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AisSnapshotSummary {
    /// Per-region counts.
    pub regions: Vec<RegionRow>,
    /// Total vessel count (across all MMSIs in the
    /// accumulator).
    pub total_vessels: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// DI trait — wraps `pellucid_streams::MaritimeState`.
#[async_trait]
pub trait AisStateReader: Send + Sync + std::fmt::Debug {
    /// Total vessel count (across all MMSIs).
    fn vessel_count(&self) -> usize;
    /// Per-region vessel counts.
    fn vessels_by_region(&self, regions: &[NamedBBox]) -> Vec<(String, usize)>;
}

/// Run one cycle.
///
/// # Errors
/// See [`MaritimeSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    reader: &dyn AisStateReader,
    config: &AisSnapshotConfig,
) -> Result<PublishOutcome, MaritimeSeederError> {
    let region_refs: Vec<NamedBBox> = config
        .regions
        .iter()
        .map(|(n, b)| (n.as_str(), *b))
        .collect();
    let counts = reader.vessels_by_region(&region_refs);
    let regions: Vec<RegionRow> = counts
        .into_iter()
        .map(|(name, vessel_count)| RegionRow { name, vessel_count })
        .collect();
    let total_vessels = reader.vessel_count();
    if total_vessels == 0 {
        return Err(MaritimeSeederError::EmptyUpstream);
    }

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AisSnapshotSummary {
        regions,
        total_vessels,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(60_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.regions.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome =
        atomic_publish(pool, "maritime", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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
        assert_eq!(CACHE_KEY, "maritime:ais-snapshot:summary:v1");
    }

    #[test]
    fn default_regions_cover_eight_basins() {
        let cfg = AisSnapshotConfig::default();
        assert_eq!(cfg.regions.len(), 8);
        assert!(cfg.regions.iter().any(|(n, _)| n == "mediterranean"));
    }

    #[tokio::test]
    async fn run_cycle_writes_per_region_counts() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 5_000,
            counts: vec![
                ("north-atlantic".into(), 1_500),
                ("mediterranean".into(), 600),
            ],
        };
        let _ = run_cycle(&pool, &reader, &AisSnapshotConfig::default())
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
            parsed.pointer("/data/total_vessels").unwrap().as_u64(),
            Some(5000)
        );
        let regions = parsed.pointer("/data/regions").unwrap().as_array().unwrap();
        assert_eq!(regions.len(), 2);
        assert_eq!(
            regions[0].get("vessel_count").unwrap().as_u64(),
            Some(1500)
        );
    }

    #[tokio::test]
    async fn run_cycle_zero_vessels_errors() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 0,
            counts: vec![],
        };
        let err = run_cycle(&pool, &reader, &AisSnapshotConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MaritimeSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 1,
            counts: vec![("med".into(), 1)],
        };
        let _ = run_cycle(&pool, &reader, &AisSnapshotConfig::default())
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
