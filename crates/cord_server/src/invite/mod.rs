//! Cord's invite links: the `cord_invite` table and the verbs over it.
//!
//! - [`queries`] — the links themselves: minting, listing, revoking, and
//!   accepting one, which grants `cord_member`
//! - [`redemption_queries`] — newcomer links redeemed for a username, in
//!   `cord_invite_redemption`, ahead of the newcomer's signup and accept
//! - [`sweep`] — the expiry sweep that releases the reservations of newcomer
//!   links expired past the accept grace
//! - [`actions`] — the `cord_invite_*` RPC verbs over them

pub mod actions;
pub mod queries;
pub mod redemption_queries;
pub mod sweep;
