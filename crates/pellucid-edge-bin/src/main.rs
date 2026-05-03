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

#[tokio::main]
async fn main() {
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
