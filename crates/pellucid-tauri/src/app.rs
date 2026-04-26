//! App builder — constructs the Tauri runtime configuration.
//!
//! Kept separate from `main.rs` so integration tests in `tests/window.rs`
//! can re-use the same builder configuration without invoking the platform
//! native window.

use std::sync::Arc;

use tauri::{App, Builder, Manager, Wry};

use pellucid_tauri::{
    ipc, InMemoryVault, KeychainVault, LocalApiState, SidecarHandle, Variant, Vault,
};

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

/// Setup hook invoked once the runtime is ready. At T0.6 the only job is to
/// confirm the main window exists and log its initial size; later tasks add
/// the sidecar spawn and IPC initialization.
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

    Ok(())
}

/// Run the Tauri event loop. Returns `Err` only if the runtime fails to
/// initialize; once running, this never returns and the process exits via
/// the normal Tauri shutdown path.
pub(crate) fn run() -> Result<(), tauri::Error> {
    builder().run(tauri::generate_context!())
}
