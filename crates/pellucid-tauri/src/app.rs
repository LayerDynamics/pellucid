//! App builder — constructs the Tauri runtime configuration.
//!
//! Kept separate from `main.rs` so integration tests in `tests/window.rs`
//! can re-use the same builder configuration without invoking the platform
//! native window.

use std::sync::Arc;

use tauri::{App, Builder, Emitter, Manager, Wry};

use pellucid_tauri::{
    generate_token, ipc, resolve_sidecar_binary_path, spawn_rotation_loop, InMemoryVault,
    KeychainVault, LocalApiState, MlEngineState, SidecarHandle, SidecarSupervisor, SystemClock,
    TokenRotator, Variant, Vault,
};

/// Tauri event emitted to the webview after every successful rotation.
const TOKEN_ROTATED_EVENT: &str = "token_rotated";

/// Payload of [`TOKEN_ROTATED_EVENT`].
#[derive(Clone, serde::Serialize)]
struct TokenRotatedPayload {
    /// Monotonic time of the rotation (rotator's clock).
    at_ms: u64,
}

/// Build the production [`Builder`] used by [`run`]. Re-exported so tests can
/// re-construct the exact same configuration against the mock runtime.
pub(crate) fn builder() -> Builder<Wry> {
    // The two `invoke_handler` arms differ only in whether the
    // telegram commands are registered. With the `telegram` feature
    // off (default), `pellucid-telegram` is not in the link unit
    // (avoids `grammers → libsql` colliding with sqlx's
    // `libsqlite3-sys` at link time) and the webview cannot reach
    // the MTProto auth flow.
    let b = tauri::Builder::default()
        .manage(default_local_api_state())
        // ML engine state — lazily builds an `Arc<dyn MlEngine>` from
        // the vault on first call and caches it. Cheap to manage as
        // long as no webview ever invokes `ml_*`.
        .manage(MlEngineState::new());
    #[cfg(not(feature = "telegram"))]
    let b = b.invoke_handler(tauri::generate_handler![
        ipc::get_local_api_port,
        ipc::get_local_api_token,
        ipc::refresh_secrets,
        ipc::get_variant,
        ipc::set_variant,
        ipc::request_updater_check,
        ipc::open_external,
        ipc::ml_embed,
        ipc::ml_batch_embed,
        ipc::ml_sentiment,
        ipc::ml_summarize,
        ipc::ml_extract_entities,
    ]);
    #[cfg(feature = "telegram")]
    let b = b.invoke_handler(tauri::generate_handler![
        ipc::get_local_api_port,
        ipc::get_local_api_token,
        ipc::refresh_secrets,
        ipc::get_variant,
        ipc::set_variant,
        ipc::request_updater_check,
        ipc::open_external,
        ipc::ml_embed,
        ipc::ml_batch_embed,
        ipc::ml_sentiment,
        ipc::ml_summarize,
        ipc::ml_extract_entities,
        ipc::telegram_login_request_code,
        ipc::telegram_login_submit_code,
        ipc::telegram_login_submit_password,
        ipc::telegram_logout,
        ipc::telegram_session_present,
    ]);
    b.setup(setup_main_window)
}

/// Pre-discovery placeholder port. Replaced with the real bound
/// port via `LocalApiState::set_sidecar_port` once the sidecar
/// finishes its `PORT=<n>` handshake.
const PRE_DISCOVERY_SIDECAR_PORT: u16 = 0;

/// Construct the [`LocalApiState`] the IPC handlers consume. Uses the
/// real OS keyring when available and falls back to an in-memory vault
/// in environments where the keyring is not (e.g. CI Linux without
/// secret-service). The sidecar handle is initialised with port 0 so
/// any IPC reads before the supervisor's `PORT=<n>` handshake clearly
/// indicate the sidecar has not yet bound.
pub(crate) fn default_local_api_state() -> LocalApiState {
    let vault: Arc<dyn Vault> = match KeychainVault::new() {
        Ok(v) => Arc::new(v),
        Err(err) => {
            tracing::warn!(
                target: "pellucid::vault",
                "keychain unavailable, falling back to in-memory vault: {err}"
            );
            Arc::new(InMemoryVault::new())
        }
    };
    let sidecar = SidecarHandle::from_port(PRE_DISCOVERY_SIDECAR_PORT);
    LocalApiState::new(sidecar, vault, Variant::Base)
}

/// Setup hook invoked once the runtime is ready. Confirms the main
/// window exists, mints the initial sidecar bearer, attaches a
/// [`TokenRotator`] to [`LocalApiState`], and spawns the background
/// rotation loop. The loop callback persists each rotation into the
/// vault and emits `token_rotated` to the webview so it re-attaches its
/// bearer header.
pub(crate) fn setup_main_window(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let main = app
        .get_webview_window("main")
        .ok_or("primary window 'main' missing from tauri.conf.json")?;

    if let Ok(size) = main.inner_size() {
        tracing::info!(
            window = "main",
            width = size.width,
            height = size.height,
            "pellucid-tauri main window ready"
        );
    }

    let state = app.state::<LocalApiState>();
    let initial = generate_token().map_err(|e| -> Box<dyn std::error::Error> {
        Box::<dyn std::error::Error>::from(format!("token mint failed: {e}"))
    })?;
    let rotator = Arc::new(TokenRotator::new(
        initial.clone(),
        Arc::new(SystemClock::new()),
    ));
    state.attach_rotator(rotator.clone());

    // Eagerly persist the seed token so the webview can see it after a
    // restart even before the first scheduled rotation lands.
    let state_for_seed = state.inner().clone();
    let seed_outcome = pellucid_tauri::RotationOutcome {
        new_token: initial,
        retired_token: String::new(),
        at_ms: 0,
    };
    tauri::async_runtime::spawn(async move {
        if let Err(err) = state_for_seed.persist_rotation(&seed_outcome).await {
            tracing::warn!(target: "pellucid::ipc", "seed persist failed: {err}");
        }
    });

    let app_handle = app.handle().clone();
    let state_for_loop = state.inner().clone();
    spawn_rotation_loop(rotator, move |outcome| {
        let state = state_for_loop.clone();
        let handle = app_handle.clone();
        let payload = TokenRotatedPayload {
            at_ms: outcome.at_ms,
        };
        let outcome_for_persist = outcome.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = state.persist_rotation(&outcome_for_persist).await {
                tracing::warn!(
                    target: "pellucid::token_rotation",
                    "persist rotation failed: {err}"
                );
            }
            // Forward to the running sidecar (no-op until the
            // supervisor finishes its handshake).
            let prev = if outcome_for_persist.retired_token.is_empty() {
                None
            } else {
                Some(outcome_for_persist.retired_token.as_str())
            };
            if let Err(err) = state
                .forward_token_rotation_to_sidecar(&outcome_for_persist.new_token, prev)
                .await
            {
                tracing::warn!(
                    target: "pellucid::token_rotation",
                    "forward TOKEN_ROTATED to sidecar failed: {err}"
                );
            }
            if let Err(err) = handle.emit(TOKEN_ROTATED_EVENT, payload) {
                tracing::warn!(
                    target: "pellucid::token_rotation",
                    "emit token_rotated failed: {err}"
                );
            }
        });
    });

    // Spawn the actual `pellucid-sidecar-bin` on a background task
    // so the `setup_main_window` hook returns quickly. The supervisor
    // parses `PORT=<n>` from stdout, updates `LocalApiState::sidecar`
    // with the discovered port, and stays attached so the rotation
    // loop above can forward control lines into the child's stdin.
    let state_for_sidecar = state.inner().clone();
    let initial_token_for_sidecar = state.inner().local_api_token().unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = launch_sidecar(state_for_sidecar, initial_token_for_sidecar).await {
            tracing::error!(
                target: "pellucid::sidecar",
                "sidecar launch failed: {err}"
            );
        }
    });

    Ok(())
}

/// Launch the sidecar binary, capture its dynamic port, and attach
/// the supervisor to `LocalApiState`. Returns `Err` only if the
/// binary cannot be located or fails its `PORT=<n>` handshake — the
/// app continues running in either case (a missing sidecar makes
/// `/api/*` calls fail but the desktop window is still usable).
async fn launch_sidecar(
    state: LocalApiState,
    initial_token: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bin = sidecar_binary_path()?;
    let env = vec![("PELLUCID_SIDECAR_TOKEN".to_string(), initial_token)];
    let supervisor = SidecarSupervisor::spawn_with_env(&bin, &[], &env).await?;
    let port = supervisor.handle().port();
    state.set_sidecar_port(port);

    // Take the telegram-session receiver BEFORE wrapping the supervisor
    // in an Arc — `take_telegram_session_rx` requires `&self` access
    // and is single-shot. The harvest loop persists every received
    // blob into the OS keychain (T4.5.0). Only when the `telegram`
    // feature is on; otherwise the sidecar is built without
    // `pellucid-telegram` and the harvest loop has nothing to drain.
    #[cfg(feature = "telegram")]
    let telegram_rx = supervisor.take_telegram_session_rx().await;
    state.attach_sidecar_supervisor(Arc::new(supervisor));
    #[cfg(feature = "telegram")]
    if let Some(rx) = telegram_rx {
        // The harvest loop is fire-and-forget; the join handle is
        // intentionally discarded. The task ends only when the mpsc
        // sender (owned by the supervisor's stdout drain task) closes,
        // which happens at process shutdown.
        drop(state.spawn_telegram_session_harvest_loop(rx));
    }

    tracing::info!(
        target: "pellucid::sidecar",
        port,
        bin = %bin.display(),
        "sidecar live on dynamic port"
    );
    Ok(())
}

/// Resolve `pellucid-sidecar-bin` next to the host binary. Search
/// rules live in `pellucid_tauri::sidecar::resolve_sidecar_binary_path`.
fn sidecar_binary_path() -> Result<std::path::PathBuf, std::io::Error> {
    let exe = std::env::current_exe()?;
    let here = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "no parent for current_exe")
    })?;
    resolve_sidecar_binary_path(here)
}

/// Run the Tauri event loop. Returns `Err` only if the runtime fails to
/// initialize; once running, this never returns and the process exits via
/// the normal Tauri shutdown path.
pub(crate) fn run() -> Result<(), tauri::Error> {
    builder().run(tauri::generate_context!())
}
