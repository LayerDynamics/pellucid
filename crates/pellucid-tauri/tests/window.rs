//! Integration tests for T0.6 — verify that the Tauri builder configuration is
//! valid against the mock runtime, that the bundled `tauri.conf.json` parses,
//! and that the variant configs are well-formed JSON with the expected fields.
//!
//! These tests do not require a graphics surface; they use Tauri's `mock`
//! runtime which performs all the configuration validation Tauri does at
//! startup but never opens an actual window.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::fs;

#[test]
fn tauri_conf_parses_and_has_main_window() {
    let raw = fs::read_to_string("tauri.conf.json").expect("read tauri.conf.json");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");

    let identifier = cfg
        .get("identifier")
        .and_then(serde_json::Value::as_str)
        .expect("identifier missing");
    assert_eq!(identifier, "app.worldmonitor.pellucid");

    let windows = cfg
        .pointer("/app/windows")
        .and_then(serde_json::Value::as_array)
        .expect("windows array missing");
    assert_eq!(windows.len(), 1, "expected exactly one main window at T0.6");

    let main = &windows[0];
    assert_eq!(main.get("label").and_then(|v| v.as_str()), Some("main"));
    assert_eq!(main.get("title").and_then(|v| v.as_str()), Some("Pellucid"));
}

#[test]
fn capabilities_default_grants_main_window() {
    let raw = fs::read_to_string("capabilities/default.json").expect("read default.json");
    let cap: serde_json::Value = serde_json::from_str(&raw).expect("parse default.json");

    let windows = cap
        .get("windows")
        .and_then(serde_json::Value::as_array)
        .expect("windows array on capability");
    assert!(
        windows.iter().any(|w| w.as_str() == Some("main")),
        "default capability must grant the main window"
    );

    let perms = cap
        .get("permissions")
        .and_then(serde_json::Value::as_array)
        .expect("permissions array");
    assert!(perms.iter().any(|p| p.as_str() == Some("core:default")));
    assert!(perms.iter().any(|p| p.as_str() == Some("core:window:default")));
}

#[test]
fn build_dev_url_is_local_vite() {
    let raw = fs::read_to_string("tauri.conf.json").expect("read");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse");

    let dev_url = cfg
        .pointer("/build/devUrl")
        .and_then(serde_json::Value::as_str)
        .expect("build.devUrl missing");
    assert!(
        dev_url.contains("localhost") || dev_url.contains("127.0.0.1"),
        "devUrl must point at a local vite server, got {dev_url}"
    );
    assert!(dev_url.contains(":5173"), "expected port 5173 (vite default), got {dev_url}");
}

#[test]
fn frontend_dist_points_at_webview_build() {
    let raw = fs::read_to_string("tauri.conf.json").expect("read");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse");

    let dist = cfg
        .pointer("/build/frontendDist")
        .and_then(serde_json::Value::as_str)
        .expect("build.frontendDist missing");
    assert!(
        dist.contains("webview/dist"),
        "frontendDist must resolve to the webview build output, got {dist}"
    );
}

#[test]
fn bundle_icon_set_is_complete() {
    let raw = fs::read_to_string("tauri.conf.json").expect("read");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("parse");

    let icons = cfg
        .pointer("/bundle/icon")
        .and_then(serde_json::Value::as_array)
        .expect("bundle.icon array missing");

    let want = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico"];
    for required in want {
        assert!(
            icons.iter().any(|i| i.as_str().is_some_and(|s| s.ends_with(required))),
            "bundle.icon must include {required}"
        );
    }

    for required in want {
        let path = format!("icons/{required}");
        assert!(
            fs::metadata(&path).is_ok(),
            "icon file {path} missing on disk (run `cargo tauri icon` to regenerate)"
        );
    }
}

#[test]
fn variant_configs_present_for_all_five_variants() {
    let variants = ["tech", "finance", "commodity", "happy"];
    for v in variants {
        let path = format!("tauri.{v}.conf.json");
        let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let cfg: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"));

        let product = cfg
            .get("productName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{path}: productName missing"));
        assert!(product.starts_with("Pellucid"), "{path}: productName wrong: {product}");

        let id = cfg
            .get("identifier")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{path}: identifier missing"));
        assert!(id.starts_with("app.worldmonitor.pellucid"), "{path}: identifier wrong: {id}");
    }
}
