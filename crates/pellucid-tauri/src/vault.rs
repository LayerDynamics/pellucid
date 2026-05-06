//! Tauri-host keychain vault adapter.
//!
//! T4.5.0 lifted the [`Vault`] trait, [`SecretsBlob`], [`VaultError`], and
//! [`InMemoryVault`] into [`pellucid_core::vault`] so the relay and sidecar
//! binaries (which must build without a Tauri runtime) can name the same
//! trait. This module re-exports those types and adds the
//! [`KeychainVault`] implementation backed by the OS keyring (the only
//! piece that needs the `keyring` crate).
//!
//! See SPEC-001 §10.4 + the M9 fix for the consolidated-keychain rationale.

use std::fmt::Debug;

use async_trait::async_trait;
use tokio::sync::watch;

pub use pellucid_core::vault::{
    EnvVault, InMemoryVault, SecretsBlob, Vault, VaultChange, VaultError, VAULT_SERVICE, VAULT_USER,
};

/// Real implementation backed by the OS keyring. Only available in the
/// Tauri host process; the relay binary uses [`EnvVault`] (Railway
/// secrets) and tests use [`InMemoryVault`].
pub struct KeychainVault {
    entry: keyring::Entry,
    tx: watch::Sender<VaultChange>,
}

fn map_keyring(err: keyring::Error) -> VaultError {
    VaultError::Backend(err.to_string())
}

impl KeychainVault {
    /// Construct a vault wrapping the canonical
    /// `pellucid:secrets-vault:v1` keychain entry.
    ///
    /// # Errors
    /// [`VaultError::Backend`] if the platform keyring refuses to open
    /// (rare; e.g. headless Linux without a Secret Service daemon).
    pub fn new() -> Result<Self, VaultError> {
        let entry = keyring::Entry::new(VAULT_SERVICE, VAULT_USER).map_err(map_keyring)?;
        let (tx, _rx) = watch::channel(VaultChange::Initial);
        Ok(Self { entry, tx })
    }

    /// Construct a vault using a custom service/user pair. Reserved for
    /// integration tests that need to keep each test's keyring entry
    /// isolated.
    ///
    /// # Errors
    /// See [`KeychainVault::new`].
    pub fn with_entry(service: &str, user: &str) -> Result<Self, VaultError> {
        let entry = keyring::Entry::new(service, user).map_err(map_keyring)?;
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
            Err(other) => Err(map_keyring(other)),
        }
    }

    async fn write(&self, blob: &SecretsBlob) -> Result<(), VaultError> {
        let payload = blob.to_json()?;
        self.entry.set_password(&payload).map_err(map_keyring)?;
        let _ = self.tx.send(VaultChange::Updated);
        Ok(())
    }

    async fn clear(&self) -> Result<(), VaultError> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {
                let _ = self.tx.send(VaultChange::Cleared);
                Ok(())
            }
            Err(other) => Err(map_keyring(other)),
        }
    }

    fn subscribe(&self) -> watch::Receiver<VaultChange> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn vault_service_constant_matches_m9_fix_value() {
        assert_eq!(VAULT_SERVICE, "pellucid:secrets-vault:v1");
        assert_eq!(VAULT_USER, "pellucid-host");
    }

    #[tokio::test]
    async fn keychain_vault_constructible_with_isolated_entry() {
        // Ensures the `with_entry` constructor compiles and yields a
        // working subscriber even if the underlying OS keyring is not
        // accessible in CI sandboxes — we never call `read`/`write`.
        let v = KeychainVault::with_entry("pellucid-test:none", "pellucid-test-user").unwrap();
        let mut rx = v.subscribe();
        assert_eq!(*rx.borrow_and_update(), VaultChange::Initial);
    }

    #[tokio::test]
    async fn re_exported_in_memory_vault_round_trips() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            sidecar_token: Some("tok".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(
            v.read().await.unwrap().sidecar_token.as_deref(),
            Some("tok")
        );
    }
}
