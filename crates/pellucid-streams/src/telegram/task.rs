//! Spawn / try_spawn / shutdown helpers for the Telegram run task.
//!
//! Both `pellucid-relay-bin` and `pellucid-sidecar-bin` use this same
//! module — the binary-side wrappers (`relay_bin::telegram_task`,
//! `sidecar_bin::telegram_task`) only differ in which `Vault` /
//! `SessionStore` impl they construct.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pellucid_core::vault::Vault;
use pellucid_db::Pool;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::client::{GrammersClient, MtprotoClient};
use super::error::TelegramRunError;
use super::run::{run, TelegramRunConfig};
use super::session::{SessionStore, VaultSessionStore};

/// Handle returned by [`spawn`] / [`try_spawn`]. Owns the join handle
/// for the run-task future and the watch sender that drives graceful
/// shutdown.
#[derive(Debug)]
pub struct TelegramTaskHandle {
    /// Join handle for the run task.
    pub task: JoinHandle<Result<(), TelegramRunError>>,
    /// Watch sender — set to `true` to request shutdown.
    pub shutdown_tx: watch::Sender<bool>,
}

/// Spawn the run task with explicit dependencies. Used by tests +
/// integration paths that build the client / sessions directly.
#[must_use]
pub fn spawn(
    pool: Pool,
    client: Arc<dyn MtprotoClient>,
    sessions: Arc<dyn SessionStore>,
    cfg: TelegramRunConfig,
) -> TelegramTaskHandle {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move { run(pool, client, sessions, cfg, shutdown_rx).await });
    TelegramTaskHandle { task, shutdown_tx }
}

/// Try to spawn the run task using a vault-backed session store. Reads
/// the credentials from `vault` once at construction; returns `None`
/// when `telegram_api_id` / `telegram_api_hash` are absent (dev mode,
/// pre-onboarding desktop). The actual `GrammersClient` is built
/// against the SQLite session at `session_path`.
///
/// # Errors
/// - [`TelegramRunError::Vault`] / [`TelegramRunError::SessionStore`] /
///   [`TelegramRunError::Mtproto`] — propagated from the underlying
///   `connect`. Returning these means the task DID NOT spawn.
pub async fn try_spawn(
    pool: Pool,
    vault: Arc<dyn Vault>,
    session_path: PathBuf,
    channel_set_path: PathBuf,
    channel_set_env_override: Option<String>,
) -> Result<Option<TelegramTaskHandle>, TelegramRunError> {
    let blob = vault.read().await?;
    let api_id = match blob.telegram_api_id {
        Some(id) if id != 0 => id,
        _ => return Ok(None),
    };
    let api_hash = match blob.telegram_api_hash.clone() {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(None),
    };

    let sessions: Arc<dyn SessionStore> = VaultSessionStore::new(vault).await?;
    let initial_bytes = sessions.load().await?;

    let client = GrammersClient::connect(
        session_path,
        initial_bytes.as_deref(),
        api_id,
        api_hash.clone(),
    )
    .await?;
    let client_arc: Arc<dyn MtprotoClient> = Arc::new(client);

    let cfg = TelegramRunConfig {
        api_id,
        api_hash,
        poll_interval: Duration::from_secs(60),
        per_channel_timeout: Duration::from_secs(15),
        channel_set_path,
        channel_set_env_override,
        per_channel_limit: super::snapshot::DEFAULT_PER_CHANNEL_LIMIT,
    };

    Ok(Some(spawn(pool, client_arc, sessions, cfg)))
}

/// Drain a running task. Sets the shutdown signal, awaits the join
/// handle with a `grace` timeout. Returns `true` iff the task exited
/// cleanly within the grace window.
pub async fn shutdown(handle: TelegramTaskHandle, grace: Duration) -> bool {
    let _ = handle.shutdown_tx.send(true);
    match tokio::time::timeout(grace, handle.task).await {
        Ok(Ok(_)) => true,
        Ok(Err(join_err)) => {
            tracing::warn!(
                target: "pellucid::telegram::task",
                ?join_err,
                "telegram run task panicked"
            );
            false
        }
        Err(_elapsed) => {
            tracing::warn!(
                target: "pellucid::telegram::task",
                ?grace,
                "telegram run task did not exit within grace; abandoning"
            );
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::telegram::client::tests::MockClient;
    use crate::telegram::session::IpcSessionStore;
    use pellucid_core::vault::{InMemoryVault, SecretsBlob};
    use pellucid_db::open_in_memory;
    use std::io::Write as _;
    use std::path::Path;

    fn temp_channel_file(channels: &[&str]) -> tempfile::NamedTempFile {
        let mut tf = tempfile::NamedTempFile::new().unwrap();
        let json = format!(
            r#"{{"channels": [{}]}}"#,
            channels
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(",")
        );
        tf.write_all(json.as_bytes()).unwrap();
        tf
    }

    fn cfg_for(path: &Path) -> TelegramRunConfig {
        TelegramRunConfig {
            api_id: 1,
            api_hash: "h".into(),
            poll_interval: Duration::from_millis(50),
            per_channel_timeout: Duration::from_millis(200),
            channel_set_path: path.to_path_buf(),
            channel_set_env_override: None,
            per_channel_limit: 3,
        }
    }

    #[tokio::test]
    async fn spawn_then_shutdown_returns_clean() {
        let pool = open_in_memory().await.unwrap();
        let client: Arc<dyn MtprotoClient> = Arc::new(MockClient::new());
        let sessions: Arc<dyn SessionStore> = Arc::new(IpcSessionStore::with_bytes(b"s".to_vec()));
        let tf = temp_channel_file(&["a"]);
        let handle = spawn(pool, client, sessions, cfg_for(tf.path()));

        let clean = shutdown(handle, Duration::from_secs(2)).await;
        assert!(clean);
    }

    #[tokio::test]
    async fn try_spawn_returns_none_when_vault_has_no_credentials() {
        let pool = open_in_memory().await.unwrap();
        let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
        let tf = temp_channel_file(&["a"]);
        let session_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let result = try_spawn(
            pool,
            vault,
            session_path.to_path_buf(),
            tf.path().to_path_buf(),
            None,
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn try_spawn_returns_none_when_only_api_id_is_set() {
        let pool = open_in_memory().await.unwrap();
        let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::with_blob(SecretsBlob {
            telegram_api_id: Some(123),
            telegram_api_hash: None,
            ..Default::default()
        }));
        let tf = temp_channel_file(&["a"]);
        let session_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let result = try_spawn(
            pool,
            vault,
            session_path.to_path_buf(),
            tf.path().to_path_buf(),
            None,
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }
}
