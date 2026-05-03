#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
//! Atomic publish integration tests (T3.5).
//!
//! Drives `atomic_publish` against a real in-memory SQLite pool
//! with migrations applied. Headline assertion: **N concurrent
//! publishes for the same domain serialise — exactly one wins
//! at any given instant, every other gets a deterministic
//! `AlreadyPublishing` (or eventually completes once the holder
//! releases)**.

use std::sync::Arc;
use std::time::Duration;

use pellucid_db::open_in_memory;
use pellucid_seeders::{
    atomic_publish, current_holder, PublishError, SeedEnvelope, SeedMeta,
};
use sqlx::Row;

fn envelope(record_count: i64, source_version: &str) -> SeedEnvelope {
    SeedEnvelope {
        seed: SeedMeta {
            fetched_at_ms: pellucid_core::now_ms(),
            ttl_ms: 60_000,
            source_version: source_version.into(),
            record_count,
            cascade_group: Some("integration-test".into()),
            run_id: String::new(),
        },
        data: serde_json::json!({"items": [1, 2, 3]}),
    }
}

#[tokio::test]
async fn single_publisher_writes_canonical_and_meta() {
    let pool = open_in_memory().await.unwrap();
    let outcome = atomic_publish(
        &pool,
        "aviation",
        "aviation:status:v1",
        &envelope(3, "test-v1"),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    assert!(outcome.bytes_written > 0);
    let row = sqlx::query("SELECT COUNT(*) FROM kv_envelope WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>(0), 1);
    let meta = sqlx::query("SELECT COUNT(*) FROM seed_meta WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(meta.get::<i64, _>(0), 1);
}

#[tokio::test]
async fn concurrent_publishers_for_same_domain_serialise() {
    // Spawn 8 concurrent publishers all targeting the same
    // domain. Outcomes:
    //  - At most one is "Acquired then publishes successfully" at
    //    any given instant (lock holds for the duration of the
    //    insert + commit).
    //  - The others either succeed serially after the holder
    //    releases, OR get `AlreadyPublishing` if they raced past
    //    the lease window.
    //  - The total number of canonical writes equals the number
    //    of successful publishes (no half-writes).
    let pool = Arc::new(open_in_memory().await.unwrap());
    const N: usize = 8;
    let mut joins = Vec::with_capacity(N);
    for i in 0..N {
        let pool = pool.clone();
        joins.push(tokio::spawn(async move {
            atomic_publish(
                &pool,
                "aviation",
                "aviation:status:v1",
                &envelope(i as i64, &format!("test-v{i}")),
                Duration::from_secs(60),
            )
            .await
        }));
    }
    let mut successes = 0usize;
    let mut already_publishing = 0usize;
    for j in joins {
        match j.await.unwrap() {
            Ok(_) => successes += 1,
            Err(PublishError::AlreadyPublishing { .. }) => already_publishing += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert!(successes >= 1, "at least one must succeed");
    assert_eq!(
        successes + already_publishing,
        N,
        "every attempt must resolve to either success or AlreadyPublishing",
    );

    // Lock is released after every attempt.
    let holder = current_holder(&pool, "aviation").await.unwrap();
    assert_eq!(holder, None);

    // The canonical row exists (whoever wrote last wins) and the
    // seed_meta row points at one of the run_ids.
    let canonical_count = sqlx::query("SELECT COUNT(*) FROM kv_envelope WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(canonical_count.get::<i64, _>(0), 1);
    let meta_count = sqlx::query("SELECT COUNT(*) FROM seed_meta WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&*pool)
        .await
        .unwrap();
    assert_eq!(meta_count.get::<i64, _>(0), 1);
}

#[tokio::test]
async fn concurrent_publishers_for_different_domains_proceed_in_parallel() {
    // 4 concurrent publishers, each on a different domain → all
    // succeed. (Lock partitioning is per-domain.)
    let pool = Arc::new(open_in_memory().await.unwrap());
    let domains = ["aviation", "maritime", "news", "cyber"];
    let mut joins = Vec::with_capacity(domains.len());
    for d in domains {
        let pool = pool.clone();
        joins.push(tokio::spawn(async move {
            atomic_publish(
                &pool,
                d,
                &format!("{d}:status:v1"),
                &envelope(1, "v1"),
                Duration::from_secs(60),
            )
            .await
        }));
    }
    for j in joins {
        let outcome = j.await.unwrap().unwrap();
        assert!(outcome.bytes_written > 0);
    }
    // Every domain wrote one canonical row.
    let total: i64 = sqlx::query("SELECT COUNT(*) FROM kv_envelope")
        .fetch_one(&*pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(total, domains.len() as i64);
}

#[tokio::test]
async fn staging_row_is_cleaned_up_after_promotion() {
    let pool = open_in_memory().await.unwrap();
    let _ = atomic_publish(
        &pool,
        "aviation",
        "aviation:status:v1",
        &envelope(1, "v1"),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    // After successful publish, no staging rows for this key.
    let staging_count: i64 = sqlx::query(
        "SELECT COUNT(*) FROM kv_envelope WHERE cache_key LIKE 'aviation:status:v1:staging:%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .get(0);
    assert_eq!(staging_count, 0);
}

#[tokio::test]
async fn validation_failure_does_not_write_canonical_or_meta() {
    let pool = open_in_memory().await.unwrap();
    let mut env = envelope(1, "v1");
    env.seed.ttl_ms = 0; // invalid
    let err = atomic_publish(
        &pool,
        "aviation",
        "aviation:status:v1",
        &env,
        Duration::from_secs(60),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, PublishError::Validation(_)));
    // No canonical row.
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM kv_envelope WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    // No seed_meta row.
    let meta_count: i64 = sqlx::query("SELECT COUNT(*) FROM seed_meta WHERE cache_key = ?")
        .bind("aviation:status:v1")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(meta_count, 0);
    // Lock released.
    let holder = current_holder(&pool, "aviation").await.unwrap();
    assert_eq!(holder, None);
}

#[tokio::test]
async fn cascade_group_persisted_to_seed_meta() {
    let pool = open_in_memory().await.unwrap();
    let mut env = envelope(1, "v1");
    env.seed.cascade_group = Some("theater-posture".into());
    let _ = atomic_publish(
        &pool,
        "military",
        "military:posture:v1",
        &env,
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    let row = sqlx::query("SELECT cascade_group FROM seed_meta WHERE cache_key = ?")
        .bind("military:posture:v1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let cg: String = row.get(0);
    assert_eq!(cg, "theater-posture");
}
