//! Cord's own migration namespace (`cord`), chained after the spine's auth
//! and cell namespaces in [`crate::router::cord_migration_namespaces`]: the
//! cord tables reference `actor` and `cell`.
//!
//! Forward-only: append new entries and never edit one that has shipped.
//! Every statement is guarded (`IF NOT EXISTS`), so a partly migrated
//! database converges.

use fuz_db::{ColumnProjections, Migration, MigrationNamespace};

use crate::cord_invite_queries::CORD_INVITE_COLUMNS;

/// The namespace key in `schema_version`.
pub const CORD_MIGRATIONS_NAMESPACE: &str = "cord";

/// `cord_invite`: a cord's invite links, one row per link.
///
/// - `token_hash` is the blake3 hex digest of the link's token; the raw token
///   is shown once, at creation, and never stored.
/// - `role` is what the link confers. Only `cord_member` for now, held by a
///   CHECK, so a later link kind widens the constraint rather than adding a
///   column and backfilling it.
/// - `admits_newcomers` marks a link that may bring someone who has no
///   account yet onto the instance. Nothing sets it yet; it stays false, and
///   every link works for existing accounts only.
/// - `created_by` keeps the minter for the record, and goes null if their
///   actor is purged, as the spine's own `created_by` columns do; the link
///   belongs to the cord, not to its minter.
/// - `max_uses` is null for unlimited; the table refuses a `use_count` past
///   it, so an overshoot can't commit even if a caller forgot the row lock.
const CORD_INVITE_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS cord_invite (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cord_id UUID NOT NULL REFERENCES cell(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'cord_member',
    admits_newcomers BOOLEAN NOT NULL DEFAULT false,
    created_by UUID REFERENCES actor(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    max_uses INTEGER,
    use_count INTEGER NOT NULL DEFAULT 0,
    revoked_at TIMESTAMPTZ,
    CONSTRAINT cord_invite_token_hash_unique UNIQUE (token_hash),
    CONSTRAINT cord_invite_role_member CHECK (role = 'cord_member'),
    CONSTRAINT cord_invite_max_uses_positive CHECK (max_uses IS NULL OR max_uses > 0),
    CONSTRAINT cord_invite_use_count_bounded CHECK (
        use_count >= 0 AND (max_uses IS NULL OR use_count <= max_uses)
    )
);
CREATE INDEX IF NOT EXISTS cord_invite_cord_idx ON cord_invite (cord_id, created_at DESC);
";

const MIGRATIONS: &[Migration] = &[Migration {
    name: "cord_invite_schema",
    sql: CORD_INVITE_SCHEMA,
}];

/// Cord's migration namespace, run after `fuz_auth::AUTH_MIGRATIONS` and
/// `fuz_cell::CELL_MIGRATIONS`.
pub const CORD_MIGRATIONS: MigrationNamespace =
    MigrationNamespace::new(CORD_MIGRATIONS_NAMESPACE, MIGRATIONS);

/// The `cord` chain's columns registry, composed with the spine's by the
/// column-projection guard (`tests/columns.rs`).
pub const CORD_COLUMN_PROJECTIONS: ColumnProjections = ColumnProjections {
    projections: &[("cord_invite", &CORD_INVITE_COLUMNS)],
    exempt: &[],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_namespace_is_not_reserved() {
        fuz_db::assert_consumer_namespace(CORD_MIGRATIONS_NAMESPACE).unwrap();
    }
}
