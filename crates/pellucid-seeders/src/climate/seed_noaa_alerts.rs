//! seed_noaa_alerts — FAST-tier NOAA active-alert snapshot.
//! Production adapters wire to api.weather.gov `/alerts/active`;
//! tests inject deterministic alert rows.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::climate::ClimateSeederError;
use crate::envelope::{SeedEnvelope, SeedMeta};

/// Cache key — FAST tier.
pub const CACHE_KEY: &str = "climate:noaa-alerts:current:v1";

/// 15 m TTL — alerts churn frequently.
pub const TTL: Duration = Duration::from_secs(15 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "noaa-alerts-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "climate";

/// One NOAA alert row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlertRow {
    /// NWS event class (e.g. `Tornado Warning`, `Flash Flood`).
    pub event: String,
    /// Affected area description.
    pub area: String,
    /// Severity bucket — `Minor`, `Moderate`, `Severe`, `Extreme`.
    pub severity: String,
    /// Urgency — `Past`, `Future`, `Expected`, `Immediate`.
    pub urgency: String,
    /// ISO-8601 effective stamp.
    pub effective: String,
    /// ISO-8601 expiry stamp.
    pub expires: String,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlertsSnapshot {
    /// Active alerts sorted by descending severity then by event.
    pub rows: Vec<AlertRow>,
    /// Total count of active alerts.
    pub total: usize,
    /// Wall-clock ms when assembled.
    pub assembled_at_ms: i64,
}

/// Distilled fetched alert.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedAlert {
    /// Event class.
    pub event: String,
    /// Area description.
    pub area: String,
    /// Severity bucket.
    pub severity: String,
    /// Urgency bucket.
    pub urgency: String,
    /// Effective ISO-8601.
    pub effective: String,
    /// Expires ISO-8601.
    pub expires: String,
}

/// DI trait.
#[async_trait]
pub trait NoaaAlertsFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch the latest alerts.
    async fn fetch_alerts(
        &self,
    ) -> Result<Vec<FetchedAlert>, Box<dyn std::error::Error + Send + Sync>>;
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "Extreme" => 4,
        "Severe" => 3,
        "Moderate" => 2,
        "Minor" => 1,
        _ => 0,
    }
}

/// Run one cycle.
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn NoaaAlertsFetcher,
) -> Result<PublishOutcome, ClimateSeederError> {
    let fetched = fetcher
        .fetch_alerts()
        .await
        .map_err(|e| ClimateSeederError::Upstream(e.to_string()))?;
    if fetched.is_empty() {
        return Err(ClimateSeederError::EmptyUpstream);
    }
    let mut rows: Vec<AlertRow> = fetched
        .into_iter()
        .map(|a| AlertRow {
            event: a.event,
            area: a.area,
            severity: a.severity,
            urgency: a.urgency,
            effective: a.effective,
            expires: a.expires,
        })
        .collect();
    rows.sort_by(|a, b| {
        severity_rank(&b.severity)
            .cmp(&severity_rank(&a.severity))
            .then_with(|| a.event.cmp(&b.event))
    });
    let total = rows.len();
    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = AlertsSnapshot {
        rows,
        total,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(900_000),
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
        rows: Vec<FetchedAlert>,
    }

    #[async_trait]
    impl NoaaAlertsFetcher for StaticFetcher {
        async fn fetch_alerts(
            &self,
        ) -> Result<Vec<FetchedAlert>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.rows.clone())
        }
    }

    fn alert(event: &str, severity: &str) -> FetchedAlert {
        FetchedAlert {
            event: event.into(),
            area: "TX".into(),
            severity: severity.into(),
            urgency: "Immediate".into(),
            effective: "2026-04-29T00:00:00Z".into(),
            expires: "2026-04-30T00:00:00Z".into(),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "climate:noaa-alerts:current:v1");
    }

    #[test]
    fn severity_rank_orders() {
        assert!(severity_rank("Extreme") > severity_rank("Severe"));
        assert!(severity_rank("Severe") > severity_rank("Moderate"));
        assert!(severity_rank("Moderate") > severity_rank("Minor"));
        assert_eq!(severity_rank("Unknown"), 0);
    }

    #[tokio::test]
    async fn run_cycle_writes_severity_sorted() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                alert("Flash Flood", "Moderate"),
                alert("Tornado Warning", "Extreme"),
                alert("Wind Advisory", "Minor"),
            ],
        };
        let _ = run_cycle(&pool, &fetcher).await.unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT payload FROM kv_envelope WHERE cache_key = ?")
                .bind(CACHE_KEY)
                .fetch_one(&pool)
                .await
                .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let events: Vec<&str> = parsed
            .pointer("/data/rows")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.get("event").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(events, vec!["Tornado Warning", "Flash Flood", "Wind Advisory"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher { rows: vec![] };
        let err = run_cycle(&pool, &fetcher).await.unwrap_err();
        assert!(matches!(err, ClimateSeederError::EmptyUpstream));
    }
}
