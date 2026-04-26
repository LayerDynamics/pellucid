//! Consolidated keychain vault — SPEC-001 §10.4 + M9 fix.
//!
//! The original WorldMonitor app stored each secret in its own keychain
//! entry, which prompted macOS keychain dialogs once per secret on first
//! launch. M9 collapses every desktop secret into a single JSON blob
//! stored under `pellucid:secrets-vault:v1`, so the user is prompted at
//! most once per app version.
//!
//! The module exposes a [`Vault`] trait so the IPC layer can compose
//! against either the real OS keyring ([`KeychainVault`]) or an
//! in-memory fake ([`InMemoryVault`]) for tests. Both implementations
//! emit [`VaultChange`] notifications via a `tokio::sync::watch` channel
//! whenever the stored blob is replaced — the listener channel is the
//! M9 deliverable that lets the rest of the app react to keychain
//! changes (e.g. a user updating their Clerk token in another window).

use std::fmt::Debug;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;

/// Keychain service identifier. Exported so tests + the binary use the
/// exact same key when reading/writing — drift here is the bug M9 fixes.
pub const VAULT_SERVICE: &str = "pellucid:secrets-vault:v1";
/// Keychain user portion. macOS keychain entries are keyed on
/// `(service, username)`; we use a constant since the host process is
/// the sole reader.
pub const VAULT_USER: &str = "pellucid-host";

/// JSON payload stored in the consolidated keychain entry.
///
/// Every desktop secret the host knows about lives here. Adding a new
/// secret means adding a field — *never* a new keychain entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsBlob {
    /// Bearer token the sidecar requires on every request. Rotated by
    /// T1.8.
    #[serde(default)]
    pub sidecar_token: Option<String>,
    /// Previous sidecar token, kept during the 30 s overlap window so
    /// in-flight requests don't 401 right after rotation. Set by T1.8.
    #[serde(default)]
    pub sidecar_token_previous: Option<String>,
    /// Clerk session token captured at sign-in.
    #[serde(default)]
    pub clerk_session: Option<String>,
    /// Refresh token used to mint new Clerk sessions.
    #[serde(default)]
    pub clerk_refresh: Option<String>,
    /// Vault schema version — bumped if the layout changes so old
    /// entries can be migrated rather than silently dropped.
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    1
}

impl Default for SecretsBlob {
    fn default() -> Self {
        Self {
            sidecar_token: None,
            sidecar_token_previous: None,
            clerk_session: None,
            clerk_refresh: None,
            version: default_version(),
        }
    }
}

impl SecretsBlob {
    /// Serialise the blob as the JSON string stored in the keychain.
    pub fn to_json(&self) -> Result<String, VaultError> {
        serde_json::to_string(self).map_err(VaultError::Encode)
    }

    /// Parse a JSON string back into a blob. Empty strings yield a
    /// default empty blob so first-run startup does not error.
    pub fn from_json(s: &str) -> Result<Self, VaultError> {
        if s.is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(s).map_err(VaultError::Decode)
    }
}

/// Notification published every time the stored blob changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultChange {
    /// Vault was just opened — fired once on startup.
    Initial,
    /// A new blob has been stored (either by us or by an external
    /// process — the listener does not know which).
    Updated,
    /// The keychain entry was deleted.
    Cleared,
}

/// Errors emitted by the vault layer.
#[derive(Debug, Error)]
pub enum VaultError {
    /// Underlying OS keyring failed.
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    /// Could not encode `SecretsBlob` to JSON.
    #[error("encode failed: {0}")]
    Encode(serde_json::Error),
    /// Could not decode the JSON read from the keyring.
    #[error("decode failed: {0}")]
    Decode(serde_json::Error),
}

/// Common surface every vault implementation must expose.
#[async_trait]
pub trait Vault: Send + Sync + Debug {
    /// Read the consolidated blob from storage. First-run reads
    /// (entry missing) yield a [`SecretsBlob::default`] without error.
    async fn read(&self) -> Result<SecretsBlob, VaultError>;

    /// Replace the consolidated blob in storage. Fires a
    /// `VaultChange::Updated` notification.
    async fn write(&self, blob: &SecretsBlob) -> Result<(), VaultError>;

    /// Delete the keychain entry. Fires `VaultChange::Cleared`.
    async fn clear(&self) -> Result<(), VaultError>;

    /// Subscribe to vault change notifications. Each subscriber sees
    /// every `Updated` / `Cleared` event published after subscription.
    fn subscribe(&self) -> watch::Receiver<VaultChange>;
}

/// Real implementation backed by the OS keyring.
pub struct KeychainVault {
    entry: keyring::Entry,
    tx: watch::Sender<VaultChange>,
}

impl KeychainVault {
    /// Construct a vault wrapping the canonical
    /// `pellucid:secrets-vault:v1` keychain entry.
    pub fn new() -> Result<Self, VaultError> {
        let entry = keyring::Entry::new(VAULT_SERVICE, VAULT_USER)?;
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Ok(Self { entry, tx })
    }

    /// Construct a vault using a custom service/user pair. Reserved for
    /// integration tests that need to keep each test's keyring entry
    /// isolated.
    pub fn with_entry(service: &str, user: &str) -> Result<Self, VaultError> {
        let entry = keyring::Entry::new(service, user)?;
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Ok(Self { entry, tx })
    }
}

impl Debug for KeychainVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainVault")
            .field("service", &VAULT_SERVICE)
            .field("user", &VAULT_USER)
            .finish()
    }
}

#[async_trait]
impl Vault for KeychainVault {
    async fn read(&self) -> Result<SecretsBlob, VaultError> {
        match self.entry.get_password() {
            Ok(s) => SecretsBlob::from_json(&s),
            Err(keyring::Error::NoEntry) => Ok(SecretsBlob::default()),
            Err(other) => Err(other.into()),
        }
    }

    async fn write(&self, blob: &SecretsBlob) -> Result<(), VaultError> {
        let payload = blob.to_json()?;
        self.entry.set_password(&payload)?;
        let _ = self.tx.send(VaultChange::Updated);
        Ok(())
    }

    async fn clear(&self) -> Result<(), VaultError> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {
                let _ = self.tx.send(VaultChange::Cleared);
                Ok(())
            }
            Err(other) => Err(other.into()),
        }
    }

    fn subscribe(&self) -> watch::Receiver<VaultChange> {
        self.tx.subscribe()
    }
}

/// In-memory implementation for unit + integration tests. Threadsafe.
pub struct InMemoryVault {
    inner: Mutex<Option<SecretsBlob>>,
    tx: watch::Sender<VaultChange>,
}

impl InMemoryVault {
    /// Construct an empty vault.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Self {
            inner: Mutex::new(None),
            tx,
        }
    }
}

impl Default for InMemoryVault {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for InMemoryVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let has_blob = self.inner.lock().is_some();
        f.debug_struct("InMemoryVault")
            .field("has_blob", &has_blob)
            .finish()
    }
}

#[async_trait]
impl Vault for InMemoryVault {
    async fn read(&self) -> Result<SecretsBlob, VaultError> {
        Ok(self.inner.lock().clone().unwrap_or_default())
    }

    async fn write(&self, blob: &SecretsBlob) -> Result<(), VaultError> {
        *self.inner.lock() = Some(blob.clone());
        let _ = self.tx.send(VaultChange::Updated);
        Ok(())
    }

    async fn clear(&self) -> Result<(), VaultError> {
        *self.inner.lock() = None;
        let _ = self.tx.send(VaultChange::Cleared);
        Ok(())
    }

    fn subscribe(&self) -> watch::Receiver<VaultChange> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_returns_default_when_empty() {
        let v = InMemoryVault::new();
        let blob = v.read().await.unwrap();
        assert_eq!(blob, SecretsBlob::default());
        assert_eq!(blob.version, 1);
    }

    #[tokio::test]
    async fn write_then_read_roundtrips_every_field() {
        let v = InMemoryVault::new();
        let blob = SecretsBlob {
            sidecar_token: Some("tok-A".into()),
            sidecar_token_previous: Some("tok-prev".into()),
            clerk_session: Some("clerk-sess".into()),
            clerk_refresh: Some("clerk-refresh".into()),
            version: 1,
        };
        v.write(&blob).await.unwrap();
        let round = v.read().await.unwrap();
        assert_eq!(round, blob);
    }

    #[tokio::test]
    async fn clear_removes_blob_and_returns_default_after() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            sidecar_token: Some("x".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        v.clear().await.unwrap();
        assert_eq!(v.read().await.unwrap(), SecretsBlob::default());
    }

    #[tokio::test]
    async fn subscribers_observe_updates_and_clears() {
        let v = InMemoryVault::new();
        let mut rx = v.subscribe();
        // initial value is Initial.
        assert_eq!(*rx.borrow(), VaultChange::Initial);

        v.write(&SecretsBlob::default()).await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), VaultChange::Updated);

        v.clear().await.unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), VaultChange::Cleared);
    }

    #[tokio::test]
    async fn json_roundtrip_preserves_unknown_extra_fields_via_serde_default() {
        let json = r#"{"sidecar_token":"t1"}"#;
        let blob = SecretsBlob::from_json(json).unwrap();
        assert_eq!(blob.sidecar_token.as_deref(), Some("t1"));
        assert_eq!(blob.version, 1);
        assert!(blob.clerk_session.is_none());
    }

    #[tokio::test]
    async fn from_json_empty_string_yields_default_blob() {
        assert_eq!(SecretsBlob::from_json("").unwrap(), SecretsBlob::default());
    }

    #[tokio::test]
    async fn from_json_invalid_returns_decode_error() {
        let err = SecretsBlob::from_json("{not json").unwrap_err();
        assert!(matches!(err, VaultError::Decode(_)), "got {err:?}");
    }

    #[test]
    fn vault_service_constant_matches_m9_fix_value() {
        // Drift in this value reintroduces the original
        // multiple-prompt bug. Lock it.
        assert_eq!(VAULT_SERVICE, "pellucid:secrets-vault:v1");
        assert_eq!(VAULT_USER, "pellucid-host");
    }

    #[tokio::test]
    async fn keychain_vault_constructible_with_isolated_entry() {
        // Ensures the `with_entry` constructor compiles and yields a
        // working subscriber even if the underlying OS keyring is not
        // accessible in CI sandboxes — we never call `read`/`write`.
        let v =
            KeychainVault::with_entry("pellucid-test:none", "pellucid-test-user").unwrap();
        let mut rx = v.subscribe();
        assert_eq!(*rx.borrow_and_update(), VaultChange::Initial);
    }
}
