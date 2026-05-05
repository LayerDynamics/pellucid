//! pellucid-relay-bin entry point.
//!
//! Boot pipeline (SPEC-001 §17.5 / §24, T3.10):
//!  1. **C1 startup gate** — refuse to start unless the
//!     env-var tuple says we're authorised to serve traffic.
//!     A production deploy that drops `RELAY_SHARED_SECRET`
//!     exits non-zero with `EXIT_REFUSED` (78, BSD `EX_CONFIG`)
//!     before touching any I/O.
//!  2. **Logging** — `tracing-subscriber` is wired only after
//!     the gate decides we're allowed to run.
//!  3. **Build the app** — DB pool, metrics recorder, AIS
//!     pipeline, OpenSky proxy, /health (with the L3 cascade
//!     fix), /metrics. The real seeder scheduler lands in
//!     T3.11 (bootstrap returns 30 hydrated keys); today the
//!     binary boots an empty scheduler so the surface area is
//!     accurate without firing live cycles.
//!  4. **Serve** — `axum::serve` with a graceful-shutdown
//!     signal driven by `Ctrl-C` (SIGINT) and `SIGTERM` on
//!     Unix (Fly's `flyctl deploy` rolls instances by sending
//!     SIGTERM and waiting for the grace period).
//!  5. **Drain** — call `BootedRelay::shutdown` to abort the
//!     AIS + scheduler tasks and wait `shutdown_grace`.
//!
//! `println!` and `eprintln!` are the appropriate way to
//! surface the boot banner / refusal message: structured
//! logging is initialised AFTER the gate, and we want the
//! refusal text to land on stderr regardless of whether
//! tracing is wired.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use pellucid_relay_bin::{
    build_app, ensure_safe_to_boot, BootDecision, Config, ConfigSource, StartupEnv,
};
use tokio::net::TcpListener;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// Process exit code emitted on a refused boot. The C1
/// regression suite pins this so a deploy supervisor (Fly
/// health monitor, Docker `restart: on-failure`) can branch
/// on it.
pub const EXIT_REFUSED: u8 = 78; // EX_CONFIG (BSD sysexits)

/// Process exit code emitted on a runtime failure (DB open,
/// listener bind, app build). Distinct from `EXIT_REFUSED`
/// so an operator can tell config rejection from runtime
/// failure at a glance.
pub const EXIT_RUNTIME: u8 = 70; // EX_SOFTWARE (BSD sysexits)

fn main() -> ExitCode {
    install_crypto_provider();
    let env = StartupEnv::from_process();
    match ensure_safe_to_boot(&env) {
        Ok(BootDecision::Authorized) => {
            println!(
                "{} {}: authorized startup",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            );
        }
        Ok(BootDecision::UnauthDevMode) => {
            eprintln!(
                "{} {}: WARNING — running unauthenticated (dev mode); \
                 set RELAY_SHARED_SECRET before deploying",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            );
        }
        Err(refusal) => {
            eprintln!("relay refused to start: {refusal}");
            return ExitCode::from(EXIT_REFUSED);
        }
    }

    init_tracing();

    let config = match Config::parse(&ConfigSource::from_process()) {
        Ok(c) => c,
        Err(e) => {
            error!("config parse failure: {e}");
            return ExitCode::from(EXIT_REFUSED);
        }
    };

    // The relay binary is intentionally synchronous up to here
    // so the C1 gate doesn't depend on a Tokio runtime. We hand
    // off to a multi-thread runtime for the actual server.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!("tokio runtime build: {e}");
            return ExitCode::from(EXIT_RUNTIME);
        }
    };

    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

/// Install the rustls process-wide `CryptoProvider`.
///
/// rustls 0.23 refuses to auto-pick a provider when more than
/// one is reachable via the dep graph (we end up with both
/// `aws-lc-rs` from hyper-rustls and `ring` paths via transitive
/// crates). Without this call, the first TLS handshake panics
/// the worker thread — which is exactly what AIS WebSocket
/// connections to aisstream.io did on Railway as soon as
/// `AIS_API_KEY` was set and the producer task tried to dial.
///
/// `install_default()` returns `Err` if a provider is already
/// installed (e.g. if a library beat us to it). That's fine —
/// we just log and continue.
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

/// Initialise structured logging. Honours `RUST_LOG`; falls
/// back to `info` for the relay crates.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,pellucid_relay_bin=info,pellucid_streams=info")
    });
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Async boot + serve loop. Returns `Ok(())` on a clean
/// shutdown, `Err(EXIT_RUNTIME)` on a runtime failure.
async fn run(config: Config) -> Result<(), u8> {
    let listen_addr = config.listen_addr;
    let booted = match build_app(config, vec![], vec![]).await {
        Ok(b) => b,
        Err(e) => {
            error!("relay boot failure: {e}");
            return Err(EXIT_RUNTIME);
        }
    };
    let app = booted.app.clone();

    let listener = match TcpListener::bind(listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("listener bind {listen_addr} failed: {e}");
            // Drain whatever we already started before returning.
            let _ = booted.shutdown().await;
            return Err(EXIT_RUNTIME);
        }
    };
    let bound = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| listen_addr.to_string());
    info!(addr = %bound, "relay listening");

    let shutdown = shutdown_signal();
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;

    if let Err(e) = serve_result {
        error!("axum serve error: {e}");
        let _ = booted.shutdown().await;
        return Err(EXIT_RUNTIME);
    }

    info!("draining background tasks");
    let clean = booted.shutdown().await;
    if clean {
        info!("relay exited cleanly");
        Ok(())
    } else {
        warn!("relay shutdown grace expired before tasks drained");
        Err(EXIT_RUNTIME)
    }
}

/// Future that resolves when the OS asks us to terminate.
/// On Unix we listen for both `Ctrl-C` (SIGINT) and SIGTERM
/// (Fly's deploy roller); on other platforms we listen only
/// for `Ctrl-C`.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                warn!("install SIGTERM handler: {e}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
            _ = sigterm.recv() => info!("received SIGTERM"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("received Ctrl-C");
    }
}
