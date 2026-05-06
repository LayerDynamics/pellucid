//! seed_station_records — SLOW-tier monthly station-record snapshot
//! (NOAA/NCEI Global Historical Climatology Network). Tests inject
//! deterministic station rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "climate:station-records:monthly:v1";

/// 24 h TTL.
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "station-records-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "climate";

/// One station-record row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRecordRow {
    /// Station identifier (NCDC ID).
    pub station_id: String,
    /// Station label (city / region).
    pub label: String,
    /// Record class — `high-temp`, `low-temp`, `precip`, `wind`.
    pub record_class: String,
    /// Recorded value in the class native unit.
    pub value: f64,
    /// ISO-8601 date the record was set.
    pub set_on: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StationRecordsSnapshot {
    /// Records sorted ascending by station id.
    pub rows: Vec<StationRecordRow>,
    /// Reference month (e.g. `2026-04`).
    pub month: String,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched record.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedRecord {
    /// Station id.
    pub station_id: String,
    /// Label.
    pub label: String,
    /// Record class.
    pub record_class: String,
    /// Value.
    pub value: f64,
    /// Set-on date.
    pub set_on: String,
}

/// DI trait.
#[async_trait]
pub trait StationRecordsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest records + month stamp.
    async fn fetch_records(
        &self,
    ) -> Result<(Vec<FetchedRecord>, String), Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn StationRecordsFetcher,
) -> Result<PublishOutcome, ClimateSeederError> {
    let (fetched, month) = fetcher
        .fetch_records()
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }
    let mut rows: Vec<StationRecordRow> = fetched
        .into_iter()
        .map(|r| StationRecordRow {
            station_id: r.station_id,
            label: r.label,
            record_class: r.record_class,
            value: r.value,
            set_on: r.set_on,
        })
        .collect();
    rows.sort_by(|a, b| a.station_id.cmp(&b.station_id));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = StationRecordsSnapshot {
        rows,
        month,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(86_400_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.rows.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "climate", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedRecord>,
        month: String,
    }

    #[async_trait]
    impl StationRecordsFetcher for StaticFetcher {
        async fn fetch_records(
            &self,
        ) -> Result<(Vec<FetchedRecord>, String), Box<dyn std::error::Error + Send + Sync>>
        {
            Ok((self.rows.clone(), self.month.clone()))
        }
    }

    fn rec(id: &str, label: &str, class: &str, value: f64) -> FetchedRecord {
        FetchedRecord {
            station_id: id.into(),
            label: label.into(),
            record_class: class.into(),
            value,
            set_on: "2026-04-15".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "climate:station-records:monthly:v1");
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_rows() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                rec("USC123", "Phoenix AZ", "high-temp", 122.0),
                rec("USC001", "Death Valley", "high-temp", 134.0),
            ],
            month: "2026-04".into(),
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let ids: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("station_id").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["USC001", "USC123"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            month: "2026-04".into(),
        };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }
}
