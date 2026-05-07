//! pellucid-sidecar — desktop API server library.
//!
//! Library half of the `pellucid-sidecar-bin` crate. Splitting library
//! from binary lets `tests/echo.rs` boot the real Axum router on a
//! random port and exercise the bearer-auth middleware end-to-end
//! without forking a child process.
//!
//! Module map
//! - [`auth`] — bearer middleware + the `TokenSet` state carrying
//!   current + previous tokens with the SPEC-001 §10.3 overlap policy.
//! - [`echo`] — `/api/echo` route handler (the only T1.9 endpoint).
//! - [`server`] — `build_router`, `serve_on_random_port`, and the
//!   `STDOUT_PORT_PREFIX` constant the host uses for port discovery.

pub mod auth;
pub mod echo;
pub mod server;
pub mod stdin_protocol;

pub use auth::{TokenSet, AUTH_HEADER, BEARER_PREFIX};
pub use echo::EchoResponse;
pub use server::{
    build_handler_state_from_env, build_router, build_router_echo_only, serve_on_random_port,
    ServerHandle, ServerLaunchError, STDOUT_PORT_PREFIX,
};
pub use stdin_protocol::process_stdin_loop;
