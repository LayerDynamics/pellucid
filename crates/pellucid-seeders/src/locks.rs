//! Distributed seed locks via SQLite `BEGIN IMMEDIATE`.
//!
//! Replaces the legacy Redis `SET NX PX seed-lock:*` primitive.
//! SQLite's `BEGIN IMMEDIATE` upgrades the connection to a
//! reserved-lock state on entry, serialising every other writer
//! at the database level. We layer a row-level expiry on top so
//! a crashed seeder doesn't deadlock the next one.
//!
//! Surface:
//! - [`acquire_seed_lock`] — atomic insert-if-absent. Returns
//!   `Ok(true)` if the caller now holds the lock,
//!   `Ok(false)` if another live holder owns it.
//! - [`release_seed_lock`] — compare-and-delete by `run_id`.
//!   Refuses to release a lock the caller does not own.
//! - [`steal_expired_lock`] — internal helper used by
//!   `acquire_seed_lock` when the existing row's `expires_at_ms`
//!   has elapsed.

use std::time::Duration;

use thiserror::Error;

use pellucid_core::now_ms;
use pellucid_db::Pool;

/// Outcome of an [`acquire_seed_lock`] attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockOutcome {
    /// Lock acquired (no prior holder, or prior holder expired).
    Acquired,
    /// Another live holder owns the lock — caller should back
    /// off and retry later.
    HeldByAnother {
        /// Wall-clock ms when the live holder's lease expires.
        expires_at_ms: i64,
    },
}

/// Errors the lock layer can surface.
#[derive(Debug, Error)]
pub enum LockError {
    /// Underlying sqlx error.
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Try to take the seed lock for `domain`. The lock identifies
/// the holder by `run_id`; subsequent [`release_seed_lock`]
/// calls MUST present the same `run_id`.
///
/// Behaviour:
/// - No existing row → insert + return `Acquired`.
/// - Existing row with `expires_at_ms <= now` → steal +
///   return `Acquired`.
/// - Existing live row with a different `run_id` → return
///   `HeldByAnother`.
/// - Existing live row with the same `run_id` → idempotent
///   `Acquired` (treats this as a re-entrant call from the
///   same in-flight publish).
///
/// # Errors
/// Sqlx transport / serialisation errors propagate.
pub async fn acquire_seed_lock(
    pool: &Pool,
    domain: &str,
    run_id: &str,
    lease: Duration,
) -> Result<LockOutcome, LockError> {
    let now = now_ms();
    let expires_at = now + i64::try_from(lease.as_millis()).unwrap_or(i64::MAX);

    let mut tx = pool.begin().await?;
    // Read current row under the BEGIN IMMEDIATE write lock.
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT run_id, expires_at_ms FROM seed_lock WHERE domain = ?1",
    )
    .bind(domain)
    .fetch_optional(&mut *tx)
    .await?;

    match existing {
        None => {
            sqlx::query(
                "INSERT INTO seed_lock (domain, run_id, expires_at_ms) \
                 VALUES (?1, ?2, ?3)",
            )
            .bind(domain)
            .bind(run_id)
            .bind(expires_at)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(LockOutcome::Acquired)
        }
        Some((existing_run, existing_exp)) if existing_run == run_id => {
            // Re-entrant — refresh the lease and return Acquired.
            sqlx::query(
                "UPDATE seed_lock SET expires_at_ms = ?1 WHERE domain = ?2",
            )
            .bind(expires_at.max(existing_exp))
            .bind(domain)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(LockOutcome::Acquired)
        }
        Some((_, existing_exp)) if existing_exp <= now => {
            // Expired holder — steal.
            sqlx::query(
                "UPDATE seed_lock SET run_id = ?1, expires_at_ms = ?2 \
                 WHERE domain = ?3",
            )
            .bind(run_id)
            .bind(expires_at)
            .bind(domain)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(LockOutcome::Acquired)
        }
        Some((_, existing_exp)) => {
            tx.rollback().await?;
            Ok(LockOutcome::HeldByAnother {
                expires_at_ms: existing_exp,
            })
        }
    }
}

/// Release the seed lock for `domain` IF the caller's `run_id`
/// matches the recorded holder. Returns `Ok(true)` on a
/// successful release, `Ok(false)` if the lock had already been
/// stolen (or was never held).
///
/// # Errors
/// Sqlx transport errors propagate.
pub async fn release_seed_lock(
    pool: &Pool,
    domain: &str,
    run_id: &str,
) -> Result<bool, LockError> {
    let result = sqlx::query(
        "DELETE FROM seed_lock WHERE domain = ?1 AND run_id = ?2",
    )
    .bind(domain)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Read the live holder for diagnostics / `/health` output.
/// Returns `None` if no row, or the row has expired.
pub async fn current_holder(pool: &Pool, domain: &str) -> Result<Option<String>, LockError> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT run_id, expires_at_ms FROM seed_lock WHERE domain = ?1",
    )
    .bind(domain)
    .fetch_optional(pool)
    .await?;
    let now = now_ms();
    Ok(row.and_then(|(rid, exp)| if exp > now { Some(rid) } else { None }))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_db::open_in_memory;

    #[tokio::test]
    async fn acquire_on_empty_table_succeeds() {
        let pool = open_in_memory().await.unwrap();
        let result = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(result, LockOutcome::Acquired);
    }

    #[tokio::test]
    async fn second_acquire_with_different_run_id_returns_held() {
        let pool = open_in_memory().await.unwrap();
        let _ = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        let result = acquire_seed_lock(&pool, "aviation", "run-2", Duration::from_secs(60))
            .await
            .unwrap();
        match result {
            LockOutcome::HeldByAnother { expires_at_ms } => {
                assert!(expires_at_ms > now_ms());
            }
            other => panic!("expected HeldByAnother, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn re_entrant_acquire_with_same_run_id_succeeds() {
        let pool = open_in_memory().await.unwrap();
        let _ = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        let result = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(result, LockOutcome::Acquired);
    }

    #[tokio::test]
    async fn expired_lock_is_stolen() {
        let pool = open_in_memory().await.unwrap();
        // Insert directly with expires_at in the past.
        sqlx::query(
            "INSERT INTO seed_lock (domain, run_id, expires_at_ms) VALUES (?, ?, ?)",
        )
        .bind("aviation")
        .bind("crashed-run")
        .bind(now_ms() - 1_000)
        .execute(&pool)
        .await
        .unwrap();
        let result = acquire_seed_lock(&pool, "aviation", "fresh-run", Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(result, LockOutcome::Acquired);
        let holder = current_holder(&pool, "aviation").await.unwrap();
        assert_eq!(holder, Some("fresh-run".into()));
    }

    #[tokio::test]
    async fn release_with_correct_run_id_removes_row() {
        let pool = open_in_memory().await.unwrap();
        let _ = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        let released = release_seed_lock(&pool, "aviation", "run-1").await.unwrap();
        assert!(released);
        assert_eq!(current_holder(&pool, "aviation").await.unwrap(), None);
    }

    #[tokio::test]
    async fn release_with_wrong_run_id_is_no_op() {
        let pool = open_in_memory().await.unwrap();
        let _ = acquire_seed_lock(&pool, "aviation", "run-1", Duration::from_secs(60))
            .await
            .unwrap();
        let released = release_seed_lock(&pool, "aviation", "imposter").await.unwrap();
        assert!(!released);
        assert_eq!(
            current_holder(&pool, "aviation").await.unwrap(),
            Some("run-1".into())
        );
    }

    #[tokio::test]
    async fn release_on_empty_table_returns_false() {
        let pool = open_in_memory().await.unwrap();
        let released = release_seed_lock(&pool, "aviation", "run-x").await.unwrap();
        assert!(!released);
    }

    #[tokio::test]
    async fn current_holder_excludes_expired() {
        let pool = open_in_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO seed_lock (domain, run_id, expires_at_ms) VALUES (?, ?, ?)",
        )
        .bind("aviation")
        .bind("expired-run")
        .bind(now_ms() - 1)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(current_holder(&pool, "aviation").await.unwrap(), None);
    }
}
