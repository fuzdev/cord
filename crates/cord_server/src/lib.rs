//! `cord_server`: the cord backend, an API-only server over the fuz Rust
//! spine (`fuz_db`, `fuz_auth`, `fuz_http`, `fuz_realtime`, `fuz_actions`).
//! It serves no static files; the static frontend is served separately.
//!
//! - `/health` — liveness; `/` — a plain-text pointer to it
//! - `/ready` — readiness, a schema-drift probe over `expected_schema.json`
//! - `/api/account/*` — the spine's account surface (status, login, logout,
//!   password, token-gated bootstrap, invite-gated signup)
//! - `/api/rpc` — JSON-RPC over the action registry (protocol, account,
//!   admin, and cord actions); POST takes any action, GET only side-effect-free
//!   reads
//! - `/api/ws` — the realtime WebSocket over the same registry
//!
//! [`app::CordApp`] holds the long-lived handles; [`router`] carves narrow route
//! states out of it; [`runtime::run_app`] owns the process lifecycle.
//!
//! Cords are cells: [`cord_queries`] reads and writes them over the spine's
//! `fuz_cell` tables, [`roles`] declares the cord-scoped membership roles,
//! [`cord_authz`] checks them with exact scope, and [`cord_actions`] exposes
//! the `cord_*` verbs. [`offer_authorize`] restates, as defense in depth,
//! the spine's refusal to offer a scoped builtin role.

pub mod app;
pub mod cord_actions;
pub mod cord_authz;
pub mod cord_queries;
pub mod env;
pub mod error;
pub mod offer_authorize;
pub mod roles;
pub mod router;
pub mod runtime;
