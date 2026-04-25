//! Integration tests for pellucid-db's pool + migration runner.
//!
//! Tests open a real in-memory SQLite database, apply migrations, and
//! exercise every table and virtual table mandated by SPEC-001 §6 to
//! prove the schema works end-to-end. Re-running migrations against
//! the populated database is a no-op (idempotency).

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use pellucid_db::{migrate, open_in_memory};
use sqlx::Row;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_database_has_every_required_table() {
    let pool = open_in_memory().await.expect("open");
    let required = [
        "kv_envelope",
        "seed_meta",
        "seed_lock",
        "entitlements_cache",
        "rate_limit_window",
        "panel_layout",
        "webhook_seen",
        "pellucid_migrations",
        "positions_meta",
        "embedding_meta",
    ];
    for name in required {
        let row =
            sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1")
                .bind(name)
                .fetch_optional(&pool)
                .await
                .expect("query sqlite_master");
        assert!(row.is_some(), "table {name} missing");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_database_has_required_virtual_tables() {
    let pool = open_in_memory().await.expect("open");
    let required = ["news_fts", "positions_rtree"];
    for name in required {
        let row =
            sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1")
                .bind(name)
                .fetch_optional(&pool)
                .await
                .expect("query");
        assert!(row.is_some(), "virtual table {name} missing");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn migration_is_idempotent() {
    let pool = open_in_memory().await.expect("open");
    // open_in_memory already applied migrations; doing it again must be safe.
    migrate(&pool).await.expect("re-apply migrations");
    migrate(&pool).await.expect("re-apply migrations twice");

    // The pellucid_migrations row count should still be 1 (INSERT OR IGNORE).
    let row = sqlx::query("SELECT COUNT(*) FROM pellucid_migrations")
        .fetch_one(&pool)
        .await
        .expect("count");
    let count: i64 = row.get(0);
    assert_eq!(count, 1, "expected exactly one migration row, got {count}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kv_envelope_round_trip() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query(
        "INSERT INTO kv_envelope (cache_key, payload, fetched_at_ms, ttl_ms, record_count, source_version, state, is_negative)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind("aviation:status:AA100:2026-04-25:JFK:v1")
    .bind(r#"{"_seed":{"fetchedAt":1726764000000},"data":{}}"#)
    .bind(1_726_764_000_000_i64)
    .bind(300_000_i64)
    .bind(0_i64)
    .bind(Option::<&str>::None)
    .bind("live")
    .bind(0_i64)
    .execute(&pool)
    .await
    .expect("insert");

    let row = sqlx::query("SELECT cache_key, state, ttl_ms FROM kv_envelope WHERE cache_key = ?1")
        .bind("aviation:status:AA100:2026-04-25:JFK:v1")
        .fetch_one(&pool)
        .await
        .expect("select");
    let key: String = row.get(0);
    let state: String = row.get(1);
    let ttl: i64 = row.get(2);
    assert!(key.starts_with("aviation:status:"));
    assert_eq!(state, "live");
    assert_eq!(ttl, 300_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_limit_window_composite_pk_enforced() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query("INSERT INTO rate_limit_window (bucket_key, request_at_ms) VALUES (?1, ?2)")
        .bind("rl:ip:1.2.3.4")
        .bind(1000_i64)
        .execute(&pool)
        .await
        .expect("insert");

    // Same key + same timestamp must be rejected by the composite PK.
    let dup = sqlx::query("INSERT INTO rate_limit_window (bucket_key, request_at_ms) VALUES (?1, ?2)")
        .bind("rl:ip:1.2.3.4")
        .bind(1000_i64)
        .execute(&pool)
        .await;
    assert!(dup.is_err(), "expected unique-violation on duplicate row");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn news_fts_match_returns_inserted_rows() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query("INSERT INTO news_fts (article_id, title, summary, body, source, lang) VALUES (?, ?, ?, ?, ?, ?)")
        .bind("art-001")
        .bind("Pellucid launch")
        .bind("Pellucid replaces WorldMonitor")
        .bind("Today, the Pellucid project shipped a new SQLite-backed cache.")
        .bind("internal")
        .bind("en")
        .execute(&pool)
        .await
        .expect("insert fts");

    let row = sqlx::query("SELECT article_id FROM news_fts WHERE news_fts MATCH 'pellucid'")
        .fetch_one(&pool)
        .await
        .expect("fts5 match");
    let id: String = row.get(0);
    assert_eq!(id, "art-001");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn positions_rtree_box_query_returns_inserted_id() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query("INSERT INTO positions_rtree (id, min_lon, max_lon, min_lat, max_lat) VALUES (?, ?, ?, ?, ?)")
        .bind(1_i64)
        .bind(34.0_f64)
        .bind(34.0_f64)
        .bind(-118.0_f64)
        .bind(-118.0_f64)
        .execute(&pool)
        .await
        .expect("insert rtree");

    let row = sqlx::query(
        "SELECT id FROM positions_rtree
         WHERE min_lon BETWEEN ?1 AND ?2 AND min_lat BETWEEN ?3 AND ?4",
    )
    .bind(33.0_f64)
    .bind(35.0_f64)
    .bind(-119.0_f64)
    .bind(-117.0_f64)
    .fetch_one(&pool)
    .await
    .expect("rtree query");
    let id: i64 = row.get(0);
    assert_eq!(id, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn entitlements_cache_upsert_round_trip() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query(
        "INSERT INTO entitlements_cache (user_id, tier, features_json, valid_until_ms, cached_at_ms)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
             tier = excluded.tier,
             features_json = excluded.features_json,
             valid_until_ms = excluded.valid_until_ms,
             cached_at_ms = excluded.cached_at_ms",
    )
    .bind("user_test_123")
    .bind(1_i64)
    .bind(r#"{"tier":1,"maxDashboards":5}"#)
    .bind(9_999_999_999_999_i64)
    .bind(1_726_764_000_000_i64)
    .execute(&pool)
    .await
    .expect("upsert");

    let row = sqlx::query("SELECT tier FROM entitlements_cache WHERE user_id = ?1")
        .bind("user_test_123")
        .fetch_one(&pool)
        .await
        .expect("select");
    let tier: i64 = row.get(0);
    assert_eq!(tier, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seed_lock_enforces_uniqueness_per_domain() {
    let pool = open_in_memory().await.expect("open");
    sqlx::query("INSERT INTO seed_lock (domain, run_id, expires_at_ms) VALUES (?1, ?2, ?3)")
        .bind("aviation")
        .bind("run-1")
        .bind(1000_i64)
        .execute(&pool)
        .await
        .expect("first insert");

    let dup = sqlx::query("INSERT INTO seed_lock (domain, run_id, expires_at_ms) VALUES (?1, ?2, ?3)")
        .bind("aviation")
        .bind("run-2")
        .bind(2000_i64)
        .execute(&pool)
        .await;
    assert!(dup.is_err(), "second insert for same domain must conflict");
}
