//! App builder — constructs the Tauri runtime configuration.
//!
//! Kept separate from `main.rs` so integration tests in `tests/window.rs`
//! can re-use the same builder configuration without invoking the platform
//! native window.

use tauri::{App, Builder, Manager, Wry};

/// Build the production [`Builder`] used by [`run`]. Re-exported so tests can
/// re-construct the exact same configuration against the mock runtime.
pub(crate) fn builder() -> Builder<Wry> {
    tauri::Builder::default().setup(setup_main_window)
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
