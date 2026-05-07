//! `SessionStore` trait + three implementations.
//!
//! - [`VaultSessionStore`] — Tauri host process; reads/writes
//!   `SecretsBlob.telegram_session` via the keyring-backed
//!   `pellucid_core::vault::Vault`. Used by the login flow.
//! - [`IpcSessionStore`] — sidecar process; in-memory state mutated by
//!   the sidecar's stdin parser when the host pushes
//!   `TELEGRAM_SESSION_UPDATED <base64>` lines. The sidecar has no
//!   keyring access by design (it's a separate process and must not
//!   pull `pellucid-tauri`).
//! - [`EnvSessionStore`] — Railway relay; reads `TELEGRAM_SESSION_BASE64`
//!   from process env at construction. `save` returns `ReadOnly`.

use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::watch;

use pellucid_core::vault::{Vault, VaultError};

/// What the run task needs to know happened at the store layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// Subscribers were just attached; no change has happened yet.
    Initial,
    /// New session bytes were saved (login completed, key rotated).
    Updated,
    /// The session was cleared (user signed out / vault wiped).
    Cleared,
}

/// Errors `SessionStore` operations can produce.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    /// Underlying vault error (only meaningful for `VaultSessionStore`).
    #[error("vault: {0}")]
    Vault(#[from] VaultError),
    /// Backing store does not allow `save` (e.g. `EnvSessionStore` — the
    /// human re-issues a Railway secret rather than the process
    /// rotating its own key).
    #[error("read-only session store")]
    ReadOnly,
    /// Stored session bytes failed base64 decode.
    #[error("invalid base64 in stored session: {0}")]
    Base64(#[from] base64::DecodeError),
}

/// Common surface every store implementation must expose.
#[async_trait]
pub trait SessionStore: Send + Sync + std::fmt::Debug {
    /// Load the current StringSession bytes if any. `Ok(None)` when the
    /// store has no session and the run task should idle / surface
    /// [`super::TelegramRunError::AuthRequired`].
    async fn load(&self) -> Result<Option<Vec<u8>>, SessionStoreError>;

    /// Persist new session bytes. The run task calls this when grammers
    /// reports the session has changed (post-sign-in, post-rotation).
    /// `EnvSessionStore` returns [`SessionStoreError::ReadOnly`] here.
    async fn save(&self, bytes: &[u8]) -> Result<(), SessionStoreError>;

    /// Subscribe to session-change events. Required so the run loop
    /// reacts to a logout / re-login without restart.
    fn subscribe(&self) -> watch::Receiver<SessionEvent>;
}

// ---------------------------------------------------------------------------
// VaultSessionStore — Tauri host process

/// Vault-backed store. Reads/writes `SecretsBlob.telegram_session` through
/// the injected `Arc<dyn Vault>`. Subscribes to the vault's own
/// change channel and republishes change events as `SessionEvent`s
/// whenever `telegram_session` actually changed (so a sidecar-token
/// rotation does not wake the run task spuriously).
pub struct VaultSessionStore {
    vault: Arc<dyn Vault>,
    last_session: Mutex<Option<String>>,
    tx: watch::Sender<SessionEvent>,
}

impl std::fmt::Debug for VaultSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultSessionStore")
            .field("vault", &self.vault)
            .finish()
    }
}

impl VaultSessionStore {
    /// Build a vault-backed store. Spawns a watcher task that bridges
    /// the vault's `subscribe` channel to ours so the run loop only
    /// wakes when the telegram session actually changed.
    pub async fn new(vault: Arc<dyn Vault>) -> Result<Arc<Self>, SessionStoreError> {
        let blob = vault.read().await?;
        let last = blob.telegram_session.clone();
        let (tx, _rx) = watch::channel(SessionEvent::Initial);
        let store = Arc::new(Self {
            vault: vault.clone(),
            last_session: Mutex::new(last),
            tx,
        });
        spawn_vault_bridge(store.clone());
        Ok(store)
    }
}

fn spawn_vault_bridge(store: Arc<VaultSessionStore>) {
    let mut rx = store.vault.subscribe();
    tokio::spawn(async move {
        loop {
            if rx.changed().await.is_err() {
                break;
            }
            let blob = match store.vault.read().await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!(target: "pellucid::telegram::session", "vault read failed: {err}");
                    continue;
                }
            };
            let mut guard = store.last_session.lock();
            if blob.telegram_session != *guard {
                let event = if blob.telegram_session.is_some() {
                    SessionEvent::Updated
                } else {
                    SessionEvent::Cleared
                };
                *guard = blob.telegram_session.clone();
                drop(guard);
                let _ = store.tx.send(event);
            }
        }
    });
}

#[async_trait]
impl SessionStore for VaultSessionStore {
    async fn load(&self) -> Result<Option<Vec<u8>>, SessionStoreError> {
        let blob = self.vault.read().await?;
        match blob.telegram_session {
            Some(ref encoded) => {
                let bytes = BASE64_STANDARD.decode(encoded)?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    async fn save(&self, bytes: &[u8]) -> Result<(), SessionStoreError> {
        let mut blob = self.vault.read().await?;
        let encoded = BASE64_STANDARD.encode(bytes);
        if blob.telegram_session.as_deref() == Some(encoded.as_str()) {
            return Ok(());
        }
        blob.telegram_session = Some(encoded.clone());
        self.vault.write(&blob).await?;
        *self.last_session.lock() = Some(encoded);
        let _ = self.tx.send(SessionEvent::Updated);
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}

// ---------------------------------------------------------------------------
// IpcSessionStore — sidecar process

/// In-memory store the sidecar mutates from its stdin parser when the
/// host pushes `TELEGRAM_SESSION_UPDATED <base64>` /
/// `TELEGRAM_SESSION_CLEARED` lines. Also writes new bytes back to
/// stdout (`TELEGRAM_SESSION_UPSTREAM=<base64>`) so the host can
/// persist a sidecar-rotated session into the OS keychain.
pub struct IpcSessionStore {
    inner: Mutex<Option<Vec<u8>>>,
    tx: watch::Sender<SessionEvent>,
}

impl std::fmt::Debug for IpcSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let has_bytes = self.inner.lock().is_some();
        f.debug_struct("IpcSessionStore")
            .field("has_bytes", &has_bytes)
            .finish()
    }
}

/// stdout prefix the sidecar writes when its run task generated fresh
/// session bytes. The host's stdout drain task strips the prefix and
/// persists the decoded bytes to the OS keychain. Both crates import
/// this constant so drift cannot occur.
pub const STDOUT_TELEGRAM_SESSION_PREFIX: &str = "TELEGRAM_SESSION_UPSTREAM=";

impl IpcSessionStore {
    /// Construct an empty store.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(SessionEvent::Initial);
        Self {
            inner: Mutex::new(None),
            tx,
        }
    }

    /// Construct with pre-loaded bytes. Used in tests where the
    /// stdin handshake is simulated.
    #[must_use]
    pub fn with_bytes(bytes: Vec<u8>) -> Self {
        let (tx, _rx) = watch::channel(SessionEvent::Initial);
        Self {
            inner: Mutex::new(Some(bytes)),
            tx,
        }
    }

    /// Apply a `TELEGRAM_SESSION_UPDATED` IPC line. Used by the
    /// sidecar's stdin parser. Errors only if the supplied base64
    /// doesn't decode.
    ///
    /// # Errors
    /// [`SessionStoreError::Base64`] if `base64_payload` is not valid
    /// base64.
    pub fn apply_ipc_updated(&self, base64_payload: &str) -> Result<(), SessionStoreError> {
        let bytes = BASE64_STANDARD.decode(base64_payload.trim())?;
        *self.inner.lock() = Some(bytes);
        let _ = self.tx.send(SessionEvent::Updated);
        Ok(())
    }

    /// Apply a `TELEGRAM_SESSION_CLEARED` IPC line.
    pub fn apply_ipc_cleared(&self) {
        *self.inner.lock() = None;
        let _ = self.tx.send(SessionEvent::Cleared);
    }
}

impl Default for IpcSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SessionStore for IpcSessionStore {
    async fn load(&self) -> Result<Option<Vec<u8>>, SessionStoreError> {
        Ok(self.inner.lock().clone())
    }

    async fn save(&self, bytes: &[u8]) -> Result<(), SessionStoreError> {
        // Sidecar's run task generated fresh bytes. Persist locally
        // AND push to host via stdout so the keychain stays consistent.
        let mut guard = self.inner.lock();
        if guard.as_deref() == Some(bytes) {
            return Ok(());
        }
        *guard = Some(bytes.to_vec());
        drop(guard);
        let encoded = BASE64_STANDARD.encode(bytes);
        // `println!` is the only IPC channel back to the host; the
        // workspace lint forbids it by default but this is the
        // canonical control path (the sidecar already uses println for
        // `PORT=<n>`).
        #[allow(clippy::print_stdout)]
        {
            println!("{STDOUT_TELEGRAM_SESSION_PREFIX}{encoded}");
        }
        let _ = self.tx.send(SessionEvent::Updated);
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}

// ---------------------------------------------------------------------------
// EnvSessionStore — Railway relay

/// Env-backed store. Reads `TELEGRAM_SESSION_BASE64` at construction;
/// `save` returns [`SessionStoreError::ReadOnly`] (Railway secrets are
/// externally managed). The run loop logs a warn and continues when
/// it tries to persist a rotated session and gets `ReadOnly` — the
/// human is expected to re-issue a fresh Railway secret on next deploy.
pub struct EnvSessionStore {
    inner: Mutex<Option<Vec<u8>>>,
    tx: watch::Sender<SessionEvent>,
}

impl std::fmt::Debug for EnvSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let has_bytes = self.inner.lock().is_some();
        f.debug_struct("EnvSessionStore")
            .field("has_bytes", &has_bytes)
            .finish()
    }
}

impl EnvSessionStore {
    /// Construct from process env. Empty / missing
    /// `TELEGRAM_SESSION_BASE64` yields an empty store; the run loop
    /// then surfaces `AuthRequired`.
    ///
    /// # Errors
    /// [`SessionStoreError::Base64`] if the env var is set but does not
    /// decode.
    ///
    /// Uses `std::env::var` directly — this is the documented env-read
    /// boundary for the relay binary, mirroring the pattern in
    /// `pellucid_relay_bin::config::ConfigSource::from_process` which
    /// applies the same `clippy::disallowed_methods` allow.
    #[allow(clippy::disallowed_methods)]
    pub fn from_env() -> Result<Self, SessionStoreError> {
        let bytes = match env::var("TELEGRAM_SESSION_BASE64") {
            Ok(s) if !s.is_empty() => Some(BASE64_STANDARD.decode(s.trim())?),
            _ => None,
        };
        let (tx, _rx) = watch::channel(SessionEvent::Initial);
        Ok(Self {
            inner: Mutex::new(bytes),
            tx,
        })
    }

    /// Construct with explicit bytes. Used by integration tests so they
    /// don't have to mutate process env.
    #[must_use]
    pub fn with_bytes(bytes: Option<Vec<u8>>) -> Self {
        let (tx, _rx) = watch::channel(SessionEvent::Initial);
        Self {
            inner: Mutex::new(bytes),
            tx,
        }
    }
}

#[async_trait]
impl SessionStore for EnvSessionStore {
    async fn load(&self) -> Result<Option<Vec<u8>>, SessionStoreError> {
        Ok(self.inner.lock().clone())
    }

    async fn save(&self, _bytes: &[u8]) -> Result<(), SessionStoreError> {
        Err(SessionStoreError::ReadOnly)
    }

    fn subscribe(&self) -> watch::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_core::vault::{InMemoryVault, SecretsBlob};

    #[tokio::test]
    async fn ipc_session_store_round_trips_updated_and_cleared() {
        let store = IpcSessionStore::new();
        let mut rx = store.subscribe();
        assert!(store.load().await.unwrap().is_none());

        let payload = BASE64_STANDARD.encode([0xAA_u8, 0xBB, 0xCC]);
        store.apply_ipc_updated(&payload).unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), SessionEvent::Updated);
        assert_eq!(store.load().await.unwrap(), Some(vec![0xAA_u8, 0xBB, 0xCC]));

        store.apply_ipc_cleared();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), SessionEvent::Cleared);
        assert!(store.load().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ipc_session_store_rejects_invalid_base64() {
        let store = IpcSessionStore::new();
        let err = store.apply_ipc_updated("not-base64!!").unwrap_err();
        assert!(matches!(err, SessionStoreError::Base64(_)));
    }

    #[tokio::test]
    async fn env_session_store_returns_read_only_on_save() {
        let store = EnvSessionStore::with_bytes(Some(vec![1, 2, 3]));
        let err = store.save(&[4, 5]).await.unwrap_err();
        assert!(matches!(err, SessionStoreError::ReadOnly));
        assert_eq!(store.load().await.unwrap(), Some(vec![1, 2, 3]));
    }

    #[tokio::test]
    async fn vault_session_store_round_trips_through_in_memory_vault() {
        let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
        let store = VaultSessionStore::new(vault.clone()).await.unwrap();
        assert!(store.load().await.unwrap().is_none());

        store.save(&[1_u8, 2, 3]).await.unwrap();
        assert_eq!(store.load().await.unwrap(), Some(vec![1, 2, 3]));

        // Vault now has the encoded session under SecretsBlob.
        let blob = vault.read().await.unwrap();
        let expected = BASE64_STANDARD.encode([1_u8, 2, 3]);
        assert_eq!(blob.telegram_session.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn vault_session_store_subscribe_emits_on_external_write() {
        let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
        let store = VaultSessionStore::new(vault.clone()).await.unwrap();
        let mut rx = store.subscribe();
        assert_eq!(*rx.borrow(), SessionEvent::Initial);

        // Simulate the host login flow writing to the vault directly.
        let encoded = BASE64_STANDARD.encode([9_u8, 9, 9]);
        let blob = SecretsBlob {
            telegram_session: Some(encoded),
            ..Default::default()
        };
        vault.write(&blob).await.unwrap();

        // The bridge task republishes Updated.
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), SessionEvent::Updated);
        assert_eq!(store.load().await.unwrap(), Some(vec![9, 9, 9]));
    }
}
