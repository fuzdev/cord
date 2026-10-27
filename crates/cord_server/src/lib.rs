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
//! [`app::CordApp`] holds the long-lived handles; [`router`] carves narrow
//! route states out of it; [`runtime::run_app`] owns the process lifecycle.
//!
//! Cords are cells: [`cord_queries`] reads and writes them over the spine's
//! `fuz_cell` tables, [`roles`] declares the cord-scoped membership roles,
//! [`cord_authz`] checks them with exact scope, and [`cord_actions`] exposes
//! the `cord_*` verbs. Membership changes live in [`cord_member_queries`]
//! and [`cord_member_actions`]; invite links, in [`invite`], over cord's own
//! `cord_invite` and `cord_invite_redemption` tables ([`migrations`]).
//! [`transaction_isolation`] is the boot check
//! that Postgres runs transactions at READ COMMITTED, which the membership
//! serialization relies on.

pub mod app;
pub mod cord_actions;
pub mod cord_authz;
pub mod cord_member_actions;
pub mod cord_member_queries;
pub mod cord_queries;
pub mod env;
pub mod error;
pub mod invite;
pub mod migrations;
pub mod roles;
pub mod router;
pub mod runtime;
pub mod transaction_isolation;
