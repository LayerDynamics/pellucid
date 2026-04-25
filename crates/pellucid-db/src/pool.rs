//! SQLite connection pool with Pellucid's mandated PRAGMA configuration.
//!
//! SPEC-001 §6 lists the seven PRAGMAs that every Pellucid SQLite
//! database must apply at open time. They are enforced here on every
//! connection acquired from the pool via [`SqliteConnectOptions::pragma`]
//! so each pooled connection ends up with the same settings — including
//! short-lived ones spawned for transactions inside [`crate::migrate`].

use std::path::Path;
use std::str::FromStr;

use sqlx::ConnectOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::Result;
use crate::migrate;

/// Re-exported alias for the workspace-wide sqlx Pool type so consumers
/// only need to import from pellucid_db.
pub type Pool = sqlx::SqlitePool;

/// Canonical PRAGMA list from SPEC-001 §6. Returned as a slice so tests
/// and diagnostic tooling can walk it without re-deriving the spec.
#[must_use]
pub const fn pragmas() -> &'static [(&'static str, &'static str)] {
    &[
        ("journal_mode", "WAL"),
        ("synchronous", "NORMAL"),
        ("busy_timeout", "5000"),
        ("temp_store", "MEMORY"),
        ("mmap_size", "268435456"),
        ("foreign_keys", "ON"),
        ("cache_size", "-65536"),
    ]
}

/// Tunables for [`open`]. Most consumers stick to [`SqliteOpenOptions::file`]
/// or [`SqliteOpenOptions::memory`]; the explicit struct makes future
/// additions (read-only mode, custom busy handler) backward-compatible.
#[derive(Debug, Clone)]
pub struct SqliteOpenOptions {
    /// Database location. `:memory:` for in-memory, otherwise a filesystem path.
    pub url: String,
    /// Maximum pooled connections.
    pub max_connections: u32,
    /// Run migrations after the pool is built.
    pub run_migrations: bool,
}

impl SqliteOpenOptions {
    /// Open a file-backed database at `path`, creating the file (and any
    /// missing parent directories) when needed.
    #[must_use]
    pub fn file(path: impl AsRef<Path>) -> Self {
        Self {
            url: format!("sqlite://{}?mode=rwc", path.as_ref().display()),
            max_connections: 8,
            run_migrations: true,
        }
    }

    /// Open an in-memory database. Pool size is forced to 1 because every
    /// connection to `:memory:` is a *separate* database.
    #[must_use]
    pub fn memory() -> Self {
        Self {
            url: "sqlite::memory:".to_string(),
            max_connections: 1,
            run_migrations: true,
        }
    }
}

/// Open a SQLite pool with the canonical PRAGMA set.
///
/// # Errors
/// Returns [`DbError::Sqlx`](crate::DbError::Sqlx) if sqlx fails to open
/// the database, or [`DbError::Migration`](crate::DbError::Migration) if
/// `run_migrations` is true and a migration step fails.
pub async fn open(opts: SqliteOpenOptions) -> Result<Pool> {
    let mut connect = SqliteConnectOptions::from_str(&opts.url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_millis(5000))
        .foreign_keys(true);

    for (name, value) in pragmas() {
        connect = connect.pragma(*name, *value);
    }

    let connect = connect.disable_statement_logging();

    let pool = SqlitePoolOptions::new()
        .max_connections(opts.max_connections)
        .connect_with(connect)
        .await?;

    if opts.run_migrations {
        migrate(&pool).await?;
    }

    tracing::debug!(
        url = %opts.url,
        max_connections = opts.max_connections,
        "pellucid-db pool opened",
    );

    Ok(pool)
}

/// Convenience: open an in-memory database with migrations applied.
///
/// # Errors
/// Returns the same error variants as [`open`].
pub async fn open_in_memory() -> Result<Pool> {
    open(SqliteOpenOptions::memory()).await
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use sqlx::Row;

    use super::*;

    #[test]
    fn pragmas_lists_every_required_setting() {
        let names: Vec<&str> = pragmas().iter().map(|(n, _)| *n).collect();
        for required in [
            "journal_mode",
            "synchronous",
            "busy_timeout",
            "temp_store",
            "mmap_size",
            "foreign_keys",
            "cache_size",
        ] {
            assert!(names.contains(&required), "pragma {required} missing");
        }
    }

    #[test]
    fn pragmas_journal_mode_is_wal() {
        let map: std::collections::HashMap<_, _> = pragmas().iter().copied().collect();
        assert_eq!(map.get("journal_mode"), Some(&"WAL"));
        assert_eq!(map.get("synchronous"), Some(&"NORMAL"));
        assert_eq!(map.get("busy_timeout"), Some(&"5000"));
        assert_eq!(map.get("foreign_keys"), Some(&"ON"));
        assert_eq!(map.get("mmap_size"), Some(&"268435456"));
        assert_eq!(map.get("cache_size"), Some(&"-65536"));
        assert_eq!(map.get("temp_store"), Some(&"MEMORY"));
    }

    #[test]
    fn options_file_builds_rwc_url() {
        let o = SqliteOpenOptions::file("/tmp/pellucid.db");
        assert!(o.url.starts_with("sqlite://"));
        assert!(o.url.ends_with("?mode=rwc"));
        assert_eq!(o.max_connections, 8);
        assert!(o.run_migrations);
    }

    #[test]
    fn options_memory_forces_single_connection() {
        let o = SqliteOpenOptions::memory();
        assert_eq!(o.url, "sqlite::memory:");
        assert_eq!(o.max_connections, 1);
        assert!(o.run_migrations);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn open_in_memory_applies_pragmas() {
        let pool = open_in_memory().await.expect("open");
        // foreign_keys returns 1 when ON. journal_mode returns "memory"
        // for in-memory databases regardless of the requested WAL flag,
        // so we only assert foreign_keys + busy_timeout here.
        let row = sqlx::query("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("query");
        let fk: i64 = row.get(0);
        assert_eq!(fk, 1, "foreign_keys must be enabled");

        let row = sqlx::query("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .expect("query");
        let bt: i64 = row.get(0);
        assert_eq!(bt, 5000);
    }
}
