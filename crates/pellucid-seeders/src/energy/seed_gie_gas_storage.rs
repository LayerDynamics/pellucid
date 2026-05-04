//! seed_gie_gas_storage — FAST-tier snapshot of EU gas
//! storage levels from GIE AGSI.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::energy::EnergySeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — NEW FAST tier slot added by T3.8 energy domain.
pub const CACHE_KEY: &str = "energy:gie-gas-storage:current:v1";

/// FAST-tier TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "gie-gas-storage-agsi-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "energy-gas-storage";

/// Default countries — major EU storage holders.
pub const DEFAULT_COUNTRIES: &[&str] = &[
    "DE", "FR", "IT", "NL", "AT", "ES", "BE", "PL", "CZ", "HU",
];

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct GieGasStorageConfig {
    /// ISO 2-letter country codes.
    pub countries: Vec<String>,
    /// `YYYY-MM-DD` start of the date window.
    pub from_date: String,
    /// `YYYY-MM-DD` end of the date window.
    pub to_date: String,
}

impl GieGasStorageConfig {
    /// Build a config covering today's UTC date for the
    /// default basket (one-day window — AGSI publishes the
    /// most recent gas-day reading at this endpoint).
    #[must_use]
    pub fn default_for_today_utc() -> Self {
        let (y, m, d) = today_utc_ymd();
        let date = format!("{y:04}-{m:02}-{d:02}");
        Self {
            countries: DEFAULT_COUNTRIES.iter().map(|s| (*s).to_string()).collect(),
            from_date: date.clone(),
            to_date: date,
        }
    }
}

/// One per-country reading row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GasStoragePublishedRow {
    /// Gas-day start.
    pub gas_day_start: String,
    /// Country name.
    pub name: String,
    /// ISO 2-letter country code.
    pub code: String,
    /// Gas in storage (TWh).
    pub gas_in_storage_twh: f64,
    /// Working gas volume (TWh).
    pub working_gas_volume_twh: f64,
    /// % full.
    pub full_pct: f64,
    /// Daily injection (TWh).
    pub injection_twh: f64,
    /// Daily withdrawal (TWh).
    pub withdrawal_twh: f64,
    /// Day-over-day trend.
    pub trend: f64,
    /// Status (`E` estimated, `C` confirmed).
    pub status: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GieGasStorageSnapshot {
    /// One row per country in the basket the upstream had data
    /// for. Sorted by `full_pct` desc.
    pub rows: Vec<GasStoragePublishedRow>,
    /// Date window the snapshot covers.
    pub date_range: (String, String),
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled storage row — mirrors `pellucid_streams::GasStorageRow`.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedStorageRow {
    /// Gas-day start.
    pub gas_day_start: String,
    /// Country name.
    pub name: String,
    /// ISO 2-letter code.
    pub code: String,
    /// Gas in storage (TWh).
    pub gas_in_storage_twh: f64,
    /// Working gas volume (TWh).
    pub working_gas_volume_twh: f64,
    /// % full.
    pub full_pct: f64,
    /// Injection (TWh).
    pub injection_twh: f64,
    /// Withdrawal (TWh).
    pub withdrawal_twh: f64,
    /// Trend.
    pub trend: f64,
    /// Status.
    pub status: String,
}

/// DI trait — wraps `pellucid_streams::GieAgsiClient::fetch_country`.
#[async_trait]
pub trait GieGasStorageFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch storage for one country over the date window.
    /// Returns the most-recent matching row when the upstream
    /// has multiple days in the window.
    async fn fetch_country(
        &self,
        country: &str,
        from_date: &str,
        to_date: &str,
    ) -> Result<Vec<FetchedStorageRow>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// See [`EnergySeederError`].
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn GieGasStorageFetcher,
    config: &GieGasStorageConfig,
) -> Result<PublishOutcome, EnergySeederError> {
    let mut rows: Vec<GasStoragePublishedRow> = Vec::with_capacity(config.countries.len());
    for country in &config.countries {
        let result = fetcher
            .fetch_country(country, &config.from_date, &config.to_date)
            .await
            .map_err(|e| EnergySeederError::Upstream(e.to_string()))?;
        if let Some(row) = result.into_iter().max_by(|a, b| a.gas_day_start.cmp(&b.gas_day_start))
        {
            rows.push(GasStoragePublishedRow {
                gas_day_start: row.gas_day_start,
                name: row.name,
                code: row.code,
                gas_in_storage_twh: row.gas_in_storage_twh,
                working_gas_volume_twh: row.working_gas_volume_twh,
                full_pct: row.full_pct,
                injection_twh: row.injection_twh,
                withdrawal_twh: row.withdrawal_twh,
                trend: row.trend,
                status: row.status,
            });
        }
    }
    if rows.is_empty() {
        return Err(EnergySeederError::EmptyUpstream);
    }
    rows.sort_by(|a, b| b.full_pct.partial_cmp(&a.full_pct).unwrap_or(std::cmp::Ordering::Equal));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = GieGasStorageSnapshot {
        rows,
        date_range: (config.from_date.clone(), config.to_date.clone()),
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
        atomic_publish(pool, "energy", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

fn today_utc_ymd() -> (u16, u8, u8) {
    let secs = pellucid_core::now_ms() / 1000;
    let days = secs / 86_400 + 719_468;
    let era = if days >= 0 { days / 146_097 } else { (days - 146_096) / 146_097 };
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

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[derive(Debug)]
    struct StaticFetcher {
        responses: std::collections::HashMap<String, Vec<FetchedStorageRow>>,
    }

    #[async_trait]
    impl GieGasStorageFetcher for StaticFetcher {
        async fn fetch_country(
            &self,
            country: &str,
            _from_date: &str,
            _to_date: &str,
        ) -> Result<Vec<FetchedStorageRow>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(self.responses.get(country).cloned().unwrap_or_default())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl GieGasStorageFetcher for FailingFetcher {
        async fn fetch_country(
            &self,
            _country: &str,
            _from_date: &str,
            _to_date: &str,
        ) -> Result<Vec<FetchedStorageRow>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn row(code: &str, gas_day: &str, full_pct: f64) -> FetchedStorageRow {
        FetchedStorageRow {
            gas_day_start: gas_day.into(),
            name: code.into(),
            code: code.into(),
            gas_in_storage_twh: 200.0,
            working_gas_volume_twh: 250.0,
            full_pct,
            injection_twh: 5.0,
            withdrawal_twh: 1.0,
            trend: 0.4,
            status: "E".into(),
        }
    }

    fn config_for(countries: &[&str], date: &str) -> GieGasStorageConfig {
        GieGasStorageConfig {
            countries: countries.iter().map(|s| (*s).to_string()).collect(),
            from_date: date.into(),
            to_date: date.into(),
        }
    }

    #[test]
    fn cache_key_is_v1() {
        assert_eq!(CACHE_KEY, "energy:gie-gas-storage:current:v1");
    }

    #[test]
    fn default_for_today_utc_yields_today_window() {
        let cfg = GieGasStorageConfig::default_for_today_utc();
        assert_eq!(cfg.countries.len(), 10);
        assert_eq!(cfg.from_date, cfg.to_date);
        assert_eq!(cfg.from_date.len(), 10);
    }

    #[tokio::test]
    async fn run_cycle_writes_snapshot_sorted_by_full_pct_desc() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("DE".into(), vec![row("DE", "2026-05-04", 91.7)]);
        responses.insert("FR".into(), vec![row("FR", "2026-05-04", 75.3)]);
        responses.insert("IT".into(), vec![row("IT", "2026-05-04", 88.2)]);
        let fetcher = StaticFetcher { responses };
        let outcome = run_cycle(
            &pool,
            &fetcher,
            &config_for(&["DE", "FR", "IT"], "2026-05-04"),
        )
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
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].get("code").unwrap().as_str().unwrap(), "DE");
        assert_eq!(rows[1].get("code").unwrap().as_str().unwrap(), "IT");
        assert_eq!(rows[2].get("code").unwrap().as_str().unwrap(), "FR");
    }

    #[tokio::test]
    async fn run_cycle_picks_latest_gas_day_per_country() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert(
            "DE".into(),
            vec![
                row("DE", "2026-05-03", 91.0), // older
                row("DE", "2026-05-04", 91.7), // newer
            ],
        );
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_for(&["DE"], "2026-05-04"))
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
        assert_eq!(
            rows[0].get("gas_day_start").unwrap().as_str().unwrap(),
            "2026-05-04"
        );
    }

    #[tokio::test]
    async fn run_cycle_drops_country_with_no_data() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("DE".into(), vec![row("DE", "2026-05-04", 91.7)]);
        // FR returns empty.
        responses.insert("FR".into(), vec![]);
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_for(&["DE", "FR"], "2026-05-04"))
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
    async fn run_cycle_no_data_for_any_country_errors() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            responses: std::collections::HashMap::new(),
        };
        let err = run_cycle(&pool, &fetcher, &config_for(&["DE"], "2026-05-04"))
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &config_for(&["DE"], "2026-05-04"))
            .await
            .unwrap_err();
        assert!(matches!(err, EnergySeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let mut responses = std::collections::HashMap::new();
        responses.insert("DE".into(), vec![row("DE", "2026-05-04", 91.7)]);
        let fetcher = StaticFetcher { responses };
        let _ = run_cycle(&pool, &fetcher, &config_for(&["DE"], "2026-05-04"))
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
