//! Queries for the `cord_invite` table: a cord's invite links.
//!
//! A link is a random token; the table keeps only its blake3 hex digest
//! (`token_hash`), made with the spine's session-token helpers
//! ([`generate_cord_invite_token`], [`hash_cord_invite_token`]). The raw
//! token is returned once, when the link is created.
//!
//! A link's status ([`CordInviteStatus`]) is derived, not stored: revoked
//! once `revoked_at` is set, expired once `expires_at` has passed, exhausted
//! once `use_count` reaches `max_uses`, and active otherwise. Reads compute
//! the expiry against the database clock, so every check agrees on "now".
//!
//! Accepting a link ([`query_cord_invite_accept`]) takes the cord's scope
//! lock shared and locks the cord's row (as the member changes in
//! [`crate::cord_member_queries`] do, which has why the scope lock comes
//! first), then its minter's account, then the link's, so concurrent
//! accepts can't push `use_count` past `max_uses` and can't race a
//! membership change. The cord and link locks are `FOR NO KEY UPDATE`:
//! enough to serialize the writers (a revoke's UPDATE waits on the link's
//! lock), and no stronger. The table's own CHECK refuses an overshoot too.
//! Redeeming a newcomer link ([`crate::invite::redemption_queries`]) takes
//! the link's lock as well.
//!
//! Both lock the link's minter's account row before the link's, the link
//! read unlocked to find the minter and re-checked under its lock. A purge
//! of the minter takes that row `FOR UPDATE` and then writes rows an accept
//! or redeem goes on to: it deletes the minter's unclaimed invites (one may
//! be a redemption's), and its cascade deletes the minter's actor and sets
//! the link's `created_by` null. A link lock taken first would hold what the
//! purge waits on while waiting on the purge. They lock the account in
//! different modes. Redeem takes `FOR SHARE`, the lock the spine's invite
//! create takes on its issuer, and no cord lock. Accept takes
//! `FOR KEY SHARE` ([`query_cord_invite_lock_with_minter_account`]), the
//! weakest lock that conflicts with the purge's `FOR UPDATE`, so a login, a
//! revoke, or a member verb of another cord doesn't wait behind an accept.
//! Accept takes the cord's lock before the minter's: a purge holds every
//! admin's account row from its first statement, so a member verb on the
//! cord can wait on the purge while holding the cord, and an accept holding
//! its minter's row while waiting on the cord would close the cycle. That
//! the cord's row is safe to hold while waiting on an account rests on a
//! purge never writing it: a cord cell names no actor
//! ([`crate::cord_queries`]).
//!
//! A newcomer's accept claims their signup invite, which waits on their
//! account, before its grant names their actor in `actor_id`, a foreign key
//! (the redemption's `accepted_by` is a record with none): a purge of the
//! newcomer holds that account, then its cascade deletes the actor, so the
//! accept waits the purge out and then fails on the removed account.
//!
//! A newcomer link spends its use at redemption, not at accept: the
//! newcomer's accept finds their redemption and joins without spending
//! another. Revoking a newcomer link releases its waiting reservations, and
//! the expiry sweep ([`crate::invite::sweep`]) releases them once the
//! link has been expired for longer than the accept grace.
//!
//! Every function takes any `GenericClient`, so it composes on a pooled
//! client or inside the action's transaction.

use deadpool_postgres::GenericClient;
use fuz_auth::{generate_session_token, hash_session_token, query_scope_lock_shared};
use fuz_cell::CellError;
use fuz_db::tokio_postgres::Row;
use fuz_db::{col, iso8601_timestamp_expr, qualify_columns};
use serde::Serialize;
use uuid::Uuid;

use crate::cord_authz::CordRole;
use crate::cord_member_queries::{
    query_cord_lock, query_cord_member_role, query_cord_role_grant_create,
};
use crate::cord_queries::CordRow;
use crate::invite::redemption_queries::{
    query_cord_invite_redemption_complete, query_cord_invite_redemption_pending_for_actor,
    query_cord_invite_redemption_release,
};

/// The `cord_invite` table's columns, in projection order. The PG-gated
/// `columns` test checks they name every live column.
pub const CORD_INVITE_COLUMNS: [&str; 11] = [
    "id",
    "cord_id",
    "token_hash",
    "role",
    "admits_newcomers",
    "created_by",
    "created_at",
    "expires_at",
    "max_uses",
    "use_count",
    "revoked_at",
];

/// [`CORD_INVITE_COLUMNS`] without `token_hash`: what every read projects,
/// so a digest never leaves the table. A unit test pins it to
/// `omit_columns(&CORD_INVITE_COLUMNS, &["token_hash"])`.
pub const CORD_INVITE_READ_COLUMNS: [&str; 10] = [
    "id",
    "cord_id",
    "role",
    "admits_newcomers",
    "created_by",
    "created_at",
    "expires_at",
    "max_uses",
    "use_count",
    "revoked_at",
];

const CORD_INVITE_TIMESTAMP_COLUMNS: [&str; 3] = ["created_at", "expires_at", "revoked_at"];

/// The longest token a lookup hashes, in UTF-16 code units, as the
/// frontend's `CordInviteToken` schema counts it.
///
/// Anything longer is no token Cord minted; generated tokens are 43 ASCII
/// characters. The frontend's `CordInviteToken` schema twins it.
pub const CORD_INVITE_TOKEN_LENGTH_MAX: usize = 128;

/// The shortest expiry a link may be given, in seconds.
pub const CORD_INVITE_EXPIRES_IN_SECONDS_MIN: u32 = 60;
/// The longest expiry a link may be given, in seconds (365 days).
pub const CORD_INVITE_EXPIRES_IN_SECONDS_MAX: u32 = 365 * 24 * 60 * 60;
/// The longest expiry a link that admits newcomers may be given, in seconds
/// (30 days): such a link reserves usernames on the instance, so it stays
/// short-lived.
pub const CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX: u32 = 30 * 24 * 60 * 60;
/// How long after a newcomer link expires a newcomer who redeemed it before
/// then may still accept it, in seconds (one hour): enough to finish the
/// signup the redemption led to, and no longer.
pub const CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS: u32 = 60 * 60;
/// The most uses a link may be given.
pub const CORD_INVITE_MAX_USES_MAX: u32 = 1000;

/// A fresh link token and its digest, the `token_hash` to store. `Debug`
/// redacts the token; the digest stays visible.
#[derive(Clone)]
pub struct CordInviteToken {
    /// Shown once, to whoever created the link; never stored.
    pub token: String,
    pub token_hash: String,
}

impl std::fmt::Debug for CordInviteToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordInviteToken")
            .field("token", &"<redacted>")
            .field("token_hash", &self.token_hash)
            .finish()
    }
}

/// Mint a link token: 32 random bytes from the OS, base64url-encoded (the
/// spine's session-token generator), with its blake3 hex digest.
#[must_use]
pub fn generate_cord_invite_token() -> CordInviteToken {
    let token = generate_session_token();
    let token_hash = hash_cord_invite_token(&token);
    CordInviteToken { token, token_hash }
}

/// The blake3 hex digest of a link token, as stored in `token_hash`.
#[must_use]
pub fn hash_cord_invite_token(token: &str) -> String {
    hash_session_token(token).into_inner()
}

/// Whether `token` is within the TS `CordInviteToken` schema's bounds.
///
/// That is, not empty and at most [`CORD_INVITE_TOKEN_LENGTH_MAX`] UTF-16
/// code units, counted as the schema counts them, so the two refuse exactly
/// the same strings.
///
/// The verbs refuse a token out of bounds at params validation, with the
/// generic `invalid params` the declared spec answers; a token in bounds
/// that Cord never minted gets `cord_invite_not_found`.
#[must_use]
pub fn is_cord_invite_token_in_bounds(token: &str) -> bool {
    !token.is_empty() && token.encode_utf16().count() <= CORD_INVITE_TOKEN_LENGTH_MAX
}

/// A link's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CordInviteStatus {
    /// Usable.
    Active,
    /// Revoked by the cord's owner or an admin.
    Revoked,
    /// Past its expiry.
    Expired,
    /// Used as many times as it allows.
    Exhausted,
}

/// A link's status from its fields. Revocation wins over expiry, and expiry
/// over exhaustion, so a link reports the most deliberate reason it isn't
/// active.
#[must_use]
pub(crate) fn cord_invite_status(
    revoked: bool,
    expired: bool,
    max_uses: Option<i32>,
    use_count: i32,
) -> CordInviteStatus {
    if revoked {
        CordInviteStatus::Revoked
    } else if expired {
        CordInviteStatus::Expired
    } else if max_uses.is_some_and(|max| use_count >= max) {
        CordInviteStatus::Exhausted
    } else {
        CordInviteStatus::Active
    }
}

/// A `cord_invite` row, without its `token_hash`, as of a read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteRow {
    pub id: Uuid,
    pub cord_id: Uuid,
    /// The role the link confers (`cord_member`).
    pub role: String,
    /// Whether the link may bring someone with no account onto the instance.
    pub admits_newcomers: bool,
    pub created_by: Option<Uuid>,
    /// UTC ISO-8601, second precision, as are the other timestamps.
    pub created_at: String,
    pub expires_at: Option<String>,
    pub max_uses: Option<i32>,
    pub use_count: i32,
    pub revoked_at: Option<String>,
    /// Whether `expires_at` had passed when the row was read.
    pub expired: bool,
}

impl CordInviteRow {
    /// The link's status as of the read.
    #[must_use]
    pub fn status(&self) -> CordInviteStatus {
        cord_invite_status(
            self.revoked_at.is_some(),
            self.expired,
            self.max_uses,
            self.use_count,
        )
    }
}

/// A link with its redemption counts: what the managing verbs return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteWithCounts {
    pub invite: CordInviteRow,
    /// How many times the link was redeemed for a newcomer.
    pub redemption_count: i64,
    /// How many of those redemptions the newcomer has since accepted.
    pub redemption_accepted_count: i64,
}

/// The read projection qualified by `alias`, timestamps ISO-formatted, then
/// the computed `expired` flag.
fn invite_cols(alias: &str) -> String {
    let columns = qualify_columns(
        &CORD_INVITE_READ_COLUMNS,
        alias,
        iso8601_timestamp_expr(
            &CORD_INVITE_READ_COLUMNS,
            &CORD_INVITE_TIMESTAMP_COLUMNS,
            alias,
        ),
    );
    format!("{columns}, ({alias}.expires_at IS NOT NULL AND {alias}.expires_at <= NOW())")
}

/// [`invite_cols`], then the redemption counts: two correlated subqueries,
/// so only the reads that return the counts project them.
fn invite_with_counts_cols(alias: &str) -> String {
    format!(
        "{},
         (SELECT COUNT(*) FROM cord_invite_redemption r WHERE r.cord_invite_id = {alias}.id),
         (SELECT COUNT(*) FROM cord_invite_redemption r
          WHERE r.cord_invite_id = {alias}.id AND r.accepted_at IS NOT NULL)",
        invite_cols(alias),
    )
}

/// Decode the [`invite_cols`] projection positionally.
fn decode_invite(row: &Row) -> CordInviteRow {
    CordInviteRow {
        id: row.get(col!(CORD_INVITE_READ_COLUMNS, "id")),
        cord_id: row.get(col!(CORD_INVITE_READ_COLUMNS, "cord_id")),
        role: row.get(col!(CORD_INVITE_READ_COLUMNS, "role")),
        admits_newcomers: row.get(col!(CORD_INVITE_READ_COLUMNS, "admits_newcomers")),
        created_by: row.get(col!(CORD_INVITE_READ_COLUMNS, "created_by")),
        created_at: row.get(col!(CORD_INVITE_READ_COLUMNS, "created_at")),
        expires_at: row.get(col!(CORD_INVITE_READ_COLUMNS, "expires_at")),
        max_uses: row.get(col!(CORD_INVITE_READ_COLUMNS, "max_uses")),
        use_count: row.get(col!(CORD_INVITE_READ_COLUMNS, "use_count")),
        revoked_at: row.get(col!(CORD_INVITE_READ_COLUMNS, "revoked_at")),
        expired: row.get(CORD_INVITE_READ_COLUMNS.len()),
    }
}

/// Decode the [`invite_with_counts_cols`] projection positionally.
fn decode_invite_with_counts(row: &Row) -> CordInviteWithCounts {
    CordInviteWithCounts {
        invite: decode_invite(row),
        redemption_count: row.get(CORD_INVITE_READ_COLUMNS.len() + 1),
        redemption_accepted_count: row.get(CORD_INVITE_READ_COLUMNS.len() + 2),
    }
}

/// Create a link on the cord `cord_id` conferring `cord_member`.
///
/// `expires_in_seconds` counts from the database's now; `None` never
/// expires. `max_uses` of `None` is unlimited. The caller validates both
/// against the bounds above, and checks that only an instance admin sets
/// `admits_newcomers`, with both bounds.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure, including a `token_hash`
/// collision (the unique constraint).
pub async fn query_cord_invite_create<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    token_hash: &str,
    created_by: Uuid,
    expires_in_seconds: Option<u32>,
    max_uses: Option<u32>,
    admits_newcomers: bool,
) -> Result<CordInviteWithCounts, CellError> {
    let sql = format!(
        "INSERT INTO cord_invite
             (cord_id, token_hash, created_by, expires_at, max_uses, admits_newcomers)
         VALUES ($1, $2, $3, NOW() + $4::bigint * INTERVAL '1 second', $5, $6)
         RETURNING {}",
        invite_with_counts_cols("cord_invite"),
    );
    let expires_in_seconds: Option<i64> = expires_in_seconds.map(i64::from);
    let max_uses: Option<i32> = max_uses.map(i32::try_from).transpose().map_err(|_| {
        CellError::Decode("max_uses is out of range for an integer column".to_owned())
    })?;
    let row = client
        .query_one(
            &sql,
            &[
                &cord_id,
                &token_hash,
                &created_by,
                &expires_in_seconds,
                &max_uses,
                &admits_newcomers,
            ],
        )
        .await?;
    Ok(decode_invite_with_counts(&row))
}

/// Every link on the cord `cord_id`, newest first, with its redemption
/// counts.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_list<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Vec<CordInviteWithCounts>, CellError> {
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.cord_id = $1 ORDER BY i.created_at DESC, i.id DESC",
        invite_with_counts_cols("i"),
    );
    let rows = client.query(&sql, &[&cord_id]).await?;
    Ok(rows.iter().map(decode_invite_with_counts).collect())
}

/// The link whose token hashes to `token_hash`, or `None`. Without the
/// redemption counts: the preview needs none.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_by_token_hash<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
) -> Result<Option<CordInviteRow>, CellError> {
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.token_hash = $1",
        invite_cols("i"),
    );
    let row = client.query_opt(&sql, &[&token_hash]).await?;
    Ok(row.as_ref().map(decode_invite))
}

/// A revoke's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteRevoked {
    /// The link, as of the revoke.
    pub invite: CordInviteWithCounts,
    /// Whether this call revoked it; `false` when it already was.
    pub revoked: bool,
    /// The spine username invites this call deleted: the reservations of the
    /// link's redemptions that were still waiting on their signup.
    pub released_signup_invite_ids: Vec<Uuid>,
}

/// Revoke the link `invite_id` on the cord `cord_id`.
///
/// Idempotent: revoking a revoked link keeps its first `revoked_at` and
/// changes nothing else. The revoke that revokes also releases the link's
/// waiting reservations
/// ([`query_cord_invite_redemption_release`]), so the stock signup no
/// longer admits their usernames. An expired link can be revoked too, which
/// releases its reservations without waiting for the expiry sweep. Returns
/// `None` when the cord has no such link.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_revoke<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    invite_id: Uuid,
) -> Result<Option<CordInviteRevoked>, CellError> {
    let sql = format!(
        "UPDATE cord_invite SET revoked_at = NOW()
         WHERE id = $1 AND cord_id = $2 AND revoked_at IS NULL
         RETURNING {}",
        invite_with_counts_cols("cord_invite"),
    );
    if let Some(row) = client.query_opt(&sql, &[&invite_id, &cord_id]).await? {
        let released_signup_invite_ids =
            query_cord_invite_redemption_release(client, invite_id).await?;
        return Ok(Some(CordInviteRevoked {
            invite: decode_invite_with_counts(&row),
            revoked: true,
            released_signup_invite_ids,
        }));
    }
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.id = $1 AND i.cord_id = $2",
        invite_with_counts_cols("i"),
    );
    Ok(client
        .query_opt(&sql, &[&invite_id, &cord_id])
        .await?
        .map(|row| CordInviteRevoked {
            invite: decode_invite_with_counts(&row),
            revoked: false,
            released_signup_invite_ids: Vec::new(),
        }))
}

/// Whether the link `invite_id` expired more than `grace_seconds` ago, by
/// the database clock. A link with no expiry never has.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub(crate) async fn query_cord_invite_expired_beyond<C: GenericClient + ?Sized>(
    client: &C,
    invite_id: Uuid,
    grace_seconds: u32,
) -> Result<bool, CellError> {
    Ok(client
        .query_one(
            "SELECT expires_at IS NOT NULL
                AND expires_at + $2::bigint * INTERVAL '1 second' <= NOW()
             FROM cord_invite WHERE id = $1",
            &[&invite_id, &i64::from(grace_seconds)],
        )
        .await?
        .get(0))
}

/// Lock the link `invite_id` if it expired more than `grace_seconds` ago,
/// returning its cord's id.
///
/// `None` when it hasn't (by the database clock), when there's no such
/// link, or when another transaction holds its lock.
///
/// The lock is the link lock accept and redeem take (`FOR NO KEY UPDATE`),
/// taken with `SKIP LOCKED`, so the expiry sweep orders against them as revoke
/// does, and passes over a busy link instead of waiting on it.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_try_lock_expired_beyond<C: GenericClient + ?Sized>(
    client: &C,
    invite_id: Uuid,
    grace_seconds: u32,
) -> Result<Option<Uuid>, CellError> {
    Ok(client
        .query_opt(
            "SELECT cord_id FROM cord_invite
             WHERE id = $1
               AND expires_at IS NOT NULL
               AND expires_at + $2::bigint * INTERVAL '1 second' <= NOW()
             FOR NO KEY UPDATE SKIP LOCKED",
            &[&invite_id, &i64::from(grace_seconds)],
        )
        .await?
        .map(|row| row.get(0)))
}

/// Lock the link whose token hashes to `token_hash` (`FOR NO KEY UPDATE`,
/// until the transaction ends) and read it, without its redemption counts,
/// or `None`.
///
/// The link lock both accept and redeem take, each after a lock on the link's
/// minter's account (the module doc has why).
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_lock_by_token_hash<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
) -> Result<Option<CordInviteRow>, CellError> {
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.token_hash = $1 FOR NO KEY UPDATE OF i",
        invite_cols("i"),
    );
    let row = client.query_opt(&sql, &[&token_hash]).await?;
    Ok(row.as_ref().map(decode_invite))
}

/// Key-share-lock the account row of the link's minter, then lock the link:
/// accept's locks after the cord's.
///
/// `minter` is the link's `created_by` as read before the lock; the link is
/// the one whose token hashes to `token_hash`, locked as
/// [`query_cord_invite_lock_by_token_hash`] does.
///
/// `FOR KEY SHARE` conflicts only with a delete of the row (or an update of
/// a key column, which no writer of an account makes). A purge of the minter
/// takes the row `FOR UPDATE` before any write that reaches the link (the
/// unclaimed invites it deletes, one of which a fallback accept locks; the
/// actor its cascade deletes (a grant's `granted_by` keeps the purged id);
/// the link's `created_by` that cascade nulls), so it either commits first or
/// waits for the accept. It doesn't conflict with the member verbs' or a
/// login's `FOR NO KEY UPDATE`, nor with redeem's `FOR SHARE`.
///
/// `created_by` is re-checked under the link's lock. Null is fine: a purge
/// committed after the unlocked read nulled it, and the grant then names no
/// grantor, as it would have had the purge committed first. Any other change
/// would leave the accept writing a `granted_by` it holds no lock on; no
/// writer changes a link's minter, so that refuses as a broken invariant
/// rather than going on.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure, [`CellError::Decode`] when the
/// link's minter changed under the lock.
pub async fn query_cord_invite_lock_with_minter_account<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
    minter: Option<Uuid>,
) -> Result<Option<CordInviteRow>, CellError> {
    let locked_minter = match minter {
        Some(minter) => client
            .query_opt(
                "SELECT ac.id FROM account a JOIN actor ac ON ac.account_id = a.id
                 WHERE ac.id = $1
                 FOR KEY SHARE OF a",
                &[&minter],
            )
            .await?
            .map(|row| row.get::<_, Uuid>(0)),
        None => None,
    };
    let Some(invite) = query_cord_invite_lock_by_token_hash(client, token_hash).await? else {
        return Ok(None);
    };
    if let Some(minter) = invite.created_by
        && locked_minter != Some(minter)
    {
        return Err(CellError::Decode(format!(
            "the minter of cord invite {} changed under its lock",
            invite.id
        )));
    }
    Ok(Some(invite))
}

/// Spend one of the link `invite_id`'s uses. The table's CHECK refuses a
/// use past `max_uses`; callers hold the link's lock and check first.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub(crate) async fn query_cord_invite_spend_use<C: GenericClient + ?Sized>(
    client: &C,
    invite_id: Uuid,
) -> Result<(), CellError> {
    client
        .execute(
            "UPDATE cord_invite SET use_count = use_count + 1 WHERE id = $1",
            &[&invite_id],
        )
        .await?;
    Ok(())
}

/// Why accepting or redeeming a link failed. Accept refuses with the first
/// four only; the newcomer and username reasons are redeem's.
#[derive(Debug, thiserror::Error)]
pub enum CordInviteError {
    /// No link has that token, or its cord is gone.
    #[error("no such invite link")]
    NotFound,
    /// The link was revoked.
    #[error("the invite link was revoked")]
    Revoked,
    /// The link has expired.
    #[error("the invite link has expired")]
    Expired,
    /// The link has been used as many times as it allows.
    #[error("the invite link has no uses left")]
    Exhausted,
    /// The link is for people who already have an account.
    #[error("the invite link doesn't admit newcomers")]
    NotForNewcomers,
    /// The link's minter is gone or no longer an instance admin, so it
    /// brings no one new onto the instance.
    #[error("the invite link no longer admits newcomers")]
    NewcomersClosed,
    /// An account has the username, or an unclaimed invite reserves it.
    #[error("the username is taken or reserved")]
    UsernameUnavailable,
    /// A driver or decode failure.
    #[error(transparent)]
    Db(#[from] CellError),
}

/// A successful accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteAccepted {
    pub cord: CordRow,
    pub invite_id: Uuid,
    /// The caller's role in the cord now.
    pub role: CordRole,
    /// The grant created, or `None` when the caller was already a member (so
    /// nothing changed and no use was spent).
    pub role_grant_id: Option<Uuid>,
    /// The newcomer redemption this accept completed, whose use was spent
    /// when it was redeemed, or `None` for an ordinary accept.
    pub redemption_id: Option<Uuid>,
    /// The redemption's spine invite when this accept claimed it for the
    /// caller's account, because their signup claimed none; `None` otherwise.
    pub claimed_signup_invite_id: Option<Uuid>,
}

/// Accept the link whose token hashes to `token_hash`, as the actor
/// `actor_id`.
///
/// Takes the cord's scope lock shared and locks the cord's row, then
/// key-shares the minter's account row and locks the link's
/// ([`query_cord_invite_lock_with_minter_account`]), then the caller's
/// pending redemption of the link, if they have one
/// ([`query_cord_invite_redemption_pending_for_actor`]): an unaccepted
/// redemption whose signup invite the caller's account claimed, or, when
/// their signup claimed none (open signup, or an older invite matching their
/// email won), one whose signup invite is still unclaimed and names the
/// caller's username. Completing the latter claims that invite for the
/// caller's account ([`query_cord_invite_redemption_complete`]). Once that
/// unclaimed invite is gone (a delete or purge of the minter voids it), the
/// fallback finds nothing: the reservation's use stays spent, and the caller
/// accepts as anyone else, spending another.
///
/// - A caller already in the cord gets their current role back and spends
///   no use, whatever the link's state (completing their pending
///   redemption, if any).
/// - A caller with a pending redemption joins unless the link was revoked
///   or expired more than [`CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS`]
///   ago: the use was spent at redemption, so an exhausted link still
///   admits them. The minter's authority isn't re-read: it was checked when
///   the link was redeemed, and it confers nothing here but the cord role.
/// - Anyone else needs an active link, and spends a use.
///
/// A join grants the link's role on the cord, in the caller's transaction,
/// with the link's minter as its `granted_by` (who may since have left the
/// cord), or none, once their actor is purged.
///
/// # Errors
///
/// [`CordInviteError`] naming why the link can't be used (`NotFound`,
/// `Revoked`, `Expired`, or `Exhausted`), or a driver failure.
pub async fn query_cord_invite_accept<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
    actor_id: Uuid,
) -> Result<CordInviteAccepted, CordInviteError> {
    // the cord's scope lock first, holding no row, then the cord's lock, then
    // the minter's, then the link's (the module doc has why); a link's cord
    // never changes, so it is read unlocked, and the grant below re-takes the
    // scope lock, free
    let Some(unlocked) = query_cord_invite_by_token_hash(client, token_hash).await? else {
        return Err(CordInviteError::NotFound);
    };
    query_scope_lock_shared(client, unlocked.cord_id)
        .await
        .map_err(CellError::Db)?;
    let Some(cord) = query_cord_lock(client, unlocked.cord_id).await? else {
        return Err(CordInviteError::NotFound);
    };
    let Some(invite) =
        query_cord_invite_lock_with_minter_account(client, token_hash, unlocked.created_by).await?
    else {
        return Err(CordInviteError::NotFound);
    };
    let pending = if invite.admits_newcomers {
        query_cord_invite_redemption_pending_for_actor(client, invite.id, actor_id).await?
    } else {
        None
    };
    let redemption_id = pending.as_ref().map(|p| p.id);
    // every successful path below completes `pending`, claiming this invite
    let claimed_signup_invite_id = pending.as_ref().and_then(|p| {
        p.unclaimed_signup
            .map(|(signup_invite_id, _)| signup_invite_id)
    });
    if let Some(role) = query_cord_member_role(client, invite.cord_id, actor_id).await? {
        if let Some(pending) = &pending {
            query_cord_invite_redemption_complete(client, pending, actor_id).await?;
        }
        return Ok(CordInviteAccepted {
            cord,
            invite_id: invite.id,
            role,
            role_grant_id: None,
            redemption_id,
            claimed_signup_invite_id,
        });
    }
    match (invite.status(), &pending) {
        (CordInviteStatus::Active, _) | (CordInviteStatus::Exhausted, Some(_)) => {}
        (CordInviteStatus::Expired, Some(_)) => {
            if query_cord_invite_expired_beyond(
                client,
                invite.id,
                CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS,
            )
            .await?
            {
                return Err(CordInviteError::Expired);
            }
        }
        (CordInviteStatus::Revoked, _) => return Err(CordInviteError::Revoked),
        (CordInviteStatus::Expired, None) => return Err(CordInviteError::Expired),
        (CordInviteStatus::Exhausted, None) => return Err(CordInviteError::Exhausted),
    }
    let role = CordRole::from_role(&invite.role)
        .ok_or_else(|| CellError::Decode(format!("not a cord role: {}", invite.role)))?;
    if let Some(pending) = &pending {
        // the use was spent when the link was redeemed; completed before the
        // grant, so its claim of the signup invite, which names the caller's
        // account, waits out a purge of that account holding no lock on the
        // caller's actor, which the purge's cascade deletes once it holds the
        // account
        query_cord_invite_redemption_complete(client, pending, actor_id).await?;
    } else {
        query_cord_invite_spend_use(client, invite.id).await?;
    }
    let role_grant_id =
        query_cord_role_grant_create(client, invite.cord_id, actor_id, role, invite.created_by)
            .await?;
    Ok(CordInviteAccepted {
        cord,
        invite_id: invite.id,
        role,
        role_grant_id: Some(role_grant_id),
        redemption_id,
        claimed_signup_invite_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_token_debug_redacts_the_token() {
        let minted = generate_cord_invite_token();
        let rendered = format!("{minted:?}");
        assert!(!rendered.contains(&minted.token), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(rendered.contains(&minted.token_hash), "{rendered}");
    }

    #[test]
    fn read_columns_omit_only_the_token_hash() {
        assert_eq!(
            fuz_db::omit_columns(&CORD_INVITE_COLUMNS, &["token_hash"]),
            CORD_INVITE_READ_COLUMNS
        );
    }

    #[test]
    fn status_prefers_the_most_deliberate_reason() {
        use CordInviteStatus::{Active, Exhausted, Expired, Revoked};
        assert_eq!(cord_invite_status(false, false, None, 0), Active);
        assert_eq!(cord_invite_status(false, false, None, 5000), Active);
        assert_eq!(cord_invite_status(false, false, Some(2), 1), Active);
        assert_eq!(cord_invite_status(false, false, Some(2), 2), Exhausted);
        assert_eq!(cord_invite_status(false, true, Some(2), 2), Expired);
        assert_eq!(cord_invite_status(true, true, Some(2), 2), Revoked);
        assert_eq!(cord_invite_status(true, false, None, 0), Revoked);
    }

    #[test]
    fn statuses_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_value(CordInviteStatus::Exhausted).unwrap(),
            serde_json::json!("exhausted")
        );
    }

    #[test]
    fn tokens_are_random_and_hash_to_their_stored_digest() {
        let a = generate_cord_invite_token();
        let b = generate_cord_invite_token();
        assert_ne!(a.token, b.token);
        assert_eq!(a.token.len(), 43, "32 bytes, base64url, unpadded");
        assert!(is_cord_invite_token_in_bounds(&a.token));
        assert_eq!(hash_cord_invite_token(&a.token), a.token_hash);
        assert_eq!(a.token_hash.len(), 64);
        assert_ne!(a.token_hash, a.token, "the digest is not the token");
    }

    #[test]
    fn token_bounds_count_utf16_code_units_like_the_ts_schema() {
        assert!(!is_cord_invite_token_in_bounds(""));
        assert!(!is_cord_invite_token_in_bounds(
            &"a".repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1)
        ));
        assert!(is_cord_invite_token_in_bounds(
            &"a".repeat(CORD_INVITE_TOKEN_LENGTH_MAX)
        ));
        // two bytes, one code unit each: in bounds though over the max in bytes
        assert!(is_cord_invite_token_in_bounds(
            &"é".repeat(CORD_INVITE_TOKEN_LENGTH_MAX)
        ));
        // four bytes, two code units (a surrogate pair) each
        assert!(is_cord_invite_token_in_bounds(
            &"😀".repeat(CORD_INVITE_TOKEN_LENGTH_MAX / 2)
        ));
        assert!(!is_cord_invite_token_in_bounds(
            &"😀".repeat(CORD_INVITE_TOKEN_LENGTH_MAX / 2 + 1)
        ));
    }
}
