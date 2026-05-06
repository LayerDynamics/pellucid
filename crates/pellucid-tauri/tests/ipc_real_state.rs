//! Integration tests for T1.7 IPC commands.
//!
//! These exercise the real `LocalApiState` (driven by the in-memory vault
//! and a synthetic sidecar handle) end-to-end so we cover the
//! `Vault → LocalApiState → command` path. A second test file in T1.9
//! adds a real Tauri runtime harness once the sidecar binary exists.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use pellucid_tauri::{InMemoryVault, LocalApiState, SecretsBlob, SidecarHandle, Variant, Vault};

fn state(default_variant: Variant) -> LocalApiState {
    let sidecar = SidecarHandle::from_port(46_111);
    let vault: Arc<dyn Vault> = Arc::new(InMemoryVault::new());
    LocalApiState::new(sidecar, vault, default_variant)
}

#[tokio::test]
async fn boot_path_refresh_secrets_then_serve_token_to_webview() {
    let st = state(Variant::Base);
    // Sign-in writes the consolidated blob.
    st.vault()
        .write(&SecretsBlob {
            sidecar_token: Some("alpha".into()),
            clerk_session: Some("clerk".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let bundle = st.refresh_secrets().await.unwrap();
    assert_eq!(bundle.sidecar_token.as_deref(), Some("alpha"));
    assert!(bundle.sidecar_token_previous.is_none());

    assert_eq!(st.local_api_port(), 46_111);
    assert_eq!(st.local_api_token().as_deref(), Some("alpha"));
}

#[tokio::test]
async fn rotation_path_set_cached_blob_lets_ipc_see_new_token_without_vault_write() {
    let st = state(Variant::Base);
    st.set_cached_blob(SecretsBlob {
        sidecar_token: Some("rotated".into()),
        sidecar_token_previous: Some("alpha".into()),
        ..Default::default()
    });
    assert_eq!(st.local_api_token().as_deref(), Some("rotated"));
    // Vault is still empty — proves the rotation path works without
    // forcing a keychain write on every tick (T1.8 needs this).
    assert_eq!(st.vault().read().await.unwrap(), SecretsBlob::default());
}

#[tokio::test]
async fn variant_round_trips_through_ipc_state() {
    let st = state(Variant::Base);
    for v in Variant::all() {
        assert_eq!(st.set_variant(*v), *v);
        assert_eq!(st.variant(), *v);
    }
}

#[tokio::test]
async fn validate_external_url_blocks_non_https_schemes() {
    let st = state(Variant::Base);
    for blocked in [
        "http://example.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,...",
        "",
    ] {
        assert!(
            st.validate_external_url(blocked).is_err(),
            "must reject {blocked:?}"
        );
    }
    st.validate_external_url("https://example.com").unwrap();
}

#[tokio::test]
async fn vault_change_subscriber_observes_writes_through_state() {
    let st = state(Variant::Base);
    let mut rx = st.vault().subscribe();
    // Initial value is published synchronously by the watch channel.
    assert!(matches!(
        *rx.borrow_and_update(),
        pellucid_tauri::VaultChange::Initial
    ));
    st.vault()
        .write(&SecretsBlob {
            sidecar_token: Some("x".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    rx.changed().await.unwrap();
    assert!(matches!(
        *rx.borrow_and_update(),
        pellucid_tauri::VaultChange::Updated
    ));
}

#[tokio::test]
async fn updater_check_completes_without_error_on_default_state() {
    let st = state(Variant::Base);
    st.updater_check().unwrap();
}
