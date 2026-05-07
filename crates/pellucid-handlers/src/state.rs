//! Per-process state every handler is parameterised with.
//!
//! Holds the SQLite [`Pool`], the cache [`CoalesceRegistry`], and a
//! type-erased upstream client per domain. The upstream clients
//! are wrapped in [`Arc<dyn Trait>`] so production builds can
//! inject the real `pellucid_streams` clients while tests inject
//! fakes — without `pellucid-handlers` having to depend on
//! `pellucid-streams` directly (which would cycle since
//! `pellucid-streams` depends on the generated types).

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use pellucid_cache::CoalesceRegistry;
use pellucid_db::Pool;
use pellucid_ml::MlEngine;

use crate::generated::aviation::v1::FlightStatus;

/// Errors construction may surface — currently only used by the
/// `for_tests` constructor, but the type is named so production
/// `pellucid-edge-bin` boot code can return it without a free
/// `Box<dyn Error>`.
#[derive(Debug, Error)]
pub enum AppStateError {
    /// SQLite pool / migration failure.
    #[error("db: {0}")]
    Db(#[from] pellucid_db::DbError),
}

/// Aviation upstream — abstracted so tests can swap in a wiremock
/// `pellucid-streams::AviationstackClient` or a hand-rolled fake
/// without dragging the streams crate as a hard dep of handlers.
#[async_trait]
pub trait FlightStatusUpstream: Send + Sync + std::fmt::Debug {
    /// Fetch the live flight status. `Ok(None)` means "no such
    /// flight on this date" (negative-cacheable). `Err` is reserved
    /// for transient upstream failures (5xx / parse / network).
    async fn fetch_flight(
        &self,
        flight: &str,
        date: &str,
        origin: &str,
    ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Shared handler state. Cheap to clone — every Arc is shallow.
#[derive(Clone)]
pub struct AppState {
    /// SQLite pool used for the KV cache + entitlements + handler
    /// queries.
    pub pool: Pool,
    /// Stampede coalescing registry — one per process.
    pub cache_registry: Arc<CoalesceRegistry>,
    /// Aviation upstream client.
    pub aviation: Arc<dyn FlightStatusUpstream>,
    /// Cloud ML engine — `None` when the binary booted without
    /// `GROQ_API_KEY` / `HF_TOKEN` configured. Handlers that need
    /// ML must check `state.ml.is_some()` and return 503 with
    /// `Retry-After: 30` when absent (matches the H2 fix's
    /// upstream-down semantics from spec §14.1).
    pub ml: Option<Arc<dyn MlEngine>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("pool", &"sqlx::Pool")
            .field("cache_registry", &self.cache_registry)
            .field("aviation", &self.aviation)
            .field("ml_present", &self.ml.is_some())
            .finish()
    }
}

impl AppState {
    /// Build a state with the supplied components.
    #[must_use]
    pub fn new(
        pool: Pool,
        cache_registry: Arc<CoalesceRegistry>,
        aviation: Arc<dyn FlightStatusUpstream>,
    ) -> Self {
        Self {
            pool,
            cache_registry,
            aviation,
            ml: None,
        }
    }

    /// In-memory state for unit tests. Aviation upstream returns
    /// `Ok(None)` for every input; ML engine is absent (handlers
    /// should return 503 in that path). Tests that exercise ML
    /// install a real engine via [`Self::with_ml`].
    #[must_use]
    pub fn for_tests() -> Self {
        // `connect_lazy_with` takes pre-parsed options so it is
        // infallible — no `expect()` needed. The pool actually
        // connects on first use; tests that touch the database
        // should prefer [`Self::for_tests_async`].
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect_lazy_with(opts);
        Self {
            pool,
            cache_registry: Arc::new(CoalesceRegistry::default()),
            aviation: Arc::new(NullAviation),
            ml: None,
        }
    }

    /// Async equivalent of [`Self::for_tests`] that runs migrations
    /// against an in-memory database. Use this in tests that need
    /// the cache layer to actually persist rows.
    ///
    /// # Errors
    /// Returns [`AppStateError::Sqlx`] if `pellucid_db::open_in_memory`
    /// or migrations fail.
    pub async fn for_tests_async() -> Result<Self, AppStateError> {
        let pool = pellucid_db::open_in_memory().await?;
        Ok(Self {
            pool,
            cache_registry: Arc::new(CoalesceRegistry::default()),
            aviation: Arc::new(NullAviation),
            ml: None,
        })
    }

    /// Replace the aviation upstream — used by tests that need to
    /// drive specific responses through the handler.
    #[must_use]
    pub fn with_aviation(mut self, upstream: Arc<dyn FlightStatusUpstream>) -> Self {
        self.aviation = upstream;
        self
    }

    /// Install a cloud ML engine. Production binaries call this
    /// after `pellucid_ml::build_from_vault(&vault, &cfg)` returns
    /// successfully; tests can install a stub `MlEngine` impl.
    #[must_use]
    pub fn with_ml(mut self, engine: Arc<dyn MlEngine>) -> Self {
        self.ml = Some(engine);
        self
    }
}

/// Null upstream — every fetch returns `Ok(None)`. Used by smoke
/// tests that just assert the route is mounted; real handler tests
/// install a hand-rolled fake or a wiremock'd `AviationstackClient`.
#[derive(Debug)]
struct NullAviation;

#[async_trait]
impl FlightStatusUpstream for NullAviation {
    async fn fetch_flight(
        &self,
        _flight: &str,
        _date: &str,
        _origin: &str,
    ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(None)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn for_tests_constructs_state() {
        let s = AppState::for_tests();
        // Aviation upstream is the null one.
        let aviation: &dyn FlightStatusUpstream = &*s.aviation;
        let dbg = format!("{aviation:?}");
        assert!(dbg.contains("NullAviation"));
    }

    #[tokio::test]
    async fn for_tests_async_runs_migrations() {
        let s = AppState::for_tests_async().await.unwrap();
        // pool is usable: a trivial query against the migrations
        // table created by `pellucid_db::migrate`.
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pellucid_migrations")
            .fetch_one(&s.pool)
            .await
            .unwrap();
        assert!(row.0 >= 1, "migrations table must have at least one row");
    }

    #[tokio::test]
    async fn null_aviation_returns_none() {
        let n = NullAviation;
        let r = n.fetch_flight("X", "2026", "AAA").await.unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn with_aviation_swaps_upstream() {
        #[derive(Debug)]
        struct Fixed;
        #[async_trait]
        impl FlightStatusUpstream for Fixed {
            async fn fetch_flight(
                &self,
                _f: &str,
                _d: &str,
                _o: &str,
            ) -> Result<Option<FlightStatus>, Box<dyn std::error::Error + Send + Sync>>
            {
                Ok(Some(FlightStatus {
                    flight: "FX1".into(),
                    ..FlightStatus::default()
                }))
            }
        }
        let s = AppState::for_tests().with_aviation(Arc::new(Fixed));
        let r = s.aviation.fetch_flight("a", "b", "c").await.unwrap();
        assert_eq!(r.unwrap().flight, "FX1");
    }
}
