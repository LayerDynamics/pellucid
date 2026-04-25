//! Migration runner — applies SQL files from the bundled `migrations/`
//! directory in lexical order. Each script is wrapped in a single
//! transaction so a partially-applied migration cannot leave the database
//! in a torn state.
//!
//! Migrations are idempotent: every script in `migrations/` uses
//! `CREATE TABLE IF NOT EXISTS` (and equivalent guards) so re-running
//! `migrate()` on a populated database is a no-op.

use sqlx::Executor;

use crate::error::{DbError, Result};

/// Static migration entries — `(version, name, sql)`.
///
/// New migrations land here in version order. `include_str!` ensures the
/// SQL ships with the binary so a deployed pellucid-edge or pellucid-relay
/// can boot a fresh database without filesystem access to the source tree.
const MIGRATIONS: &[(u32, &str, &str)] = &[(
    1,
    "0001_initial",
    include_str!("../migrations/0001_initial.sql"),
)];

/// Apply every bundled migration in version order. Idempotent.
///
/// # Errors
/// Returns [`DbError::Migration`] if any migration fails to execute.
pub async fn migrate(pool: &crate::Pool) -> Result<()> {
    for (version, name, sql) in MIGRATIONS {
        apply_one(pool, *version, name, sql).await?;
    }
    Ok(())
}

async fn apply_one(pool: &crate::Pool, version: u32, name: &str, sql: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    tx.execute(sql).await.map_err(|e| DbError::Migration {
        name: name.to_string(),
        message: e.to_string(),
    })?;
    tx.commit().await?;
    tracing::debug!(version, name, "migration applied");
    Ok(())
}

/// Returns the list of bundled migration `(version, name)` pairs. Useful
/// for diagnostic endpoints (e.g. `/api/health`) that surface schema
/// version to operators.
#[must_use]
pub fn manifest() -> Vec<(u32, &'static str)> {
    MIGRATIONS.iter().map(|(v, n, _)| (*v, *n)).collect()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn manifest_contains_initial() {
        let m = manifest();
        assert_eq!(m, vec![(1, "0001_initial")]);
    }

    #[test]
    fn migrations_are_sorted_by_version() {
        let mut prev = 0u32;
        for (v, _, _) in MIGRATIONS {
            assert!(*v > prev, "migrations must be strictly ascending");
            prev = *v;
        }
    }

    #[test]
    fn each_migration_uses_if_not_exists() {
        // Idempotency guard — every CREATE in the bundled scripts must
        // be guarded so re-running is a no-op.
        for (_, name, sql) in MIGRATIONS {
            let lower = sql.to_lowercase();
            // Count CREATE statements (excluding INSERT lines).
            let creates = lower.matches("create ").count();
            let if_not_exists = lower.matches("if not exists").count();
            assert!(
                if_not_exists >= creates,
                "{name}: {creates} CREATE(s) but only {if_not_exists} IF NOT EXISTS guards"
            );
        }
    }
}
