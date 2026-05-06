//! Atomic seed publish — the staging → canonical → seed_meta
//! flow per SPEC-001 §7.4.
//!
//! Five steps, each isolated for unit-testability:
//!
//! 1. **Acquire seed lock** — SPEC §7.4 step 1. Returns
//!    `PublishError::AlreadyPublishing` if a different holder
//!    is live.
//! 2. **Validate envelope** — SPEC §7.4 step 2. Shape +
//!    5 MiB cap.
//! 3. **Stage** — SPEC §7.4 step 3. Write to
//!    `kv_envelope` under `<key>:staging:<run_id>` with a
//!    5-minute TTL.
//! 4. **Promote** — SPEC §7.4 step 4. In one transaction:
//!    write canonical row + delete staging row + write
//!    `seed_meta`.
//! 5. **Release lock** — SPEC §7.4 step 5. Compare-and-del.
//!
//! On any failure between steps 1 and 5, the lock is released
//! and the caller sees a typed error. The staging row is
//! short-lived (5 min) so a crashed publish self-heals.

use std::time::Duration;

use thiserror::Error;
use uuid::Uuid;

use pellucid_core::now_ms;
use pellucid_db::Pool;

use crate::envelope::{EnvelopeError, SeedEnvelope};
use crate::locks::{acquire_seed_lock, release_seed_lock, LockError, LockOutcome};

/// Default lock lease — SPEC-001 §7.4 sets this at 60 s.
pub const DEFAULT_LOCK_LEASE: Duration = Duration::from_secs(60);

/// Staging-row TTL — SPEC-001 §7.4 step 3 pins 5 minutes.
pub const STAGING_TTL_MS: i64 = 5 * 60 * 1000;

/// Floor on the `seed_meta` TTL — at least 7 days so the
/// `/health` cascade has a meaningful "last seen this seed"
/// floor even if the underlying cache TTL is short-lived.
pub const SEED_META_MIN_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// Errors `atomic_publish` can produce.
#[derive(Debug, Error)]
pub enum PublishError {
    /// Lock held by another live publisher. Caller should retry
    /// after `expires_at_ms`.
    #[error("seed lock for domain {domain} held by another publisher until {expires_at_ms} ms")]
    AlreadyPublishing {
        /// Domain the caller tried to publish into.
        domain: String,
        /// When the holder's lease elapses.
        expires_at_ms: i64,
    },
    /// Envelope validation failed.
    #[error("envelope validation: {0}")]
    Validation(#[from] EnvelopeError),
    /// Lock layer failed.
    #[error("lock: {0}")]
    Lock(#[from] LockError),
    /// SQLite write failed.
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Outcome of a successful publish — useful for tests +
/// `/health` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishOutcome {
    /// `run_id` the publisher was assigned.
    pub run_id: String,
    /// Number of bytes written to the canonical row.
    pub bytes_written: usize,
    /// Wall-clock ms at canonical write.
    pub published_at_ms: i64,
}

/// Atomically publish `envelope` under `cache_key` for `domain`.
/// `cache_key` is the key the cache layer reads on bootstrap;
/// `domain` is the lock partition (typically the `<domain>` part
/// of `<domain>:<resource>:v1`).
///
/// `ttl` is the canonical row's TTL; the `seed_meta` row uses
/// `max(ttl, 7d)` so the `/health` cascade has a meaningful
/// floor.
///
/// # Errors
/// See [`PublishError`].
pub async fn atomic_publish(
    pool: &Pool,
    domain: &str,
    cache_key: &str,
    envelope: &SeedEnvelope<serde_json::Value>,
    ttl: Duration,
) -> Result<PublishOutcome, PublishError> {
    let run_id = Uuid::new_v4().to_string();

    // Step 1 — acquire lock.
    let outcome = acquire_seed_lock(pool, domain, &run_id, DEFAULT_LOCK_LEASE).await?;
    match outcome {
        LockOutcome::Acquired => {}
        LockOutcome::HeldByAnother { expires_at_ms } => {
            return Err(PublishError::AlreadyPublishing {
                domain: domain.to_string(),
                expires_at_ms,
            });
        }
    }

    // Steps 2-5 wrapped so we always release the lock on the way
    // out — including failure paths.
    let result = atomic_publish_inner(pool, cache_key, envelope, ttl, &run_id).await;
    let _ = release_seed_lock(pool, domain, &run_id).await;
    result
}

async fn atomic_publish_inner(
    pool: &Pool,
    cache_key: &str,
    envelope: &SeedEnvelope<serde_json::Value>,
    ttl: Duration,
    run_id: &str,
) -> Result<PublishOutcome, PublishError> {
    // Step 2 — validate + encode (one pass).
    let payload = envelope.validate_and_encode()?;
    let bytes_written = payload.len();
    let now = now_ms();
    let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
    let staging_key = format!("{cache_key}:staging:{run_id}");

    // Step 3 — staging row (5 min TTL).
    insert_envelope(
        pool,
        &staging_key,
        &payload,
        now,
        STAGING_TTL_MS,
        envelope.seed.record_count,
        &envelope.seed.source_version,
        "live",
    )
    .await?;

    // Step 4 — promote in one transaction.
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT OR REPLACE INTO kv_envelope \
         (cache_key, payload, fetched_at_ms, ttl_ms, record_count, \
          source_version, state, is_negative) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
    )
    .bind(cache_key)
    .bind(&payload)
    .bind(now)
    .bind(ttl_ms)
    .bind(envelope.seed.record_count)
    .bind(&envelope.seed.source_version)
    .bind("live")
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM kv_envelope WHERE cache_key = ?1")
        .bind(&staging_key)
        .execute(&mut *tx)
        .await?;

    let meta_ttl_ms = ttl_ms.max(SEED_META_MIN_TTL_MS);
    sqlx::query(
        "INSERT OR REPLACE INTO seed_meta \
         (cache_key, fetched_at_ms, ttl_ms, last_run_id, source_version, \
          record_count, cascade_group) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(cache_key)
    .bind(now)
    .bind(meta_ttl_ms)
    .bind(run_id)
    .bind(&envelope.seed.source_version)
    .bind(envelope.seed.record_count)
    .bind(envelope.seed.cascade_group.as_deref())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(PublishOutcome {
        run_id: run_id.to_string(),
        bytes_written,
        published_at_ms: now,
    })
}

#[allow(clippy::too_many_arguments)]
async fn insert_envelope(
    pool: &Pool,
    cache_key: &str,
    payload: &str,
    fetched_at_ms: i64,
    ttl_ms: i64,
    record_count: i64,
    source_version: &str,
    state: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR REPLACE INTO kv_envelope \
         (cache_key, payload, fetched_at_ms, ttl_ms, record_count, \
          source_version, state, is_negative) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
    )
    .bind(cache_key)
    .bind(payload)
    .bind(fetched_at_ms)
    .bind(ttl_ms)
    .bind(record_count)
    .bind(source_version)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::envelope::SeedMeta;
    use pellucid_db::open_in_memory;
    use sqlx::Row;

    fn envelope() -> SeedEnvelope {
        SeedEnvelope {
            seed: SeedMeta {
                fetched_at_ms: now_ms(),
                ttl_ms: 60_000,
                source_version: "test-v1".into(),
                record_count: 5,
                cascade_group: Some("aviation".into()),
                run_id: String::new(),
            },
            data: serde_json::json!({"items": [1, 2, 3, 4, 5]}),
        }
    }

    async fn count_kv(pool: &Pool, key: &str) -> i64 {
        let row = sqlx::query("SELECT COUNT(*) FROM kv_envelope WHERE cache_key = ?")
            .bind(key)
            .fetch_one(pool)
            .await
            .unwrap();
        row.get::<i64, _>(0)
    }

    #[tokio::test]
    async fn publish_writes_canonical_row_and_seed_meta() {
        let pool = open_in_memory().await.unwrap();
        let env = envelope();
        let outcome = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert!(outcome.bytes_written > 0);

        // Canonical row exists.
        assert_eq!(count_kv(&pool, "aviation:foo:v1").await, 1);
        // No staging row remains.
        let any_staging: i64 = sqlx::query(
            "SELECT COUNT(*) FROM kv_envelope WHERE cache_key LIKE 'aviation:foo:v1:staging:%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
        assert_eq!(any_staging, 0);

        // seed_meta row exists with the right run_id + cascade group.
        let row =
            sqlx::query("SELECT last_run_id, cascade_group FROM seed_meta WHERE cache_key = ?")
                .bind("aviation:foo:v1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let last_run: String = row.get(0);
        assert_eq!(last_run, outcome.run_id);
        let cascade: String = row.get(1);
        assert_eq!(cascade, "aviation");

        // Lock released.
        let holder = crate::locks::current_holder(&pool, "aviation")
            .await
            .unwrap();
        assert_eq!(holder, None);
    }

    #[tokio::test]
    async fn publish_validation_failure_releases_lock() {
        let pool = open_in_memory().await.unwrap();
        let mut env = envelope();
        env.seed.ttl_ms = 0; // invalid
        let err = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env,
            Duration::from_secs(60),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PublishError::Validation(_)));
        // Lock must be released even on failure.
        let holder = crate::locks::current_holder(&pool, "aviation")
            .await
            .unwrap();
        assert_eq!(holder, None);
    }

    #[tokio::test]
    async fn publish_when_lock_held_returns_already_publishing() {
        let pool = open_in_memory().await.unwrap();
        // Pre-acquire lock under a different run_id.
        let other_outcome = crate::locks::acquire_seed_lock(
            &pool,
            "aviation",
            "other-run",
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert_eq!(other_outcome, crate::locks::LockOutcome::Acquired);

        let env = envelope();
        let err = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env,
            Duration::from_secs(60),
        )
        .await
        .unwrap_err();
        match err {
            PublishError::AlreadyPublishing {
                domain,
                expires_at_ms,
            } => {
                assert_eq!(domain, "aviation");
                assert!(expires_at_ms > now_ms());
            }
            other => panic!("expected AlreadyPublishing, got {other:?}"),
        }

        // Other holder still owns the lock.
        let holder = crate::locks::current_holder(&pool, "aviation")
            .await
            .unwrap();
        assert_eq!(holder, Some("other-run".into()));
    }

    #[tokio::test]
    async fn publish_on_oversized_envelope_rejected() {
        let pool = open_in_memory().await.unwrap();
        let mut env = envelope();
        // Force payload over the 5 MiB cap.
        env.data = serde_json::json!({
            "blob": "x".repeat(crate::envelope::MAX_ENVELOPE_BYTES)
        });
        let err = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env,
            Duration::from_secs(60),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            PublishError::Validation(EnvelopeError::SizeExceeded { .. })
        ));
    }

    #[tokio::test]
    async fn publish_overwrites_previous_canonical_row() {
        let pool = open_in_memory().await.unwrap();
        let env_v1 = envelope();
        let _ = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env_v1,
            Duration::from_secs(60),
        )
        .await
        .unwrap();

        let mut env_v2 = envelope();
        env_v2.seed.source_version = "test-v2".into();
        env_v2.data = serde_json::json!({"items": [10]});
        let _ = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env_v2,
            Duration::from_secs(60),
        )
        .await
        .unwrap();

        let row = sqlx::query("SELECT source_version FROM seed_meta WHERE cache_key = ?")
            .bind("aviation:foo:v1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let sv: String = row.get(0);
        assert_eq!(sv, "test-v2");
    }

    #[tokio::test]
    async fn seed_meta_ttl_floors_at_seven_days() {
        let pool = open_in_memory().await.unwrap();
        let env = envelope();
        // Short cache TTL (5 min) — meta TTL should still be ≥ 7 d.
        let _ = atomic_publish(
            &pool,
            "aviation",
            "aviation:foo:v1",
            &env,
            Duration::from_secs(300),
        )
        .await
        .unwrap();
        let row = sqlx::query("SELECT ttl_ms FROM seed_meta WHERE cache_key = ?")
            .bind("aviation:foo:v1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let meta_ttl: i64 = row.get(0);
        assert_eq!(meta_ttl, SEED_META_MIN_TTL_MS);
    }
}
