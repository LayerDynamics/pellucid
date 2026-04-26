//! `pellucid-tauri` library — exposes the desktop host's IPC layer, the
//! consolidated keychain vault, and the sidecar process supervisor so the
//! binary entry point and the integration tests can both build against the
//! same surface.
//!
//! The crate is dual-targeted: `src/main.rs` contains the desktop binary,
//! `src/lib.rs` (this file) is what tests and any future helpers compile
//! against. Splitting the lib out lets `tests/ipc_real_state.rs` exercise
//! the real `LocalApiState` without spinning up the Tauri runtime.
//!
//! Module map
//! - [`vault`] — `Vault` trait + `KeychainVault` (real OS keyring) and
//!   `InMemoryVault` (test). SPEC-001 §10.4 + M9 fix (single consolidated
//!   `pellucid:secrets-vault:v1` entry, change-listener channel).
//! - [`sidecar`] — `SidecarSupervisor` spawns and monitors
//!   `pellucid-sidecar-bin`, parses the `PORT=<n>` line printed on stdout
//!   so the desktop host can route webview API calls dynamically.
//! - [`ipc`] — `LocalApiState` plus the seven `#[tauri::command]` handlers
//!   listed in SPEC-001 §10.2 (`get_local_api_port`,
//!   `get_local_api_token`, `refresh_secrets`, `get_variant`,
//!   `set_variant`, `request_updater_check`, `open_external`).

pub mod ipc;
pub mod sidecar;
pub mod token_rotation;
pub mod vault;

pub use ipc::{LocalApiState, SecretBundle, Variant, VariantParseError};
pub use sidecar::{SidecarHandle, SidecarLaunchError, SidecarSupervisor};
pub use token_rotation::{
    generate_token, spawn_rotation_loop, Clock, ManualClock, RotationOutcome, SystemClock,
    TokenGenError, TokenRotator, DEFAULT_OVERLAP_MS, DEFAULT_ROTATION_INTERVAL_MS, TOKEN_BYTES,
};
pub use vault::{
    InMemoryVault, KeychainVault, SecretsBlob, Vault, VaultChange, VaultError,
    VAULT_SERVICE, VAULT_USER,
};
