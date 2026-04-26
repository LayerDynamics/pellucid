//! App builder — constructs the Tauri runtime configuration.
//!
//! Kept separate from `main.rs` so integration tests in `tests/window.rs`
//! can re-use the same builder configuration without invoking the platform
//! native window.

use std::sync::Arc;

use tauri::{App, Builder, Emitter, Manager, Wry};

use pellucid_tauri::{
    generate_token, ipc, spawn_rotation_loop, InMemoryVault, KeychainVault, LocalApiState,
    SidecarHandle, SystemClock, TokenRotator, Variant, Vault,
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
    tauri::Builder::default()
        .manage(default_local_api_state())
        .invoke_handler(tauri::generate_handler![
            ipc::get_local_api_port,
            ipc::get_local_api_token,
            ipc::refresh_secrets,
            ipc::get_variant,
            ipc::set_variant,
            ipc::request_updater_check,
            ipc::open_external,
        ])
        .setup(setup_main_window)
}

/// Construct the [`LocalApiState`] the IPC handlers consume. Uses the
/// real OS keyring when available and falls back to an in-memory vault
/// in environments where the keyring is not (e.g. CI Linux without
/// secret-service). The sidecar handle is initialised with the
/// configured fallback port; T1.9 swaps it for the real spawned port.
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
    let sidecar = SidecarHandle::from_port(46_123);
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
        let payload = TokenRotatedPayload { at_ms: outcome.at_ms };
        let outcome_for_persist = outcome.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(err) = state.persist_rotation(&outcome_for_persist).await {
                tracing::warn!(
                    target: "pellucid::token_rotation",
                    "persist rotation failed: {err}"
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

    Ok(())
}

/// Run the Tauri event loop. Returns `Err` only if the runtime fails to
/// initialize; once running, this never returns and the process exits via
/// the normal Tauri shutdown path.
pub(crate) fn run() -> Result<(), tauri::Error> {
    builder().run(tauri::generate_context!())
}
