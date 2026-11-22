//! Queries for the `cord_invite_redemption` table: newcomer links redeemed
//! for a username.
//!
//! A link minted by an instance admin with `admits_newcomers` can bring
//! someone with no account onto the instance. They redeem the link for a
//! username ([`query_cord_invite_redeem`]), which reserves it as a spine
//! username invite created in the minter's name, then sign up through the
//! spine's stock invite-gated signup, which claims that invite, then accept
//! the link signed in ([`crate::invite::queries::query_cord_invite_accept`]).
//! The redemption row ties the three together: the link, the signup invite,
//! and, once they accept, the actor who joined.
//!
//! The spine's username invite carries no secret: whoever signs up first
//! with the reserved username claims it. Revoking the link deletes the
//! reservations still waiting on their signup
//! ([`query_cord_invite_redemption_release`]), and so does the expiry sweep
//! ([`crate::invite::sweep`]) once the link has been expired for longer
//! than the accept grace
//! ([`query_cord_invite_expired_with_waiting_redemptions`]).
//!
//! The link's use is spent at redemption, so the accept that follows spends
//! none and still succeeds once the link's other uses run out. When the
//! newcomer's signup claimed no invite (under open signup, or because an
//! older invite matching their email won), accept finds the redemption by
//! their username instead and claims its invite then.
//!
//! Redemption locks the link's row, as accept does, so concurrent
//! redemptions and accepts of one link run one at a time and never spend
//! more uses than it has. Before it, the minter's account row is taken under
//! a share lock (the link read unlocked to find the minter, then re-checked
//! under its lock), so a purge of the minter, which holds that row and then
//! sets the link's `created_by` null, either commits first or waits for the
//! redemption. A new reservation then reads the minter's global `admin` grant
//! under another share lock, account before grant as the last-admin guard
//! takes them, so a delete of the minter or a revoke of the grant either
//! commits first and refuses the redemption, or waits for the redemption to
//! commit (a delete then deletes its signup invite with the minter's other
//! unclaimed ones). Redeeming a username already waiting on the link doesn't
//! read the minter's `admin` grant, as accept doesn't re-read the minter: the
//! reservation was authorized when it was made.
//!
//! Every function takes any `GenericClient`; call the writers inside a
//! transaction.

use deadpool_postgres::GenericClient;
use fuz_auth::ROLE_ADMIN;
use fuz_auth::account_queries::{query_account_by_username, query_account_lock_live_many};
use fuz_auth::invite_queries::{
    InviteCreateError, query_create_invite, query_invite_claim_unscoped,
    query_invite_delete_unclaimed,
};
use fuz_cell::CellError;
use fuz_db::tokio_postgres::Row;
use fuz_db::{col, iso8601_timestamp_expr, qualify_columns};
use uuid::Uuid;

use crate::cord_member_queries::live_role_grant_sql;
use crate::cord_queries::{CordRow, query_cord_get};
use crate::invite::queries::{
    CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS, CordInviteError, CordInviteRow, CordInviteStatus,
    query_cord_invite_by_token_hash, query_cord_invite_expired_beyond,
    query_cord_invite_lock_by_token_hash, query_cord_invite_spend_use,
};

/// The `cord_invite_redemption` table's columns, in projection order. The
/// PG-gated `columns` test checks they name every live column.
pub const CORD_INVITE_REDEMPTION_COLUMNS: [&str; 7] = [
    "id",
    "cord_invite_id",
    "signup_invite_id",
    "username",
    "created_at",
    "accepted_by",
    "accepted_at",
];

const CORD_INVITE_REDEMPTION_TIMESTAMP_COLUMNS: [&str; 2] = ["created_at", "accepted_at"];

/// A `cord_invite_redemption` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteRedemptionRow {
    pub id: Uuid,
    /// The link redeemed.
    pub cord_invite_id: Uuid,
    /// The spine username invite the redemption created, or `None` once an
    /// admin deleted it unclaimed.
    pub signup_invite_id: Option<Uuid>,
    /// The reserved username, lowercased.
    pub username: String,
    /// UTC ISO-8601, second precision, as is `accepted_at`.
    pub created_at: String,
    /// The actor who accepted the link after signing up.
    pub accepted_by: Option<Uuid>,
    pub accepted_at: Option<String>,
}

/// The projection qualified by `alias`, timestamps ISO-formatted.
fn redemption_cols(alias: &str) -> String {
    qualify_columns(
        &CORD_INVITE_REDEMPTION_COLUMNS,
        alias,
        iso8601_timestamp_expr(
            &CORD_INVITE_REDEMPTION_COLUMNS,
            &CORD_INVITE_REDEMPTION_TIMESTAMP_COLUMNS,
            alias,
        ),
    )
}

/// Decode the [`redemption_cols`] projection positionally.
fn decode_redemption(row: &Row) -> CordInviteRedemptionRow {
    CordInviteRedemptionRow {
        id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "id")),
        cord_invite_id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "cord_invite_id")),
        signup_invite_id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "signup_invite_id")),
        username: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "username")),
        created_at: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "created_at")),
        accepted_by: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "accepted_by")),
        accepted_at: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "accepted_at")),
    }
}

/// Selects a row when the actor `$1` holds a live global `$2` (`admin`)
/// grant ([`live_role_grant_sql`]); `lock` is the locking clause, if any.
fn global_admin_grant_sql(lock: &str) -> String {
    live_role_grant_sql(
        "1",
        &format!("rg.actor_id = $1 AND rg.role = $2 AND rg.scope_id IS NULL LIMIT 1 {lock}"),
    )
}

/// Whether the actor `actor_id` holds an active **global** `admin` grant, on
/// a live actor of a live account: the authority a newcomer link needs,
/// both when it's minted and whenever it's redeemed.
///
/// A locking read: `FOR SHARE OF rg` takes a share lock on the admitting
/// grant, so a concurrent revoke of it is ordered against this transaction
/// (it commits first and this read excludes the grant, or it waits for this
/// transaction). The actor and account tombstones are read but not locked,
/// as in the spine's own re-check.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_actor_holds_global_admin<C: GenericClient + ?Sized>(
    client: &C,
    actor_id: Uuid,
) -> Result<bool, CellError> {
    Ok(client
        .query_opt(
            &global_admin_grant_sql("FOR SHARE OF rg"),
            &[&actor_id, &ROLE_ADMIN],
        )
        .await?
        .is_some())
}

/// [`query_actor_holds_global_admin`] without the lock, for reads that
/// decide nothing: the preview's `newcomers_open`.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_actor_holds_global_admin_unlocked<C: GenericClient + ?Sized>(
    client: &C,
    actor_id: Uuid,
) -> Result<bool, CellError> {
    Ok(client
        .query_opt(&global_admin_grant_sql(""), &[&actor_id, &ROLE_ADMIN])
        .await?
        .is_some())
}

/// A successful redemption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteRedeemed {
    pub cord: CordRow,
    pub invite_id: Uuid,
    pub redemption: CordInviteRedemptionRow,
    /// `false` when this username's redemption through the link was already
    /// waiting, so nothing changed and no use was spent.
    pub created: bool,
}

/// Redeem the newcomer link whose token hashes to `token_hash` for
/// `username`, which the caller has validated with the spine's username
/// rules and lowercased.
///
/// In order: read the link unlocked and share-lock its minter's account
/// (the module doc has why); lock the link; refuse it unknown (or its cord
/// gone), revoked, without `admits_newcomers`, or expired more than
/// [`CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS`] ago. Then, when this
/// username's redemption through the link is still waiting (unaccepted, its
/// signup invite unclaimed, and no account holding the username), return it
/// unchanged, spending nothing: accept
/// would still honor that reservation, so a newcomer retrying after a failed
/// signup can redeem it again. Otherwise the link must be open to a new
/// reservation: refuse it expired, or closed because its minter's account is
/// deleted or the minter no longer holds a global `admin`
/// ([`query_actor_holds_global_admin`]; a purged minter's null `created_by`
/// refuses too), or exhausted; refuse a username
/// an account has (live or soft-deleted) or an unclaimed spine invite
/// reserves; then create the spine username invite in the minter's name,
/// record the redemption, and spend one use.
///
/// So a waiting username is answered with `created: false` wherever accept
/// would take it: on an active or exhausted link, on one expired within the
/// grace, and on one whose minter is no longer an admin (accept doesn't
/// re-read the minter). A dead link, revoked, expired past the grace, or not
/// for newcomers, refuses a waiting username as it would any other, so it
/// tells nobody which usernames are waiting on it.
///
/// # Errors
///
/// [`CordInviteError`] naming why the link can't be redeemed for the
/// username, or a driver failure.
pub async fn query_cord_invite_redeem<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
    username: &str,
) -> Result<CordInviteRedeemed, CordInviteError> {
    let Some(unlocked) = query_cord_invite_by_token_hash(client, token_hash).await? else {
        return Err(CordInviteError::NotFound);
    };
    let live_minter = match unlocked.created_by {
        Some(minter) if unlocked.admits_newcomers => query_lock_live_minter(client, minter).await?,
        _ => None,
    };
    let Some(invite) = query_cord_invite_lock_by_token_hash(client, token_hash).await? else {
        return Err(CordInviteError::NotFound);
    };
    // no cord lock: a redemption changes no membership
    let Some(cord) = query_cord_get(client, invite.cord_id).await? else {
        return Err(CordInviteError::NotFound);
    };
    let status = invite.status();
    if status == CordInviteStatus::Revoked {
        return Err(CordInviteError::Revoked);
    }
    if !invite.admits_newcomers {
        return Err(CordInviteError::NotForNewcomers);
    }
    if status == CordInviteStatus::Expired
        && query_cord_invite_expired_beyond(
            client,
            invite.id,
            CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS,
        )
        .await?
    {
        return Err(CordInviteError::Expired);
    }
    // a waiting reservation is answered wherever accept would still honor it
    if let Some(redemption) = query_waiting_redemption(client, invite.id, username).await? {
        return Ok(CordInviteRedeemed {
            cord,
            invite_id: invite.id,
            redemption,
            created: false,
        });
    }
    // a new reservation needs a live link and a minter who is still an admin
    if status == CordInviteStatus::Expired {
        return Err(CordInviteError::Expired);
    }
    let minted_by = require_admin_minter(client, &invite, live_minter).await?;
    if status == CordInviteStatus::Exhausted {
        return Err(CordInviteError::Exhausted);
    }
    if query_account_by_username(client, username)
        .await
        .map_err(CellError::Db)?
        .is_some()
        || query_username_reserved(client, username).await?
    {
        return Err(CordInviteError::UsernameUnavailable);
    }
    let signup_invite = match query_create_invite(client, None, Some(username), &minted_by).await {
        Ok(signup_invite) => signup_invite,
        // a concurrent reservation of the same username committed first
        Err(InviteCreateError::Duplicate) => return Err(CordInviteError::UsernameUnavailable),
        Err(InviteCreateError::IssuerNotActive) => return Err(CordInviteError::NewcomersClosed),
        Err(InviteCreateError::Query(e)) => return Err(CellError::Db(e).into()),
    };
    let sql = format!(
        "INSERT INTO cord_invite_redemption (cord_invite_id, signup_invite_id, username)
         VALUES ($1, $2, $3)
         RETURNING {}",
        redemption_cols("cord_invite_redemption"),
    );
    let redemption = decode_redemption(
        &client
            .query_one(&sql, &[&invite.id, &signup_invite.id, &username])
            .await
            .map_err(CellError::Db)?,
    );
    query_cord_invite_spend_use(client, invite.id).await?;
    Ok(CordInviteRedeemed {
        cord,
        invite_id: invite.id,
        redemption,
        created: true,
    })
}

/// Share-lock the account row of the actor `minter`, returning `minter` when
/// the account is live, `None` when it's deleted, tombstoned, or the actor is
/// gone.
///
/// Taken before the link's lock, under the share lock `query_create_invite`
/// takes on its issuer's account. A removal of the minter takes that row
/// first: a purge then sets the link's `created_by` null through its
/// cascade, so the link's lock taken first would hold what the cascade waits
/// on while waiting on the account the purge holds. Every removal also takes
/// the row before the minter's `admin` grant (the last-admin guard's order),
/// which [`require_admin_minter`] reads after it. A removal committed first is
/// waited for and closes the link to new reservations; one arriving later
/// waits for the redemption, and a delete or purge then deletes its signup
/// invite with the minter's other unclaimed ones.
async fn query_lock_live_minter<C: GenericClient + ?Sized>(
    client: &C,
    minter: Uuid,
) -> Result<Option<Uuid>, CordInviteError> {
    // an actor's account never changes, so it is read unlocked
    let Some(minter_account) = client
        .query_opt("SELECT account_id FROM actor WHERE id = $1", &[&minter])
        .await
        .map_err(CellError::Db)?
        .map(|row| row.get::<_, Uuid>(0))
    else {
        return Ok(None);
    };
    let live = query_account_lock_live_many(client, &[minter_account])
        .await
        .map_err(CellError::Db)?
        .contains(&minter_account);
    Ok(live.then_some(minter))
}

/// The link's minter, if it is `live_minter` (whose account
/// [`query_lock_live_minter`] locked live before the link's lock) and still
/// holds a global `admin`; a null `created_by` fails closed.
///
/// `created_by` is re-read under the link's lock: a purge of the minter
/// committed between the unlocked read and the account lock has nulled it,
/// and any other change leaves the account locked for an actor that is no
/// longer the minter, so both refuse.
async fn require_admin_minter<C: GenericClient + ?Sized>(
    client: &C,
    invite: &CordInviteRow,
    live_minter: Option<Uuid>,
) -> Result<Uuid, CordInviteError> {
    let Some(minted_by) = invite.created_by else {
        return Err(CordInviteError::NewcomersClosed);
    };
    if live_minter != Some(minted_by) {
        return Err(CordInviteError::NewcomersClosed);
    }
    if !query_actor_holds_global_admin(client, minted_by).await? {
        return Err(CordInviteError::NewcomersClosed);
    }
    Ok(minted_by)
}

/// The redemption of `username` through the link `cord_invite_id` that is
/// still waiting on its newcomer: unaccepted, with its signup invite still
/// there and unclaimed, and no account (live or soft-deleted) holding the
/// username, compared lowercased as the spine compares usernames. An account
/// can take a waiting username without claiming its invite (under open
/// signup, or through an older email invite); the reservation can then never
/// be signed up, so redeem doesn't answer it as waiting: a redeem of the name
/// falls through to the new-username path, which refuses it. (Release and the
/// sweep still count it among a link's waiting reservations; dropping its
/// stale signup invite is harmless.)
async fn query_waiting_redemption<C: GenericClient + ?Sized>(
    client: &C,
    cord_invite_id: Uuid,
    username: &str,
) -> Result<Option<CordInviteRedemptionRow>, CellError> {
    let sql = format!(
        "SELECT {} FROM cord_invite_redemption r
         JOIN invite i ON i.id = r.signup_invite_id
         WHERE r.cord_invite_id = $1
           AND r.username = $2
           AND r.accepted_at IS NULL
           AND i.claimed_at IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM account a WHERE LOWER(a.username) = LOWER($2)
           )",
        redemption_cols("r"),
    );
    Ok(client
        .query_opt(&sql, &[&cord_invite_id, &username])
        .await?
        .as_ref()
        .map(decode_redemption))
}

/// Whether an unclaimed spine invite names `username`, which the spine's
/// unique index would refuse to reserve again.
async fn query_username_reserved<C: GenericClient + ?Sized>(
    client: &C,
    username: &str,
) -> Result<bool, CellError> {
    Ok(client
        .query_opt(
            "SELECT 1 FROM invite
             WHERE claimed_at IS NULL AND username IS NOT NULL
               AND LOWER(username) = LOWER($1)
             LIMIT 1",
            &[&username],
        )
        .await?
        .is_some())
}

/// A redemption waiting on its newcomer's accept, locked by
/// [`query_cord_invite_redemption_pending_for_actor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteRedemptionPending {
    pub id: Uuid,
    /// The redemption's signup invite and the caller's account, when the
    /// invite is still unclaimed: completing the redemption claims it.
    pub unclaimed_signup: Option<(Uuid, Uuid)>,
}

/// The actor's pending redemption of the link `cord_invite_id`, locked.
///
/// That is an unaccepted redemption whose signup invite was claimed by the
/// account the actor `actor_id` belongs to, or failing that, one whose
/// signup invite is still unclaimed and names that account's username
/// (lowercased, as the spine matches it).
///
/// The fallback covers a signup that claimed no invite: under open signup
/// the spine claims none, and with an older email-only invite matching the
/// signup's email it claims that one. It trusts the username exactly as far
/// as the spine's own signup would have, since that invite admits whoever
/// signs up with the username.
///
/// The redemption, and in the fallback its signup invite, stay locked until
/// the transaction ends.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_redemption_pending_for_actor<C: GenericClient + ?Sized>(
    client: &C,
    cord_invite_id: Uuid,
    actor_id: Uuid,
) -> Result<Option<CordInviteRedemptionPending>, CellError> {
    if let Some(row) = client
        .query_opt(
            "SELECT r.id FROM cord_invite_redemption r
             JOIN invite i ON i.id = r.signup_invite_id
             JOIN actor ac ON ac.account_id = i.claimed_by
             WHERE r.cord_invite_id = $1
               AND ac.id = $2
               AND r.accepted_at IS NULL
             ORDER BY r.created_at, r.id
             LIMIT 1
             FOR NO KEY UPDATE OF r",
            &[&cord_invite_id, &actor_id],
        )
        .await?
    {
        return Ok(Some(CordInviteRedemptionPending {
            id: row.get(0),
            unclaimed_signup: None,
        }));
    }
    Ok(client
        .query_opt(
            "SELECT r.id, i.id, a.id FROM cord_invite_redemption r
             JOIN invite i ON i.id = r.signup_invite_id
             JOIN actor ac ON ac.id = $2
             JOIN account a ON a.id = ac.account_id
             WHERE r.cord_invite_id = $1
               AND r.accepted_at IS NULL
               AND i.claimed_at IS NULL
               AND r.username = LOWER(a.username)
             ORDER BY r.created_at, r.id
             LIMIT 1
             FOR NO KEY UPDATE OF r, i",
            &[&cord_invite_id, &actor_id],
        )
        .await?
        .map(|row| CordInviteRedemptionPending {
            id: row.get(0),
            unclaimed_signup: Some((row.get(1), row.get(2))),
        }))
}

/// Mark the pending redemption `pending` accepted by the actor `actor_id`,
/// now, first claiming its signup invite for the caller's account when their
/// signup didn't.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure, or [`CellError::Decode`] if the
/// locked signup invite was somehow claimed already.
pub async fn query_cord_invite_redemption_complete<C: GenericClient + ?Sized>(
    client: &C,
    pending: &CordInviteRedemptionPending,
    actor_id: Uuid,
) -> Result<(), CellError> {
    if let Some((signup_invite_id, account_id)) = pending.unclaimed_signup
        && !query_invite_claim_unscoped(client, &signup_invite_id, &account_id).await?
    {
        return Err(CellError::Decode(format!(
            "signup invite {signup_invite_id} was claimed despite its row lock"
        )));
    }
    client
        .execute(
            "UPDATE cord_invite_redemption SET accepted_by = $2, accepted_at = NOW()
             WHERE id = $1 AND accepted_at IS NULL",
            &[&pending.id, &actor_id],
        )
        .await?;
    Ok(())
}

/// Delete the spine username invites of the link `cord_invite_id`'s
/// redemptions that are still waiting on their signup, through the spine's
/// own [`query_invite_delete_unclaimed`], and return their ids.
///
/// Callers hold the link's lock and call this once the link refuses its
/// newcomers' accepts: revoked, or expired past the accept grace. A claimed
/// invite stays: its account exists, and the dead link refuses its accept.
/// The redemptions stay as the record, their `signup_invite_id` nulled by
/// the foreign key, so nothing can accept through them.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_redemption_release<C: GenericClient + ?Sized>(
    client: &C,
    cord_invite_id: Uuid,
) -> Result<Vec<Uuid>, CellError> {
    let waiting: Vec<Uuid> = client
        .query(
            "SELECT i.id FROM cord_invite_redemption r
             JOIN invite i ON i.id = r.signup_invite_id
             WHERE r.cord_invite_id = $1
               AND r.accepted_at IS NULL
               AND i.claimed_at IS NULL
             ORDER BY r.created_at, r.id",
            &[&cord_invite_id],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    let mut released = Vec::with_capacity(waiting.len());
    for signup_invite_id in waiting {
        // rechecks `claimed_at`, so a signup that claimed it meanwhile keeps it
        if query_invite_delete_unclaimed(client, &signup_invite_id).await? {
            released.push(signup_invite_id);
        }
    }
    Ok(released)
}

/// The links that expired more than `grace_seconds` ago and still have
/// waiting reservations: what the expiry sweep releases.
///
/// A waiting reservation is an unaccepted redemption whose spine invite is
/// unclaimed. Expiry is by the database clock; the soonest expired come
/// first. Revoked links qualify too; their revoke released what was waiting
/// then.
///
/// Unlocked: the sweep locks each link on its own before releasing.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_expired_with_waiting_redemptions<C: GenericClient + ?Sized>(
    client: &C,
    grace_seconds: u32,
) -> Result<Vec<Uuid>, CellError> {
    Ok(client
        .query(
            "SELECT ci.id FROM cord_invite ci
             WHERE ci.expires_at IS NOT NULL
               AND ci.expires_at + $1::bigint * INTERVAL '1 second' <= NOW()
               AND EXISTS (
                 SELECT 1 FROM cord_invite_redemption r
                 JOIN invite i ON i.id = r.signup_invite_id
                 WHERE r.cord_invite_id = ci.id
                   AND r.accepted_at IS NULL
                   AND i.claimed_at IS NULL
               )
             ORDER BY ci.expires_at, ci.id",
            &[&i64::from(grace_seconds)],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}
