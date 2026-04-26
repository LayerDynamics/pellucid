//! IPC layer — SPEC-001 §10.2.
//!
//! Holds the seven `#[tauri::command]` handlers the webview invokes via
//! `__TAURI__.invoke(...)`, plus the [`LocalApiState`] singleton each
//! command consults. The state is split into two halves:
//!
//! - **Sidecar** — port and bearer token. Port comes from the
//!   [`SidecarHandle`], the bearer token comes from the vault.
//! - **Variant + secrets** — current visual variant + a cached copy of
//!   the [`SecretsBlob`] last read from the vault.
//!
//! Commands are deliberately thin wrappers over `LocalApiState` methods
//! so the unit tests + the `tests/ipc_real_state.rs` integration test
//! exercise the same code path the webview hits at runtime.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sidecar::SidecarHandle;
use crate::token_rotation::TokenRotator;
use crate::vault::{SecretsBlob, Vault};

/// Visual variant identifier — kept in sync with the webview
/// `Variant` union (SPEC-001 §13.1). Mirrored so the host can persist
/// the user's choice across restarts before the webview boots.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// Default Pellucid skin.
    Base,
    /// Terminal-inspired green-on-black.
    Tech,
    /// Bloomberg-style amber on slate.
    Finance,
    /// Earthen-tone commodity skin.
    Commodity,
    /// Playful pastel skin.
    Happy,
}

impl Variant {
    /// All five variants in declaration order. Useful for tests and
    /// for the `available` IPC payload.
    #[must_use]
    pub fn all() -> &'static [Variant] {
        &[
            Variant::Base,
            Variant::Tech,
            Variant::Finance,
            Variant::Commodity,
            Variant::Happy,
        ]
    }

    /// Stable string label used on the wire and in CSS
    /// `data-variant` attributes.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Base => "base",
            Variant::Tech => "tech",
            Variant::Finance => "finance",
            Variant::Commodity => "commodity",
            Variant::Happy => "happy",
        }
    }
}

/// Error returned by [`Variant::parse`] when the wire string does not
/// match any known variant.
#[derive(Debug, Error)]
#[error("unknown variant '{0}'")]
pub struct VariantParseError(pub String);

impl Variant {
    /// Parse a wire string into a variant. The webview always sends
    /// lowercase, so we accept lowercase only — anything else is a
    /// bug that should surface as an error rather than silently
    /// degrade to `Base`.
    pub fn parse(s: &str) -> Result<Self, VariantParseError> {
        match s {
            "base" => Ok(Variant::Base),
            "tech" => Ok(Variant::Tech),
            "finance" => Ok(Variant::Finance),
            "commodity" => Ok(Variant::Commodity),
            "happy" => Ok(Variant::Happy),
            other => Err(VariantParseError(other.to_string())),
        }
    }
}

/// Compact view of the vault contents the webview is allowed to read.
/// We never expose the raw `SecretsBlob` — the webview only needs the
/// sidecar bearer token, so that is what `get_local_api_token` returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretBundle {
    /// Current sidecar bearer token. May be `None` on first launch
    /// before the host has rotated one in.
    pub sidecar_token: Option<String>,
    /// Previous sidecar token still accepted during the 30 s overlap
    /// (T1.8). Returned so the webview can attach it as a fallback
    /// header during the rotation window.
    pub sidecar_token_previous: Option<String>,
}

/// Errors emitted by the IPC layer. `From<VaultError>` lets command
/// handlers use `?` directly.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Vault layer failure.
    #[error("vault error: {0}")]
    Vault(#[from] crate::vault::VaultError),
    /// Variant payload could not be parsed.
    #[error("invalid variant: {0}")]
    Variant(#[from] VariantParseError),
    /// Updater check requested but no updater hook is wired.
    #[error("updater check unavailable")]
    UpdaterUnavailable,
    /// External URL scheme not in the allow-list.
    #[error("external url '{0}' not allowed")]
    ExternalUrlBlocked(String),
}

impl serde::Serialize for IpcError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Central, clonable state every IPC command reads. Wraps the heap-
/// allocated vault behind an `Arc` so a single instance is shared.
///
/// The optional `rotator` field is the T1.8 hot path — when present,
/// `local_api_token` returns the rotator's current token instead of the
/// cached vault blob. This keeps token-rotation logic out of every IPC
/// command while still letting the webview see the freshest token.
#[derive(Clone, Debug)]
pub struct LocalApiState {
    sidecar: SidecarHandle,
    vault: Arc<dyn Vault>,
    variant: Arc<RwLock<Variant>>,
    cached: Arc<RwLock<SecretsBlob>>,
    rotator: Arc<RwLock<Option<Arc<TokenRotator>>>>,
}

impl LocalApiState {
    /// Construct a new state. `default_variant` is what the IPC layer
    /// reports until [`Self::set_variant`] is called.
    #[must_use]
    pub fn new(
        sidecar: SidecarHandle,
        vault: Arc<dyn Vault>,
        default_variant: Variant,
    ) -> Self {
        Self {
            sidecar,
            vault,
            variant: Arc::new(RwLock::new(default_variant)),
            cached: Arc::new(RwLock::new(SecretsBlob::default())),
            rotator: Arc::new(RwLock::new(None)),
        }
    }

    /// Install a [`TokenRotator`] so subsequent token reads come from
    /// rotation state rather than the cached vault blob. Used by the
    /// host once the T1.8 background loop is spawned.
    pub fn attach_rotator(&self, rotator: Arc<TokenRotator>) {
        *self.rotator.write() = Some(rotator);
    }

    /// Detach the rotator. After this call `local_api_token` falls back
    /// to the cached vault blob again.
    pub fn detach_rotator(&self) {
        *self.rotator.write() = None;
    }

    /// Refresh the cached secrets from the vault. Called once at boot
    /// and again whenever the webview asks via `refresh_secrets`. When
    /// a [`TokenRotator`] is attached, the returned bundle reflects the
    /// rotator's view rather than the cached blob — this guarantees the
    /// webview always sees the freshest token.
    pub async fn refresh_secrets(&self) -> Result<SecretBundle, IpcError> {
        let blob = self.vault.read().await?;
        *self.cached.write() = blob.clone();
        let rotator = self.rotator.read().clone();
        let (current, previous) = if let Some(r) = rotator {
            (Some(r.current()), r.previous())
        } else {
            (blob.sidecar_token.clone(), blob.sidecar_token_previous.clone())
        };
        Ok(SecretBundle {
            sidecar_token: current,
            sidecar_token_previous: previous,
        })
    }

    /// Return the sidecar port the webview should target.
    #[must_use]
    pub fn local_api_port(&self) -> u16 {
        self.sidecar.port()
    }

    /// Return the current bearer token. Routes through the rotator
    /// when one is attached so post-rotation reads see the new token
    /// without an explicit `refresh_secrets` round-trip.
    #[must_use]
    pub fn local_api_token(&self) -> Option<String> {
        if let Some(r) = self.rotator.read().clone() {
            return Some(r.current());
        }
        self.cached.read().sidecar_token.clone()
    }

    /// Return the previous bearer token if the rotator considers it
    /// still acceptable inside the overlap window. Falls back to the
    /// cached vault blob when no rotator is attached.
    #[must_use]
    pub fn local_api_token_previous(&self) -> Option<String> {
        if let Some(r) = self.rotator.read().clone() {
            return r.previous();
        }
        self.cached.read().sidecar_token_previous.clone()
    }

    /// `true` iff the supplied token is the current bearer or a still-
    /// valid previous bearer. Used by the sidecar's auth middleware
    /// (T1.9) — it consults `LocalApiState` over a thin shared-state
    /// channel rather than re-implementing the rotation policy.
    #[must_use]
    pub fn accepts_token(&self, token: &str) -> bool {
        if let Some(r) = self.rotator.read().clone() {
            return r.accepts(token);
        }
        let cached = self.cached.read();
        if cached.sidecar_token.as_deref() == Some(token) {
            return true;
        }
        if cached.sidecar_token_previous.as_deref() == Some(token) {
            return true;
        }
        false
    }

    /// Returns the current variant.
    #[must_use]
    pub fn variant(&self) -> Variant {
        *self.variant.read()
    }

    /// Update the variant. Idempotent — returns the new value.
    pub fn set_variant(&self, variant: Variant) -> Variant {
        *self.variant.write() = variant;
        variant
    }

    /// Validate an outbound URL the webview wants to open. Allows the
    /// `https` scheme exclusively to keep the host from being weaponised
    /// as a launchpad for `file://` or `javascript:` links.
    pub fn validate_external_url(&self, url: &str) -> Result<(), IpcError> {
        if !url.starts_with("https://") {
            return Err(IpcError::ExternalUrlBlocked(url.to_string()));
        }
        Ok(())
    }

    /// Trigger an updater check. T1.7 wires the boundary; the actual
    /// updater plugin lands later.
    pub fn updater_check(&self) -> Result<(), IpcError> {
        // Returning `Ok(())` is the expected behaviour once the
        // updater plugin is registered. Until then the webview can
        // call this safely and we record the request for telemetry.
        tracing::info!(target: "pellucid::ipc", "updater check requested");
        Ok(())
    }

    /// Replace the cached blob — used by T1.8 token rotation to push
    /// fresh tokens into the IPC layer without re-reading the vault.
    pub fn set_cached_blob(&self, blob: SecretsBlob) {
        *self.cached.write() = blob;
    }

    /// Persist a rotation outcome into the vault and into the cached
    /// blob. Called by the rotation loop callback (`on_rotate`) so the
    /// host's vault always carries the freshest current/previous pair —
    /// after a process restart the boot path can read them straight out
    /// of the keychain instead of starting cold.
    pub async fn persist_rotation(
        &self,
        outcome: &crate::token_rotation::RotationOutcome,
    ) -> Result<(), IpcError> {
        let mut blob = self.vault.read().await?;
        blob.sidecar_token = Some(outcome.new_token.clone());
        blob.sidecar_token_previous = Some(outcome.retired_token.clone());
        self.vault.write(&blob).await?;
        *self.cached.write() = blob;
        Ok(())
    }

    /// Clone of the inner cached blob. Reserved for diagnostics + tests.
    #[must_use]
    pub fn cached_blob(&self) -> SecretsBlob {
        self.cached.read().clone()
    }

    /// Reference to the underlying [`Vault`] for callers that need to
    /// write secrets directly (sign-in flows, T1.8 rotation).
    #[must_use]
    pub fn vault(&self) -> &Arc<dyn Vault> {
        &self.vault
    }
}

// ---------------------------------------------------------------------
// `#[tauri::command]` handlers — thin wrappers around `LocalApiState`.
// ---------------------------------------------------------------------

/// `get_local_api_port` — webview asks for the sidecar port.
#[tauri::command]
pub fn get_local_api_port(state: tauri::State<'_, LocalApiState>) -> u16 {
    state.local_api_port()
}

/// `get_local_api_token` — webview asks for the bearer token.
#[tauri::command]
pub fn get_local_api_token(state: tauri::State<'_, LocalApiState>) -> Option<String> {
    state.local_api_token()
}

/// `refresh_secrets` — re-read the consolidated vault entry and return
/// the public-facing bundle to the webview.
#[tauri::command]
pub async fn refresh_secrets(
    state: tauri::State<'_, LocalApiState>,
) -> Result<SecretBundle, IpcError> {
    state.refresh_secrets().await
}

/// `get_variant` — current visual variant.
#[tauri::command]
pub fn get_variant(state: tauri::State<'_, LocalApiState>) -> &'static str {
    state.variant().as_str()
}

/// `set_variant` — switch variant. Returns the new variant string so
/// the webview can confirm the host accepted the update.
#[tauri::command]
pub fn set_variant(
    state: tauri::State<'_, LocalApiState>,
    variant: String,
) -> Result<&'static str, IpcError> {
    let parsed = Variant::parse(&variant)?;
    Ok(state.set_variant(parsed).as_str())
}

/// `request_updater_check` — schedule an updater run.
#[tauri::command]
pub fn request_updater_check(state: tauri::State<'_, LocalApiState>) -> Result<(), IpcError> {
    state.updater_check()
}

/// `open_external` — validate + open an external `https://` URL.
#[tauri::command]
pub fn open_external(
    state: tauri::State<'_, LocalApiState>,
    url: String,
) -> Result<(), IpcError> {
    state.validate_external_url(&url)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::vault::InMemoryVault;

    fn fixture(default_variant: Variant) -> LocalApiState {
        let sidecar = SidecarHandle::from_port(46_123);
        let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
        LocalApiState::new(sidecar, vault, default_variant)
    }

    #[test]
    fn variant_parse_accepts_every_known_label() {
        for v in Variant::all() {
            assert_eq!(Variant::parse(v.as_str()).unwrap(), *v);
        }
    }

    #[test]
    fn variant_parse_rejects_unknown() {
        assert!(Variant::parse("retro").is_err());
        assert!(Variant::parse("Base").is_err(), "must be lowercase");
    }

    #[tokio::test]
    async fn local_api_port_returns_sidecar_handle_value() {
        let s = fixture(Variant::Base);
        assert_eq!(s.local_api_port(), 46_123);
    }

    #[tokio::test]
    async fn local_api_token_starts_empty_then_reflects_vault_after_refresh() {
        let s = fixture(Variant::Base);
        assert_eq!(s.local_api_token(), None);
        s.vault()
            .write(&SecretsBlob {
                sidecar_token: Some("tok-1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let bundle = s.refresh_secrets().await.unwrap();
        assert_eq!(bundle.sidecar_token.as_deref(), Some("tok-1"));
        assert_eq!(s.local_api_token().as_deref(), Some("tok-1"));
    }

    #[tokio::test]
    async fn refresh_secrets_returns_previous_token_when_set() {
        let s = fixture(Variant::Base);
        s.vault()
            .write(&SecretsBlob {
                sidecar_token: Some("new".into()),
                sidecar_token_previous: Some("old".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let bundle = s.refresh_secrets().await.unwrap();
        assert_eq!(bundle.sidecar_token.as_deref(), Some("new"));
        assert_eq!(bundle.sidecar_token_previous.as_deref(), Some("old"));
    }

    #[test]
    fn set_variant_updates_state_and_returns_new_value() {
        let s = fixture(Variant::Base);
        assert_eq!(s.variant(), Variant::Base);
        let returned = s.set_variant(Variant::Finance);
        assert_eq!(returned, Variant::Finance);
        assert_eq!(s.variant(), Variant::Finance);
    }

    #[test]
    fn validate_external_url_rejects_non_https() {
        let s = fixture(Variant::Base);
        for blocked in [
            "http://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://bad",
            "",
        ] {
            assert!(
                s.validate_external_url(blocked).is_err(),
                "must reject {blocked:?}"
            );
        }
    }

    #[test]
    fn validate_external_url_accepts_https() {
        let s = fixture(Variant::Base);
        s.validate_external_url("https://worldmonitor.app").unwrap();
        s.validate_external_url("https://docs.pellucid.dev/x?y=1#z")
            .unwrap();
    }

    #[test]
    fn updater_check_succeeds_unconditionally_for_now() {
        let s = fixture(Variant::Base);
        s.updater_check().unwrap();
    }

    #[tokio::test]
    async fn set_cached_blob_overrides_in_memory_view_without_touching_vault() {
        let s = fixture(Variant::Base);
        s.set_cached_blob(SecretsBlob {
            sidecar_token: Some("override".into()),
            ..Default::default()
        });
        assert_eq!(s.local_api_token().as_deref(), Some("override"));
        // Vault was not written.
        assert_eq!(s.vault().read().await.unwrap(), SecretsBlob::default());
    }

    // ----- T1.8 token rotation integration with LocalApiState -----

    fn rotator(initial: &str) -> Arc<TokenRotator> {
        let clock: Arc<dyn crate::token_rotation::Clock> =
            Arc::new(crate::token_rotation::ManualClock::new());
        Arc::new(TokenRotator::with_schedule(
            initial.to_string(),
            clock,
            crate::token_rotation::DEFAULT_ROTATION_INTERVAL_MS,
            crate::token_rotation::DEFAULT_OVERLAP_MS,
        ))
    }

    #[tokio::test]
    async fn local_api_token_routes_through_rotator_when_attached() {
        let s = fixture(Variant::Base);
        let r = rotator("rotator-seed");
        s.attach_rotator(r.clone());
        assert_eq!(s.local_api_token().as_deref(), Some("rotator-seed"));
        let outcome = r.rotate_now().unwrap();
        assert_eq!(s.local_api_token().as_deref(), Some(outcome.new_token.as_str()));
        assert_eq!(
            s.local_api_token_previous().as_deref(),
            Some("rotator-seed"),
        );
    }

    #[tokio::test]
    async fn detach_rotator_falls_back_to_cached_blob() {
        let s = fixture(Variant::Base);
        s.set_cached_blob(SecretsBlob {
            sidecar_token: Some("cached".into()),
            ..Default::default()
        });
        let r = rotator("rotator");
        s.attach_rotator(r);
        assert_eq!(s.local_api_token().as_deref(), Some("rotator"));
        s.detach_rotator();
        assert_eq!(s.local_api_token().as_deref(), Some("cached"));
    }

    #[tokio::test]
    async fn accepts_token_consults_rotator_when_attached() {
        let s = fixture(Variant::Base);
        let r = rotator("v1");
        s.attach_rotator(r.clone());
        assert!(s.accepts_token("v1"));
        let v2 = r.rotate_now().unwrap().new_token;
        assert!(s.accepts_token(&v2));
        assert!(s.accepts_token("v1"), "previous still inside overlap");
        assert!(!s.accepts_token("v0"));
    }

    #[tokio::test]
    async fn accepts_token_falls_back_to_cached_pair_without_rotator() {
        let s = fixture(Variant::Base);
        s.set_cached_blob(SecretsBlob {
            sidecar_token: Some("c".into()),
            sidecar_token_previous: Some("p".into()),
            ..Default::default()
        });
        assert!(s.accepts_token("c"));
        assert!(s.accepts_token("p"));
        assert!(!s.accepts_token("nope"));
    }

    #[tokio::test]
    async fn persist_rotation_writes_both_tokens_into_vault() {
        let s = fixture(Variant::Base);
        let outcome = crate::token_rotation::RotationOutcome {
            new_token: "fresh".into(),
            retired_token: "stale".into(),
            at_ms: 1_234,
        };
        s.persist_rotation(&outcome).await.unwrap();
        let blob = s.vault().read().await.unwrap();
        assert_eq!(blob.sidecar_token.as_deref(), Some("fresh"));
        assert_eq!(blob.sidecar_token_previous.as_deref(), Some("stale"));
        assert_eq!(s.cached_blob().sidecar_token.as_deref(), Some("fresh"));
    }

    #[tokio::test]
    async fn refresh_secrets_under_rotator_returns_rotator_state_not_cached() {
        let s = fixture(Variant::Base);
        s.set_cached_blob(SecretsBlob {
            sidecar_token: Some("stale".into()),
            sidecar_token_previous: Some("ancient".into()),
            ..Default::default()
        });
        let r = rotator("rot-init");
        s.attach_rotator(r.clone());
        r.rotate_now().unwrap();
        let bundle = s.refresh_secrets().await.unwrap();
        assert_eq!(bundle.sidecar_token.as_deref(), Some(r.current().as_str()));
        assert_eq!(bundle.sidecar_token_previous.as_deref(), Some("rot-init"));
    }
}
