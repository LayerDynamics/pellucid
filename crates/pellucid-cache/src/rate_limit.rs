//! Sliding-window rate limit over SQLite. Replaces the original
//! WorldMonitor `server/_shared/rate-limit.ts` Upstash sliding window.
//!
//! Three buckets per request, each with its own `(limit, window_ms)`:
//!   1. **endpoint** (`rl:ep:<rpc>:<ip>`) — per-endpoint cap, e.g.
//!      classify-event 600/60s, summarize-article-cache 3000/60s.
//!   2. **global** (`rl:ip:<ip>`) — per-IP cap across all endpoints
//!      (default 600/60s).
//!   3. **aggregate umbrella** (`rl:agg:<ip>`) — total ceiling across
//!      every bucket so a high-limit endpoint cannot let a single IP
//!      blow past the deployment-wide cap. **SPEC-001 §24.3 M8 fix.**
//!
//! Order of evaluation: endpoint → global → aggregate. Whichever fails
//! first short-circuits with the matching [`RateLimitDecision::Denied`]
//! variant. On all-pass, the request_at_ms timestamp is recorded in
//! every bucket's row inside a single transaction so partial writes
//! cannot occur.

use sqlx::Row;

use pellucid_db::Pool;

/// Configuration for a single sliding-window bucket.
#[derive(Debug, Clone, Copy)]
pub struct BucketConfig {
    /// Max requests inside `window_ms`.
    pub limit: u32,
    /// Window length in milliseconds.
    pub window_ms: i64,
}

/// Three-bucket rate-limit configuration. Defaults match SPEC-001 §25.5.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Per-endpoint cap. Override per-RPC.
    pub endpoint: BucketConfig,
    /// Per-IP cap across all endpoints.
    pub global: BucketConfig,
    /// Aggregate umbrella cap (M8 fix).
    pub aggregate: BucketConfig,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            endpoint: BucketConfig {
                limit: 600,
                window_ms: 60_000,
            },
            global: BucketConfig {
                limit: 600,
                window_ms: 60_000,
            },
            aggregate: BucketConfig {
                limit: 4000,
                window_ms: 60_000,
            },
        }
    }
}

/// Outcome of a rate-limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitDecision {
    /// Every bucket has remaining capacity. The check has already
    /// recorded the request_at_ms timestamp in each bucket.
    Allowed {
        /// Recorded request timestamp.
        request_at_ms: i64,
        /// Remaining capacity in the most-constrained bucket.
        remaining_min: u32,
    },
    /// Denied because one bucket's window count reached its limit.
    Denied {
        /// Which bucket tripped.
        bucket: BucketKind,
        /// `request_at_ms + window_ms - earliest_window_ms` — caller
        /// can render this as `Retry-After`.
        retry_after_ms: i64,
    },
}

/// Identifies which bucket flagged a deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketKind {
    /// Per-endpoint bucket.
    Endpoint,
    /// Per-IP global bucket.
    Global,
    /// Umbrella aggregate (M8 fix).
    Aggregate,
}

impl BucketKind {
    /// Static name used in metric labels.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Endpoint => "endpoint",
            Self::Global => "global",
            Self::Aggregate => "aggregate",
        }
    }
}

/// Run a sliding-window rate-limit check for `(rpc, ip)` pair against
/// `config`. Records the request timestamp in every bucket on Allow.
///
/// # Errors
/// Returns [`sqlx::Error`] when the underlying transactional query fails.
pub async fn check_rate_limit(
    pool: &Pool,
    rpc: &str,
    ip: &str,
    config: &RateLimitConfig,
) -> Result<RateLimitDecision, sqlx::Error> {
    let now = pellucid_core::now_ms();
    let endpoint_key = format!("rl:ep:{rpc}:{ip}");
    let global_key = format!("rl:ip:{ip}");
    let aggregate_key = format!("rl:agg:{ip}");

    let mut tx = pool.begin().await?;

    // GC all three buckets first — keeps the table from growing
    // without bound and lets count() reflect only the active window.
    for (key, window) in [
        (&endpoint_key, config.endpoint.window_ms),
        (&global_key, config.global.window_ms),
        (&aggregate_key, config.aggregate.window_ms),
    ] {
        sqlx::query(
            "DELETE FROM rate_limit_window WHERE bucket_key = ?1 AND request_at_ms <= ?2",
        )
        .bind(key)
        .bind(now - window)
        .execute(&mut *tx)
        .await?;
    }

    // Read window counts.
    let endpoint_count = window_count(&mut tx, &endpoint_key).await?;
    let global_count = window_count(&mut tx, &global_key).await?;
    let aggregate_count = window_count(&mut tx, &aggregate_key).await?;

    // Check in order: endpoint → global → aggregate. The umbrella
    // aggregate runs last because it's the broadest signal; a deny
    // there means even endpoints that haven't yet exhausted their
    // own caps are still blocked.
    if endpoint_count >= i64::from(config.endpoint.limit) {
        let earliest = earliest_in_window(&mut tx, &endpoint_key).await?;
        tx.rollback().await?;
        return Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Endpoint,
            retry_after_ms: retry_after(earliest, config.endpoint.window_ms, now),
        });
    }
    if global_count >= i64::from(config.global.limit) {
        let earliest = earliest_in_window(&mut tx, &global_key).await?;
        tx.rollback().await?;
        return Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Global,
            retry_after_ms: retry_after(earliest, config.global.window_ms, now),
        });
    }
    if aggregate_count >= i64::from(config.aggregate.limit) {
        let earliest = earliest_in_window(&mut tx, &aggregate_key).await?;
        tx.rollback().await?;
        return Ok(RateLimitDecision::Denied {
            bucket: BucketKind::Aggregate,
            retry_after_ms: retry_after(earliest, config.aggregate.window_ms, now),
        });
    }

    // Allow — record request_at_ms in every bucket. The PK is
    // (bucket_key, request_at_ms); two calls inside the same ms would
    // collide on `now`, so we insert at max(now, prev_max + 1) per
    // bucket to keep timestamps strictly monotonic without changing
    // the GC arithmetic (everything still measured in ms).
    let ep_at = next_request_at(&mut tx, &endpoint_key, now).await?;
    insert_request(&mut tx, &endpoint_key, ep_at).await?;
    let gl_at = next_request_at(&mut tx, &global_key, now).await?;
    insert_request(&mut tx, &global_key, gl_at).await?;
    let ag_at = next_request_at(&mut tx, &aggregate_key, now).await?;
    insert_request(&mut tx, &aggregate_key, ag_at).await?;
    tx.commit().await?;

    let remaining_min = u32::min(
        u32::min(
            saturating_remaining(config.endpoint.limit, endpoint_count + 1),
            saturating_remaining(config.global.limit, global_count + 1),
        ),
        saturating_remaining(config.aggregate.limit, aggregate_count + 1),
    );

    Ok(RateLimitDecision::Allowed {
        request_at_ms: now,
        remaining_min,
    })
}

const fn saturating_remaining(limit: u32, used: i64) -> u32 {
    let used_u = if used < 0 { 0 } else { used as u32 };
    limit.saturating_sub(used_u)
}

const fn retry_after(earliest_ms: i64, window_ms: i64, now: i64) -> i64 {
    let release = earliest_ms.saturating_add(window_ms);
    if release > now { release - now } else { 0 }
}

async fn window_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    bucket_key: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query("SELECT COUNT(*) FROM rate_limit_window WHERE bucket_key = ?1")
        .bind(bucket_key)
        .fetch_one(&mut **tx)
        .await?;
    Ok(row.get(0))
}

async fn earliest_in_window(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    bucket_key: &str,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT MIN(request_at_ms) FROM rate_limit_window WHERE bucket_key = ?1",
    )
    .bind(bucket_key)
    .fetch_one(&mut **tx)
    .await?;
    let v: Option<i64> = row.try_get(0).ok();
    Ok(v.unwrap_or(0))
}

async fn insert_request(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    bucket_key: &str,
    request_at_ms: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO rate_limit_window (bucket_key, request_at_ms) VALUES (?1, ?2)",
    )
    .bind(bucket_key)
    .bind(request_at_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns `max(now, max(request_at_ms) + 1)` for the given bucket so
/// inserts never collide on the (bucket_key, request_at_ms) PK even when
/// two calls land inside the same wall-clock millisecond.
async fn next_request_at(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    bucket_key: &str,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT MAX(request_at_ms) FROM rate_limit_window WHERE bucket_key = ?1",
    )
    .bind(bucket_key)
    .fetch_one(&mut **tx)
    .await?;
    let prev_max: Option<i64> = row.try_get(0).ok();
    Ok(match prev_max {
        Some(m) if m >= now => m.saturating_add(1),
        _ => now,
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    fn small_config(endpoint: u32, global: u32, aggregate: u32) -> RateLimitConfig {
        RateLimitConfig {
            endpoint: BucketConfig { limit: endpoint, window_ms: 60_000 },
            global: BucketConfig { limit: global, window_ms: 60_000 },
            aggregate: BucketConfig { limit: aggregate, window_ms: 60_000 },
        }
    }

    #[tokio::test]
    async fn first_request_is_allowed() {
        let pool = open_in_memory().await.expect("pool");
        let cfg = RateLimitConfig::default();
        let decision = check_rate_limit(&pool, "/api/aviation/v1/get-flight-status", "1.2.3.4", &cfg)
            .await
            .expect("check");
        match decision {
            RateLimitDecision::Allowed { remaining_min, .. } => assert!(remaining_min > 0),
            other => panic!("expected Allowed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn endpoint_bucket_trips_at_exact_cap() {
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(2, 100, 100);
        // Allow → Allow → Deny on third.
        for _ in 0..2 {
            assert!(matches!(
                check_rate_limit(&pool, "rpc", "1.2.3.4", &cfg).await.expect("check"),
                RateLimitDecision::Allowed { .. }
            ));
        }
        let third = check_rate_limit(&pool, "rpc", "1.2.3.4", &cfg).await.expect("check");
        assert_eq!(
            third,
            RateLimitDecision::Denied {
                bucket: BucketKind::Endpoint,
                retry_after_ms: match &third {
                    RateLimitDecision::Denied { retry_after_ms, .. } => *retry_after_ms,
                    _ => unreachable!(),
                }
            }
        );
    }

    #[tokio::test]
    async fn global_bucket_trips_when_endpoint_has_capacity() {
        // Endpoint = 100, global = 2 — second call across endpoints
        // should be the one that trips the global bucket.
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(100, 2, 100);
        check_rate_limit(&pool, "rpc-a", "1.2.3.4", &cfg).await.expect("a1");
        check_rate_limit(&pool, "rpc-b", "1.2.3.4", &cfg).await.expect("b1");
        let third = check_rate_limit(&pool, "rpc-c", "1.2.3.4", &cfg).await.expect("c1");
        match third {
            RateLimitDecision::Denied { bucket, .. } => assert_eq!(bucket, BucketKind::Global),
            other => panic!("expected Denied(Global), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn umbrella_aggregate_trips_before_specific_buckets_exhaust() {
        // M8 FIX REGRESSION TEST. endpoint = 1000, global = 1000, agg = 3.
        // No specific bucket can exhaust at 3 calls, but the aggregate
        // umbrella must trip.
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(1000, 1000, 3);
        check_rate_limit(&pool, "rpc-a", "1.2.3.4", &cfg).await.expect("a1");
        check_rate_limit(&pool, "rpc-b", "1.2.3.4", &cfg).await.expect("b1");
        check_rate_limit(&pool, "rpc-c", "1.2.3.4", &cfg).await.expect("c1");
        let fourth = check_rate_limit(&pool, "rpc-d", "1.2.3.4", &cfg).await.expect("d1");
        match fourth {
            RateLimitDecision::Denied { bucket, .. } => assert_eq!(bucket, BucketKind::Aggregate),
            other => panic!("expected Denied(Aggregate), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn separate_ips_have_independent_buckets() {
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(2, 100, 100);
        check_rate_limit(&pool, "rpc", "1.1.1.1", &cfg).await.expect("a1");
        check_rate_limit(&pool, "rpc", "1.1.1.1", &cfg).await.expect("a2");
        // a's third call would be denied; b should still be allowed.
        let b = check_rate_limit(&pool, "rpc", "2.2.2.2", &cfg).await.expect("b1");
        assert!(matches!(b, RateLimitDecision::Allowed { .. }));
    }

    #[tokio::test]
    async fn separate_endpoints_have_independent_endpoint_buckets() {
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(2, 100, 100);
        check_rate_limit(&pool, "rpc-a", "1.1.1.1", &cfg).await.expect("a1");
        check_rate_limit(&pool, "rpc-a", "1.1.1.1", &cfg).await.expect("a2");
        // rpc-a is now at endpoint cap; rpc-b is independent.
        let b = check_rate_limit(&pool, "rpc-b", "1.1.1.1", &cfg).await.expect("b1");
        assert!(matches!(b, RateLimitDecision::Allowed { .. }));
    }

    #[tokio::test]
    async fn retry_after_is_positive_after_deny() {
        let pool = open_in_memory().await.expect("pool");
        let cfg = small_config(1, 100, 100);
        check_rate_limit(&pool, "rpc", "1.1.1.1", &cfg).await.expect("first");
        let denied = check_rate_limit(&pool, "rpc", "1.1.1.1", &cfg).await.expect("second");
        match denied {
            RateLimitDecision::Denied { retry_after_ms, .. } => {
                assert!(retry_after_ms > 0, "retry_after_ms must be positive");
                assert!(retry_after_ms <= 60_000, "retry_after_ms cannot exceed window");
            }
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[test]
    fn bucket_kind_as_str_round_trips_every_variant() {
        assert_eq!(BucketKind::Endpoint.as_str(), "endpoint");
        assert_eq!(BucketKind::Global.as_str(), "global");
        assert_eq!(BucketKind::Aggregate.as_str(), "aggregate");
    }

    #[test]
    fn default_config_matches_spec_baseline() {
        let cfg = RateLimitConfig::default();
        assert_eq!(cfg.endpoint.limit, 600);
        assert_eq!(cfg.global.limit, 600);
        assert_eq!(cfg.aggregate.limit, 4000);
        assert_eq!(cfg.endpoint.window_ms, 60_000);
    }
}
