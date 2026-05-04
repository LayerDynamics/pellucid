//! seed_port_congestion — FAST-tier vessel-density snapshot
//! for the world's busiest container ports, derived from the
//! AIS state accumulator's per-bbox vessel counts.
//!
//! Paid sources (Project44, Sea-Intelligence, MarineTraffic
//! Pro) hold canonical port-call dwell times. The free signal
//! Pellucid can publish is the count of AIS-reporting vessels
//! within each port's bounding box at snapshot time — a
//! reasonable proxy for queue length / throughput pressure.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::logistics::LogisticsSeederError;
use crate::maritime::seed_ais_snapshot::{AisStateReader, BBox, NamedBBox, NamedBBoxOwned};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "supply-chain:port-congestion:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "port-congestion-ais-state-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "supply-chain-ports";

/// Default port basket — top container hubs by TEU throughput
/// (Shanghai, Singapore, Ningbo-Zhoushan, Shenzhen, Guangzhou,
/// Qingdao, Busan, Tianjin, Rotterdam, Antwerp, LA/Long Beach,
/// New York/New Jersey, Hamburg).
pub const DEFAULT_PORTS: &[NamedBBox] = &[
    ("shanghai",       (30.5,  121.5,  31.5,  122.5)),
    ("singapore",      ( 1.1,  103.5,   1.5,  104.0)),
    ("ningbo-zhoushan",(29.7,  121.3,  30.3,  122.5)),
    ("shenzhen",       (22.4,  113.7,  22.7,  114.4)),
    ("guangzhou",      (23.0,  113.5,  23.3,  113.8)),
    ("qingdao",        (35.9,  119.9,  36.3,  120.6)),
    ("busan",          (35.0,  128.8,  35.2,  129.2)),
    ("tianjin",        (38.9,  117.5,  39.2,  118.0)),
    ("rotterdam",      (51.8,    3.9,  52.0,    4.5)),
    ("antwerp",        (51.2,    4.2,  51.4,    4.5)),
    ("la-long-beach",  (33.6, -118.4,  33.9, -118.0)),
    ("ny-nj",          (40.5,  -74.4,  40.8,  -73.8)),
    ("hamburg",        (53.5,    9.7,  53.7,   10.2)),
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct PortCongestionConfig {
    /// `(name, bbox)` pairs.
    pub ports: Vec<NamedBBoxOwned>,
}

impl Default for PortCongestionConfig {
    fn default() -> Self {
        Self {
            ports: DEFAULT_PORTS
                .iter()
                .map(|(n, b)| ((*n).to_string(), *b))
                .collect(),
        }
    }
}

/// One port row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortRow {
    /// Port slug.
    pub name: String,
    /// Bounding box.
    pub bbox: BBox,
    /// Vessel count in the bbox.
    pub vessel_count: usize,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortCongestionSnapshot {
    /// Per-port rows in input-basket order.
    pub rows: Vec<PortRow>,
    /// Sum of vessel counts across the basket. The headline
    /// "global container congestion" proxy.
    pub total_vessels_at_ports: usize,
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
    reader: &dyn AisStateReader,
    config: &PortCongestionConfig,
) -> Result<PublishOutcome, LogisticsSeederError> {
    if reader.vessel_count() == 0 {
        return Err(LogisticsSeederError::EmptyUpstream);
    }
    let bboxes: Vec<NamedBBox> = config
        .ports
        .iter()
        .map(|(n, b)| (n.as_str(), *b))
        .collect();
    let counts = reader.vessels_by_region(&bboxes);
    let rows: Vec<PortRow> = config
        .ports
        .iter()
        .zip(counts.into_iter())
        .map(|((name, bbox), (_returned_name, vessel_count))| PortRow {
            name: name.clone(),
            bbox: *bbox,
            vessel_count,
        })
        .collect();
    let total_vessels_at_ports: usize = rows.iter().map(|r| r.vessel_count).sum();

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = PortCongestionSnapshot {
        rows,
        total_vessels_at_ports,
        source_note: "AIS-state per-port-bbox vessel count proxy. \
            Canonical port-call dwell times (Project44 / Sea-Intelligence \
            / MarineTraffic Pro) are paywalled."
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
        fn vessels_by_region(&self, _r: &[NamedBBox]) -> Vec<(String, usize)> {
            self.counts.clone()
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "supply-chain:port-congestion:current:v1");
    }

    #[test]
    fn default_ports_include_shanghai_and_la() {
        let cfg = PortCongestionConfig::default();
        assert!(cfg.ports.iter().any(|(n, _)| n == "shanghai"));
        assert!(cfg.ports.iter().any(|(n, _)| n == "la-long-beach"));
        assert_eq!(cfg.ports.len(), 13);
    }

    #[tokio::test]
    async fn run_cycle_writes_per_port_counts_and_total() {
        let pool = open_in_memory().await.unwrap();
        let counts: Vec<(String, usize)> = DEFAULT_PORTS
            .iter()
            .enumerate()
            .map(|(i, (name, _))| ((*name).to_string(), 100 + i))
            .collect();
        let reader = StaticReader {
            total: 5_000,
            counts,
        };
        let _ = run_cycle(&pool, &reader, &PortCongestionConfig::default())
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
        assert_eq!(rows.len(), 13);
        // Sum 100..=112 = 13*106 = 1378
        let total = parsed
            .pointer("/data/total_vessels_at_ports")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(total, 1378);
    }

    #[tokio::test]
    async fn run_cycle_zero_vessels_errors() {
        let pool = open_in_memory().await.unwrap();
        let reader = StaticReader {
            total: 0,
            counts: vec![],
        };
        let err = run_cycle(&pool, &reader, &PortCongestionConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, LogisticsSeederError::EmptyUpstream));
    }
}
