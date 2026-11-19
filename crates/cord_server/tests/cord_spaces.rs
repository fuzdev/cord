//! Postgres-backed tests for a cord's spaces (`cord_server::cord_space_queries`).
//!
//! What only the database can show: a space is a private child cell of its
//! cord with no `created_by` and the cord's role-shaped grants stamped on it,
//! which the spine's own visibility query agrees with; the list is the cord's
//! live spaces, oldest first; owners and admins manage spaces and plain
//! members don't, by the role as the database has it, not a stale snapshot,
//! and a create queued behind a demotion on the cord lock honors it;
//! anyone outside the cord (an instance admin included) gets
//! `CordNotFound` before any space is looked up, and a space under another
//! cord reads as missing; a deleted space is gone from the list and from
//! every verb; and the cap on live spaces holds, under concurrent creates
//! too.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and its `CORD_SCHEMA_READY_TEST_URL` override). It resets the `public`
//! schema, so the scenarios run in order from one test function:
//!
//! ```bash
//! cargo test -p cord_server --test cord_spaces -- --ignored
//! ```

#![expect(
    clippy::expect_used,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

mod common;

use common::{add_member, assert_refused, create_cord, fresh_db_pool, in_tx, seed_actor};
use cord_server::cord_authz::CordRole;
use cord_server::cord_queries::{CORD_STAMPED_GRANTS, CordRow};
use cord_server::cord_space_queries::{
    CORD_SPACE_CELL_KIND, CORD_SPACE_COUNT_MAX, CordSpaceError, CordSpaceKind, CordSpaceRow,
    query_cord_space_create, query_cord_space_delete, query_cord_space_rename, query_cord_spaces,
};
use fuz_auth::actor_queries::query_role_grant_find_active_for_actor;
use fuz_auth::{ROLE_ADMIN, query_create_role_grant};
use fuz_cell::cell_grant_queries::query_cell_grant_list_for_cell;
use fuz_cell::cell_queries::{CellListParams, query_cell_get, query_cell_list};
use fuz_cell::cell_view_sql::CellViewer;
use fuz_cell::{CellGrantLevel, CellVisibility};
use fuz_db::Pool;
use uuid::Uuid;

async fn create(
    pool: &Pool,
    cord_id: Uuid,
    caller: Uuid,
    name: &str,
) -> Result<CordSpaceRow, CordSpaceError> {
    in_tx!(pool, |tx| query_cord_space_create(
        &tx,
        cord_id,
        caller,
        CordSpaceKind::Line,
        name
    )
    .await)
}

async fn rename(
    pool: &Pool,
    cord_id: Uuid,
    space_id: Uuid,
    caller: Uuid,
    name: &str,
) -> Result<CordSpaceRow, CordSpaceError> {
    in_tx!(pool, |tx| query_cord_space_rename(
        &tx, cord_id, space_id, caller, name
    )
    .await)
}

async fn delete(
    pool: &Pool,
    cord_id: Uuid,
    space_id: Uuid,
    caller: Uuid,
) -> Result<CordSpaceRow, CordSpaceError> {
    in_tx!(pool, |tx| query_cord_space_delete(
        &tx, cord_id, space_id, caller
    )
    .await)
}

async fn list(pool: &Pool, cord_id: Uuid) -> Vec<CordSpaceRow> {
    let client = pool.get().await.expect("pool.get");
    query_cord_spaces(&client, cord_id).await.expect("list")
}

async fn names(pool: &Pool, cord_id: Uuid) -> Vec<String> {
    list(pool, cord_id)
        .await
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// Whether the spine's visibility-filtered `cell_list` shows `cell_id` to an
/// actor holding their active role grants.
async fn cell_list_shows(pool: &Pool, actor_id: Uuid, cell_id: Uuid) -> bool {
    let client = pool.get().await.expect("pool.get");
    let grants = query_role_grant_find_active_for_actor(&client, &actor_id)
        .await
        .expect("grants");
    let ids = [cell_id];
    let params = CellListParams {
        ids: Some(&ids),
        viewer: CellViewer::for_actor(
            actor_id,
            grants.iter().map(|g| (g.role.as_str(), g.scope_id)),
        ),
        limit: 10,
        ..CellListParams::default()
    };
    !query_cell_list(&client, &params)
        .await
        .expect("cell_list")
        .is_empty()
}

/// The cord and the people in and around it.
#[derive(Debug, Clone)]
struct Den {
    cord: CordRow,
    owner: Uuid,
    admin: Uuid,
    member: Uuid,
    stranger: Uuid,
    instance_admin: Uuid,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn spaces_are_cord_cells_its_managers_shape() {
    let pool = fresh_db_pool().await;
    let owner = seed_actor(&pool, "owner").await;
    let admin = seed_actor(&pool, "admin").await;
    let member = seed_actor(&pool, "member").await;
    let stranger = seed_actor(&pool, "stranger").await;
    let instance_admin = seed_actor(&pool, "operator").await;
    {
        let client = pool.get().await.expect("pool.get");
        query_create_role_grant(&client, &instance_admin, ROLE_ADMIN, None, None, None, None)
            .await
            .expect("global admin grant");
    }
    let cord = create_cord(&pool, "the den", owner).await;
    add_member(&pool, cord.id, admin, CordRole::Admin).await;
    add_member(&pool, cord.id, member, CordRole::Member).await;
    let den = Den {
        cord,
        owner,
        admin,
        member,
        stranger,
        instance_admin,
    };

    scenario_a_space_is_a_child_cell(&pool, &den).await;
    scenario_list_order(&pool, &den).await;
    scenario_roles(&pool, &den).await;
    scenario_outside_the_cord(&pool, &den).await;
    scenario_rename_keeps_the_rest(&pool, &den).await;
    scenario_delete(&pool, &den).await;
    scenario_the_role_is_read_afresh(&pool, &den).await;
    scenario_a_demotion_holding_the_lock(&pool).await;
    scenario_the_cap(&pool).await;
    scenario_concurrent_creates_at_the_cap(&pool).await;
}

/// The cell: private, a child of the cord, no owner column, the cord's
/// grants stamped, and the spine's own view query agreeing.
async fn scenario_a_space_is_a_child_cell(pool: &Pool, den: &Den) {
    let space = create(pool, den.cord.id, den.owner, "the hearth")
        .await
        .expect("create");
    assert_eq!(space.name, "the hearth");
    assert_eq!(space.cord_id, den.cord.id);
    assert_eq!(space.kind, CordSpaceKind::Line);
    assert!(space.created_at.ends_with('Z'), "{}", space.created_at);

    let client = pool.get().await.expect("pool.get");
    let cell = query_cell_get(&client, space.id)
        .await
        .expect("cell_get")
        .expect("the space cell exists");
    assert_eq!(cell.kind.as_deref(), Some(CORD_SPACE_CELL_KIND));
    assert_eq!(cell.visibility, CellVisibility::Private);
    assert_eq!(cell.parent_id, Some(den.cord.id));
    assert_eq!(cell.root_id, Some(den.cord.id));
    assert_eq!(cell.created_by, None, "role grants are the only authority");
    assert_eq!(
        cell.data,
        serde_json::json!({ "kind": "line", "name": "the hearth" })
    );

    let mut stamped: Vec<(String, CellGrantLevel)> =
        query_cell_grant_list_for_cell(&client, space.id)
            .await
            .expect("cell grants")
            .into_iter()
            .map(|g| {
                assert_eq!(g.actor_id, None, "no per-member grants");
                assert_eq!(g.scope_id, Some(den.cord.id), "scoped to the cord");
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

    for (actor, sees) in [
        (den.owner, true),
        (den.admin, true),
        (den.member, true),
        (den.stranger, false),
    ] {
        assert_eq!(
            cell_list_shows(pool, actor, space.id).await,
            sees,
            "{actor}"
        );
    }

    assert_eq!(list(pool, den.cord.id).await, vec![space.clone()]);
    delete(pool, den.cord.id, space.id, den.owner)
        .await
        .expect("clean up");
}

/// Oldest first, only this cord's spaces.
async fn scenario_list_order(pool: &Pool, den: &Den) {
    let other_cord = create_cord(pool, "next door", den.stranger).await;
    let mut made = Vec::new();
    for name in ["c", "a", "b"] {
        made.push(
            create(pool, den.cord.id, den.admin, name)
                .await
                .expect("create"),
        );
        create(pool, other_cord.id, den.stranger, name)
            .await
            .expect("create next door");
    }
    assert_eq!(list(pool, den.cord.id).await, made);
    assert_eq!(names(pool, other_cord.id).await, ["c", "a", "b"]);
    // two spaces born in one transaction share `created_at`; the id breaks the tie
    let (first, second) = in_tx!(pool, |tx| async {
        let first =
            query_cord_space_create(&tx, den.cord.id, den.owner, CordSpaceKind::Line, "x").await?;
        let second =
            query_cord_space_create(&tx, den.cord.id, den.owner, CordSpaceKind::Line, "y").await?;
        Ok::<_, CordSpaceError>((first, second))
    }
    .await)
    .expect("two at once");
    assert_eq!(first.created_at, second.created_at);
    let tail: Vec<Uuid> = list(pool, den.cord.id)
        .await
        .iter()
        .skip(made.len())
        .map(|s| s.id)
        .collect();
    let mut by_id = vec![first.id, second.id];
    by_id.sort();
    assert_eq!(tail, by_id);
    for space in made.iter().chain([&first, &second]) {
        delete(pool, den.cord.id, space.id, den.owner)
            .await
            .expect("clean up");
    }
    assert!(list(pool, den.cord.id).await.is_empty());
}

/// Owners and admins manage spaces; plain members only see them.
async fn scenario_roles(pool: &Pool, den: &Den) {
    let by_admin = create(pool, den.cord.id, den.admin, "admin's")
        .await
        .expect("an admin creates");
    rename(pool, den.cord.id, by_admin.id, den.owner, "owner renamed")
        .await
        .expect("an owner renames an admin's space");
    assert_refused(
        create(pool, den.cord.id, den.member, "mine").await,
        "Forbidden",
    );
    assert_refused(
        rename(pool, den.cord.id, by_admin.id, den.member, "mine").await,
        "Forbidden",
    );
    assert_refused(
        delete(pool, den.cord.id, by_admin.id, den.member).await,
        "Forbidden",
    );
    // a member asking about a space that isn't there still learns only that
    // they can't manage spaces
    assert_refused(
        delete(pool, den.cord.id, Uuid::nil(), den.member).await,
        "Forbidden",
    );
    assert_eq!(names(pool, den.cord.id).await, ["owner renamed"]);
    delete(pool, den.cord.id, by_admin.id, den.admin)
        .await
        .expect("an admin deletes");
}

/// Outside the cord, every verb answers `CordNotFound` first, whatever the
/// space id; inside it, a space of another cord is missing.
async fn scenario_outside_the_cord(pool: &Pool, den: &Den) {
    let space = create(pool, den.cord.id, den.owner, "the hearth")
        .await
        .expect("create");
    let elsewhere = create_cord(pool, "elsewhere", den.stranger).await;
    let foreign = create(pool, elsewhere.id, den.stranger, "theirs")
        .await
        .expect("create elsewhere");

    for outsider in [den.stranger, den.instance_admin] {
        assert_refused(
            create(pool, den.cord.id, outsider, "taken over").await,
            "CordNotFound",
        );
        for space_id in [space.id, foreign.id, Uuid::nil()] {
            assert_refused(
                rename(pool, den.cord.id, space_id, outsider, "taken over").await,
                "CordNotFound",
            );
            assert_refused(
                delete(pool, den.cord.id, space_id, outsider).await,
                "CordNotFound",
            );
        }
    }
    // no such cord at all reads the same
    assert_refused(
        create(pool, Uuid::nil(), den.owner, "nowhere").await,
        "CordNotFound",
    );
    assert_refused(
        create(pool, space.id, den.owner, "a space is no cord").await,
        "CordNotFound",
    );
    // the den's owner isn't in `elsewhere`
    assert_refused(
        rename(pool, elsewhere.id, foreign.id, den.owner, "taken over").await,
        "CordNotFound",
    );
    // naming another cord's space under the den, or the den itself as a space
    for space_id in [foreign.id, den.cord.id, elsewhere.id, Uuid::nil()] {
        assert_refused(
            rename(pool, den.cord.id, space_id, den.owner, "taken over").await,
            "SpaceNotFound",
        );
        assert_refused(
            delete(pool, den.cord.id, space_id, den.owner).await,
            "SpaceNotFound",
        );
    }
    assert_eq!(names(pool, elsewhere.id).await, ["theirs"]);
    assert_eq!(names(pool, den.cord.id).await, ["the hearth"]);
    delete(pool, den.cord.id, space.id, den.owner)
        .await
        .expect("clean up");
}

/// A rename changes the name and keeps the rest of `data`.
async fn scenario_rename_keeps_the_rest(pool: &Pool, den: &Den) {
    let space = create(pool, den.cord.id, den.owner, "the hearth")
        .await
        .expect("create");
    let client = pool.get().await.expect("pool.get");
    client
        .execute(
            "UPDATE cell SET data = data || '{\"topic\": \"kept\"}'::jsonb WHERE id = $1",
            &[&space.id],
        )
        .await
        .expect("seed an extra data field");
    let renamed = rename(pool, den.cord.id, space.id, den.admin, "the stove")
        .await
        .expect("rename");
    assert_eq!(
        renamed,
        CordSpaceRow {
            name: "the stove".to_owned(),
            ..space.clone()
        }
    );
    let cell = query_cell_get(&client, space.id)
        .await
        .expect("cell_get")
        .expect("exists");
    assert_eq!(
        cell.data,
        serde_json::json!({ "kind": "line", "name": "the stove", "topic": "kept" })
    );
    assert_eq!(cell.created_by, None);
    assert_eq!(cell.updated_by, Some(den.admin));
    delete(pool, den.cord.id, space.id, den.owner)
        .await
        .expect("clean up");
}

/// A deleted space is gone from the list and from every verb, but its cell
/// stays, tombstoned.
async fn scenario_delete(pool: &Pool, den: &Den) {
    let keep = create(pool, den.cord.id, den.owner, "kept")
        .await
        .expect("create");
    let gone = create(pool, den.cord.id, den.owner, "gone")
        .await
        .expect("create");
    let deleted = delete(pool, den.cord.id, gone.id, den.admin)
        .await
        .expect("delete");
    assert_eq!(deleted, gone);
    assert_eq!(list(pool, den.cord.id).await, vec![keep.clone()]);
    assert_refused(
        delete(pool, den.cord.id, gone.id, den.owner).await,
        "SpaceNotFound",
    );
    assert_refused(
        rename(pool, den.cord.id, gone.id, den.owner, "back").await,
        "SpaceNotFound",
    );
    let client = pool.get().await.expect("pool.get");
    let row = client
        .query_one(
            "SELECT deleted_at IS NOT NULL, updated_by FROM cell WHERE id = $1",
            &[&gone.id],
        )
        .await
        .expect("the tombstone");
    assert!(row.get::<_, bool>(0), "soft-deleted");
    assert_eq!(row.get::<_, Option<Uuid>>(1), Some(den.admin));
    delete(pool, den.cord.id, keep.id, den.owner)
        .await
        .expect("clean up");
}

/// A demotion or removal that committed is honored by every managing verb,
/// whatever the request's snapshot said.
async fn scenario_the_role_is_read_afresh(pool: &Pool, den: &Den) {
    let deputy = seed_actor(pool, "deputy").await;
    add_member(pool, den.cord.id, deputy, CordRole::Admin).await;
    let space = create(pool, den.cord.id, deputy, "deputy's")
        .await
        .expect("an admin creates");
    in_tx!(pool, |tx| {
        cord_server::cord_member_queries::query_cord_member_role_set(
            &tx,
            den.cord.id,
            den.owner,
            deputy,
            CordRole::Member,
        )
        .await
    })
    .expect("demote");
    assert_refused(
        create(pool, den.cord.id, deputy, "again").await,
        "Forbidden",
    );
    assert_refused(
        rename(pool, den.cord.id, space.id, deputy, "again").await,
        "Forbidden",
    );
    assert_refused(
        delete(pool, den.cord.id, space.id, deputy).await,
        "Forbidden",
    );
    in_tx!(pool, |tx| {
        cord_server::cord_member_queries::query_cord_member_remove(
            &tx,
            den.cord.id,
            den.owner,
            deputy,
        )
        .await
    })
    .expect("remove");
    assert_refused(
        rename(pool, den.cord.id, space.id, deputy, "again").await,
        "CordNotFound",
    );
    delete(pool, den.cord.id, space.id, den.owner)
        .await
        .expect("clean up");
}

/// A cord holds at most `CORD_SPACE_COUNT_MAX` live spaces; a deleted one
/// frees its place.
async fn scenario_the_cap(pool: &Pool) {
    let owner = seed_actor(pool, "cap_owner").await;
    let cord = create_cord(pool, "the full house", owner).await;
    let mut spaces = Vec::new();
    for i in 0..CORD_SPACE_COUNT_MAX {
        spaces.push(
            create(pool, cord.id, owner, &format!("room {i}"))
                .await
                .expect("under the cap"),
        );
    }
    assert_refused(
        create(pool, cord.id, owner, "one more").await,
        "LimitReached",
    );
    assert_eq!(list(pool, cord.id).await.len(), CORD_SPACE_COUNT_MAX);
    let freed = spaces.pop().expect("a space");
    delete(pool, cord.id, freed.id, owner)
        .await
        .expect("delete");
    create(pool, cord.id, owner, "one more")
        .await
        .expect("a deleted space frees its place");
    assert_refused(
        create(pool, cord.id, owner, "and another").await,
        "LimitReached",
    );
}

/// Creates racing for the last place: the cord lock lets exactly one in.
async fn scenario_concurrent_creates_at_the_cap(pool: &Pool) {
    let owner = seed_actor(pool, "race_owner").await;
    let admin = seed_actor(pool, "race_admin").await;
    let cord = create_cord(pool, "the last seat", owner).await;
    add_member(pool, cord.id, admin, CordRole::Admin).await;
    for i in 0..CORD_SPACE_COUNT_MAX - 1 {
        create(pool, cord.id, owner, &format!("room {i}"))
            .await
            .expect("under the cap");
    }
    for round in 0..4_usize {
        let racers = 4_usize;
        let run = |slot: usize| {
            let pool = pool.clone();
            let caller = if slot.is_multiple_of(2) { owner } else { admin };
            let cord_id = cord.id;
            tokio::spawn(async move {
                let mut client = pool.get().await.expect("pool.get");
                let tx = client.transaction().await.expect("begin");
                let result = query_cord_space_create(
                    &tx,
                    cord_id,
                    caller,
                    CordSpaceKind::Line,
                    &format!("last {round}.{slot}"),
                )
                .await;
                // hold the lock a moment so the others queue on it
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                if result.is_ok() {
                    tx.commit().await.expect("commit");
                } else {
                    tx.rollback().await.expect("rollback");
                }
                result.map(|_| ())
            })
        };
        let handles: Vec<_> = (0..racers).map(run).collect();
        let all = async {
            let mut results = Vec::new();
            for handle in handles {
                results.push(handle.await.expect("join"));
            }
            results
        };
        let results = tokio::time::timeout(std::time::Duration::from_secs(10), all)
            .await
            .expect("no hang: every create finishes");
        let created = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(created, 1, "round {round}: {results:?}");
        for result in results.into_iter().filter(Result::is_err) {
            assert_refused(result, "LimitReached");
        }
        let spaces = list(pool, cord.id).await;
        assert_eq!(spaces.len(), CORD_SPACE_COUNT_MAX, "round {round}");
        // free the last place for the next round
        let last = spaces.last().expect("a space");
        delete(pool, cord.id, last.id, owner).await.expect("delete");
    }
}

/// A create that queues behind a demotion holding the cord lock reads the
/// role after the lock, so the demotion wins.
async fn scenario_a_demotion_holding_the_lock(pool: &Pool) {
    let owner = seed_actor(pool, "queue_owner").await;
    let deputy = seed_actor(pool, "queue_deputy").await;
    let cord = create_cord(pool, "the queue", owner).await;
    add_member(pool, cord.id, deputy, CordRole::Admin).await;
    let mut client = pool.get().await.expect("pool.get");
    let demotion = client.transaction().await.expect("begin");
    let demotion_pid: i32 = demotion
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("pid")
        .get(0);
    cord_server::cord_member_queries::query_cord_member_role_set(
        &demotion,
        cord.id,
        owner,
        deputy,
        CordRole::Member,
    )
    .await
    .expect("demote, holding the cord lock");
    let create = tokio::spawn({
        let pool = pool.clone();
        let cord_id = cord.id;
        async move { create(&pool, cord_id, deputy, "slipped in").await }
    });
    // wait until the create is queued on the cord lock the demotion holds
    let probe = pool.get().await.expect("pool.get");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while probe
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
            &[&demotion_pid],
        )
        .await
        .expect("probe")
        .get::<_, i64>(0)
        == 0
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the create never queued on the cord lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!create.is_finished(), "the create waits on the cord lock");
    demotion.commit().await.expect("commit the demotion");
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), create)
        .await
        .expect("no hang")
        .expect("join");
    assert_refused(result, "Forbidden");
    assert!(list(pool, cord.id).await.is_empty());
}
