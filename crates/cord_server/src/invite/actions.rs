//! Cord's invite-link actions.
//!
//! - `cord_invite_create`: the cord's owner or admins. Mints a link with an
//!   optional expiry and use limit and returns its raw token, the only time
//!   the token is ever shown. Audited as `cord_invite_create`. A link that
//!   admits newcomers (`admits_newcomers`) also needs the caller to be an
//!   instance admin (a global `admin` grant) and carries both a use limit
//!   and an expiry, of at most
//!   [`CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`].
//! - `cord_invite_list`: the owner or admins. The cord's links with their
//!   status and redemption counts, never their tokens.
//! - `cord_invite_revoke`: the owner or admins. Idempotent: revoking a
//!   revoked link succeeds and changes nothing. The revoke that revokes also
//!   deletes the reservations still waiting on their signup, and is audited
//!   as `cord_invite_revoke`. An expired link can be revoked too, which
//!   releases its reservations ahead of the expiry sweep
//!   ([`crate::invite::sweep`]).
//! - `cord_invite_preview`: anyone holding a token, signed in or not. The
//!   link's status, whether it admits newcomers, and whether it can be
//!   redeemed right now (`newcomers_open`: active, admitting newcomers, and
//!   its minter still an instance admin). A signed-in caller also sees the
//!   cord's name when the link is active or they're already in the cord; an
//!   anonymous caller sees it only when `newcomers_open`, the one case they
//!   can use the link. A link that isn't active shows nothing about its
//!   cord to anyone outside it.
//! - `cord_invite_redeem`: anyone, identity unused. Reserves a username on
//!   the instance through a newcomer link, spending one of its uses
//!   ([`crate::invite::redemption_queries`]); the newcomer then signs up
//!   through the stock invite-gated signup with that username and accepts
//!   the link. Redeeming a username already waiting on the link answers
//!   `created: false` and spends nothing wherever accept would still take
//!   it (exhausted, expired within the grace, or its minter no longer an
//!   admin). Audited as `cord_invite_redeem` when it reserves.
//! - `cord_invite_accept`: a cookie session only, like the spine's other
//!   self-service grant paths. Grants `cord_member` on the cord and spends a
//!   use, or completes the caller's redemption of the link, whose use was
//!   spent when it was redeemed; audited as the spine's `role_grant_create`
//!   plus `cord_invite_accept`. A caller already in the cord gets their role
//!   back and spends nothing; when that completes their redemption, it's
//!   audited as `cord_invite_accept` with no grant. When the accept claims
//!   the redemption's spine invite (the caller's signup claimed none), the
//!   event names it (`claimed_signup_invite_id`).
//!
//! The managing verbs answer `cord_not_found` to anyone outside the cord, as
//! `cord_get` does; what a token's holder learns is below. No audit event
//! carries a token or its digest.
//!
//! The bounds the declared TS spec's schema puts on create's expiry and use
//! limit and on a token (`CordInviteToken`) are refused here at params
//! validation too, ahead of the handler and so ahead of every gate in it,
//! with the same generic `invalid params` and no reason: the limits before
//! the manager gate and its `cord_not_found` mask (the bounds are public, so
//! refusing them first tells an outsider nothing), and a token before any
//! lookup. Redeem's username is refused there too, after the token, and every
//! refusal of it carries `cord_invite_username_invalid` (the join page reads
//! it), the reason the TS field declares: missing, `null`, not a string, or
//! off the spine's creation grammar.
//!
//! Redeem takes no session: it grants its caller nothing, only reserves a
//! username for an account that doesn't exist yet, so who calls it doesn't
//! matter, and a signed-in caller is treated like anyone else. Refusing
//! sessions would stop nobody, since the same person can call it signed
//! out.
//!
//! Create and revoke read the caller's role afresh inside the action's
//! transaction (`require_cord_manager_fresh`), not from the request's
//! role-grant snapshot, which the dispatcher takes before the transaction
//! opens: an admin removed in that gap can't mint a link to rejoin through.
//! Neither takes the cord's row lock. Accept locks the link and then the
//! cord, so a revoke that locked the cord and then the link could deadlock
//! against it. List is a read, so it trusts the snapshot.
//!
//! Create is rate-limited per account (the spine's
//! [`RateLimitClass::Account`], as on `role_grant_assign`), since each call
//! mints a live credential. Preview and redeem, which anonymous callers
//! reach, are rate-limited per IP ([`RateLimitClass::Ip`], the one class a
//! public action can carry), in the budget the spine's IP class shares
//! across every such action. Redeem answers whether a username is free, so
//! it is also held to its own, tighter per-IP budget,
//! [`CORD_INVITE_REDEEM_RATE_LIMIT`] ([`CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS`]
//! calls per [`CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS`]): a real join takes
//! one or two redeems, and a few more after a taken username. The spine's
//! spec rate limit takes only its fixed classes over the shared limiters, so
//! the redeem handler charges its own limiter first thing, before it parses
//! its input or runs a query (the dispatcher has already opened the
//! transaction by then, which a refusal rolls back), through the spine's
//! `check_action_rate_limit`, which refuses with the dispatcher's own
//! `rate_limited` error (a `retry_after` in seconds in its `data`). A caller
//! over the budget has already spent a unit of the shared budget; the
//! dispatcher charges that before the handler runs. `testing_cordd` builds
//! no limiters ([`fuz_auth::RateLimiterMode::DisabledForTesting`]), this one
//! included.
//!
//! ## What a token tells its holder
//!
//! The token is the secret, so a real token's refusals name their reason
//! (`cord_invite_revoked`, `cord_invite_expired`, `cord_invite_exhausted`,
//! and for redeem `cord_invite_not_for_newcomers` and
//! `cord_invite_newcomers_closed`, all `conflict`), while any other token in
//! bounds gets `cord_invite_not_found`. A token out of bounds (empty, or
//! over [`CORD_INVITE_TOKEN_LENGTH_MAX`] UTF-16 code units) is no secret, so
//! it gets the spec's params refusal instead. A member who can't manage
//! links gets `cord_invite_forbidden`.
//!
//! Past what an anonymous preview shows, redeem on a live newcomer link
//! also tells its caller whether a username is free (no account has it and
//! no unclaimed invite reserves it), the cord's name, and whether this link
//! already holds a waiting reservation for that username, at most
//! [`CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS`] times per IP per window.
//! The rest is bounded:
//!
//! - A dead link (revoked, expired past the accept grace, or not for
//!   newcomers) answers every username alike.
//! - A link that is exhausted, expired within the grace, or closed (its
//!   minter no longer an instance admin) refuses every other username with
//!   that one refusal, checked before the username is, so it says nothing
//!   about whether a name is free. It still answers a waiting username with
//!   `created: false` and the cord's name, which only someone who knows a
//!   waiting username can learn.
//! - A reservation whose username an account took without claiming its
//!   invite (under open signup, or through an older email invite) isn't
//!   answered as waiting: its signup could never succeed, so a redeem of the
//!   name falls through to the new-username path and its refusal, and the
//!   newcomer can pick another name.
//!
//! A reserved username belongs to whoever signs up with it first: the
//! spine's username invites carry no secret, and its signup matches a
//! username-only invite on the username alone. The join page keeps that
//! window short by running redeem and signup back to back, and a revoke or
//! the expiry sweep ([`crate::invite::sweep`]) deletes what a link still
//! reserves. Accept's fallback, which claims a waiting reservation for an
//! account whose signup claimed none, trusts the username no more than the
//! spine's own claim would have.

use std::sync::Arc;

use deadpool_postgres::GenericClient;
use fuz_actions::{ActionContext, ActionSpec};
use fuz_auth::primitive_schemas::{USERNAME_LENGTH_MAX, USERNAME_LENGTH_MIN};
use fuz_auth::{
    AuditEventType, AuthSpec, CredentialGate, DEFAULT_RATE_LIMITER_MAX_KEYS, RateLimitClass,
    RateLimiter, RateLimiterOptions, RequestActorContext, is_valid_username_for_creation,
    parse_str_param, reject_unknown_keys,
};
use fuz_http::{
    JsonrpcError, conflict, forbidden, internal_error_with_source, invalid_input_params,
    invalid_params, not_found, parse_strict_params,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cord_actions::{
    CordGrantAudit, audit_cord_event, audit_cord_grant_event, cord_not_found, dispatch_db,
    dispatch_db_ctx, handler, require_actor, require_cord_manager_fresh, to_cord_json,
};
use crate::cord_authz::{CordRole, cord_role_in};
use crate::cord_queries::query_cord_get;
use crate::invite::queries::{
    CORD_INVITE_EXPIRES_IN_SECONDS_MAX, CORD_INVITE_EXPIRES_IN_SECONDS_MIN,
    CORD_INVITE_MAX_USES_MAX, CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX,
    CORD_INVITE_TOKEN_LENGTH_MAX, CordInviteError, CordInviteRevoked, CordInviteRow,
    CordInviteStatus, CordInviteWithCounts, generate_cord_invite_token, hash_cord_invite_token,
    is_cord_invite_token_in_bounds, query_cord_invite_accept, query_cord_invite_by_token_hash,
    query_cord_invite_create, query_cord_invite_list, query_cord_invite_revoke,
};
use crate::invite::redemption_queries::{
    query_actor_holds_global_admin, query_actor_holds_global_admin_unlocked,
    query_cord_invite_redeem,
};

/// Error reason: no link has that token (or id, on this cord).
pub const ERROR_CORD_INVITE_NOT_FOUND: &str = "cord_invite_not_found";
/// Error reason: the link was revoked.
pub const ERROR_CORD_INVITE_REVOKED: &str = "cord_invite_revoked";
/// Error reason: the link has expired.
pub const ERROR_CORD_INVITE_EXPIRED: &str = "cord_invite_expired";
/// Error reason: the link has no uses left.
pub const ERROR_CORD_INVITE_EXHAUSTED: &str = "cord_invite_exhausted";
/// Error reason: only the cord's owner or admins manage its links.
pub const ERROR_CORD_INVITE_FORBIDDEN: &str = "cord_invite_forbidden";
/// Error reason: a newcomer link's expiry is past its cap.
///
/// The cap is [`CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`]. The bounds
/// every link keeps are refused at params validation, with no reason, as the
/// declared spec refuses them.
pub const ERROR_CORD_INVITE_INVALID: &str = "cord_invite_invalid";
/// Error reason: only instance admins mint links that admit newcomers.
pub const ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN: &str = "cord_invite_newcomer_forbidden";
/// Error reason: a link that admits newcomers needs both an expiry and a use
/// limit.
pub const ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED: &str = "cord_invite_newcomer_unbounded";
/// Error reason: the link is for people who already have an account.
pub const ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS: &str = "cord_invite_not_for_newcomers";
/// Error reason: the link's minter is no longer an instance admin, so it
/// brings no one new onto the instance.
pub const ERROR_CORD_INVITE_NEWCOMERS_CLOSED: &str = "cord_invite_newcomers_closed";
/// Error reason: the username breaks the instance's username rules.
pub const ERROR_CORD_INVITE_USERNAME_INVALID: &str = "cord_invite_username_invalid";
/// Error reason: an account has the username, or an invite reserves it.
pub const ERROR_CORD_INVITE_USERNAME_UNAVAILABLE: &str = "cord_invite_username_unavailable";

/// The most `cord_invite_redeem` calls one IP may make per
/// [`CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS`].
pub const CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS: u32 = 10;
/// The sliding window of [`CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS`], in
/// milliseconds (15 minutes).
pub const CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS: u64 = 15 * 60_000;
/// The per-IP budget of `cord_invite_redeem`, on top of the spine's shared
/// IP class: tighter, since redeem answers whether a username is free.
pub const CORD_INVITE_REDEEM_RATE_LIMIT: RateLimiterOptions = RateLimiterOptions {
    max_attempts: CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS,
    window_ms: CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS,
    max_keys: Some(DEFAULT_RATE_LIMITER_MAX_KEYS),
};

/// Charge one redeem to the caller's IP in `limiter`, refusing with the
/// spine's `rate_limited` error once the IP is over
/// [`CORD_INVITE_REDEEM_RATE_LIMIT`]. An unresolved IP buckets under the
/// spine's shared sentinel rather than passing; no limiter (under
/// `testing_cordd`) passes everything.
fn check_cord_invite_redeem_rate_limit(
    limiter: Option<&RateLimiter>,
    client_ip: Option<&str>,
) -> Result<(), JsonrpcError> {
    fuz_actions::check_action_rate_limit(Some(RateLimitClass::Ip), None, client_ip, None, limiter)
}

// -- Wire inputs --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteCreateInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(default)]
    expires_in_seconds: Option<u32>,
    #[serde(default)]
    max_uses: Option<u32>,
    #[serde(default)]
    admits_newcomers: Option<bool>,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteListInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteRevokeInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    invite_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

/// The input of both `cord_invite_preview` and `cord_invite_accept`. `Debug`
/// redacts the link token.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteTokenInput {
    token: String,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

impl std::fmt::Debug for CordInviteTokenInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordInviteTokenInput")
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// The input of `cord_invite_redeem`: a public action, so no `acting`.
/// Decoded by hand ([`parse_cord_invite_redeem_params`]), so every refusal of
/// `username` can carry its reason. `Debug` redacts the link token.
struct CordInviteRedeemInput {
    token: String,
    username: String,
}

impl std::fmt::Debug for CordInviteRedeemInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordInviteRedeemInput")
            .field("token", &"<redacted>")
            .field("username", &self.username)
            .finish()
    }
}

// -- Wire outputs -------------------------------------------------------------

/// A link as its cord's owner and admins see it: `CordInviteJson` in
/// `cord_action_specs.ts`. Never carries the token or its digest.
fn to_cord_invite_json(invite: &CordInviteWithCounts) -> Value {
    let row = &invite.invite;
    json!({
        "id": row.id,
        "cord_id": row.cord_id,
        "role": row.role,
        "admits_newcomers": row.admits_newcomers,
        "created_by": row.created_by,
        "created_at": row.created_at,
        "expires_at": row.expires_at,
        "max_uses": row.max_uses,
        "use_count": row.use_count,
        "revoked_at": row.revoked_at,
        "status": row.status(),
        "redemption_count": invite.redemption_count,
        "redemption_accepted_count": invite.redemption_accepted_count,
    })
}

// -- Helpers ------------------------------------------------------------------

fn invite_not_found() -> JsonrpcError {
    not_found("cord invite", Some(ERROR_CORD_INVITE_NOT_FOUND))
}

/// The wire error for why a link can't be accepted or redeemed.
fn invite_error(method: &str, e: CordInviteError) -> JsonrpcError {
    match e {
        CordInviteError::NotFound => invite_not_found(),
        CordInviteError::Revoked => conflict(
            "this invite link was revoked",
            Some(ERROR_CORD_INVITE_REVOKED),
        ),
        CordInviteError::Expired => conflict(
            "this invite link has expired",
            Some(ERROR_CORD_INVITE_EXPIRED),
        ),
        CordInviteError::Exhausted => conflict(
            "this invite link has no uses left",
            Some(ERROR_CORD_INVITE_EXHAUSTED),
        ),
        CordInviteError::NotForNewcomers => conflict(
            "this invite link is for people who already have an account; sign in to use it",
            Some(ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS),
        ),
        CordInviteError::NewcomersClosed => conflict(
            "this invite link no longer brings new people onto the instance",
            Some(ERROR_CORD_INVITE_NEWCOMERS_CLOSED),
        ),
        CordInviteError::UsernameUnavailable => conflict(
            "that username is taken",
            Some(ERROR_CORD_INVITE_USERNAME_UNAVAILABLE),
        ),
        CordInviteError::Db(e) => internal_error_with_source(method, &e),
    }
}

fn invite_forbidden() -> JsonrpcError {
    forbidden(
        "only the cord's owner or admins manage its invite links",
        Some(ERROR_CORD_INVITE_FORBIDDEN),
    )
}

/// Check, from the request's role-grant snapshot, that the caller may manage
/// the cord's links: `cord_not_found` outside the cord (the IDOR mask),
/// `cord_invite_forbidden` for a plain member. For the read-only list; the
/// writes read the role afresh ([`require_cord_manager_fresh`]).
fn require_invite_manager(auth: &RequestActorContext, cord_id: Uuid) -> Result<(), JsonrpcError> {
    let Some(role) = cord_role_in(&auth.role_grants, cord_id) else {
        return Err(cord_not_found());
    };
    if !role.can_manage() {
        return Err(invite_forbidden());
    }
    Ok(())
}

/// Refuse a newcomer link unless the caller holds a global `admin`, read
/// afresh in the action's transaction, and the link carries both bounds.
async fn require_newcomer_minter_fresh<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordInviteCreateInput,
) -> Result<(), JsonrpcError> {
    if input.admits_newcomers != Some(true) {
        return Ok(());
    }
    if !query_actor_holds_global_admin(client, auth.actor.id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_create", &e))?
    {
        return Err(forbidden(
            "only instance admins make links that bring new people onto the instance",
            Some(ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN),
        ));
    }
    validate_newcomer_bounds(input)
}

/// Seconds in a day, for stating the newcomer expiry cap in days.
const SECONDS_PER_DAY: u32 = 24 * 60 * 60;

/// A newcomer link must expire, within
/// [`CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`], and must have a use
/// limit.
fn validate_newcomer_bounds(input: &CordInviteCreateInput) -> Result<(), JsonrpcError> {
    if input.admits_newcomers != Some(true) {
        return Ok(());
    }
    let (Some(seconds), Some(_)) = (input.expires_in_seconds, input.max_uses) else {
        return Err(invalid_params(
            "a link that admits newcomers needs both expires_in_seconds and max_uses",
            Some(ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED),
        ));
    };
    if seconds > CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX {
        return Err(invalid_params(
            &format!(
                "a link that admits newcomers expires within \
                 {CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX} seconds ({} days)",
                CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX / SECONDS_PER_DAY
            ),
            Some(ERROR_CORD_INVITE_INVALID),
        ));
    }
    Ok(())
}

/// Refuse an expiry or use limit out of the bounds every link keeps, with the
/// generic `invalid params` the TS spec's schema answers (no reason).
fn validate_invite_limits(input: &CordInviteCreateInput) -> Result<(), JsonrpcError> {
    if let Some(seconds) = input.expires_in_seconds
        && !(CORD_INVITE_EXPIRES_IN_SECONDS_MIN..=CORD_INVITE_EXPIRES_IN_SECONDS_MAX)
            .contains(&seconds)
    {
        return Err(invalid_input_params(
            &format!(
                "expires_in_seconds must be from {CORD_INVITE_EXPIRES_IN_SECONDS_MIN} to \
                 {CORD_INVITE_EXPIRES_IN_SECONDS_MAX}"
            ),
            None,
        ));
    }
    if let Some(max_uses) = input.max_uses
        && !(1..=CORD_INVITE_MAX_USES_MAX).contains(&max_uses)
    {
        return Err(invalid_input_params(
            &format!("max_uses must be from 1 to {CORD_INVITE_MAX_USES_MAX}"),
            None,
        ));
    }
    Ok(())
}

/// Refuse a token out of `CordInviteToken`'s bounds
/// ([`is_cord_invite_token_in_bounds`]), with the generic `invalid params`
/// the TS spec's schema answers (no reason).
fn validate_cord_invite_token(token: &str) -> Result<(), JsonrpcError> {
    if is_cord_invite_token_in_bounds(token) {
        return Ok(());
    }
    Err(invalid_input_params(
        &format!("token must be 1 to {CORD_INVITE_TOKEN_LENGTH_MAX} UTF-16 code units"),
        None,
    ))
}

// -- Params validation --------------------------------------------------------
//
// Each verb's params are parsed and checked against what the declared TS
// spec's schema refuses before its handler runs, so the refusal precedes
// every gate in the handler, as the spine's order has it.

fn parse_cord_invite_create_params(params: Value) -> Result<CordInviteCreateInput, JsonrpcError> {
    let input: CordInviteCreateInput = parse_strict_params(params)?;
    validate_invite_limits(&input)?;
    Ok(input)
}

fn parse_cord_invite_token_params(params: Value) -> Result<CordInviteTokenInput, JsonrpcError> {
    let input: CordInviteTokenInput = parse_strict_params(params)?;
    validate_cord_invite_token(&input.token)?;
    Ok(input)
}

/// The keys of `CordInviteRedeemInput`'s TS twin.
const CORD_INVITE_REDEEM_KEYS: &[&str] = &["token", "username"];

/// Parse redeem's params in the twin's issue order: an unknown key, then the
/// token, both with no reason, then the username. Every refusal of `username`
/// carries [`ERROR_CORD_INVITE_USERNAME_INVALID`], the reason the twin's field
/// declares (the join page reads it): missing, `null`, not a string, or off
/// the spine's creation grammar (`is_valid_username_for_creation`, which also
/// bounds its length).
fn parse_cord_invite_redeem_params(params: &Value) -> Result<CordInviteRedeemInput, JsonrpcError> {
    reject_unknown_keys(params, CORD_INVITE_REDEEM_KEYS)?;
    let token = parse_str_param(params, "token")?;
    validate_cord_invite_token(token)?;
    let username = params
        .get("username")
        .and_then(Value::as_str)
        .filter(|username| is_valid_username_for_creation(username))
        .ok_or_else(|| {
            invalid_input_params(
                &format!(
                    "username must be {USERNAME_LENGTH_MIN} to {USERNAME_LENGTH_MAX} ASCII \
                     letters, digits, '_' or '-', starting with a letter and ending with a \
                     letter or digit"
                ),
                Some(ERROR_CORD_INVITE_USERNAME_INVALID),
            )
        })?;
    Ok(CordInviteRedeemInput {
        token: token.to_owned(),
        username: username.to_owned(),
    })
}

// -- Handlers -----------------------------------------------------------------

async fn handle_cord_invite_create<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteCreateInput,
) -> Result<Value, JsonrpcError> {
    require_cord_manager_fresh(
        client,
        "cord_invite_create",
        auth,
        input.cord_id,
        invite_forbidden,
    )
    .await?;
    require_newcomer_minter_fresh(client, auth, input).await?;
    // a grant can outlive its cell only in a race with deletion; answer as
    // for any missing cord
    if query_cord_get(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_create", &e))?
        .is_none()
    {
        return Err(cord_not_found());
    }
    let minted = generate_cord_invite_token();
    let invite = query_cord_invite_create(
        client,
        input.cord_id,
        &minted.token_hash,
        auth.actor.id,
        input.expires_in_seconds,
        input.max_uses,
        input.admits_newcomers == Some(true),
    )
    .await
    .map_err(|e| internal_error_with_source("cord_invite_create", &e))?;
    let row = &invite.invite;
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_invite_create",
        json!({
            "cord_id": row.cord_id,
            "invite_id": row.id,
            "expires_at": row.expires_at,
            "max_uses": row.max_uses,
            "admits_newcomers": row.admits_newcomers,
        }),
    )
    .await?;
    Ok(json!({
        "invite": to_cord_invite_json(&invite),
        "token": minted.token,
    }))
}

async fn handle_cord_invite_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordInviteListInput,
) -> Result<Value, JsonrpcError> {
    require_invite_manager(auth, input.cord_id)?;
    let rows = query_cord_invite_list(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_list", &e))?;
    let invites: Vec<Value> = rows.iter().map(to_cord_invite_json).collect();
    Ok(json!({ "invites": invites }))
}

async fn handle_cord_invite_revoke<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteRevokeInput,
) -> Result<Value, JsonrpcError> {
    // no cord lock: accept locks the link then the cord, and this UPDATE
    // locks the link, so taking the cord first would invert that order
    require_cord_manager_fresh(
        client,
        "cord_invite_revoke",
        auth,
        input.cord_id,
        invite_forbidden,
    )
    .await?;
    let Some(CordInviteRevoked {
        invite,
        revoked,
        released_signup_invite_ids,
    }) = query_cord_invite_revoke(client, input.cord_id, input.invite_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_revoke", &e))?
    else {
        return Err(invite_not_found());
    };
    if revoked {
        audit_cord_event(
            client,
            ctx,
            Some(auth),
            "cord_invite_revoke",
            json!({
                "cord_id": invite.invite.cord_id,
                "invite_id": invite.invite.id,
                "released_signup_invite_ids": released_signup_invite_ids,
            }),
        )
        .await?;
    }
    Ok(json!({ "invite": to_cord_invite_json(&invite) }))
}

async fn handle_cord_invite_preview<C: GenericClient + ?Sized>(
    client: &C,
    auth: Option<&RequestActorContext>,
    input: &CordInviteTokenInput,
) -> Result<Value, JsonrpcError> {
    let Some(invite) =
        query_cord_invite_by_token_hash(client, &hash_cord_invite_token(&input.token))
            .await
            .map_err(|e| internal_error_with_source("cord_invite_preview", &e))?
    else {
        return Err(invite_not_found());
    };
    let Some(cord) = query_cord_get(client, invite.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_preview", &e))?
    else {
        return Err(invite_not_found());
    };
    let status = invite.status();
    let newcomers_open = newcomers_open(client, &invite).await?;
    let role = auth.and_then(|auth| cord_role_in(&auth.role_grants, cord.id));
    let cord_json = preview_shows_cord(status, newcomers_open, auth.is_some(), role)
        .then(|| json!({ "id": cord.id, "name": cord.name }));
    Ok(json!({
        "status": status,
        "admits_newcomers": invite.admits_newcomers,
        "newcomers_open": newcomers_open,
        "cord": cord_json,
        "role": role,
    }))
}

/// Whether a redeem of the link could succeed now, for some free username:
/// it's active, admits newcomers, and its minter still holds a global
/// `admin`. A non-locking read, since the preview decides nothing.
async fn newcomers_open<C: GenericClient + ?Sized>(
    client: &C,
    invite: &CordInviteRow,
) -> Result<bool, JsonrpcError> {
    let Some(minted_by) = invite.created_by else {
        return Ok(false);
    };
    if !invite.admits_newcomers || invite.status() != CordInviteStatus::Active {
        return Ok(false);
    }
    query_actor_holds_global_admin_unlocked(client, minted_by)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_preview", &e))
}

/// Whether a preview names the link's cord: to a signed-in caller, when the
/// link is active or they're in the cord; to an anonymous one, only when
/// the link is open to newcomers, the one case they could use it. A link
/// that isn't active names no cord to anyone outside it.
const fn preview_shows_cord(
    status: CordInviteStatus,
    newcomers_open: bool,
    signed_in: bool,
    role: Option<CordRole>,
) -> bool {
    if signed_in {
        matches!(status, CordInviteStatus::Active) || role.is_some()
    } else {
        newcomers_open
    }
}

async fn handle_cord_invite_redeem<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    input: &CordInviteRedeemInput,
) -> Result<Value, JsonrpcError> {
    // the spine's creation rule held at params parse; its canonical
    // (lowercase) form, as the spine's own `invite_create` stores it
    let username = input.username.to_lowercase();
    let token_hash = hash_cord_invite_token(&input.token);
    let redeemed = query_cord_invite_redeem(client, &token_hash, &username)
        .await
        .map_err(|e| invite_error("cord_invite_redeem", e))?;
    if redeemed.created {
        // anonymous: the row carries no actor or account, only the IP
        audit_cord_event(
            client,
            ctx,
            None,
            "cord_invite_redeem",
            json!({
                "cord_id": redeemed.cord.id,
                "invite_id": redeemed.invite_id,
                "redemption_id": redeemed.redemption.id,
                "signup_invite_id": redeemed.redemption.signup_invite_id,
                "username": redeemed.redemption.username,
            }),
        )
        .await?;
    }
    Ok(json!({
        "cord": { "id": redeemed.cord.id, "name": redeemed.cord.name },
        "username": redeemed.redemption.username,
        "created": redeemed.created,
    }))
}

async fn handle_cord_invite_accept<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteTokenInput,
) -> Result<Value, JsonrpcError> {
    let token_hash = hash_cord_invite_token(&input.token);
    let accepted = query_cord_invite_accept(client, &token_hash, auth.actor.id)
        .await
        .map_err(|e| invite_error("cord_invite_accept", e))?;
    if let Some(role_grant_id) = accepted.role_grant_id {
        // the spine's grant event, as its offer accept emits: the caller is
        // both the actor and the target
        audit_cord_grant_event(
            client,
            ctx,
            auth,
            AuditEventType::RoleGrantCreate,
            &CordGrantAudit {
                target_account_id: auth.account.id,
                target_actor_id: auth.actor.id,
                cord_id: accepted.cord.id,
                role: accepted.role,
                role_grant_id,
            },
            Some(("source_invite_id", json!(accepted.invite_id))),
        )
        .await?;
    }
    // a grant, or an existing member completing their redemption
    if accepted.role_grant_id.is_some() || accepted.redemption_id.is_some() {
        audit_cord_event(
            client,
            ctx,
            Some(auth),
            "cord_invite_accept",
            json!({
                "cord_id": accepted.cord.id,
                "invite_id": accepted.invite_id,
                "role_grant_id": accepted.role_grant_id,
                "redemption_id": accepted.redemption_id,
                "claimed_signup_invite_id": accepted.claimed_signup_invite_id,
            }),
        )
        .await?;
    }
    Ok(json!({
        "cord": to_cord_json(&accepted.cord, accepted.role),
        "joined": accepted.role_grant_id.is_some(),
    }))
}

// -- Specs --------------------------------------------------------------------

/// The invite-link action specs; `redeem_rate_limiter` is redeem's own
/// per-IP limiter ([`CORD_INVITE_REDEEM_RATE_LIMIT`]), `None` under
/// `testing_cordd`.
pub(crate) fn cord_invite_action_specs(
    redeem_rate_limiter: Option<Arc<RateLimiter>>,
) -> Vec<ActionSpec> {
    vec![
        ActionSpec::with_side_effects(
            "cord_invite_create",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_invite_create_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_create(auth, &input))
                })
            }),
        )
        .with_rate_limit(RateLimitClass::Account),
        ActionSpec::read_only(
            "cord_invite_list",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_invite_list(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_invite_revoke",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteRevokeInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_revoke(auth, &input))
                })
            }),
        ),
        ActionSpec::read_only(
            "cord_invite_preview",
            AuthSpec::optional_identity(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = ctx.auth.actor();
                    let input = parse_cord_invite_token_params(params)?;
                    dispatch_db!(ctx, handle_cord_invite_preview(auth, &input))
                })
            }),
        )
        .with_rate_limit(RateLimitClass::Ip),
        ActionSpec::with_side_effects(
            "cord_invite_redeem",
            AuthSpec::public(),
            handler(move |params, ctx| {
                // first thing, so a flood of malformed input spends the budget
                // and a throttled caller costs no query of its own
                let throttled = check_cord_invite_redeem_rate_limit(
                    redeem_rate_limiter.as_deref(),
                    ctx.client_ip.as_deref(),
                );
                Box::pin(async move {
                    throttled?;
                    let input = parse_cord_invite_redeem_params(&params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_redeem(&input))
                })
            }),
        )
        .with_rate_limit(RateLimitClass::Ip),
        ActionSpec::with_side_effects(
            "cord_invite_accept",
            AuthSpec::authenticated(CredentialGate::SESSION_ONLY),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_invite_token_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_accept(auth, &input))
                })
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuz_auth::AuthAxis;

    #[test]
    fn invite_token_input_debug_redacts_the_token() {
        let input: CordInviteTokenInput =
            serde_json::from_value(json!({"token": "invite-link-Gk2wE7"})).unwrap();
        let rendered = format!("{input:?}");
        assert!(!rendered.contains("invite-link-Gk2wE7"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn invite_redeem_input_debug_redacts_the_token() {
        let input = parse_cord_invite_redeem_params(
            &json!({"token": "invite-link-Rb9yU4", "username": "newcomer"}),
        )
        .unwrap();
        let rendered = format!("{input:?}");
        assert!(!rendered.contains("invite-link-Rb9yU4"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains("newcomer"), "{rendered}");
    }

    #[test]
    fn specs_declare_their_auth_and_effects() {
        let specs = cord_invite_action_specs(None);
        let find = |method: &str| {
            specs
                .iter()
                .find(|s| s.method == method)
                .unwrap_or_else(|| panic!("missing {method}"))
        };
        for spec in &specs {
            assert_eq!(spec.auth.roles, None, "{}", spec.method);
            let actor = match spec.method {
                "cord_invite_preview" => AuthAxis::Optional,
                "cord_invite_redeem" => AuthAxis::None,
                _ => AuthAxis::Required,
            };
            assert_eq!(spec.auth.actor, actor, "{}", spec.method);
        }
        for method in [
            "cord_invite_create",
            "cord_invite_revoke",
            "cord_invite_redeem",
            "cord_invite_accept",
        ] {
            assert!(find(method).side_effects, "{method}");
        }
        for method in ["cord_invite_list", "cord_invite_preview"] {
            assert!(!find(method).side_effects, "{method}");
        }
        assert_eq!(
            find("cord_invite_accept").auth,
            AuthSpec::authenticated(CredentialGate::SESSION_ONLY),
            "accept is cookie-session only"
        );
        assert_eq!(
            find("cord_invite_preview").auth,
            AuthSpec::optional_identity(CredentialGate::Any),
            "preview reads the caller when there is one, and admits anonymous callers"
        );
        assert_eq!(
            find("cord_invite_redeem").auth,
            AuthSpec::public(),
            "redeem never reads the caller"
        );
    }

    #[test]
    fn minting_is_rate_limited_per_account_and_anonymous_reach_per_ip() {
        for spec in cord_invite_action_specs(None) {
            let expected = match spec.method {
                "cord_invite_create" => Some(RateLimitClass::Account),
                "cord_invite_preview" | "cord_invite_redeem" => Some(RateLimitClass::Ip),
                _ => None,
            };
            assert_eq!(spec.rate_limit, expected, "{}", spec.method);
        }
    }

    #[test]
    fn redeem_has_its_own_per_ip_budget() {
        use fuz_http::JsonrpcErrorCode;

        let limiter = RateLimiter::new(CORD_INVITE_REDEEM_RATE_LIMIT);
        let ip = Some("203.0.113.7");
        for attempt in 0..CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS {
            check_cord_invite_redeem_rate_limit(Some(&limiter), ip)
                .unwrap_or_else(|e| panic!("attempt {attempt} is within the budget: {e:?}"));
        }
        let err = check_cord_invite_redeem_rate_limit(Some(&limiter), ip)
            .expect_err("one past the budget is refused");
        // the spine dispatcher's own refusal, so the frontend reads it alike
        assert_eq!(err.code, JsonrpcErrorCode::RateLimited);
        assert_eq!(err.message, "rate limited");
        let retry_after = err
            .data
            .and_then(|d| d["retry_after"].as_u64())
            .expect("carries retry_after");
        assert!(
            retry_after > 0 && retry_after <= CORD_INVITE_REDEEM_RATE_LIMIT_WINDOW_MS / 1000,
            "{retry_after}"
        );
        // per IP: another address keeps its own budget
        check_cord_invite_redeem_rate_limit(Some(&limiter), Some("198.51.100.9"))
            .expect("another IP is unaffected");
        // an unresolved IP is a bucket, not an exemption
        for _ in 0..CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS {
            check_cord_invite_redeem_rate_limit(Some(&limiter), None).expect("within the budget");
        }
        check_cord_invite_redeem_rate_limit(Some(&limiter), None)
            .expect_err("the unresolved bucket is held to the budget");
        // no limiter, as under `testing_cordd`: never throttled
        for _ in 0..=CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS {
            check_cord_invite_redeem_rate_limit(None, ip).expect("no limiter, no gate");
        }
    }

    #[test]
    fn redeem_takes_a_token_and_a_username_only() {
        let parsed =
            parse_cord_invite_redeem_params(&json!({ "token": "t", "username": "Newcomer" }))
                .expect("parses");
        // the handler lowercases; the parse keeps what was sent
        assert_eq!(
            (parsed.token.as_str(), parsed.username.as_str()),
            ("t", "Newcomer")
        );
        // an unknown key or a bad token is refused first, with no reason,
        // whatever the username: the twin answers no reason for an issue on
        // the input itself or on `token`, its first field
        for params in [
            json!({ "token": "t", "username": "newcomer", "acting": Uuid::nil() }),
            json!({ "token": "t", "username": 42, "acting": Uuid::nil() }),
            json!({ "username": "newcomer" }),
            json!({ "token": null, "username": "newcomer" }),
            json!({ "token": 42, "username": "newcomer" }),
            json!({ "token": "", "username": "newcomer" }),
            json!({ "username": 42 }),
            json!({ "token": "", "username": "" }),
        ] {
            assert_spec_params_refusal(
                parse_cord_invite_redeem_params(&params),
                &params.to_string(),
            );
        }
        // no params at all is the empty object: the token is missing
        assert_spec_params_refusal(parse_cord_invite_redeem_params(&Value::Null), "null params");
    }

    #[test]
    fn every_username_refusal_carries_its_reason() {
        use fuz_http::{INVALID_PARAMS_MESSAGE, JsonrpcErrorCode};
        let too_long = "a".repeat(USERNAME_LENGTH_MAX + 1);
        let too_short = "a".repeat(USERNAME_LENGTH_MIN - 1);
        let cases = [
            ("missing", json!({ "token": "t" })),
            ("null", json!({ "token": "t", "username": null })),
            ("a number", json!({ "token": "t", "username": 42 })),
            ("an object", json!({ "token": "t", "username": {} })),
            ("empty", json!({ "token": "t", "username": "" })),
            ("too short", json!({ "token": "t", "username": too_short })),
            ("too long", json!({ "token": "t", "username": too_long })),
            (
                "a leading digit",
                json!({ "token": "t", "username": "9lives" }),
            ),
            (
                "a trailing '_'",
                json!({ "token": "t", "username": "trailing_" }),
            ),
            ("a space", json!({ "token": "t", "username": "new comer" })),
            ("non-ASCII", json!({ "token": "t", "username": "newcomér" })),
        ];
        for (case, params) in cases {
            let err = parse_cord_invite_redeem_params(&params).expect_err(case);
            assert_eq!(err.code, JsonrpcErrorCode::InvalidParams, "{case}");
            assert_eq!(err.message, INVALID_PARAMS_MESSAGE, "{case}");
            assert_eq!(
                err.data,
                Some(json!({ "reason": ERROR_CORD_INVITE_USERNAME_INVALID })),
                "{case}"
            );
        }
        // the grammar's bounds themselves are in
        let at_min = "a".repeat(USERNAME_LENGTH_MIN);
        let at_max = "a".repeat(USERNAME_LENGTH_MAX);
        for username in [at_min.as_str(), at_max.as_str(), "new-comer_2"] {
            parse_cord_invite_redeem_params(&json!({ "token": "t", "username": username }))
                .expect(username);
        }
    }

    #[test]
    fn a_newcomer_link_needs_both_bounds() {
        let input = |admits_newcomers, expires_in_seconds, max_uses| CordInviteCreateInput {
            cord_id: Uuid::nil(),
            expires_in_seconds,
            max_uses,
            admits_newcomers,
            _acting: None,
        };
        assert!(validate_newcomer_bounds(&input(Some(true), Some(3600), Some(5))).is_ok());
        assert!(
            validate_newcomer_bounds(&input(
                Some(true),
                Some(CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX),
                Some(5)
            ))
            .is_ok()
        );
        let too_long = Some(CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX + 1);
        assert!(validate_newcomer_bounds(&input(Some(true), too_long, Some(5))).is_err());
        // an ordinary link keeps the longer limit
        assert!(validate_newcomer_bounds(&input(None, too_long, Some(5))).is_ok());
        assert!(validate_invite_limits(&input(None, too_long, Some(5))).is_ok());
        for (expires, uses) in [(None, None), (Some(3600), None), (None, Some(5))] {
            assert!(
                validate_newcomer_bounds(&input(Some(true), expires, uses)).is_err(),
                "{expires:?} {uses:?}"
            );
            assert!(validate_newcomer_bounds(&input(Some(false), expires, uses)).is_ok());
            assert!(validate_newcomer_bounds(&input(None, expires, uses)).is_ok());
        }
    }

    #[test]
    fn anonymous_previews_name_the_cord_only_on_an_open_newcomer_link() {
        use CordInviteStatus::{Active, Exhausted, Expired, Revoked};
        for status in [Active, Revoked, Expired, Exhausted] {
            let active = status == Active;
            // `newcomers_open` implies active; anonymous callers go by it alone
            assert_eq!(preview_shows_cord(status, active, false, None), active);
            assert!(!preview_shows_cord(status, false, false, None));
            // signed in: active, or already in the cord
            assert_eq!(preview_shows_cord(status, false, true, None), active);
            assert!(preview_shows_cord(
                status,
                false,
                true,
                Some(CordRole::Member)
            ));
        }
    }

    #[test]
    fn optional_limits_may_be_absent_but_not_null() {
        let cord_id = "11111111-1111-4111-8111-111111111111";
        let parsed: CordInviteCreateInput =
            parse_strict_params(json!({ "cord_id": cord_id })).expect("limits are optional");
        assert_eq!((parsed.expires_in_seconds, parsed.max_uses), (None, None));
        assert_eq!(parsed.admits_newcomers, None);
        for field in ["expires_in_seconds", "max_uses", "admits_newcomers"] {
            let parsed: Result<CordInviteCreateInput, _> =
                parse_strict_params(json!({ "cord_id": cord_id, field: null }));
            assert!(parsed.is_err(), "{field}: null is refused");
            let parsed: Result<CordInviteCreateInput, _> =
                parse_strict_params(json!({ "cord_id": cord_id, field: -1 }));
            assert!(parsed.is_err(), "{field}: negative is refused");
        }
    }

    #[test]
    fn limits_are_bounded() {
        let input = |expires_in_seconds, max_uses| CordInviteCreateInput {
            cord_id: Uuid::nil(),
            expires_in_seconds,
            max_uses,
            admits_newcomers: None,
            _acting: None,
        };
        assert!(validate_invite_limits(&input(None, None)).is_ok());
        assert!(
            validate_invite_limits(&input(
                Some(CORD_INVITE_EXPIRES_IN_SECONDS_MIN),
                Some(CORD_INVITE_MAX_USES_MAX)
            ))
            .is_ok()
        );
        assert!(
            validate_invite_limits(&input(Some(CORD_INVITE_EXPIRES_IN_SECONDS_MAX), Some(1)))
                .is_ok()
        );
        for (expires, uses) in [
            (Some(CORD_INVITE_EXPIRES_IN_SECONDS_MIN - 1), None),
            (Some(CORD_INVITE_EXPIRES_IN_SECONDS_MAX + 1), None),
            (None, Some(0)),
            (None, Some(CORD_INVITE_MAX_USES_MAX + 1)),
        ] {
            assert_spec_params_refusal(
                validate_invite_limits(&input(expires, uses)),
                &format!("{expires:?} {uses:?}"),
            );
        }
    }

    /// The refusal the declared spec's schema answers on the spine: `400`
    /// `invalid params`, with no reason.
    fn assert_spec_params_refusal(result: Result<impl std::fmt::Debug, JsonrpcError>, case: &str) {
        use fuz_http::{INVALID_PARAMS_MESSAGE, JsonrpcErrorCode};
        let err = result.expect_err(case);
        assert_eq!(err.code, JsonrpcErrorCode::InvalidParams, "{case}");
        assert_eq!(err.message, INVALID_PARAMS_MESSAGE, "{case}");
        assert_eq!(err.data, None, "{case}: no reason");
    }

    #[test]
    fn out_of_bounds_limits_are_refused_at_params_validation() {
        // a cord the caller isn't in: the refusal comes before the handler's
        // manager gate could answer `cord_not_found`
        let cord_id = Uuid::nil();
        for limits in [
            json!({ "expires_in_seconds": CORD_INVITE_EXPIRES_IN_SECONDS_MIN - 1 }),
            json!({ "expires_in_seconds": CORD_INVITE_EXPIRES_IN_SECONDS_MAX + 1 }),
            json!({ "expires_in_seconds": 0 }),
            json!({ "max_uses": 0 }),
            json!({ "max_uses": CORD_INVITE_MAX_USES_MAX + 1 }),
        ] {
            let mut params = json!({ "cord_id": cord_id });
            params
                .as_object_mut()
                .unwrap()
                .extend(limits.as_object().unwrap().clone());
            assert_spec_params_refusal(
                parse_cord_invite_create_params(params),
                &limits.to_string(),
            );
        }
        let parsed = parse_cord_invite_create_params(json!({
            "cord_id": cord_id,
            "expires_in_seconds": CORD_INVITE_EXPIRES_IN_SECONDS_MIN,
            "max_uses": CORD_INVITE_MAX_USES_MAX,
        }))
        .expect("in bounds reaches the handler");
        assert_eq!(parsed.cord_id, cord_id);
    }

    #[test]
    fn out_of_bounds_tokens_are_refused_at_params_validation() {
        let too_long = "x".repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1);
        for token in ["", too_long.as_str()] {
            let case = format!("token of {} chars", token.len());
            assert_spec_params_refusal(
                parse_cord_invite_token_params(json!({ "token": token })),
                &case,
            );
            assert_spec_params_refusal(
                parse_cord_invite_redeem_params(&json!({ "token": token, "username": "newcomer" })),
                &case,
            );
        }
        // a wrong token in bounds reaches the handler, and its masked 404
        let at_max = "x".repeat(CORD_INVITE_TOKEN_LENGTH_MAX);
        for token in ["nope", at_max.as_str()] {
            parse_cord_invite_token_params(json!({ "token": token })).expect("in bounds");
            parse_cord_invite_redeem_params(&json!({ "token": token, "username": "newcomer" }))
                .expect("in bounds");
        }
    }

    #[test]
    fn errors_map_to_their_reasons() {
        let reason = |e| {
            invite_error("m", e)
                .data
                .and_then(|d| d.get("reason").cloned())
        };
        for (e, expected) in [
            (CordInviteError::NotFound, ERROR_CORD_INVITE_NOT_FOUND),
            (CordInviteError::Revoked, ERROR_CORD_INVITE_REVOKED),
            (CordInviteError::Expired, ERROR_CORD_INVITE_EXPIRED),
            (CordInviteError::Exhausted, ERROR_CORD_INVITE_EXHAUSTED),
            (
                CordInviteError::NotForNewcomers,
                ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
            ),
            (
                CordInviteError::NewcomersClosed,
                ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
            ),
            (
                CordInviteError::UsernameUnavailable,
                ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
            ),
        ] {
            assert_eq!(reason(e), Some(json!(expected)), "{expected}");
        }
    }

    #[test]
    fn invite_json_never_carries_a_token() {
        let row = CordInviteWithCounts {
            invite: CordInviteRow {
                id: Uuid::from_u128(1),
                cord_id: Uuid::from_u128(2),
                role: "cord_member".to_owned(),
                admits_newcomers: false,
                created_by: None,
                created_at: "2026-01-01T00:00:00Z".to_owned(),
                expires_at: None,
                max_uses: Some(3),
                use_count: 3,
                revoked_at: None,
                expired: false,
            },
            redemption_count: 1,
            redemption_accepted_count: 0,
        };
        let json = to_cord_invite_json(&row);
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(!keys.iter().any(|k| k.contains("token")), "{keys:?}");
        assert_eq!(json["status"], json!("exhausted"));
        assert_eq!(
            (
                &json["redemption_count"],
                &json["redemption_accepted_count"]
            ),
            (&json!(1), &json!(0))
        );
    }
}
