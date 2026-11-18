//! `cord_server`: the cord backend, an API-only server over the fuz Rust
//! spine (`fuz_db`, `fuz_auth`, `fuz_http`, `fuz_realtime`, `fuz_actions`,
//! `fuz_cell`). It serves no static files; the static frontend is served
//! separately.
//!
//! - `/health` — liveness; `/` — a plain-text pointer to it
//! - `/ready` — readiness, a schema-drift probe over `expected_schema.json`
//! - `/api/account/*` — the spine's account surface (status, login, logout,
//!   password, token-gated bootstrap, invite-gated signup)
//! - `/api/rpc` — JSON-RPC over the action registry: the account, admin,
//!   role-grant offer, and cord actions; POST takes any of them, GET only
//!   side-effect-free reads. The registry also holds the protocol actions,
//!   which work only over `/api/ws`.
//! - `/api/ws` — the realtime WebSocket over the same registry
//! - `/api/admin/audit/stream` — the audit log as server-sent events, for
//!   instance admins over a cookie session
//!
//! [`app::CordApp`] holds the long-lived handles — the spine's
//! `fuz_actions::server::AppBackend` beside cord's invite limiter; [`router`]
//! carves narrow route states out of it; [`runtime::run_app`] owns the process
//! lifecycle, over the spine's server toolkit (`fuz_actions::server`, whose
//! boot refuses a database not at READ COMMITTED, which the membership
//! serialization relies on).
//!
//! Cords are cells: [`cord_queries`] reads and writes them over the spine's
//! `fuz_cell` tables, [`roles`] declares the cord-scoped membership roles,
//! [`cord_authz`] checks them with exact scope, and [`cord_actions`] exposes
//! the `cord_*` verbs. Membership changes live in [`cord_member_queries`]
//! and [`cord_member_actions`]; a cord's spaces, child cells of the cord's,
//! in [`cord_space_queries`] and [`cord_space_actions`]; invite links, in
//! [`invite`], over cord's own `cord_invite` and `cord_invite_redemption`
//! tables ([`migrations`]).

pub mod app;
pub mod cord_actions;
pub mod cord_authz;
pub mod cord_member_actions;
pub mod cord_member_queries;
pub mod cord_queries;
pub mod cord_space_actions;
pub mod cord_space_queries;
pub mod invite;
pub mod migrations;
pub mod roles;
pub mod router;
pub mod runtime;
