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

#[cfg(feature = "telegram")]
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sidecar::{SidecarHandle, SidecarLaunchError, SidecarSupervisor};
#[cfg(feature = "telegram")]
use crate::telegram_login::{self, LoginCtx, LoginError, RequestCodeResponse, SubmitCodeResponse};
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
///
/// The optional `supervisor` field is the M0 Gate hook — when the host
/// spawns the real `pellucid-sidecar-bin` process it stashes the
/// supervisor here so (a) the Arc keeps the child alive for the
/// lifetime of the app, and (b) the rotation loop can forward
/// `TOKEN_ROTATED` lines to the sidecar's stdin.
#[derive(Clone, Debug)]
pub struct LocalApiState {
    sidecar: SidecarHandle,
    vault: Arc<dyn Vault>,
    variant: Arc<RwLock<Variant>>,
    cached: Arc<RwLock<SecretsBlob>>,
    rotator: Arc<RwLock<Option<Arc<TokenRotator>>>>,
    supervisor: Arc<RwLock<Option<Arc<SidecarSupervisor>>>>,
    /// Login state machine — populated between
    /// `telegram_login_request_code` and the final
    /// `telegram_login_submit_*` call (T4.5.0). `None` while no flow
    /// is in progress. Field present only with the `telegram`
    /// feature; the dep links `grammers → libsql` which collides with
    /// `sqlx → libsqlite3-sys` at link time until the session backend
    /// is moved off libsql.
    #[cfg(feature = "telegram")]
    telegram_login: Arc<tokio::sync::Mutex<Option<LoginCtx>>>,
    /// Disk path for the temporary SQLite session used during login.
    /// Defaults to `<tempdir>/pellucid-telegram-login.session`.
    #[cfg(feature = "telegram")]
    telegram_login_session_path: Arc<PathBuf>,
}

impl LocalApiState {
    /// Construct a new state. `default_variant` is what the IPC layer
    /// reports until [`Self::set_variant`] is called.
    #[must_use]
    pub fn new(sidecar: SidecarHandle, vault: Arc<dyn Vault>, default_variant: Variant) -> Self {
        #[cfg(feature = "telegram")]
        let temp = std::env::temp_dir().join("pellucid-telegram-login.session");
        Self {
            sidecar,
            vault,
            variant: Arc::new(RwLock::new(default_variant)),
            cached: Arc::new(RwLock::new(SecretsBlob::default())),
            rotator: Arc::new(RwLock::new(None)),
            supervisor: Arc::new(RwLock::new(None)),
            #[cfg(feature = "telegram")]
            telegram_login: Arc::new(tokio::sync::Mutex::new(None)),
            #[cfg(feature = "telegram")]
            telegram_login_session_path: Arc::new(temp),
        }
    }

    /// Override the path used for the temporary login-session SQLite
    /// file. Used by tests to keep the file inside a temp directory
    /// they own.
    #[cfg(feature = "telegram")]
    pub fn set_telegram_login_session_path(&mut self, path: PathBuf) {
        self.telegram_login_session_path = Arc::new(path);
    }

    /// Clone of the inner [`SidecarHandle`]. The host needs this when
    /// it composes the rotation-loop callback so it can update the
    /// port without holding a reference to `LocalApiState`.
    #[must_use]
    pub fn sidecar_handle(&self) -> SidecarHandle {
        self.sidecar.clone()
    }

    /// Replace the sidecar port. Called by the host after the real
    /// `pellucid-sidecar-bin` finishes its handshake on a dynamic
    /// port.
    pub fn set_sidecar_port(&self, port: u16) {
        self.sidecar.set_port(port);
    }

    /// Install a [`SidecarSupervisor`] so the host's rotation loop
    /// can forward `TOKEN_ROTATED` control lines to the running
    /// sidecar. The supervisor's `Arc` is also held here so the
    /// child process stays alive for the lifetime of `LocalApiState`.
    pub fn attach_sidecar_supervisor(&self, supervisor: Arc<SidecarSupervisor>) {
        *self.supervisor.write() = Some(supervisor);
    }

    /// Detach the supervisor. After this call
    /// `forward_token_rotation_to_sidecar` becomes a no-op (returns
    /// `Ok(())`). The supervisor itself is dropped on the last
    /// remaining `Arc` so the child terminates.
    pub fn detach_sidecar_supervisor(&self) {
        *self.supervisor.write() = None;
    }

    /// Forward an H1 rotation outcome to the running sidecar's stdin
    /// control channel. Returns `Ok(())` (no-op) when no supervisor
    /// is attached so tests + non-desktop builds can call this
    /// uniformly.
    pub async fn forward_token_rotation_to_sidecar(
        &self,
        current: &str,
        previous: Option<&str>,
    ) -> Result<(), SidecarLaunchError> {
        let sup = self.supervisor.read().clone();
        match sup {
            Some(s) => s.send_token_rotation(current, previous).await,
            None => Ok(()),
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
            (
                blob.sidecar_token.clone(),
                blob.sidecar_token_previous.clone(),
            )
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

    /// Persist new MTProto session bytes into the consolidated vault
    /// entry AND push them to the running sidecar via stdin so the
    /// sidecar's run task picks them up without restart (T4.5.0).
    /// Mirrors `persist_rotation` for the telegram session.
    #[cfg(feature = "telegram")]
    pub async fn persist_telegram_session(&self, bytes: &[u8]) -> Result<(), IpcError> {
        use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
        use base64::Engine as _;
        let encoded = BASE64_STANDARD.encode(bytes);

        let mut blob = self.vault.read().await?;
        if blob.telegram_session.as_deref() == Some(encoded.as_str()) {
            return Ok(());
        }
        blob.telegram_session = Some(encoded);
        self.vault.write(&blob).await?;
        *self.cached.write() = blob;

        // Best-effort sidecar push — failures here are logged but do
        // not break the login flow. The sidecar will pick the bytes up
        // on next start when it reads the keychain via Tauri-host
        // bootstrap.
        let sup = self.supervisor.read().clone();
        if let Some(s) = sup {
            if let Err(err) = s.send_telegram_session_updated(bytes).await {
                tracing::warn!(
                    target: "pellucid::ipc",
                    error = %err,
                    "failed to forward telegram session bytes to sidecar; will retry on next host restart"
                );
            }
        }
        Ok(())
    }

    /// Clear the stored telegram session and notify the sidecar so its
    /// run task drains. Mirrors a logout.
    #[cfg(feature = "telegram")]
    pub async fn clear_telegram_session(&self) -> Result<(), IpcError> {
        let mut blob = self.vault.read().await?;
        if blob.telegram_session.is_none() {
            return Ok(());
        }
        blob.telegram_session = None;
        self.vault.write(&blob).await?;
        *self.cached.write() = blob;

        let sup = self.supervisor.read().clone();
        if let Some(s) = sup {
            if let Err(err) = s.send_telegram_session_cleared().await {
                tracing::warn!(
                    target: "pellucid::ipc",
                    error = %err,
                    "failed to notify sidecar of telegram logout; sidecar will see cleared session on next restart"
                );
            }
        }
        Ok(())
    }

    /// Begin the telegram login flow — call `request_login_code` on a
    /// fresh grammers client, store the in-flight context.
    #[cfg(feature = "telegram")]
    pub async fn telegram_login_begin(
        &self,
        phone: String,
    ) -> Result<RequestCodeResponse, LoginError> {
        let blob = self.vault.read().await?;
        let api_id = blob
            .telegram_api_id
            .ok_or(LoginError::ApiCredentialsMissing)?;
        let api_hash = blob
            .telegram_api_hash
            .clone()
            .ok_or(LoginError::ApiCredentialsMissing)?;

        let session_path = (*self.telegram_login_session_path).clone();
        let (ctx, response) = telegram_login::begin(session_path, api_id, api_hash, phone).await?;

        let mut guard = self.telegram_login.lock().await;
        // Replace any in-flight ctx — caller may have abandoned a
        // previous attempt without going through submit_*.
        if let Some(prev) = guard.take() {
            telegram_login::shutdown(prev).await;
        }
        *guard = Some(ctx);
        Ok(response)
    }

    /// Submit the SMS code. On `Done`, persists the new session bytes
    /// to vault + sidecar and drops the in-flight ctx.
    #[cfg(feature = "telegram")]
    pub async fn telegram_login_submit_code_step(
        &self,
        code: String,
    ) -> Result<SubmitCodeResponse, LoginError> {
        let outcome = {
            let guard = self.telegram_login.lock().await;
            let ctx = guard.as_ref().ok_or(LoginError::NoLoginInFlight)?;
            telegram_login::submit_code(ctx, &code).await?
        };
        match outcome {
            pellucid_telegram::client::LoginCodeOutcome::Done => {
                self.finalize_login_after_success().await?;
                Ok(SubmitCodeResponse {
                    ok: true,
                    needs_password: false,
                })
            }
            pellucid_telegram::client::LoginCodeOutcome::NeedsPassword => {
                Ok(SubmitCodeResponse {
                    ok: false,
                    needs_password: true,
                })
            }
        }
    }

    /// Submit the 2FA password. Persists the new session on success.
    #[cfg(feature = "telegram")]
    pub async fn telegram_login_submit_password_step(
        &self,
        password: String,
    ) -> Result<SubmitCodeResponse, LoginError> {
        {
            let guard = self.telegram_login.lock().await;
            let ctx = guard.as_ref().ok_or(LoginError::NoLoginInFlight)?;
            telegram_login::submit_password(ctx, &password).await?;
        }
        self.finalize_login_after_success().await?;
        Ok(SubmitCodeResponse {
            ok: true,
            needs_password: false,
        })
    }

    /// Helper: read the freshly-minted session bytes from the
    /// in-flight client, persist to vault + sidecar, drop the ctx.
    #[cfg(feature = "telegram")]
    async fn finalize_login_after_success(&self) -> Result<(), LoginError> {
        let bytes = {
            let guard = self.telegram_login.lock().await;
            let ctx = guard.as_ref().ok_or(LoginError::NoLoginInFlight)?;
            telegram_login::current_session_bytes(ctx).await?
        };
        self.persist_telegram_session(&bytes)
            .await
            .map_err(|e| LoginError::Mtproto(e.to_string()))?;
        let prev = {
            let mut guard = self.telegram_login.lock().await;
            guard.take()
        };
        if let Some(ctx) = prev {
            telegram_login::shutdown(ctx).await;
        }
        Ok(())
    }

    /// `true` when the vault carries a non-empty `telegram_session`.
    /// Used by the webview to decide whether to render the onboarding
    /// dialog on first focus of the panel.
    #[cfg(feature = "telegram")]
    pub async fn telegram_session_present(&self) -> Result<bool, IpcError> {
        let blob = self.vault.read().await?;
        Ok(blob
            .telegram_session
            .as_ref()
            .is_some_and(|s| !s.is_empty()))
    }

    /// Spawn a background task that drains
    /// [`SidecarSupervisor::take_telegram_session_rx`] and persists
    /// every received blob into the vault. Called once during host
    /// boot.
    #[cfg(feature = "telegram")]
    pub fn spawn_telegram_session_harvest_loop(
        &self,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> tokio::task::JoinHandle<()> {
        let state = self.clone();
        tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if let Err(err) = state.persist_telegram_session(&bytes).await {
                    tracing::warn!(
                        target: "pellucid::ipc",
                        error = %err,
                        "failed to persist sidecar-reported telegram session"
                    );
                }
            }
        })
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
pub fn open_external(state: tauri::State<'_, LocalApiState>, url: String) -> Result<(), IpcError> {
    state.validate_external_url(&url)
}

// Telegram MTProto auth commands — only registered when the
// `telegram` feature is enabled. With the feature off, the dep
// closure (`grammers → libsql`) would collide with sqlx's
// `libsqlite3-sys` at link time inside the Tauri host binary.
/// `telegram_login_request_code` — start the MTProto auth flow.
#[cfg(feature = "telegram")]
#[tauri::command]
pub async fn telegram_login_request_code(
    state: tauri::State<'_, LocalApiState>,
    phone: String,
) -> Result<RequestCodeResponse, LoginError> {
    state.telegram_login_begin(phone).await
}

/// `telegram_login_submit_code` — submit the SMS code.
#[cfg(feature = "telegram")]
#[tauri::command]
pub async fn telegram_login_submit_code(
    state: tauri::State<'_, LocalApiState>,
    code: String,
) -> Result<SubmitCodeResponse, LoginError> {
    state.telegram_login_submit_code_step(code).await
}

/// `telegram_login_submit_password` — submit the 2FA password.
#[cfg(feature = "telegram")]
#[tauri::command]
pub async fn telegram_login_submit_password(
    state: tauri::State<'_, LocalApiState>,
    password: String,
) -> Result<SubmitCodeResponse, LoginError> {
    state.telegram_login_submit_password_step(password).await
}

/// `telegram_logout` — clear the stored session and notify the sidecar.
#[cfg(feature = "telegram")]
#[tauri::command]
pub async fn telegram_logout(state: tauri::State<'_, LocalApiState>) -> Result<(), IpcError> {
    state.clear_telegram_session().await
}

/// `telegram_session_present` — `true` when the vault carries a
/// non-empty `telegram_session`. Drives the webview's onboarding gate.
#[cfg(feature = "telegram")]
#[tauri::command]
pub async fn telegram_session_present(
    state: tauri::State<'_, LocalApiState>,
) -> Result<bool, IpcError> {
    state.telegram_session_present().await
}

// ============================================================================
// ML IPC commands (mirror of `/api/intelligence/v1/*` for desktop).
//
// Each delegates to the matching `crate::ml::handle_*` body; the
// state object pulls the `Arc<dyn MlEngine>` from the vault on
// first call and caches it. With the keychain unlocked but missing
// keys (`groq_api_key` / `hf_token` unset), every call returns
// `MlIpcError::MissingKey(...)` so the webview can surface an
// explicit "configure ML provider" prompt.
// ============================================================================

/// `ml_embed` — single-text embedding via HuggingFace Inference API.
#[tauri::command]
pub async fn ml_embed(
    state: tauri::State<'_, LocalApiState>,
    ml: tauri::State<'_, crate::ml::MlEngineState>,
    args: crate::ml::EmbedArgs,
) -> Result<crate::ml::EmbedResponse, crate::ml::MlIpcError> {
    crate::ml::handle_embed(&ml, state.vault().as_ref(), args).await
}

/// `ml_batch_embed` — batched embeddings (single HF call).
#[tauri::command]
pub async fn ml_batch_embed(
    state: tauri::State<'_, LocalApiState>,
    ml: tauri::State<'_, crate::ml::MlEngineState>,
    args: crate::ml::BatchEmbedArgs,
) -> Result<crate::ml::BatchEmbedResponse, crate::ml::MlIpcError> {
    crate::ml::handle_batch_embed(&ml, state.vault().as_ref(), args).await
}

/// `ml_sentiment` — Groq-backed sentiment classification.
#[tauri::command]
pub async fn ml_sentiment(
    state: tauri::State<'_, LocalApiState>,
    ml: tauri::State<'_, crate::ml::MlEngineState>,
    args: crate::ml::SentimentArgs,
) -> Result<crate::ml::SentimentResponse, crate::ml::MlIpcError> {
    crate::ml::handle_sentiment(&ml, state.vault().as_ref(), args).await
}

/// `ml_summarize` — Groq-backed article summarization.
#[tauri::command]
pub async fn ml_summarize(
    state: tauri::State<'_, LocalApiState>,
    ml: tauri::State<'_, crate::ml::MlEngineState>,
    args: crate::ml::SummarizeArgs,
) -> Result<crate::ml::SummarizeResponse, crate::ml::MlIpcError> {
    crate::ml::handle_summarize(&ml, state.vault().as_ref(), args).await
}

/// `ml_extract_entities` — Groq-backed named-entity extraction.
#[tauri::command]
pub async fn ml_extract_entities(
    state: tauri::State<'_, LocalApiState>,
    ml: tauri::State<'_, crate::ml::MlEngineState>,
    args: crate::ml::ExtractEntitiesArgs,
) -> Result<crate::ml::ExtractEntitiesResponse, crate::ml::MlIpcError> {
    crate::ml::handle_extract_entities(&ml, state.vault().as_ref(), args).await
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
        assert_eq!(
            s.local_api_token().as_deref(),
            Some(outcome.new_token.as_str())
        );
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

    // ----- M0 Gate: sidecar supervisor wiring -----

    #[test]
    fn set_sidecar_port_updates_handle() {
        let s = fixture(Variant::Base);
        assert_eq!(s.local_api_port(), 46_123);
        s.set_sidecar_port(50_000);
        assert_eq!(s.local_api_port(), 50_000);
    }

    #[test]
    fn sidecar_handle_clone_observes_port_writes() {
        let s = fixture(Variant::Base);
        let h = s.sidecar_handle();
        s.set_sidecar_port(50_500);
        assert_eq!(h.port(), 50_500);
    }

    #[tokio::test]
    async fn forward_token_rotation_is_a_noop_when_no_supervisor_attached() {
        let s = fixture(Variant::Base);
        // Must not error even though no supervisor is attached.
        s.forward_token_rotation_to_sidecar("a", Some("b"))
            .await
            .unwrap();
        s.forward_token_rotation_to_sidecar("a", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn forward_token_rotation_writes_to_attached_supervisor_stdin() {
        // Synthetic sidecar process that captures stdin.
        let program = std::path::PathBuf::from("/bin/sh");
        let path = "/tmp/pellucid-state-stdin-1";
        let _ = std::fs::remove_file(path);
        let args = vec!["-c".to_string(), format!("echo PORT=51200; cat > {path}")];
        let sup = Arc::new(
            crate::sidecar::SidecarSupervisor::spawn(&program, &args)
                .await
                .unwrap(),
        );
        let s = fixture(Variant::Base);
        s.attach_sidecar_supervisor(sup.clone());

        s.forward_token_rotation_to_sidecar("fresh", Some("stale"))
            .await
            .unwrap();
        s.forward_token_rotation_to_sidecar("solo", None)
            .await
            .unwrap();

        // Drop our retained supervisor reference + the one in state
        // so the child sees stdin EOF and the test can read the file.
        s.detach_sidecar_supervisor();
        sup.shutdown().await.unwrap();
        drop(sup);

        for _ in 0..40 {
            if std::path::Path::new(path).exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let captured = std::fs::read_to_string(path).expect("captured stdin");
        let _ = std::fs::remove_file(path);
        let mut lines = captured.lines();
        assert_eq!(lines.next(), Some("TOKEN_ROTATED fresh stale"));
        assert_eq!(lines.next(), Some("TOKEN_ROTATED solo -"));
    }
}
