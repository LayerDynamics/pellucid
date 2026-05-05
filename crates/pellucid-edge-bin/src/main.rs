//! pellucid-edge-bin entry point.
//!
//! Boots the public API surface:
//!   1. Parse env via `Config::parse(ConfigSource::from_process())`.
//!   2. `build_app` opens SQLite, wires the gateway + handlers.
//!   3. Bind `tokio::net::TcpListener` on `cfg.listen_addr`.
//!   4. Serve forever via `axum::serve`.
//!
//! `println!`/`eprintln!` are the appropriate way to surface
//! startup banners + boot errors before the structured log
//! subscriber is initialised.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use pellucid_edge_bin::{build_app, Config, ConfigSource};

const EXIT_CONFIG: i32 = 78;
const EXIT_RUNTIME: i32 = 1;

/// rustls 0.23 refuses to auto-pick a `CryptoProvider` when
/// multiple are reachable in the dep graph (we end up with both
/// `aws-lc-rs` and `ring` paths via different transitive
/// crates). Without an explicit install the first TLS handshake
/// — the aviationstack call inside the gateway — would panic
/// the worker thread. Install once at boot, before anything
/// touches TLS.
fn install_crypto_provider() {
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        eprintln!(
            "{} {}: CryptoProvider already installed (continuing)",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        );
    }
}

#[tokio::main]
async fn main() {
    install_crypto_provider();
    let src = ConfigSource::from_process();
    let cfg = match Config::parse(&src) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(EXIT_CONFIG);
        }
    };

    let (app, _pool) = match build_app(&cfg).await {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("boot error: {e}");
            std::process::exit(EXIT_RUNTIME);
        }
    };

    let listener = match tokio::net::TcpListener::bind(cfg.listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("listen on {} failed: {e}", cfg.listen_addr);
            std::process::exit(EXIT_RUNTIME);
        }
    };
    println!(
        "{} {}: listening on {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        cfg.listen_addr,
    );

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("server crashed: {e}");
        std::process::exit(EXIT_RUNTIME);
    }
}
