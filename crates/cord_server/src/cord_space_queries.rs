//! Queries for a cord's spaces, over the spine's `cell` and `cell_grant`
//! tables.
//!
//! A space is a `cell` row with `kind = 'space'`, a child of its cord's cell
//! (`parent_id` and `root_id` both the cord), private, and born with
//! `created_by = NULL` like the cord itself, so role grants are its only
//! authority. Its fields live in the cell's `data` ([`CordSpaceData`]): the
//! space's own kind ([`CordSpaceKind`]) and its name. One cell kind for every
//! space keeps a cord's spaces one query away whatever their kind.
//!
//! The cord's three role-shaped `cell_grant` rows are stamped on each space
//! too, scoped to the cord ([`crate::cord_queries::CORD_STAMPED_GRANTS`]), so
//! the spine's view checks admit exactly the cord's members, as they do for
//! the cord. As for the cord, the grants don't open the spine's edit paths
//! (`can_edit_cell` treats a `created_by = NULL` cell as admin-only), so a
//! space changes only through Cord's space verbs. Per-space visibility, a
//! space open to a chosen few, will narrow the member grant; owners and
//! admins keep seeing every space.
//!
//! Every managing write reads the caller's role afresh in its transaction
//! ([`query_cord_member_role`]), never the request's snapshot:
//!
//! - **Create** locks the cord's row ([`query_cord_lock`]), reads the role,
//!   counts the cord's live spaces, then inserts. The lock makes the count
//!   exact: two creates on one cord run one at a time, so the cord never
//!   holds more than [`CORD_SPACE_COUNT_MAX`]. It takes the cord lock and no
//!   other, so it sits beside the membership changes in the lock order
//!   (link, then cord, then redemption) without adding a step to it. Since
//!   membership changes take the same lock, a demotion that commits first is
//!   honored.
//! - **Rename and delete** take no cord lock. They read the role with a
//!   plain SELECT, then lock the space's own row, which only they lock, so
//!   they add no edge to the lock order either. A demotion that commits
//!   before that read is honored; one that commits while the verb runs is
//!   ordered after it.
//!
//! A space is addressed with its cord's id. One that isn't a live space
//! under that cord reads as missing ([`CordSpaceError::SpaceNotFound`]), but
//! only for someone in the cord: anyone outside it gets
//! [`CordSpaceError::CordNotFound`] first, before any space is looked up.
//!
//! Call the write functions inside a transaction; their locks are held until
//! it ends.

use deadpool_postgres::GenericClient;
use fuz_cell::cell_queries::{query_cell_create, query_cell_delete, query_cell_update};
use fuz_cell::{CellError, CellVisibility};
use fuz_db::tokio_postgres::Row;
use fuz_db::{col, iso8601_timestamp_expr, qualify_columns};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::cord_member_queries::{query_cord_lock, query_cord_member_role};
use crate::cord_queries::{
    CordNameError, query_cord_cell_grants_stamp, validate_name_under_cord_rules,
};

/// The `cell.kind` of a space.
pub const CORD_SPACE_CELL_KIND: &str = "space";

/// The longest space name, in characters. The TS twin is
/// `CORD_SPACE_NAME_LENGTH_MAX` in `cord_action_specs.ts`.
pub const CORD_SPACE_NAME_LENGTH_MAX: usize = 100;

/// The most live spaces a cord may hold. The TS twin is
/// `CORD_SPACE_COUNT_MAX` in `cord_action_specs.ts`.
pub const CORD_SPACE_COUNT_MAX: usize = 50;

/// The `cell` columns a space read projects, a subset of `fuz_cell`'s
/// `CELL_COLUMNS` (a unit test pins it), decoded by name ([`col!`]).
const CORD_SPACE_CELL_COLUMNS: [&str; 4] = ["id", "parent_id", "data", "created_at"];

/// What a space is. A closed set; each kind is its own thing to be in, and
/// new ones are additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CordSpaceKind {
    /// A live room: voice, video, screenshare, and text that isn't kept.
    Line,
}

/// The space's fields in its cell's `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CordSpaceData {
    pub kind: CordSpaceKind,
    pub name: String,
}

/// Validate and normalize a space name: the cord name rules
/// ([`validate_name_under_cord_rules`]), at most
/// [`CORD_SPACE_NAME_LENGTH_MAX`] characters.
///
/// # Errors
///
/// [`CordNameError`] naming the first rule the name breaks.
pub fn validate_cord_space_name(name: &str) -> Result<String, CordNameError> {
    validate_name_under_cord_rules(name, CORD_SPACE_NAME_LENGTH_MAX)
}

/// A space, read from its cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordSpaceRow {
    pub id: Uuid,
    pub cord_id: Uuid,
    pub kind: CordSpaceKind,
    pub name: String,
    /// UTC ISO-8601, second precision.
    pub created_at: String,
}

/// Why a space verb was refused.
#[derive(Debug, thiserror::Error)]
pub enum CordSpaceError {
    /// No active cord has that id, or the caller isn't in it.
    #[error("no such cord, or the caller isn't in it")]
    CordNotFound,
    /// The caller is a plain member, who may not manage spaces.
    #[error("only the cord's owner or admins manage its spaces")]
    Forbidden,
    /// No live space under the cord has that id.
    #[error("no such space in the cord")]
    SpaceNotFound,
    /// The cord already holds [`CORD_SPACE_COUNT_MAX`] live spaces.
    #[error("the cord holds as many spaces as it may")]
    LimitReached,
    /// A driver or decode failure.
    #[error(transparent)]
    Db(#[from] CellError),
}

/// Read the caller's role in the cord afresh ([`query_cord_member_role`])
/// and require that it may manage the cord's spaces: `CordNotFound` outside
/// the cord, `Forbidden` for a plain member.
async fn require_space_manager<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
) -> Result<(), CordSpaceError> {
    match query_cord_member_role(client, cord_id, caller_actor_id).await? {
        None => Err(CordSpaceError::CordNotFound),
        Some(role) if !role.can_manage() => Err(CordSpaceError::Forbidden),
        Some(_) => Ok(()),
    }
}

/// The [`CORD_SPACE_CELL_COLUMNS`] projection over the cell aliased `alias`.
fn space_columns(alias: &str) -> String {
    let timestamps = iso8601_timestamp_expr(&CORD_SPACE_CELL_COLUMNS, &["created_at"], alias);
    qualify_columns(&CORD_SPACE_CELL_COLUMNS, alias, |c| match c {
        "data" => Some(format!("{alias}.data::text")),
        _ => timestamps(c),
    })
}

/// Decode a [`space_columns`] row; `None` when its `data` doesn't parse as
/// [`CordSpaceData`] (a kind this build doesn't know), so it reads as no
/// space at all.
fn decode_space_row(row: &Row) -> Result<Option<CordSpaceRow>, CellError> {
    let cord_id: Option<Uuid> = row.get(col!(CORD_SPACE_CELL_COLUMNS, "parent_id"));
    let cord_id =
        cord_id.ok_or_else(|| CellError::Decode("a space cell has no parent".to_owned()))?;
    let data: String = row.get(col!(CORD_SPACE_CELL_COLUMNS, "data"));
    let Ok(data) = serde_json::from_str::<CordSpaceData>(&data) else {
        return Ok(None);
    };
    Ok(Some(CordSpaceRow {
        id: row.get(col!(CORD_SPACE_CELL_COLUMNS, "id")),
        cord_id,
        kind: data.kind,
        name: data.name,
        created_at: row.get(col!(CORD_SPACE_CELL_COLUMNS, "created_at")),
    }))
}

/// The `WHERE` that picks out a cord's live spaces: the cord is `$1`, the
/// space kind `$2`. `root_id` reaches the spine's governing-root index;
/// `parent_id` keeps it to the cord's direct children.
const LIVE_SPACES_OF_CORD: &str = "c.root_id = $1 AND c.parent_id = $1 AND c.kind = $2
           AND c.deleted_at IS NULL";

/// The cord's live spaces, oldest first (`created_at`, then id). Reads no
/// membership: the caller has already checked the reader is in the cord.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_spaces<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Vec<CordSpaceRow>, CellError> {
    let sql = format!(
        "SELECT {cols} FROM cell c
         WHERE {LIVE_SPACES_OF_CORD}
         ORDER BY c.created_at, c.id",
        cols = space_columns("c"),
    );
    let rows = client
        .query(&sql, &[&cord_id, &CORD_SPACE_CELL_KIND])
        .await?;
    let mut spaces = Vec::with_capacity(rows.len());
    for row in &rows {
        if let Some(space) = decode_space_row(row)? {
            spaces.push(space);
        }
    }
    Ok(spaces)
}

/// How many live spaces the cord holds, whatever their kind.
async fn query_cord_space_count<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<i64, CellError> {
    let sql = format!("SELECT COUNT(*) FROM cell c WHERE {LIVE_SPACES_OF_CORD}");
    let row = client
        .query_one(&sql, &[&cord_id, &CORD_SPACE_CELL_KIND])
        .await?;
    Ok(row.get(0))
}

/// Lock the live space `space_id` under the cord `cord_id` until the
/// transaction ends, returning it; `None` when there's no such space.
async fn query_cord_space_lock<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    space_id: Uuid,
) -> Result<Option<(CordSpaceRow, Value)>, CellError> {
    let sql = format!(
        "SELECT {cols} FROM cell c
         WHERE c.id = $3 AND {LIVE_SPACES_OF_CORD}
         FOR NO KEY UPDATE",
        cols = space_columns("c"),
    );
    let Some(row) = client
        .query_opt(&sql, &[&cord_id, &CORD_SPACE_CELL_KIND, &space_id])
        .await?
    else {
        return Ok(None);
    };
    let Some(space) = decode_space_row(&row)? else {
        return Ok(None);
    };
    let data: String = row.get(col!(CORD_SPACE_CELL_COLUMNS, "data"));
    let data: Value =
        serde_json::from_str(&data).map_err(|e| CellError::Decode(format!("data: {e}")))?;
    Ok(Some((space, data)))
}

/// Create a space in the cord `cord_id` as the caller: its cell, as a child
/// of the cord's, and the stamped grants.
///
/// The cord's owner or admins only, by the role read under the cord's lock.
/// Refuses once the cord holds [`CORD_SPACE_COUNT_MAX`] live spaces. `name`
/// must already be validated ([`validate_cord_space_name`]). The cell's
/// `created_by` stays `NULL`; the caller records the creator in an audit
/// event.
///
/// # Errors
///
/// [`CordSpaceError`] naming the refusal, or a driver failure.
pub async fn query_cord_space_create<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
    kind: CordSpaceKind,
    name: &str,
) -> Result<CordSpaceRow, CordSpaceError> {
    if query_cord_lock(client, cord_id).await?.is_none() {
        return Err(CordSpaceError::CordNotFound);
    }
    require_space_manager(client, cord_id, caller_actor_id).await?;
    let count = query_cord_space_count(client, cord_id).await?;
    if usize::try_from(count).map_or(true, |count| count >= CORD_SPACE_COUNT_MAX) {
        return Err(CordSpaceError::LimitReached);
    }
    let data = serde_json::json!(CordSpaceData {
        kind,
        name: name.to_owned(),
    });
    let cell = query_cell_create(
        client,
        &data,
        Some(CORD_SPACE_CELL_KIND),
        Some(CellVisibility::Private),
        None,
        Some(cord_id),
        Some(cord_id),
        None,
        None,
        None,
    )
    .await?;
    query_cord_cell_grants_stamp(client, cell.id, cord_id).await?;
    Ok(CordSpaceRow {
        id: cell.id,
        cord_id,
        kind,
        name: name.to_owned(),
        created_at: cell.created_at,
    })
}

/// Rename the space `space_id` in the cord `cord_id` as the caller, keeping
/// the rest of its `data`.
///
/// The cord's owner or admins only, by the role read afresh. `name` must
/// already be validated ([`validate_cord_space_name`]).
///
/// # Errors
///
/// [`CordSpaceError`] naming the refusal, or a driver failure.
pub async fn query_cord_space_rename<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    space_id: Uuid,
    caller_actor_id: Uuid,
    name: &str,
) -> Result<CordSpaceRow, CordSpaceError> {
    require_space_manager(client, cord_id, caller_actor_id).await?;
    let Some((space, mut data)) = query_cord_space_lock(client, cord_id, space_id).await? else {
        return Err(CordSpaceError::SpaceNotFound);
    };
    let Value::Object(fields) = &mut data else {
        return Err(CellError::Decode("space data is not an object".to_owned()).into());
    };
    fields.insert("name".to_owned(), Value::String(name.to_owned()));
    // the row is locked and live, so the update finds it
    query_cell_update(
        client,
        space_id,
        Some(&data),
        None,
        None,
        Some(caller_actor_id),
    )
    .await?
    .ok_or(CordSpaceError::SpaceNotFound)?;
    Ok(CordSpaceRow {
        name: name.to_owned(),
        ..space
    })
}

/// Delete the space `space_id` in the cord `cord_id` as the caller: a soft
/// delete, so its cell and grants stay but nothing lists or finds it.
/// Returns the space as it was.
///
/// The cord's owner or admins only, by the role read afresh.
///
/// # Errors
///
/// [`CordSpaceError`] naming the refusal, or a driver failure.
pub async fn query_cord_space_delete<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    space_id: Uuid,
    caller_actor_id: Uuid,
) -> Result<CordSpaceRow, CordSpaceError> {
    require_space_manager(client, cord_id, caller_actor_id).await?;
    let Some((space, _)) = query_cord_space_lock(client, cord_id, space_id).await? else {
        return Err(CordSpaceError::SpaceNotFound);
    };
    if !query_cell_delete(client, space_id, Some(caller_actor_id)).await? {
        return Err(CordSpaceError::SpaceNotFound);
    }
    Ok(space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_columns_are_cell_columns() {
        assert_eq!(
            fuz_db::columns_not_in(
                &CORD_SPACE_CELL_COLUMNS,
                &fuz_cell::cell_queries::CELL_COLUMNS
            ),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn space_names_follow_the_cord_name_rules() {
        assert_eq!(validate_cord_space_name("  the den  ").unwrap(), "the den");
        assert_eq!(validate_cord_space_name("   "), Err(CordNameError::Empty));
        assert_eq!(
            validate_cord_space_name("two\nlines"),
            Err(CordNameError::ForbiddenCharacter)
        );
        assert_eq!(
            validate_cord_space_name("\u{200B}"),
            Err(CordNameError::ForbiddenCharacter)
        );
        assert_eq!(
            validate_cord_space_name("a\u{200D}\u{200D}b"),
            Err(CordNameError::StrayJoiner)
        );
        let longest = "é".repeat(CORD_SPACE_NAME_LENGTH_MAX);
        assert_eq!(validate_cord_space_name(&longest).unwrap(), longest);
        assert_eq!(
            validate_cord_space_name(&"a".repeat(CORD_SPACE_NAME_LENGTH_MAX + 1)),
            Err(CordNameError::TooLong {
                length_max: CORD_SPACE_NAME_LENGTH_MAX
            })
        );
        assert_eq!(
            validate_cord_space_name("🧑\u{200D}💻 desk").unwrap(),
            "🧑\u{200D}💻 desk"
        );
    }

    #[test]
    fn space_data_round_trips_with_its_kind() {
        let data = serde_json::json!(CordSpaceData {
            kind: CordSpaceKind::Line,
            name: "the den".to_owned(),
        });
        assert_eq!(
            data,
            serde_json::json!({ "kind": "line", "name": "the den" })
        );
        for unknown in [
            serde_json::json!({ "kind": "chat", "name": "x" }),
            serde_json::json!({ "kind": "Line", "name": "x" }),
            serde_json::json!({ "name": "x" }),
        ] {
            assert!(serde_json::from_value::<CordSpaceData>(unknown).is_err());
        }
    }
}
