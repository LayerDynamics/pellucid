//! seed_gpsjam — FAST-tier daily snapshot of the worst GPS-
//! interference H3 hexagons from gpsjam.org.
//!
//! The seeder fetches today's daily H3-level-4 file
//! (`/data/{Y}/{M}/{D}/h3_4.json`); when today's file is not
//! yet published (gpsjam rolls over around 03:00 UTC), it
//! falls back to yesterday's file. The published snapshot
//! keeps the top-`limit` cells by `bad_pos_pct` so the
//! airspace-restrictions panel doesn't have to render the
//! full daily 30-50 MB GeoJSON.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::aviation::AviationSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier slot already in `FAST_KEYS`.
pub const CACHE_KEY: &str = "aviation:airspace-restrictions:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "gpsjam-h3l4-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "aviation-gpsjam";

/// Default top-cell cap. The full daily file has 30-100k
/// cells; the panel only renders the worst few hundred.
pub const DEFAULT_TOP_LIMIT: usize = 500;

/// Default minimum sample count — discards cells with too few
/// observations to be statistically meaningful.
pub const DEFAULT_MIN_SAMPLES: i64 = 10;

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct GpsjamConfig {
    /// Target UTC date `(year, month, day)`. Production uses
    /// today; the seeder falls back to `target - 1 day` when
    /// the upstream returns 404.
    pub target_date: (u16, u8, u8),
    /// Top-N cap on the published snapshot.
    pub top_limit: usize,
    /// Discard cells whose `samples` is below this threshold.
    pub min_samples: i64,
}

impl GpsjamConfig {
    /// Build a config with today's UTC date.
    #[must_use]
    pub fn default_for_today_utc() -> Self {
        Self {
            target_date: today_utc_ymd(),
            top_limit: DEFAULT_TOP_LIMIT,
            min_samples: DEFAULT_MIN_SAMPLES,
        }
    }
}

/// One H3 cell row in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpsjamCellRow {
    /// H3 cell id (15-character hex).
    pub h3: String,
    /// Fraction of aircraft that reported degraded GPS
    /// accuracy in this cell (`[0.0, 1.0]`).
    pub bad_pos_pct: f64,
    /// Sample size (number of observed aircraft).
    pub samples: i64,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpsjamSnapshot {
    /// Top-N cells by `bad_pos_pct` (descending).
    pub rows: Vec<GpsjamCellRow>,
    /// `(YYYY, MM, DD)` of the upstream file the snapshot was
    /// derived from (may differ from today if the seeder
    /// fell back to yesterday).
    pub source_date: (u16, u8, u8),
    /// Total cell count returned by the upstream (before the
    /// `top_limit` cap was applied).
    pub total_cells: usize,
    /// Wall-clock ms when the snapshot was assembled.
    pub assembled_at_ms: i64,
}

/// Distilled cell — mirrors `pellucid_streams::JammingCell`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedCell {
    /// H3 cell id.
    pub h3: String,
    /// Fraction of aircraft with degraded GPS in `[0, 1]`.
    pub bad_pos_pct: f64,
    /// Sample count.
    pub samples: i64,
}

/// DI trait — wraps `pellucid_streams::GpsjamClient::fetch_daily_h3`.
#[async_trait]
pub trait GpsjamFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch one day's H3 cells. `Ok(None)` indicates the
    /// upstream returned 404 (file not yet published) — the
    /// seeder retries with the previous day.
    async fn fetch_daily_h3(
        &self,
        year: u16,
        month: u8,
        day: u8,
    ) -> Result<Option<Vec<FetchedCell>>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`AviationSeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GpsjamFetcher,
    config: &GpsjamConfig,
) -> Result<PublishOutcome, AviationSeederError> {
    let (mut y, mut m, mut d) = config.target_date;
    let mut fetched_opt = fetcher
        .fetch_daily_h3(y, m, d)
        .await
        .map_err(|e| AviationSeederError::Upstream(e.to_string()))?;
    if fetched_opt.is_none() {
        // Try yesterday once.
        let (py, pm, pd) = previous_day(y, m, d);
        y = py;
        m = pm;
        d = pd;
        fetched_opt = fetcher
            .fetch_daily_h3(y, m, d)
            .await
            .map_err(|e| AviationSeederError::Upstream(e.to_string()))?;
    }
    let fetched = fetched_opt.ok_or(AviationSeederError::EmptyUpstream)?;

    let total_cells = fetched.len();
    let mut filtered: Vec<GpsjamCellRow> = fetched
        .into_iter()
        .filter(|c| c.samples >= config.min_samples)
        .map(|c| GpsjamCellRow {
            h3: c.h3,
            bad_pos_pct: c.bad_pos_pct,
            samples: c.samples,
        })
        .collect();
    if filtered.is_empty() {
        return Err(AviationSeederError::EmptyUpstream);
    }
    filtered.sort_by(|a, b| {
        b.bad_pos_pct
            .partial_cmp(&a.bad_pos_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    filtered.truncate(config.top_limit);

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GpsjamSnapshot {
        rows: filtered,
        source_date: (y, m, d),
        total_cells,
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
    let outcome = atomic_publish(pool, "aviation", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn today_utc_ymd() -> (u16, u8, u8) {
    let secs = pellucid_core::now_ms() / 1000;
    let days = secs / 86_400;
    epoch_days_to_ymd(days)
}

fn epoch_days_to_ymd(days: i64) -> (u16, u8, u8) {
    // Same Hinnant algorithm as seed_aviation_status::epoch_days_to_ymd
    // but typed for u16/u8 (gpsjam's path layout).
    let days = days + 719_468;
    let era = if days >= 0 {
        days / 146_097
    } else {
        (days - 146_096) / 146_097
    };
    let doe = (days - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = (y + i64::from(m <= 2)) as u16;
    (year, m as u8, d as u8)
}

/// Compute the previous day in the proleptic Gregorian calendar.
const fn previous_day(year: u16, month: u8, day: u8) -> (u16, u8, u8) {
    if day > 1 {
        return (year, month, day - 1);
    }
    if month > 1 {
        let prev_month = month - 1;
        return (year, prev_month, days_in_month(year, prev_month));
    }
    (year - 1, 12, 31)
}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let y = year as u32;
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        // Map from (y, m, d) → response. None means upstream
        // returns Ok(None) (404 path); absent key means
        // upstream returns Err.
        responses: std::collections::HashMap<(u16, u8, u8), Option<Vec<FetchedCell>>>,
    }

    #[async_trait]
    impl GpsjamFetcher for StaticFetcher {
        async fn fetch_daily_h3(
            &self,
            year: u16,
            month: u8,
            day: u8,
        ) -> Result<Option<Vec<FetchedCell>>, Box<dyn std::error::Error + Send + Sync>> {
            match self.responses.get(&(year, month, day)) {
                Some(v) => Ok(v.clone()),
                None => Err("upstream missing fixture for this date".into()),
            }
        }
    }

    fn cell(h3: &str, bad: f64, n: i64) -> FetchedCell {
        FetchedCell {
            h3: h3.into(),
            bad_pos_pct: bad,
            samples: n,
        }
    }

    fn config_with_target(date: (u16, u8, u8)) -> GpsjamConfig {
        GpsjamConfig {
            target_date: date,
            top_limit: DEFAULT_TOP_LIMIT,
            min_samples: DEFAULT_MIN_SAMPLES,
        }
    }

    #[test]
    fn cache_key_is_fast_tier_slot() {
        assert_eq!(CACHE_KEY, "aviation:airspace-restrictions:v1");
    }

    #[test]
    fn epoch_days_to_ymd_2026_05_04() {
        assert_eq!(epoch_days_to_ymd(20577), (2026, 5, 4));
    }

    #[test]
    fn previous_day_handles_month_and_year_boundaries() {
        assert_eq!(previous_day(2026, 5, 4), (2026, 5, 3));
        assert_eq!(previous_day(2026, 5, 1), (2026, 4, 30));
        assert_eq!(previous_day(2026, 1, 1), (2025, 12, 31));
        // 2024 is leap.
        assert_eq!(previous_day(2024, 3, 1), (2024, 2, 29));
        assert_eq!(previous_day(2025, 3, 1), (2025, 2, 28));
    }

    #[tokio::test]
    async fn run_cycle_writes_top_n_sorted_descending() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            (2026, 5, 4),
            Some(vec![
                cell("a", 0.10, 50),
                cell("b", 0.80, 50), // worst
                cell("c", 0.40, 50),
            ]),
        );
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);

        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].get("h3").unwrap().as_str().unwrap(), "b");
        assert_eq!(rows[1].get("h3").unwrap().as_str().unwrap(), "c");
        assert_eq!(rows[2].get("h3").unwrap().as_str().unwrap(), "a");
        assert_eq!(
            parsed.pointer("/data/total_cells").unwrap().as_u64(),
            Some(3)
        );
        assert_eq!(
            parsed.pointer("/data/source_date/0").unwrap().as_u64(),
            Some(2026)
        );
    }

    #[tokio::test]
    async fn run_cycle_falls_back_to_previous_day_on_404() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert((2026, 5, 4), None); // 404
        responses.insert(
            (2026, 5, 3),
            Some(vec![cell("a", 0.5, 100), cell("b", 0.7, 100)]),
        );
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
            .await
            .unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        // source_date should be yesterday.
        assert_eq!(
            parsed.pointer("/data/source_date/2").unwrap().as_u64(),
            Some(3)
        );
    }

    #[tokio::test]
    async fn run_cycle_drops_low_sample_cells() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            (2026, 5, 4),
            Some(vec![
                cell("a", 0.99, 1), // dropped — below min_samples
                cell("b", 0.40, 50),
            ]),
        );
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
            .await
            .unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("h3").unwrap().as_str().unwrap(), "b");
    }

    #[tokio::test]
    async fn run_cycle_truncates_to_top_limit() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        let cells: Vec<FetchedCell> = (0..1000)
            .map(|i| cell(&format!("cell-{i:04}"), f64::from(i) / 1000.0, 50))
            .collect();
        responses.insert((2026, 5, 4), Some(cells));
        let fetcher = StaticFetcher { responses };
        let cfg = GpsjamConfig {
            target_date: (2026, 5, 4),
            top_limit: 10,
            min_samples: DEFAULT_MIN_SAMPLES,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
            .bind(CACHE_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let rows = parsed.pointer("/data/rows").unwrap().as_array().unwrap();
        assert_eq!(rows.len(), 10);
        // The top row must be cell-0999 (highest bad_pos_pct).
        assert_eq!(rows[0].get("h3").unwrap().as_str().unwrap(), "cell-0999");
        assert_eq!(
            parsed.pointer("/data/total_cells").unwrap().as_u64(),
            Some(1000)
        );
    }

    #[tokio::test]
    async fn run_cycle_both_days_404_errors() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert((2026, 5, 4), None);
        responses.insert((2026, 5, 3), None);
        let fetcher = StaticFetcher { responses };
        let err = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let err = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
            .await
            .unwrap_err();
        assert!(matches!(err, AviationSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert((2026, 5, 4), Some(vec![cell("a", 0.5, 50)]));
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_with_target((2026, 5, 4)))
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
