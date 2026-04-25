//! Error type local to pellucid-db. Wraps sqlx + io + custom failure
//! cases so callers can pattern-match without pulling in sqlx's full
//! error vocabulary.

use thiserror::Error;

/// Result alias for pellucid-db operations.
pub type Result<T> = core::result::Result<T, DbError>;

/// Failure modes when opening or migrating a Pellucid SQLite database.
#[derive(Debug, Error)]
pub enum DbError {
    /// Underlying sqlx error (connect, query, migration).
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// Migration script could not be parsed or executed.
    #[error("migration {name} failed: {message}")]
    Migration {
        /// Migration filename (e.g. `0001_initial.sql`).
        name: String,
        /// Underlying error message.
        message: String,
    },

    /// PRAGMA enforcement saw an unexpected value (database opened
    /// elsewhere with conflicting settings).
    #[error("pragma {name} = {actual}, expected {expected}")]
    PragmaMismatch {
        /// PRAGMA name.
        name: &'static str,
        /// Observed value.
        actual: String,
        /// Required value.
        expected: &'static str,
    },

    /// Embedding/vec extension not available — the consumer asked for
    /// it but the runtime cannot load the extension. Non-fatal: pool
    /// creation continues without the extension.
    #[error("sqlite-vec extension not loadable: {0}")]
    VecExtensionUnavailable(String),

    /// I/O failure (database path doesn't exist, can't create parent dir).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn migration_variant_renders_name_and_message() {
        let e = DbError::Migration {
            name: "0001_initial".into(),
            message: "table missing".into(),
        };
        let s = e.to_string();
        assert!(s.contains("0001_initial"));
        assert!(s.contains("table missing"));
    }

    #[test]
    fn pragma_mismatch_renders_all_three_fields() {
        let e = DbError::PragmaMismatch {
            name: "journal_mode",
            actual: "DELETE".into(),
            expected: "WAL",
        };
        let s = e.to_string();
        assert!(s.contains("journal_mode"));
        assert!(s.contains("DELETE"));
        assert!(s.contains("WAL"));
    }

    #[test]
    fn vec_extension_unavailable_renders_message() {
        let e = DbError::VecExtensionUnavailable("dlopen failed".into());
        assert!(e.to_string().contains("dlopen failed"));
    }

    #[test]
    fn from_io_error_routes_to_io_variant() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let e: DbError = io.into();
        assert!(matches!(e, DbError::Io(_)));
        assert!(e.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn from_sqlx_error_routes_to_sqlx_variant() {
        // Trigger a real sqlx error by attempting to connect to a bogus URL.
        let r = sqlx::SqlitePool::connect("sqlite:///dev/null/does-not-exist").await;
        let e: DbError = r.expect_err("expected sqlx error").into();
        assert!(matches!(e, DbError::Sqlx(_)));
    }
}
