//! `cord_server`: the cord backend, an API-only server over the fuz Rust
//! spine (`fuz_db`, `fuz_auth`, `fuz_http`, `fuz_realtime`, `fuz_actions`).
//! It serves no static files; the static frontend is served separately.
//!
//! - `/health` — liveness; `/` — a plain-text pointer to it
//! - `/ready` — readiness, a schema-drift probe over `expected_schema.json`
//! - `/api/account/*` — the spine's account surface (status, login, logout,
//!   password, token-gated bootstrap, invite-gated signup)
//! - `/api/rpc` — JSON-RPC over the action registry (protocol, account, and
//!   admin actions); POST takes any action, GET only side-effect-free reads
//! - `/api/ws` — the realtime WebSocket over the same registry
//!
//! [`app::CordApp`] holds the long-lived handles; [`router`] carves narrow route
//! states out of it; [`runtime::run_app`] owns the process lifecycle.

pub mod app;
pub mod env;
pub mod error;
pub mod router;
pub mod runtime;
