//! Tauri-host MTProto auth flow.
//!
//! Wraps `pellucid_telegram::GrammersClient` (the same client
//! the run task uses) with the three-step login state machine the
//! webview drives via IPC commands. The host owns the [`LoginCtx`]
//! between calls — it is parked under
//! `LocalApiState::telegram_login` so consecutive
//! `telegram_login_request_code` → `telegram_login_submit_code`
//! → `telegram_login_submit_password` calls share state.
//!
//! After a successful login the freshly-minted session bytes are
//! persisted into the OS keychain via
//! `LocalApiState::persist_telegram_session` and forwarded to the
//! sidecar via `SidecarSupervisor::send_telegram_session_updated` so
//! the running run task picks up the new credentials without process
//! restart.

use std::path::PathBuf;
use std::sync::Arc;

use pellucid_telegram::client::{GrammersClient, GrammersClientError, LoginCodeOutcome};
use pellucid_telegram::session::SessionStoreError;
use pellucid_telegram::MtprotoClient;
use serde::Serialize;
use thiserror::Error;

/// State held between login steps. After `submit_password` (or
/// `submit_code` returning `Done`) the ctx is dropped — the next
/// `request_code` call rebuilds it.
pub struct LoginCtx {
    pub(crate) client: Arc<dyn MtprotoClient>,
    pub(crate) session_path: PathBuf,
    pub(crate) phone: String,
}

impl std::fmt::Debug for LoginCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginCtx")
            .field("session_path", &self.session_path)
            .field("phone", &self.phone)
            .finish_non_exhaustive()
    }
}

/// Outcome the IPC layer returns to the webview from
/// `telegram_login_submit_code`. The webview branches on
/// `needs_password` to decide whether to show the 2FA password input.
#[derive(Clone, Debug, Serialize)]
pub struct SubmitCodeResponse {
    /// `true` when the underlying session is now authenticated and the
    /// vault has the new bytes.
    pub ok: bool,
    /// `true` when the account has 2FA — webview must follow up with
    /// `telegram_login_submit_password`.
    pub needs_password: bool,
}

/// Outcome returned from `telegram_login_request_code`. The webview
/// uses `phone` to display feedback ("we sent a code to <phone>") but
/// does not need to send anything back to the host — `submit_code`
/// references the in-flight context the host already holds.
#[derive(Clone, Debug, Serialize)]
pub struct RequestCodeResponse {
    /// Echo of the phone number the host accepted.
    pub phone: String,
}

/// Errors returned by the login state machine. Each variant carries a
/// stable `code` field on the wire so the webview can branch on it.
#[derive(Debug, Error)]
pub enum LoginError {
    /// Vault has no `telegram_api_id` / `telegram_api_hash` yet — the
    /// user has to enter them before the host can call
    /// `request_login_code`. The webview surfaces a separate
    /// "configure API credentials" prompt.
    #[error("api credentials missing in vault — set telegram_api_id + telegram_api_hash")]
    ApiCredentialsMissing,
    /// `submit_code` was called without a preceding `request_code`.
    #[error("no login flow in progress — call request_code first")]
    NoLoginInFlight,
    /// The phone number failed Telegram's basic format check.
    #[error("phone number rejected by Telegram")]
    PhoneInvalid,
    /// Wrong SMS code.
    #[error("SMS code rejected by Telegram")]
    CodeInvalid,
    /// Wrong 2FA password.
    #[error("2FA password rejected — restart the login flow")]
    PasswordInvalid,
    /// Underlying grammers / MTProto failure (network, DC migration,
    /// transient flood-wait outside the login budget).
    #[error("mtproto: {0}")]
    Mtproto(String),
    /// Vault layer rejected a session write.
    #[error("vault: {0}")]
    Vault(#[from] pellucid_core::vault::VaultError),
    /// Session store layer rejected a save.
    #[error("session store: {0}")]
    SessionStore(#[from] SessionStoreError),
}

impl LoginError {
    /// Stable error-code string the webview branches on. Mirrored in
    /// `webview/src/data/loaders/intel/telegramLogin.ts`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ApiCredentialsMissing => "api_credentials_missing",
            Self::NoLoginInFlight => "no_login_in_flight",
            Self::PhoneInvalid => "phone_invalid",
            Self::CodeInvalid => "code_invalid",
            Self::PasswordInvalid => "password_invalid",
            Self::Mtproto(_) => "mtproto",
            Self::Vault(_) => "vault",
            Self::SessionStore(_) => "session_store",
        }
    }
}

impl Serialize for LoginError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut state = ser.serialize_struct("LoginError", 2)?;
        state.serialize_field("code", &self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

fn map_grammers(err: GrammersClientError) -> LoginError {
    match &err {
        GrammersClientError::SignIn(msg) if msg.contains("InvalidCode") => LoginError::CodeInvalid,
        GrammersClientError::Invocation(msg) if msg.contains("PHONE_NUMBER_INVALID") => {
            LoginError::PhoneInvalid
        }
        GrammersClientError::Invocation(msg) if msg.contains("PASSWORD_HASH_INVALID") => {
            LoginError::PasswordInvalid
        }
        _ => LoginError::Mtproto(err.to_string()),
    }
}

/// Begin a login flow. Builds a fresh [`GrammersClient`] over the
/// session at `session_path`, calls `request_login_code`, and returns
/// a [`LoginCtx`] the caller stores between calls. `api_id` /
/// `api_hash` come from the vault; the caller is expected to surface
/// [`LoginError::ApiCredentialsMissing`] if they are absent.
///
/// # Errors
/// See [`LoginError`].
pub async fn begin(
    session_path: PathBuf,
    api_id: i32,
    api_hash: String,
    phone: String,
) -> Result<(LoginCtx, RequestCodeResponse), LoginError> {
    if phone.trim().is_empty() {
        return Err(LoginError::PhoneInvalid);
    }
    // Always start from a clean session file — the user may have
    // partial state from an aborted earlier attempt.
    let _ = std::fs::remove_file(&session_path);
    let client = GrammersClient::connect(&session_path, None, api_id, api_hash)
        .await
        .map_err(map_grammers)?;
    let arc_client: Arc<dyn MtprotoClient> = Arc::new(client);
    arc_client
        .login_request_code(&phone)
        .await
        .map_err(map_grammers)?;
    Ok((
        LoginCtx {
            client: arc_client,
            session_path,
            phone: phone.clone(),
        },
        RequestCodeResponse { phone },
    ))
}

/// Submit the SMS code. Returns `Done` (login complete, caller should
/// drop the [`LoginCtx`] and persist the session bytes) or
/// `NeedsPassword` (caller follows up with [`submit_password`]).
///
/// # Errors
/// See [`LoginError`].
pub async fn submit_code(ctx: &LoginCtx, code: &str) -> Result<LoginCodeOutcome, LoginError> {
    if code.trim().is_empty() {
        return Err(LoginError::CodeInvalid);
    }
    ctx.client
        .login_submit_code(code)
        .await
        .map_err(map_grammers)
}

/// Submit the 2FA password. On success the underlying session is
/// authenticated and `current_session_bytes` returns the persistable
/// bytes.
///
/// # Errors
/// See [`LoginError`].
pub async fn submit_password(ctx: &LoginCtx, password: &str) -> Result<(), LoginError> {
    if password.is_empty() {
        return Err(LoginError::PasswordInvalid);
    }
    ctx.client
        .login_submit_password(password)
        .await
        .map_err(map_grammers)
}

/// Read the freshly-minted session bytes from the in-flight client.
/// Called by the IPC layer after a successful `submit_code` or
/// `submit_password` so the bytes can land in vault + sidecar.
///
/// # Errors
/// See [`LoginError`].
pub async fn current_session_bytes(ctx: &LoginCtx) -> Result<Vec<u8>, LoginError> {
    ctx.client
        .current_session_bytes()
        .await
        .map_err(map_grammers)
}

/// Drop the in-flight client gracefully. Called by the IPC layer
/// after a successful login so the in-process gramers connection
/// closes cleanly.
pub async fn shutdown(ctx: LoginCtx) {
    ctx.client.shutdown().await;
    let _ = std::fs::remove_file(&ctx.session_path);
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn login_error_codes_are_stable() {
        assert_eq!(
            LoginError::ApiCredentialsMissing.code(),
            "api_credentials_missing"
        );
        assert_eq!(LoginError::NoLoginInFlight.code(), "no_login_in_flight");
        assert_eq!(LoginError::PhoneInvalid.code(), "phone_invalid");
        assert_eq!(LoginError::CodeInvalid.code(), "code_invalid");
        assert_eq!(LoginError::PasswordInvalid.code(), "password_invalid");
        assert_eq!(LoginError::Mtproto("x".into()).code(), "mtproto");
    }

    #[test]
    fn login_error_serializes_with_code_and_message_fields() {
        let err = LoginError::CodeInvalid;
        let v: serde_json::Value = serde_json::to_value(&err).unwrap();
        assert_eq!(v.get("code").unwrap().as_str().unwrap(), "code_invalid");
        assert!(v.get("message").is_some());
    }

    #[test]
    fn submit_code_response_round_trips_through_serde() {
        let r = SubmitCodeResponse {
            ok: true,
            needs_password: false,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("ok").unwrap().as_bool().unwrap());
        assert!(!json.get("needs_password").unwrap().as_bool().unwrap());
    }
}
