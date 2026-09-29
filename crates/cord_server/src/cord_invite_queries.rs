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
//! Accepting a link ([`query_cord_invite_accept`]) locks the link's row, then
//! the cord's (as the member changes in [`crate::cord_member_queries`] do),
//! so concurrent accepts can't push `use_count` past `max_uses` and can't
//! race a membership change. Both locks are `FOR NO KEY UPDATE`: enough to
//! serialize the writers (a revoke's UPDATE waits on the link's lock), and
//! no stronger. The table's own CHECK refuses an overshoot too.
//!
//! Every function takes any `GenericClient`, so it composes on a pooled
//! client or inside the action's transaction.

use deadpool_postgres::GenericClient;
use fuz_auth::{generate_session_token, hash_session_token, query_create_role_grant};
use fuz_cell::CellError;
use fuz_db::tokio_postgres::Row;
use fuz_db::{col, iso8601_timestamp_expr, qualify_columns};
use serde::Serialize;
use uuid::Uuid;

use crate::cord_authz::CordRole;
use crate::cord_member_queries::{query_cord_lock, query_cord_members};
use crate::cord_queries::{CordRow, query_cord_get};
use crate::roles::CORD_SCOPE_KIND;

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

/// The longest token, in bytes, that a lookup hashes.
///
/// Anything longer is no token Cord minted; generated tokens are 43 ASCII
/// characters. The frontend's `CordInviteToken` schema twins it.
pub const CORD_INVITE_TOKEN_LENGTH_MAX: usize = 128;

/// The shortest expiry a link may be given, in seconds.
pub const CORD_INVITE_EXPIRES_IN_SECONDS_MIN: u32 = 60;
/// The longest expiry a link may be given, in seconds (365 days).
pub const CORD_INVITE_EXPIRES_IN_SECONDS_MAX: u32 = 365 * 24 * 60 * 60;
/// The most uses a link may be given.
pub const CORD_INVITE_MAX_USES_MAX: u32 = 1000;

/// A fresh link token and its digest, the `token_hash` to store.
#[derive(Debug, Clone)]
pub struct CordInviteToken {
    /// Shown once, to whoever created the link; never stored.
    pub token: String,
    pub token_hash: String,
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

/// Whether `token` is shaped like something Cord could have minted.
///
/// That is, not empty and at most [`CORD_INVITE_TOKEN_LENGTH_MAX`] bytes.
/// Anything else is answered as an unknown link without a lookup, not as a
/// params error, so a malformed token gets the same answer as a wrong one.
#[must_use]
pub const fn is_plausible_cord_invite_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= CORD_INVITE_TOKEN_LENGTH_MAX
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
/// over exhaustion, so a link reports the most deliberate reason it's dead.
#[must_use]
pub fn cord_invite_status(
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

/// A `cord_invite` row, without its `token_hash`.
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

/// Create a link on the cord `cord_id` conferring `cord_member`.
///
/// `expires_in_seconds` counts from the database's now; `None` never
/// expires. `max_uses` of `None` is unlimited. The caller validates both
/// against the bounds above.
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
) -> Result<CordInviteRow, CellError> {
    let sql = format!(
        "INSERT INTO cord_invite (cord_id, token_hash, created_by, expires_at, max_uses)
         VALUES ($1, $2, $3, NOW() + $4::bigint * INTERVAL '1 second', $5)
         RETURNING {}",
        invite_cols("cord_invite"),
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
            ],
        )
        .await?;
    Ok(decode_invite(&row))
}

/// Every link on the cord `cord_id`, newest first.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_list<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Vec<CordInviteRow>, CellError> {
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.cord_id = $1 ORDER BY i.created_at DESC, i.id DESC",
        invite_cols("i"),
    );
    let rows = client.query(&sql, &[&cord_id]).await?;
    Ok(rows.iter().map(decode_invite).collect())
}

/// The link whose token hashes to `token_hash`, or `None`.
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

/// Revoke the link `invite_id` on the cord `cord_id`.
///
/// Idempotent: revoking a revoked link keeps its first `revoked_at`. Returns
/// the link and whether this call revoked it, or `None` when the cord has no
/// such link.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_invite_revoke<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    invite_id: Uuid,
) -> Result<Option<(CordInviteRow, bool)>, CellError> {
    let sql = format!(
        "UPDATE cord_invite SET revoked_at = NOW()
         WHERE id = $1 AND cord_id = $2 AND revoked_at IS NULL
         RETURNING {}",
        invite_cols("cord_invite"),
    );
    if let Some(row) = client.query_opt(&sql, &[&invite_id, &cord_id]).await? {
        return Ok(Some((decode_invite(&row), true)));
    }
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.id = $1 AND i.cord_id = $2",
        invite_cols("i"),
    );
    Ok(client
        .query_opt(&sql, &[&invite_id, &cord_id])
        .await?
        .map(|row| (decode_invite(&row), false)))
}

/// Why accepting a link failed.
#[derive(Debug, thiserror::Error)]
pub enum CordInviteAcceptError {
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
}

/// Accept the link whose token hashes to `token_hash`, as the actor
/// `actor_id`.
///
/// Locks the link's row, then the cord's. A caller already in the cord gets
/// their current role back and spends no use, whatever the link's state.
/// Otherwise the link must be active: then the caller is granted the link's
/// role on the cord and `use_count` goes up by one, in the caller's
/// transaction.
///
/// # Errors
///
/// [`CordInviteAcceptError`] naming why the link can't be used, or a driver
/// failure.
pub async fn query_cord_invite_accept<C: GenericClient + ?Sized>(
    client: &C,
    token_hash: &str,
    actor_id: Uuid,
) -> Result<CordInviteAccepted, CordInviteAcceptError> {
    let sql = format!(
        "SELECT {} FROM cord_invite i WHERE i.token_hash = $1 FOR NO KEY UPDATE",
        invite_cols("i"),
    );
    let Some(invite) = client
        .query_opt(&sql, &[&token_hash])
        .await
        .map_err(CellError::Db)?
        .as_ref()
        .map(decode_invite)
    else {
        return Err(CordInviteAcceptError::NotFound);
    };
    if !query_cord_lock(client, invite.cord_id).await? {
        return Err(CordInviteAcceptError::NotFound);
    }
    let Some(cord) = query_cord_get(client, invite.cord_id).await? else {
        return Err(CordInviteAcceptError::NotFound);
    };
    let members = query_cord_members(client, invite.cord_id).await?;
    if let Some(member) = members.iter().find(|m| m.actor_id == actor_id) {
        return Ok(CordInviteAccepted {
            cord,
            invite_id: invite.id,
            role: member.role,
            role_grant_id: None,
        });
    }
    match invite.status() {
        CordInviteStatus::Active => {}
        CordInviteStatus::Revoked => return Err(CordInviteAcceptError::Revoked),
        CordInviteStatus::Expired => return Err(CordInviteAcceptError::Expired),
        CordInviteStatus::Exhausted => return Err(CordInviteAcceptError::Exhausted),
    }
    let role = CordRole::from_role(&invite.role)
        .ok_or_else(|| CellError::Decode(format!("not a cord role: {}", invite.role)))?;
    let grant = query_create_role_grant(
        client,
        &actor_id,
        role.as_str(),
        Some(CORD_SCOPE_KIND),
        Some(&invite.cord_id),
        invite.created_by.as_ref(),
        None,
    )
    .await
    .map_err(CellError::Db)?;
    client
        .execute(
            "UPDATE cord_invite SET use_count = use_count + 1 WHERE id = $1",
            &[&invite.id],
        )
        .await
        .map_err(CellError::Db)?;
    Ok(CordInviteAccepted {
        cord,
        invite_id: invite.id,
        role,
        role_grant_id: Some(grant.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(is_plausible_cord_invite_token(&a.token));
        assert_eq!(hash_cord_invite_token(&a.token), a.token_hash);
        assert_eq!(a.token_hash.len(), 64);
        assert_ne!(a.token_hash, a.token, "the digest is not the token");
    }

    #[test]
    fn implausible_tokens_are_refused_before_a_lookup() {
        assert!(!is_plausible_cord_invite_token(""));
        assert!(!is_plausible_cord_invite_token(
            &"a".repeat(CORD_INVITE_TOKEN_LENGTH_MAX + 1)
        ));
        assert!(is_plausible_cord_invite_token(
            &"a".repeat(CORD_INVITE_TOKEN_LENGTH_MAX)
        ));
    }
}
