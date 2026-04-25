//! pellucid-tauri
//!
//! Tauri 2 desktop host. At T0.6 this is the **window-only scaffold** — it
//! constructs a `tauri::Builder`, installs `tracing-subscriber`, and runs the
//! single main window pointed at the Vite dev server (or the bundled webview
//! `dist/` in release builds). Subsequent tasks layer in capabilities:
//!
//! * T1.7 — IPC commands (`get_local_api_port`, `get_local_api_token`,
//!   `refresh_secrets`, `get_variant`, `set_variant`, `request_updater_check`,
//!   `open_external`) and the consolidated keychain vault.
//! * T1.8 — rotated-token state (H1 fix from `docs/specs/SPEC-001-pellucid-stack-rebuild.md` §10.3).
//! * T1.9 — sidecar spawn + dynamic-port discovery.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use tracing_subscriber::EnvFilter;

mod app;

fn main() {
    init_tracing();

    if let Err(err) = app::run() {
        eprintln!("pellucid-tauri: fatal: {err}");
        std::process::exit(1);
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("PELLUCID_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tauri=warn,wry=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_level(true)
        .try_init();
}

/// Returns the Tauri identifier the app is built against, for diagnostics.
#[must_use]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn product_identifier() -> &'static str {
    "app.worldmonitor.pellucid"
}

/// Re-exported so integration tests can assert on the window label.
#[must_use]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn primary_window_label() -> &'static str {
    "main"
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn identifier_matches_tauri_conf() {
        let raw = include_str!("../tauri.conf.json");
        let cfg: serde_json::Value = serde_json::from_str(raw).expect("parse tauri.conf.json");
        let id = cfg
            .get("identifier")
            .and_then(serde_json::Value::as_str)
            .expect("identifier field present");
        assert_eq!(id, product_identifier());
    }

    #[test]
    fn primary_window_label_matches_conf() {
        let raw = include_str!("../tauri.conf.json");
        let cfg: serde_json::Value = serde_json::from_str(raw).unwrap();
        let label = cfg
            .pointer("/app/windows/0/label")
            .and_then(serde_json::Value::as_str)
            .expect("primary window label present");
        assert_eq!(label, primary_window_label());
    }

    #[test]
    fn variant_configs_inherit_base_schema() {
        for variant in ["tech", "finance", "commodity", "happy"] {
            let path = format!("crates/pellucid-tauri/tauri.{variant}.conf.json");
            let raw = std::fs::read_to_string(&path)
                .or_else(|_| std::fs::read_to_string(format!("tauri.{variant}.conf.json")))
                .unwrap_or_else(|e| panic!("read {variant}: {e}"));
            let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse variant");
            assert!(
                cfg.get("productName")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| s.starts_with("Pellucid")),
                "variant {variant} productName missing or wrong prefix"
            );
            assert!(
                cfg.get("identifier")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| s.starts_with("app.worldmonitor.pellucid")),
                "variant {variant} identifier missing or wrong namespace"
            );
        }
    }
}
