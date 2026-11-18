//! Queries for cords, over the spine's `cell`, `cell_grant`, and `role_grant`
//! tables.
//!
//! A cord is a `cell` row with `kind = 'cord'`: a private root (no parent),
//! born with `created_by = NULL` so the spine's owner branch never admits
//! anyone and role grants are the only authority. Its small fields live in
//! the cell's `data` ([`CordData`]). Membership is cord-scoped `role_grant`
//! rows ([`crate::roles`]); three role-shaped `cell_grant` rows are stamped
//! on the cord cell at creation ([`CORD_STAMPED_GRANTS`]).
//!
//! The stamped grants make the spine agree with Cord on **viewing** only:
//! `can_view_cell`, `cell_list`, and fact serving admit exactly the cord's
//! members, plus a global `admin` through the spine's admin bypass (nothing
//! cord mounts serves cells through those paths today).
//! They do **not** enable any spine edit path: `can_edit_cell` treats a
//! `created_by = NULL` cell as admin-only and ignores its editor grants. So
//! cord cells change only through Cord's own verbs, and nothing may rely on
//! the spine's cell edit paths for them.
//!
//! Every function takes any `GenericClient`, so it composes on a pooled
//! client or inside the action's transaction. Cell reads project through
//! `fuz_cell`'s queries (and so its `CELL_COLUMNS`); the membership reads
//! select only the aggregates they need.

use std::collections::HashMap;

use deadpool_postgres::GenericClient;
use fuz_cell::cell_grant_queries::query_cell_grant_create;
use fuz_cell::cell_grant_schema::{CellGrantLevel, CellGrantPrincipalInput};
use fuz_cell::cell_queries::{
    query_cell_create, query_cell_get, query_cell_list_by_kind, query_cell_load_many,
    query_cell_update,
};
use fuz_cell::{CellError, CellRow, CellVisibility};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::cord_authz::CordRole;
use crate::cord_member_queries::{live_cord_grant_sql, query_cord_role_grant_create};
use crate::roles::{CORD_ADMIN, CORD_MEMBER, CORD_OWNER, CORD_ROLES, CORD_SCOPE_KIND};

/// The `cell.kind` of a cord.
pub const CORD_CELL_KIND: &str = "cord";

/// The longest cord name, in characters. The TS twin is
/// `CORD_NAME_LENGTH_MAX` in `cord_action_specs.ts`.
pub const CORD_NAME_LENGTH_MAX: usize = 100;

/// The role-shaped `cell_grant` rows stamped on every cord cell and space
/// cell: members view, owners and admins edit.
///
/// One per cord role, never one per member, each scoped to the cord, on the
/// cord and on each of its spaces ([`crate::cord_space_queries`]).
pub const CORD_STAMPED_GRANTS: [(&str, CellGrantLevel); 3] = [
    (CORD_MEMBER, CellGrantLevel::Viewer),
    (CORD_ADMIN, CellGrantLevel::Editor),
    (CORD_OWNER, CellGrantLevel::Editor),
];

/// The cord's fields in its cell's `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CordData {
    pub name: String,
}

/// Why a name was refused: a cord's ([`validate_cord_name`]) or a space's
/// (`validate_cord_space_name` in [`crate::cord_space_queries`]), which keep
/// the same rules and differ only in their longest length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CordNameError {
    #[error("name is empty")]
    Empty,
    #[error("name is longer than {length_max} characters")]
    TooLong { length_max: usize },
    #[error("name contains an invisible, control, or line-breaking character")]
    ForbiddenCharacter,
    #[error("name has a joining character at an edge, by a space, or doubled")]
    StrayJoiner,
}

/// Characters refused anywhere in a cord name beside the control
/// characters, as inclusive ranges.
///
/// The formula: (Cf ∪ `Default_Ignorable_Code_Point` ∪ {U+2028, U+2029}) −
/// {U+200C, U+200D} − [U+FE00, U+FE0F], as of Unicode 17, with the control
/// characters (Cc) refused separately. That is every format character, every
/// character Unicode says to render invisibly when unsupported (zero-width
/// spaces, bidi marks, overrides and isolates, fillers, tags, the byte order
/// mark, unassigned default-ignorables), and the line and paragraph
/// separators, except:
///
/// - the join characters U+200C (ZWNJ) and U+200D (ZWJ), which Persian
///   spelling, Indic conjuncts, and joined emoji need, allowed under the
///   placement rule (`has_stray_cord_name_joiner`)
/// - the variation selectors U+FE00–FE0F, which emoji need
///
/// The tag characters stay refused, at the cost of the three subdivision
/// flags (England, Scotland, Wales).
///
/// Listed explicitly because std has no Unicode property lookup. The TS twin
/// is `CORD_NAME_FORBIDDEN_CHAR_RANGES` in `cord_action_specs.ts`; a TS test
/// checks the two lists are identical and derives the set from the Unicode
/// properties in both directions.
pub const CORD_NAME_FORBIDDEN_CHAR_RANGES: [(char, char); 27] = [
    ('\u{00AD}', '\u{00AD}'),   // soft hyphen
    ('\u{034F}', '\u{034F}'),   // combining grapheme joiner
    ('\u{0600}', '\u{0605}'),   // Arabic number signs (Cf)
    ('\u{061C}', '\u{061C}'),   // Arabic letter mark
    ('\u{06DD}', '\u{06DD}'),   // Arabic end of ayah (Cf)
    ('\u{070F}', '\u{070F}'),   // Syriac abbreviation mark (Cf)
    ('\u{0890}', '\u{0891}'),   // Arabic pound and piastre marks (Cf)
    ('\u{08E2}', '\u{08E2}'),   // Arabic disputed end of ayah (Cf)
    ('\u{115F}', '\u{1160}'),   // Hangul choseong and jungseong fillers
    ('\u{17B4}', '\u{17B5}'),   // Khmer inherent vowels
    ('\u{180B}', '\u{180F}'),   // Mongolian variation selectors and vowel separator
    ('\u{200B}', '\u{200B}'),   // zero-width space
    ('\u{200E}', '\u{200F}'),   // left-to-right and right-to-left marks
    ('\u{2028}', '\u{2029}'),   // line and paragraph separators (Zl, Zp)
    ('\u{202A}', '\u{202E}'),   // bidirectional embeddings and overrides
    ('\u{2060}', '\u{206F}'), // word joiner, invisible operators, bidi isolates, deprecated formats
    ('\u{3164}', '\u{3164}'), // Hangul filler
    ('\u{FEFF}', '\u{FEFF}'), // byte order mark
    ('\u{FFA0}', '\u{FFA0}'), // halfwidth Hangul filler
    ('\u{FFF0}', '\u{FFF8}'), // unassigned, default-ignorable
    ('\u{FFF9}', '\u{FFFB}'), // interlinear annotation marks
    ('\u{110BD}', '\u{110BD}'), // Kaithi number sign (Cf)
    ('\u{110CD}', '\u{110CD}'), // Kaithi number sign above (Cf)
    ('\u{13430}', '\u{1343F}'), // Egyptian hieroglyph format controls (Cf)
    ('\u{1BCA0}', '\u{1BCA3}'), // shorthand format controls
    ('\u{1D173}', '\u{1D17A}'), // musical symbol format controls
    ('\u{E0000}', '\u{E0FFF}'), // tags and supplementary variation selectors
];

/// The two join characters: U+200C (zero-width non-joiner) and U+200D
/// (zero-width joiner).
pub(crate) const CORD_NAME_JOIN_CHARS: [char; 2] = ['\u{200C}', '\u{200D}'];

/// Whether `c` may never appear in a cord name: a control character (Cc,
/// which covers `\n`, `\t`, and U+0085) or one of
/// [`CORD_NAME_FORBIDDEN_CHAR_RANGES`].
#[must_use]
pub(crate) fn is_forbidden_cord_name_char(c: char) -> bool {
    c.is_control()
        || CORD_NAME_FORBIDDEN_CHAR_RANGES
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&c))
}

/// Whether a join character in `chars` sits where it only pads: at either
/// end, or beside whitespace or another join character.
///
/// Whitespace is `char::is_whitespace` (Unicode `White_Space`). The TS twin's
/// `\s` differs from it only on U+FEFF and U+0085, both refused before this
/// runs.
#[must_use]
pub(crate) fn has_stray_cord_name_joiner(chars: &[char]) -> bool {
    let is_join = |c: char| CORD_NAME_JOIN_CHARS.contains(&c);
    let pads = |c: Option<&char>| c.is_none_or(|&c| c.is_whitespace() || is_join(c));
    chars.iter().enumerate().any(|(i, &c)| {
        is_join(c) && (pads(i.checked_sub(1).and_then(|p| chars.get(p))) || pads(chars.get(i + 1)))
    })
}

/// Validate and normalize a name under the cord name rules, in this order:
///
/// 1. no forbidden character anywhere (`is_forbidden_cord_name_char`)
/// 2. trimmed, then non-empty
/// 3. no join character at either end or beside whitespace or another join
///    character (`has_stray_cord_name_joiner`)
/// 4. at most `length_max` characters (code points)
///
/// The forbidden-character check runs on the untrimmed input so the TS twin
/// agrees exactly: JS `trim` strips U+FEFF and Rust's doesn't, and Rust's
/// strips U+0085 and JS's doesn't, but both are forbidden anywhere, so what
/// remains trims the same in both.
///
/// Cord names ([`validate_cord_name`]) and space names
/// (`validate_cord_space_name`) both go through this. The TS twin is
/// `create_cord_name_schema` in `cord_action_specs.ts`.
///
/// # Errors
///
/// [`CordNameError`] naming the first rule the name breaks.
pub fn validate_name_under_cord_rules(
    name: &str,
    length_max: usize,
) -> Result<String, CordNameError> {
    if name.chars().any(is_forbidden_cord_name_char) {
        return Err(CordNameError::ForbiddenCharacter);
    }
    let name = name.trim();
    if name.is_empty() {
        return Err(CordNameError::Empty);
    }
    let chars: Vec<char> = name.chars().collect();
    if has_stray_cord_name_joiner(&chars) {
        return Err(CordNameError::StrayJoiner);
    }
    if chars.len() > length_max {
        return Err(CordNameError::TooLong { length_max });
    }
    Ok(name.to_owned())
}

/// Validate and normalize a cord name: the cord name rules
/// ([`validate_name_under_cord_rules`]), at most [`CORD_NAME_LENGTH_MAX`]
/// characters.
///
/// # Errors
///
/// [`CordNameError`] naming the first rule the name breaks.
pub fn validate_cord_name(name: &str) -> Result<String, CordNameError> {
    validate_name_under_cord_rules(name, CORD_NAME_LENGTH_MAX)
}

/// A cord, read from its cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordRow {
    pub id: Uuid,
    pub name: String,
    /// UTC ISO-8601, second precision.
    pub created_at: String,
}

impl CordRow {
    /// The cord a cell holds, or `None` when the cell isn't a cord or its
    /// `data` doesn't parse as [`CordData`].
    #[must_use]
    pub fn from_cell(cell: &CellRow) -> Option<Self> {
        if cell.kind.as_deref() != Some(CORD_CELL_KIND) {
            return None;
        }
        let data = CordData::deserialize(&cell.data).ok()?;
        Some(Self {
            id: cell.id,
            name: data.name,
            created_at: cell.created_at.clone(),
        })
    }
}

/// A cord just created, with the id of its owner's role grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordCreated {
    pub cord: CordRow,
    pub owner_role_grant_id: Uuid,
}

/// A row of the instance admins' ops list: a cord and how many actors hold a
/// role in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordAdminRow {
    pub cord: CordRow,
    pub member_count: i64,
}

/// Create a cord: its cell, the creator's `cord_owner` grant, and the
/// stamped `cell_grant` rows.
///
/// Call it inside one transaction (the `cord_create` action runs with side
/// effects, so it does) so the three writes land together or not at all.
/// `name` must already be validated ([`validate_cord_name`]). The cell's
/// `created_by` stays `NULL`; the caller records the creator in an audit
/// event.
///
/// The owner starts with no other role in the new cord, so this can't break
/// the one-role-per-(actor, cord) rule; see [`crate::cord_authz`].
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_create<C: GenericClient + ?Sized>(
    client: &C,
    name: &str,
    owner_actor_id: Uuid,
) -> Result<CordCreated, CellError> {
    let data = serde_json::json!(CordData {
        name: name.to_owned()
    });
    let cell = query_cell_create(
        client,
        &data,
        Some(CORD_CELL_KIND),
        Some(CellVisibility::Private),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await?;
    let owner_role_grant_id = query_cord_role_grant_create(
        client,
        cell.id,
        owner_actor_id,
        CordRole::Owner,
        Some(owner_actor_id),
    )
    .await?;
    query_cord_cell_grants_stamp(client, cell.id, cell.id).await?;
    let cord = CordRow::from_cell(&cell)
        .ok_or_else(|| CellError::Decode("a created cord cell didn't read back".to_owned()))?;
    Ok(CordCreated {
        cord,
        owner_role_grant_id,
    })
}

/// Stamp [`CORD_STAMPED_GRANTS`] on the cell `cell_id`, a cord or one of
/// its spaces: one role-shaped `cell_grant` per cord role, each scoped to the
/// cord `cord_id`, never one per member.
///
/// # Errors
///
/// [`CellError`] on a driver failure.
pub(crate) async fn query_cord_cell_grants_stamp<C: GenericClient + ?Sized>(
    client: &C,
    cell_id: Uuid,
    cord_id: Uuid,
) -> Result<(), CellError> {
    for (role, level) in CORD_STAMPED_GRANTS {
        query_cell_grant_create(
            client,
            cell_id,
            level,
            &CellGrantPrincipalInput::Role {
                role: role.to_owned(),
                scope_id: Some(cord_id),
            },
            None,
        )
        .await?;
    }
    Ok(())
}

/// Fetch a cord by id; `None` when no active cord cell has that id.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_get<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Option<CordRow>, CellError> {
    Ok(query_cell_get(client, cord_id)
        .await?
        .as_ref()
        .and_then(CordRow::from_cell))
}

/// Fetch the active cords among `cord_ids`, in no particular order; ids that
/// name no active cord are absent.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cords_by_ids<C: GenericClient + ?Sized>(
    client: &C,
    cord_ids: &[Uuid],
) -> Result<Vec<CordRow>, CellError> {
    Ok(query_cell_load_many(client, cord_ids)
        .await?
        .iter()
        .filter_map(CordRow::from_cell)
        .collect())
}

/// Rename a cord, keeping the rest of its `data`. `name` must already be
/// validated. Returns `None` when no active cord has that id.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_rename<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    name: &str,
    updated_by: Uuid,
) -> Result<Option<CordRow>, CellError> {
    let Some(cell) = query_cell_get(client, cord_id).await? else {
        return Ok(None);
    };
    if CordRow::from_cell(&cell).is_none() {
        return Ok(None);
    }
    let mut data = cell.data;
    let Value::Object(fields) = &mut data else {
        return Err(CellError::Decode("cord data is not an object".to_owned()));
    };
    fields.insert("name".to_owned(), Value::String(name.to_owned()));
    Ok(
        query_cell_update(client, cord_id, Some(&data), None, None, Some(updated_by))
            .await?
            .as_ref()
            .and_then(CordRow::from_cell),
    )
}

/// Every active cord with its member count, newest first: the instance
/// admins' ops list.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_admin_list<C: GenericClient + ?Sized>(
    client: &C,
) -> Result<Vec<CordAdminRow>, CellError> {
    let cords: Vec<CordRow> = query_cell_list_by_kind(client, CORD_CELL_KIND, None, 0)
        .await?
        .iter()
        .filter_map(CordRow::from_cell)
        .collect();
    let ids: Vec<Uuid> = cords.iter().map(|c| c.id).collect();
    let counts = query_cord_member_counts(client, &ids).await?;
    Ok(cords
        .into_iter()
        .map(|cord| {
            let member_count = counts.get(&cord.id).copied().unwrap_or(0);
            CordAdminRow { cord, member_count }
        })
        .collect())
}

/// How many distinct actors hold a live cord role in each of `cord_ids`.
///
/// Counts only the grants that make members ([`live_cord_grant_sql`]):
/// soft-deleted actors and accounts keep their grants but aren't members
/// anyone sees. Cords with no members are absent from the map.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub(crate) async fn query_cord_member_counts<C: GenericClient + ?Sized>(
    client: &C,
    cord_ids: &[Uuid],
) -> Result<HashMap<Uuid, i64>, CellError> {
    if cord_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = live_cord_grant_sql(
        "rg.scope_id, COUNT(DISTINCT rg.actor_id)",
        "rg.scope_id = ANY($3::uuid[]) GROUP BY rg.scope_id",
    );
    let rows = client
        .query(&sql, &[&CORD_SCOPE_KIND, &CORD_ROLES.as_slice(), &cord_ids])
        .await?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(validate_cord_name("  the porch  ").unwrap(), "the porch");
        assert_eq!(validate_cord_name("   "), Err(CordNameError::Empty));
        assert_eq!(validate_cord_name(""), Err(CordNameError::Empty));
        let longest = "é".repeat(CORD_NAME_LENGTH_MAX);
        assert_eq!(validate_cord_name(&longest).unwrap(), longest);
        assert_eq!(
            validate_cord_name(&"a".repeat(CORD_NAME_LENGTH_MAX + 1)),
            Err(CordNameError::TooLong {
                length_max: CORD_NAME_LENGTH_MAX
            })
        );
    }

    #[test]
    fn names_refuse_invisible_control_and_line_breaking_characters() {
        for name in [
            "two\nlines",
            "tab\there",
            "\u{0085}next line",
            "\u{200B}",
            "zero\u{200B}width",
            "ltr\u{200E}mark",
            "evil\u{202E}txt.exe",
            "isolate\u{2066}",
            "\u{FEFF}bom",
            "soft\u{00AD}hyphen",
            "word\u{2060}joiner",
            "tag\u{E0041}",
            "line\u{2028}separator",
            "\u{3164}",
            "hangul\u{115F}filler",
            "half\u{FFA0}width",
            "selector\u{E0100}",
            "grapheme\u{034F}joiner",
            "khmer\u{17B4}vowel",
            "mongolian\u{180B}selector",
            "unassigned\u{2065}",
            "reserved\u{FFF0}",
            "tag\u{E0000}",
        ] {
            assert_eq!(
                validate_cord_name(name),
                Err(CordNameError::ForbiddenCharacter),
                "{name:?}"
            );
        }
        // visible non-ASCII, interior spaces, and emoji variation selectors
        // are fine
        assert_eq!(
            validate_cord_name("\u{3000}la cabaña 🌙\u{00A0}").unwrap(),
            "la cabaña 🌙"
        );
        assert_eq!(validate_cord_name("☀\u{FE0F}").unwrap(), "☀\u{FE0F}");
    }

    #[test]
    fn join_characters_are_allowed_between_letters_and_emoji() {
        for name in [
            "🏳\u{FE0F}\u{200D}🌈",
            "❤\u{FE0F}\u{200D}🔥",
            "🧑\u{200D}💻",
            "🧑🏽\u{200D}💻",
            "👨\u{200D}👩\u{200D}👧\u{200D}👦",
            "می\u{200C}خواهم",
            "ශ්\u{200D}රී",
        ] {
            assert_eq!(validate_cord_name(name).unwrap(), name, "{name:?}");
        }
    }

    #[test]
    fn join_characters_never_pad() {
        for name in [
            "\u{200D}",
            "\u{200C}",
            "a\u{200D}",
            "\u{200C}a",
            " \u{200D}a ",
            "a \u{200D}b",
            "a\u{200D} b",
            "a\u{200D}\u{200D}b",
            "a\u{200C}\u{200D}b",
        ] {
            assert_eq!(
                validate_cord_name(name),
                Err(CordNameError::StrayJoiner),
                "{name:?}"
            );
        }
    }

    #[test]
    fn trim_parity_with_the_ts_twin() {
        // JS `trim` would strip a leading BOM and keep U+0085; Rust's does the
        // reverse. Both are refused before trimming, so the twins agree.
        assert_eq!(
            validate_cord_name("\u{FEFF}x"),
            Err(CordNameError::ForbiddenCharacter)
        );
        assert_eq!(
            validate_cord_name("\u{0085}x"),
            Err(CordNameError::ForbiddenCharacter)
        );
        // every whitespace both trims strip goes
        assert_eq!(
            validate_cord_name("\u{2000}\u{202F}\u{205F} x \u{1680}\u{00A0}").unwrap(),
            "x"
        );
    }

    #[test]
    fn forbidden_char_ranges_are_ordered_and_disjoint() {
        let mut ranges = CORD_NAME_FORBIDDEN_CHAR_RANGES.to_vec();
        ranges.sort_unstable();
        for pair in ranges.windows(2) {
            assert!(pair[0].1 < pair[1].0, "{pair:?}");
        }
        for (start, end) in ranges {
            assert!(start <= end);
        }
    }

    #[test]
    fn stamped_grants_cover_every_cord_role_once() {
        let mut roles: Vec<&str> = CORD_STAMPED_GRANTS.iter().map(|(r, _)| *r).collect();
        roles.sort_unstable();
        let mut expected = CORD_ROLES.to_vec();
        expected.sort_unstable();
        assert_eq!(roles, expected);
    }

    fn cell(kind: Option<&str>, data: Value) -> CellRow {
        CellRow {
            id: Uuid::from_u128(7),
            data,
            kind: kind.map(str::to_owned),
            visibility: CellVisibility::Private,
            path: None,
            refs: None,
            parent_id: None,
            root_id: None,
            moderation: None,
            policy: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: None,
            deleted_at: None,
            created_by: None,
            updated_by: None,
            grant_count: 3,
        }
    }

    #[test]
    fn only_cord_cells_read_as_cords() {
        let data = serde_json::json!({"name": "the porch"});
        let cord = CordRow::from_cell(&cell(Some(CORD_CELL_KIND), data.clone())).unwrap();
        assert_eq!(cord.name, "the porch");
        assert!(CordRow::from_cell(&cell(Some("space"), data.clone())).is_none());
        assert!(CordRow::from_cell(&cell(None, data)).is_none());
        assert!(CordRow::from_cell(&cell(Some(CORD_CELL_KIND), serde_json::json!({}))).is_none());
    }
}
