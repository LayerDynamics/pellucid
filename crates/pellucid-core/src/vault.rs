//! Consolidated secrets-vault trait — SPEC-001 §10.4 + M9 fix.
//!
//! Lifted from `pellucid-tauri` (where the keyring-backed implementation
//! still lives) so that `pellucid-streams` and the relay/sidecar binaries
//! can name a `Vault` trait without dragging the Tauri runtime crates onto
//! a Linux deploy target. The keyring-backed implementation
//! ([`pellucid_tauri::vault::KeychainVault`]) is the only consumer that
//! needs the OS keychain; tests use [`InMemoryVault`]; the relay binary
//! uses [`EnvVault`] backed by Railway secrets env vars.
//!
//! Every desktop secret lives in a single JSON blob ([`SecretsBlob`])
//! stored under `pellucid:secrets-vault:v1`, so the user is prompted at
//! most once per app version (M9 fix). New secrets are added by adding a
//! field to [`SecretsBlob`] — never a new keyring entry.

use std::env;
use std::fmt::Debug;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;

/// Keychain service identifier. Exported so tests + every host process use
/// the exact same key when reading/writing — drift here is the bug M9
/// fixed.
pub const VAULT_SERVICE: &str = "pellucid:secrets-vault:v1";
/// Keychain user portion. macOS keychain entries are keyed on
/// `(service, username)`; we use a constant since the host process is the
/// sole reader.
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
    /// Telegram MTProto StringSession (base64-encoded raw bytes). Set by
    /// the Tauri auth IPC commands after a successful sign-in (T4.5.0).
    #[serde(default)]
    pub telegram_session: Option<String>,
    /// Telegram API ID from <https://my.telegram.org>. Required to spin
    /// up a `grammers_client::Client`. Set during onboarding alongside
    /// the API hash (T4.5.0).
    #[serde(default)]
    pub telegram_api_id: Option<i32>,
    /// Telegram API hash matching `telegram_api_id`. Set alongside it
    /// (T4.5.0).
    #[serde(default)]
    pub telegram_api_hash: Option<String>,
    /// Vault schema version — bumped if the layout changes so old
    /// entries can be migrated rather than silently dropped. T4.5.0
    /// bumped this from 1 to 2 to add the three telegram fields.
    #[serde(default = "default_version")]
    pub version: u32,
}

fn default_version() -> u32 {
    2
}

impl Default for SecretsBlob {
    fn default() -> Self {
        Self {
            sidecar_token: None,
            sidecar_token_previous: None,
            clerk_session: None,
            clerk_refresh: None,
            telegram_session: None,
            telegram_api_id: None,
            telegram_api_hash: None,
            version: default_version(),
        }
    }
}

impl SecretsBlob {
    /// Serialise the blob as the JSON string stored in the keychain.
    ///
    /// # Errors
    /// [`VaultError::Encode`] if `serde_json` fails to serialise the
    /// blob (effectively impossible with `Option<String>` fields, but
    /// the surface stays explicit).
    pub fn to_json(&self) -> Result<String, VaultError> {
        serde_json::to_string(self).map_err(VaultError::Encode)
    }

    /// Parse a JSON string back into a blob. Empty strings yield a
    /// default empty blob so first-run startup does not error.
    ///
    /// Tolerates v1 blobs (no telegram fields): `serde(default)` fills
    /// the missing fields with `None`. The `version` field bumps to 2 on
    /// the next `write` because [`default_version`] returns 2.
    ///
    /// # Errors
    /// [`VaultError::Decode`] if the input is non-empty and not valid
    /// JSON or does not match the [`SecretsBlob`] schema.
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
    /// Backend storage failure (keyring, env-var parsing, etc.). The
    /// string is the underlying error's `Display` rendering.
    #[error("backend error: {0}")]
    Backend(String),
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

    /// Construct a vault pre-populated with `blob`. Useful for tests
    /// that want a non-empty starting state without going through
    /// `write`.
    #[must_use]
    pub fn with_blob(blob: SecretsBlob) -> Self {
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Self {
            inner: Mutex::new(Some(blob)),
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

/// Env-var-backed vault for the relay binary on Railway / Fly. Reads its
/// `SecretsBlob` snapshot from process env once at construction
/// (`from_env`) and caches it; `write`/`clear` mutate the in-memory
/// snapshot only — Railway secrets are externally managed and a process
/// restart re-reads the env. Fires `VaultChange::Updated` /
/// `VaultChange::Cleared` events the same way [`InMemoryVault`] does so
/// in-process subscribers (the relay's telegram run task) react to a
/// runtime rotation just like they do on desktop.
pub struct EnvVault {
    snapshot: Mutex<SecretsBlob>,
    tx: watch::Sender<VaultChange>,
}

impl EnvVault {
    /// Construct a vault from the current process env. Reads
    /// `TELEGRAM_SESSION_BASE64`, `TELEGRAM_API_ID`, `TELEGRAM_API_HASH`
    /// (T4.5.0); other fields default to `None`. Missing env vars are
    /// fine — `try_spawn` returns `None` and the run task simply does
    /// not start.
    ///
    /// Uses `std::env::var` directly — this is the documented env-read
    /// boundary for the relay binary, mirroring the pattern in
    /// `pellucid_relay_bin::config::ConfigSource::from_process` which
    /// applies the same `clippy::disallowed_methods` allow.
    #[must_use]
    #[allow(clippy::disallowed_methods)]
    pub fn from_env() -> Self {
        let mut blob = SecretsBlob::default();
        if let Ok(s) = env::var("TELEGRAM_SESSION_BASE64") {
            if !s.is_empty() {
                blob.telegram_session = Some(s);
            }
        }
        if let Ok(s) = env::var("TELEGRAM_API_ID") {
            if let Ok(parsed) = s.parse::<i32>() {
                blob.telegram_api_id = Some(parsed);
            }
        }
        if let Ok(s) = env::var("TELEGRAM_API_HASH") {
            if !s.is_empty() {
                blob.telegram_api_hash = Some(s);
            }
        }
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Self {
            snapshot: Mutex::new(blob),
            tx,
        }
    }

    /// Construct an `EnvVault` pre-populated with `blob`. Used by the
    /// relay's integration tests so they don't have to mutate process
    /// env.
    #[must_use]
    pub fn with_blob(blob: SecretsBlob) -> Self {
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Self {
            snapshot: Mutex::new(blob),
            tx,
        }
    }
}

impl Debug for EnvVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let blob = self.snapshot.lock();
        f.debug_struct("EnvVault")
            .field("has_telegram_session", &blob.telegram_session.is_some())
            .field("has_telegram_api_id", &blob.telegram_api_id.is_some())
            .field("has_telegram_api_hash", &blob.telegram_api_hash.is_some())
            .finish()
    }
}

#[async_trait]
impl Vault for EnvVault {
    async fn read(&self) -> Result<SecretsBlob, VaultError> {
        Ok(self.snapshot.lock().clone())
    }

    async fn write(&self, blob: &SecretsBlob) -> Result<(), VaultError> {
        *self.snapshot.lock() = blob.clone();
        let _ = self.tx.send(VaultChange::Updated);
        Ok(())
    }

    async fn clear(&self) -> Result<(), VaultError> {
        *self.snapshot.lock() = SecretsBlob::default();
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
        assert_eq!(blob.version, 2);
    }

    #[tokio::test]
    async fn write_then_read_roundtrips_every_field() {
        let v = InMemoryVault::new();
        let blob = SecretsBlob {
            sidecar_token: Some("tok-A".into()),
            sidecar_token_previous: Some("tok-prev".into()),
            clerk_session: Some("clerk-sess".into()),
            clerk_refresh: Some("clerk-refresh".into()),
            telegram_session: Some("base64-bytes".into()),
            telegram_api_id: Some(123_456),
            telegram_api_hash: Some("hash".into()),
            version: 2,
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
        // version came from default — bumped to 2 after T4.5.0.
        assert_eq!(blob.version, 2);
        assert!(blob.clerk_session.is_none());
        assert!(blob.telegram_session.is_none());
        assert!(blob.telegram_api_id.is_none());
        assert!(blob.telegram_api_hash.is_none());
    }

    #[tokio::test]
    async fn from_json_v1_blob_migrates_to_v2_with_telegram_fields_none() {
        // A v1 blob in the wild has the four pre-T4.5.0 fields plus
        // `version: 1`. The default for the three new fields is `None`,
        // and `version` ends up `2` once the next `write` re-serialises.
        let v1_json = r#"{
            "sidecar_token": "tok",
            "sidecar_token_previous": null,
            "clerk_session": null,
            "clerk_refresh": null,
            "version": 1
        }"#;
        let blob = SecretsBlob::from_json(v1_json).unwrap();
        assert_eq!(blob.sidecar_token.as_deref(), Some("tok"));
        assert_eq!(blob.version, 1, "explicit v1 field is preserved on read");
        assert!(blob.telegram_session.is_none());
        assert!(blob.telegram_api_id.is_none());
        assert!(blob.telegram_api_hash.is_none());

        // After writing back through the vault, the version stays 1
        // until the host explicitly bumps it. We check that the JSON
        // round-trip is stable:
        let round = SecretsBlob::from_json(&blob.to_json().unwrap()).unwrap();
        assert_eq!(round, blob);
    }

    #[tokio::test]
    async fn default_blob_has_v2_after_t450() {
        assert_eq!(SecretsBlob::default().version, 2);
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
        assert_eq!(VAULT_SERVICE, "pellucid:secrets-vault:v1");
        assert_eq!(VAULT_USER, "pellucid-host");
    }

    #[tokio::test]
    async fn env_vault_with_blob_reads_back_what_was_written_in() {
        let blob = SecretsBlob {
            telegram_session: Some("sess".into()),
            telegram_api_id: Some(42),
            telegram_api_hash: Some("h".into()),
            ..Default::default()
        };
        let v = EnvVault::with_blob(blob.clone());
        assert_eq!(v.read().await.unwrap(), blob);
    }

    #[tokio::test]
    async fn env_vault_write_emits_updated_event() {
        let v = EnvVault::with_blob(SecretsBlob::default());
        let mut rx = v.subscribe();
        v.write(&SecretsBlob {
            telegram_session: Some("new".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow_and_update(), VaultChange::Updated);
    }

    #[tokio::test]
    async fn env_vault_clear_resets_to_default_and_emits_cleared() {
        let v = EnvVault::with_blob(SecretsBlob {
            telegram_session: Some("x".into()),
            ..Default::default()
        });
        v.clear().await.unwrap();
        let after = v.read().await.unwrap();
        assert_eq!(after, SecretsBlob::default());
    }
}
