//! seed_earnings_calendar — SLOW-tier 7-day forward earnings
//! calendar.
//!
//! The webview's `EarningsCalendarPanel` (T4.2.8) renders a
//! grouped-by-day table of upcoming earnings releases for the
//! same broad-market basket the rest of the markets domain
//! tracks. Per SPEC-001 §24's H3 fix the edge handler reads
//! this cache slot directly; this seeder is the only piece in
//! the loop that talks to the upstream calendar provider.
//!
//! The DI trait [`EarningsCalendarFetcher`] keeps the seeder
//! independent of any specific calendar provider — the
//! production `pellucid-edge-bin` wires its concrete adapter
//! (Yahoo Finance v1 `/v1/finance/calendar/events`,
//! Financial Modeling Prep, Polygon.io, etc.) at boot.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use pellucid_db::Pool;

use crate::atomic_publish::{atomic_publish, PublishOutcome};
use crate::envelope::{SeedEnvelope, SeedMeta};
use crate::markets::seed_market_quotes::DEFAULT_SYMBOLS;
use crate::markets::MarketsSeederError;

/// Cache key — SLOW tier.
pub const CACHE_KEY: &str = "market:earnings-calendar:7d:v1";

/// SLOW-tier TTL — 6 h. The earnings calendar moves at
/// company-announcement cadence; a 6 h refresh comfortably
/// covers same-day repostings from the upstream.
pub const TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Source-version stamp.
pub const SOURCE_VERSION: &str = "earnings-calendar-7d-v1";

/// Cascade group tag.
pub const CASCADE_GROUP: &str = "markets";

/// Default lookahead window — 7 days.
pub const DEFAULT_LOOKAHEAD_DAYS: u32 = 7;

/// Earnings event timing relative to the trading day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EarningsTiming {
    /// Before the market open.
    BeforeOpen,
    /// After the market close.
    AfterClose,
    /// During regular trading hours (rare).
    DuringHours,
    /// Timing not reported by the upstream.
    Unknown,
}

/// Run-time configuration.
#[derive(Clone, Debug)]
pub struct EarningsCalendarConfig {
    /// Symbols to fetch upcoming earnings for. Defaults to the
    /// same basket as `seed_market_quotes`.
    pub symbols: Vec<String>,
    /// Lookahead window in days.
    pub lookahead_days: u32,
}

impl Default for EarningsCalendarConfig {
    fn default() -> Self {
        Self {
            symbols: DEFAULT_SYMBOLS.iter().map(|s| (*s).to_string()).collect(),
            lookahead_days: DEFAULT_LOOKAHEAD_DAYS,
        }
    }
}

/// One earnings event in the published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EarningsEventRow {
    /// Ticker symbol.
    pub symbol: String,
    /// Company name as reported by the upstream.
    pub company: String,
    /// Earnings date (`YYYY-MM-DD`, exchange-local).
    pub date: String,
    /// Timing relative to the trading day.
    pub timing: EarningsTiming,
    /// EPS estimate the market consensus has set, when reported.
    /// `None` when the upstream omitted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps_estimate: Option<f64>,
    /// Most-recent reported EPS (the prior quarter), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eps_actual_prior: Option<f64>,
}

/// Published snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EarningsCalendarSnapshot {
    /// Events sorted ascending by `date`, then `symbol`.
    pub events: Vec<EarningsEventRow>,
    /// Echo of the configured lookahead window.
    pub lookahead_days: u32,
    /// Wall-clock ms when the seeder assembled the snapshot.
    pub assembled_at_ms: i64,
}

/// Distilled earnings event — mirrors what the upstream returns.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedEarningsEvent {
    /// Ticker symbol.
    pub symbol: String,
    /// Company name.
    pub company: String,
    /// Earnings date (`YYYY-MM-DD`).
    pub date: String,
    /// Timing relative to the trading day.
    pub timing: EarningsTiming,
    /// EPS estimate.
    pub eps_estimate: Option<f64>,
    /// Prior-quarter EPS actual.
    pub eps_actual_prior: Option<f64>,
}

/// DI trait — wraps the production calendar provider.
#[async_trait]
pub trait EarningsCalendarFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch earnings events for the given symbols within
    /// `lookahead_days` days from "now". Implementations must
    /// return events in any order; the seeder sorts before
    /// publishing.
    async fn fetch_calendar(
        &self,
        symbols: &[&str],
        lookahead_days: u32,
    ) -> Result<Vec<FetchedEarningsEvent>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Run one cycle.
///
/// # Errors
/// - [`MarketsSeederError::Upstream`] when the fetcher returns Err.
/// - [`MarketsSeederError::EmptyUpstream`] when the fetcher
///   returns an empty list (no upcoming earnings is unusual
///   enough to flag — likely a feed outage).
pub async fn run_cycle(
    pool: &Pool,
    fetcher: &dyn EarningsCalendarFetcher,
    config: &EarningsCalendarConfig,
) -> Result<PublishOutcome, MarketsSeederError> {
    let symbol_refs: Vec<&str> = config.symbols.iter().map(String::as_str).collect();
    let fetched = fetcher
        .fetch_calendar(&symbol_refs, config.lookahead_days)
        .await
        .map_err(|e| MarketsSeederError::Upstream(e.to_string()))?;

    if fetched.is_empty() {
        return Err(MarketsSeederError::EmptyUpstream);
    }

    let mut events: Vec<EarningsEventRow> = fetched
        .into_iter()
        .map(|e| EarningsEventRow {
            symbol: e.symbol,
            company: e.company,
            date: e.date,
            timing: e.timing,
            eps_estimate: e.eps_estimate,
            eps_actual_prior: e.eps_actual_prior,
        })
        .collect();
    events.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.symbol.cmp(&b.symbol)));

    let assembled_at_ms = pellucid_core::now_ms();
    let snapshot = EarningsCalendarSnapshot {
        events,
        lookahead_days: config.lookahead_days,
        assembled_at_ms,
    };
    let envelope = SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: assembled_at_ms,
            ttl_ms: i64::try_from(TTL.as_millis()).unwrap_or(21_600_000),
            source_version: SOURCE_VERSION.to_string(),
            record_count: i64::try_from(snapshot.events.len()).unwrap_or(0),
            cascade_group: Some(CASCADE_GROUP.to_string()),
            run_id: String::new(),
        },
        data: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
    };
    let outcome = atomic_publish(pool, "markets", CACHE_KEY, &envelope, TTL).await?;
    Ok(outcome)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct StaticFetcher {
        rows: Vec<FetchedEarningsEvent>,
        last_call: Mutex<(Vec<String>, u32)>,
    }

    #[async_trait]
    impl EarningsCalendarFetcher for StaticFetcher {
        async fn fetch_calendar(
            &self,
            symbols: &[&str],
            lookahead_days: u32,
        ) -> Result<Vec<FetchedEarningsEvent>, Box<dyn std::error::Error + Send + Sync>>
        {
            *self.last_call.lock().unwrap() =
                (symbols.iter().map(|s| s.to_string()).collect(), lookahead_days);
            Ok(self.rows.clone())
        }
    }

    #[derive(Debug)]
    struct FailingFetcher;

    #[async_trait]
    impl EarningsCalendarFetcher for FailingFetcher {
        async fn fetch_calendar(
            &self,
            _symbols: &[&str],
            _lookahead_days: u32,
        ) -> Result<Vec<FetchedEarningsEvent>, Box<dyn std::error::Error + Send + Sync>>
        {
            Err("upstream down".into())
        }
    }

    fn ev(
        symbol: &str,
        company: &str,
        date: &str,
        timing: EarningsTiming,
    ) -> FetchedEarningsEvent {
        FetchedEarningsEvent {
            symbol: symbol.into(),
            company: company.into(),
            date: date.into(),
            timing,
            eps_estimate: Some(1.23),
            eps_actual_prior: Some(1.10),
        }
    }

    #[test]
    fn cache_key_pinned() {
        assert_eq!(CACHE_KEY, "market:earnings-calendar:7d:v1");
    }

    #[test]
    fn ttl_is_six_hours() {
        assert_eq!(TTL, Duration::from_secs(21_600));
    }

    #[test]
    fn default_config_uses_market_quotes_basket() {
        let cfg = EarningsCalendarConfig::default();
        assert_eq!(cfg.symbols.len(), DEFAULT_SYMBOLS.len());
        assert_eq!(cfg.lookahead_days, 7);
    }

    #[tokio::test]
    async fn run_cycle_writes_sorted_events() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![
                ev("QQQ", "Invesco QQQ", "2026-05-08", EarningsTiming::AfterClose),
                ev("SPY", "SPDR S&P 500 ETF", "2026-05-06", EarningsTiming::BeforeOpen),
                ev("DIA", "SPDR Dow Jones ETF", "2026-05-06", EarningsTiming::Unknown),
            ],
            last_call: Mutex::new((vec![], 0)),
        };
        let cfg = EarningsCalendarConfig {
            symbols: vec!["SPY".into(), "QQQ".into(), "DIA".into()],
            lookahead_days: 7,
        };
        let outcome = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        assert!(outcome.bytes_written > 0);
        let row: (String,) = sqlx::query_as(
            "SELECT payload FROM kv_envelope WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&row.0).unwrap();
        let events = parsed.pointer("/data/events").unwrap().as_array().unwrap();
        // Sorted by date asc, then symbol asc → DIA(05-06) / SPY(05-06) / QQQ(05-08).
        let symbols: Vec<&str> = events
            .iter()
            .map(|e| e.get("symbol").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(symbols, vec!["DIA", "SPY", "QQQ"]);
    }

    #[tokio::test]
    async fn run_cycle_empty_returns_empty_upstream_error() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![],
            last_call: Mutex::new((vec![], 0)),
        };
        let err = run_cycle(&pool, &fetcher, &EarningsCalendarConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::EmptyUpstream));
    }

    #[tokio::test]
    async fn run_cycle_upstream_failure_propagates() {
        let pool = open_in_memory().await.unwrap();
        let err = run_cycle(&pool, &FailingFetcher, &EarningsCalendarConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, MarketsSeederError::Upstream(_)));
    }

    #[tokio::test]
    async fn run_cycle_writes_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![ev("SPY", "S&P 500", "2026-05-06", EarningsTiming::BeforeOpen)],
            last_call: Mutex::new((vec![], 0)),
        };
        let _ = run_cycle(&pool, &fetcher, &EarningsCalendarConfig::default())
            .await
            .unwrap();
        let row: (String, String) = sqlx::query_as(
            "SELECT source_version, cascade_group FROM seed_meta WHERE cache_key = ?",
        )
        .bind(CACHE_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, SOURCE_VERSION);
        assert_eq!(row.1, CASCADE_GROUP);
    }

    #[tokio::test]
    async fn run_cycle_passes_symbols_and_lookahead_through() {
        let pool = open_in_memory().await.unwrap();
        let fetcher = StaticFetcher {
            rows: vec![ev("SPY", "S&P 500", "2026-05-06", EarningsTiming::BeforeOpen)],
            last_call: Mutex::new((vec![], 0)),
        };
        let cfg = EarningsCalendarConfig {
            symbols: vec!["SPY".into(), "QQQ".into()],
            lookahead_days: 14,
        };
        let _ = run_cycle(&pool, &fetcher, &cfg).await.unwrap();
        let (last_symbols, last_lookahead) = fetcher.last_call.lock().unwrap().clone();
        assert_eq!(last_symbols, vec!["SPY".to_string(), "QQQ".to_string()]);
        assert_eq!(last_lookahead, 14);
    }
}
