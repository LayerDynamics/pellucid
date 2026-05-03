//! pellucid-relay-bin entry point.
//!
//! Today the binary is **just** the C1 startup gate — the AIS /
//! OpenSky / RSS / OREF tasks + scheduler land in T3.10 on top
//! of this same crate. Even today the binary is functional: it
//! validates env, prints the boot decision, and exits. A
//! production deploy that loses `RELAY_SHARED_SECRET` exits
//! non-zero immediately — exactly the behaviour the C1
//! regression suite verifies.
//!
//! `println!` and `eprintln!` are the appropriate way to surface
//! the boot banner / refusal message: the relay's structured
//! logging (`tracing-subscriber`) is initialised AFTER the gate,
//! and we want the refusal text to land on stderr regardless of
//! whether tracing is wired.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use pellucid_relay_bin::{ensure_safe_to_boot, BootDecision, StartupEnv};

/// Process exit code emitted on a refused boot. The integration
/// test pins this so a deploy supervisor (Fly health monitor,
/// Docker `restart: on-failure`) can branch on it.
pub const EXIT_REFUSED: i32 = 78; // EX_CONFIG (BSD sysexits)

fn main() {
    let env = StartupEnv::from_process();
    match ensure_safe_to_boot(&env) {
        Ok(BootDecision::Authorized) => {
            println!(
                "{} {}: authorized startup",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            );
            // T3.10 hooks the AIS / OpenSky / RSS / OREF tasks +
            // the seeder scheduler + the /health Axum surface in
            // here. Today's binary is a strict no-op that proves
            // the C1 gate is wired.
        }
        Ok(BootDecision::UnauthDevMode) => {
            // Loud stderr warning — this branch is dev-only and
            // we want it visible in container logs.
            eprintln!(
                "{} {}: WARNING — running unauthenticated (dev mode); \
                 set RELAY_SHARED_SECRET before deploying",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            );
        }
        Err(refusal) => {
            eprintln!("relay refused to start: {refusal}");
            std::process::exit(EXIT_REFUSED);
        }
    }
}
