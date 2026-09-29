//! Postgres-backed tests for cord creation and reads (`cord_server::cord_queries`).
//!
//! What only the database can show: a created cord is a private root cell with
//! no `created_by`, its creator holds `cord_owner` scoped to it (with the
//! paired `scope_kind`), the three role-shaped `cell_grant` rows are stamped
//! on it, and the spine's own visibility query (`query_cell_list`) admits
//! exactly the cord's members through those grants.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and its `CORD_SCHEMA_READY_TEST_URL` override). It drops and recreates the
//! `public` schema, so the scenarios run in one test function rather than
//! racing each other:
//!
//! ```bash
//! cargo test -p cord_server --test cords -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cord_server::cord_queries::{
    CORD_CELL_KIND, CORD_STAMPED_GRANTS, query_cord_admin_list, query_cord_create, query_cord_get,
    query_cord_rename, query_cords_by_ids,
};
use cord_server::roles::{CORD_MEMBER, CORD_OWNER, CORD_SCOPE_KIND};
use cord_server::router::cord_migration_namespaces;
use fuz_auth::actor_queries::query_role_grants_for_actor;
use fuz_auth::query_create_role_grant;
use fuz_cell::cell_grant_queries::query_cell_grant_list_for_cell;
use fuz_cell::cell_queries::{CellListParams, query_cell_get, query_cell_list};
use fuz_cell::{CellGrantLevel, CellVisibility};
use fuz_db::{Pool, create_pool, run_migrations};
use uuid::Uuid;

fn test_url() -> String {
    std::env::var("CORD_SCHEMA_READY_TEST_URL")
        .unwrap_or_else(|_| "postgres://localhost/cord_schema_ready_test".to_owned())
}

async fn fresh_db_pool() -> Pool {
    let pool = create_pool(&test_url()).expect("create_pool");
    {
        let client = pool.get().await.expect("pool.get");
        client
            .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
            .await
            .expect("reset schema");
    }
    run_migrations(&pool, &cord_migration_namespaces())
        .await
        .expect("migrations");
    pool
}

/// Seed an account and its actor, returning the actor id.
async fn seed_actor(pool: &Pool, username: &str) -> Uuid {
    let client = pool.get().await.expect("pool.get");
    let account_id: Uuid = client
        .query_one(
            "INSERT INTO account (username, password_hash) VALUES ($1, 'x') RETURNING id",
            &[&username],
        )
        .await
        .expect("insert account")
        .get(0);
    client
        .query_one(
            "INSERT INTO actor (account_id, name) VALUES ($1, $2) RETURNING id",
            &[&account_id, &username],
        )
        .await
        .expect("insert actor")
        .get(0)
}

/// Whether the spine's visibility-filtered `cell_list` shows `cord_id` to an
/// actor holding `grants`, with no admin bypass.
async fn cell_list_shows(pool: &Pool, actor_id: Uuid, cord_id: Uuid) -> bool {
    let client = pool.get().await.expect("pool.get");
    let grants = query_role_grants_for_actor(&client, &actor_id)
        .await
        .expect("grants");
    let roles: Vec<&str> = grants.iter().map(|g| g.role.as_str()).collect();
    let scopes: Vec<Option<Uuid>> = grants.iter().map(|g| g.scope_id).collect();
    let ids = [cord_id];
    let params = CellListParams {
        ids: Some(&ids),
        viewer_actor_id: Some(actor_id),
        viewer_is_admin: false,
        caller_role_grant_roles: &roles,
        caller_role_grant_scope_ids: &scopes,
        limit: 10,
        ..CellListParams::default()
    };
    !query_cell_list(&client, &params)
        .await
        .expect("cell_list")
        .is_empty()
}

#[tokio::test]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn cords_are_structural_cells_governed_by_role_grants() {
    let pool = fresh_db_pool().await;
    let owner = seed_actor(&pool, "owner").await;
    let member = seed_actor(&pool, "member").await;
    let stranger = seed_actor(&pool, "stranger").await;

    // create, in one transaction as the action does
    let created = {
        let mut client = pool.get().await.expect("pool.get");
        let tx = client.transaction().await.expect("begin");
        let created = query_cord_create(&tx, "the porch", owner)
            .await
            .expect("create cord");
        tx.commit().await.expect("commit");
        created
    };
    let cord = created.cord.clone();
    assert_eq!(cord.name, "the porch");
    assert!(cord.created_at.ends_with('Z'), "{}", cord.created_at);

    let client = pool.get().await.expect("pool.get");

    // the cell: a private root cord with no owner column
    let cell = query_cell_get(&client, cord.id)
        .await
        .expect("cell_get")
        .expect("the cord cell exists");
    assert_eq!(cell.kind.as_deref(), Some(CORD_CELL_KIND));
    assert_eq!(cell.visibility, CellVisibility::Private);
    assert_eq!(cell.parent_id, None);
    assert_eq!(cell.root_id, None);
    assert_eq!(cell.created_by, None, "role grants are the only authority");
    assert_eq!(cell.data, serde_json::json!({ "name": "the porch" }));

    // the creator's grant: cord_owner, scoped exactly to the cord
    let owner_grants: Vec<(Uuid, String, Option<String>, Option<Uuid>)> = client
        .query(
            "SELECT id, role, scope_kind, scope_id FROM role_grant
             WHERE actor_id = $1 AND revoked_at IS NULL",
            &[&owner],
        )
        .await
        .expect("owner grants")
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .collect();
    assert_eq!(
        owner_grants,
        vec![(
            created.owner_role_grant_id,
            CORD_OWNER.to_owned(),
            Some(CORD_SCOPE_KIND.to_owned()),
            Some(cord.id)
        )]
    );

    // the stamped grants: one per cord role, role-shaped, scoped to the cord
    let mut stamped: Vec<(String, CellGrantLevel)> =
        query_cell_grant_list_for_cell(&client, cord.id)
            .await
            .expect("cell grants")
            .into_iter()
            .map(|g| {
                assert_eq!(g.actor_id, None, "no per-member grants");
                assert_eq!(g.scope_id, Some(cord.id));
                assert_eq!(g.granted_by, None);
                (g.role.expect("role-shaped"), g.level)
            })
            .collect();
    stamped.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected: Vec<(String, CellGrantLevel)> = CORD_STAMPED_GRANTS
        .iter()
        .map(|(role, level)| ((*role).to_owned(), *level))
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(stamped, expected);

    // reads
    assert_eq!(
        query_cord_get(&client, cord.id).await.expect("get"),
        Some(cord.clone())
    );
    assert_eq!(
        query_cord_get(&client, Uuid::nil()).await.expect("get"),
        None
    );
    assert_eq!(
        query_cords_by_ids(&client, &[cord.id, Uuid::nil()])
            .await
            .expect("by ids"),
        vec![cord.clone()]
    );

    // add a member the way a cord verb will, then check the spine's own
    // visibility query agrees with the stamped grants
    query_create_role_grant(
        &client,
        &member,
        CORD_MEMBER,
        Some(CORD_SCOPE_KIND),
        Some(&cord.id),
        Some(&owner),
        None,
    )
    .await
    .expect("member grant");
    assert!(
        cell_list_shows(&pool, owner, cord.id).await,
        "owner sees it"
    );
    assert!(
        cell_list_shows(&pool, member, cord.id).await,
        "member sees it"
    );
    assert!(
        !cell_list_shows(&pool, stranger, cord.id).await,
        "a stranger doesn't"
    );

    // a second cord: owning it confers nothing in the first
    let other = query_cord_create(&client, "elsewhere", stranger)
        .await
        .expect("create second cord")
        .cord;
    assert!(
        !cell_list_shows(&pool, stranger, cord.id).await,
        "owning one cord doesn't reveal another"
    );
    assert!(cell_list_shows(&pool, stranger, other.id).await);

    // the ops list: every cord, newest first, with member counts
    let admin_rows = query_cord_admin_list(&client).await.expect("admin list");
    let summary: Vec<(Uuid, i64)> = admin_rows
        .iter()
        .map(|r| (r.cord.id, r.member_count))
        .collect();
    assert_eq!(summary, vec![(other.id, 1), (cord.id, 2)]);

    // a soft-deleted member, or a member on a soft-deleted account, drops out
    // of the count while their grant stays
    client
        .execute(
            "UPDATE actor SET deleted_at = NOW() WHERE id = $1",
            &[&member],
        )
        .await
        .expect("soft-delete the member's actor");
    let counts = |rows: Vec<cord_server::cord_queries::CordAdminRow>| -> Vec<i64> {
        rows.iter().map(|r| r.member_count).collect()
    };
    assert_eq!(
        counts(query_cord_admin_list(&client).await.expect("admin list")),
        vec![1, 1]
    );
    client
        .execute(
            "UPDATE account SET deleted_at = NOW()
             WHERE id = (SELECT account_id FROM actor WHERE id = $1)",
            &[&stranger],
        )
        .await
        .expect("soft-delete the stranger's account");
    assert_eq!(
        counts(query_cord_admin_list(&client).await.expect("admin list")),
        vec![0, 1],
        "a cord whose only member is gone counts zero"
    );

    // rename keeps the rest of `data` and still has no owner column
    client
        .execute(
            "UPDATE cell SET data = data || '{\"topic\": \"kept\"}'::jsonb WHERE id = $1",
            &[&cord.id],
        )
        .await
        .expect("seed an extra data field");
    let renamed = query_cord_rename(&client, cord.id, "the stoop", owner)
        .await
        .expect("rename")
        .expect("the cord exists");
    assert_eq!(renamed.name, "the stoop");
    let cell = query_cell_get(&client, cord.id)
        .await
        .expect("cell_get")
        .expect("exists");
    assert_eq!(
        cell.data,
        serde_json::json!({ "name": "the stoop", "topic": "kept" })
    );
    assert_eq!(cell.created_by, None);
    assert_eq!(
        query_cord_rename(&client, Uuid::nil(), "x", owner)
            .await
            .expect("rename a missing cord"),
        None
    );
}
