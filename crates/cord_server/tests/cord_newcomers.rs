//! Postgres-backed tests for newcomer links
//! (`cord_server::invite::redemption_queries`, and accept after a
//! redemption in `cord_server::invite::queries`).
//!
//! What only the database can show: a redemption creates the spine's
//! username invite in the minter's name, records the redemption, and spends
//! one use; redeeming the same username again changes nothing wherever
//! accept would still honor it (an exhausted link, one expired within the
//! grace, one whose minter is no longer an admin), is refused on a link
//! expired past the grace, and answers as a fresh redemption once its spine
//! invite is gone, or once an account holds its username; a taken or
//! reserved username is refused, though a closed or expired link answers it
//! with its own refusal, as it does a free one; the minter's global `admin`
//! is re-read at every new reservation, so a demoted, deleted, or purged
//! minter closes the link to new usernames; concurrent redemptions never
//! spend more uses than the link has;
//! the newcomer's accept after signing up spends nothing, even from an
//! exhausted link or within the grace after expiry, and finds the
//! redemption by username when their signup claimed no invite; and a
//! revoke deletes the waiting reservations, so the stock signup no longer
//! admits them, as does the expiry sweep's pass
//! (`cord_server::invite::sweep`) for links expired past the grace,
//! skipping a link another transaction holds. Last, the upkeep tasks the
//! server runs (`cord_server::runtime::upkeep_options`, spawned by the spine's
//! `fuz_actions::server::spawn_upkeep`): their startup passes
//! run that sweep and the spine's auth cleanup, which closes the expired
//! session it deletes through the closer they're handed, and they end on
//! shutdown.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and its `CORD_SCHEMA_READY_TEST_URL` override). It resets the `public`
//! schema, so the scenarios run in order from one test function:
//!
//! ```bash
//! cargo test -p cord_server --test cord_newcomers -- --ignored
//! ```

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    accept, add_member, await_lock_waiters, create_cord, fresh_db_pool, in_tx, lock_waiters, mint,
    purge_account_steps, redeem, redemption_list, seed_actor, status_of,
};
use cord_server::cord_authz::CordRole;
use cord_server::cord_member_queries::query_cord_leave;
use cord_server::cord_queries::CordRow;
use cord_server::cord_queries::query_cord_rename;
use cord_server::invite::queries::{
    CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS, CordInviteError, CordInviteStatus,
    query_cord_invite_list, query_cord_invite_revoke,
};
use cord_server::invite::redemption_queries::{
    CordInviteRedeemed, query_actor_holds_global_admin, query_actor_holds_global_admin_unlocked,
};
use cord_server::invite::sweep::{
    CORD_INVITE_EXPIRE_AUDIT_EVENT, CordInviteExpireReleased, cord_invite_sweep_pass,
};
use cord_server::runtime::upkeep_options;
use fuz_actions::server::spawn_upkeep;
use fuz_auth::account_queries::{
    query_account_lock_admins_for_purge, query_account_lock_for_purge,
    query_account_lock_for_revoke, query_account_soft_delete, query_purge_account,
};
use fuz_auth::actor_queries::{
    query_actor_soft_delete, query_count_active_accounts_with_global_role_locked,
};
use fuz_auth::invite_queries::{
    query_create_invite, query_invite_claim_unscoped, query_invite_delete_unclaimed_by_creators,
    query_invite_find_unclaimed_match_for_update,
};
use fuz_auth::role_grant_offer_queries::query_revoke_role_grant;
use fuz_auth::{
    AuditEmitter, AuditLogEvent, ConnectionCloseTarget, ConnectionCloser,
    CreateRoleGrantOfferInput, ROLE_ADMIN, query_create_role_grant, query_role_grant_offer_create,
};
use fuz_cell::CellError;
use fuz_db::Pool;
use fuz_db::tokio_postgres::error::SqlState;
use uuid::Uuid;

/// Seed an actor holding a global `admin` grant.
async fn seed_admin(pool: &Pool, username: &str) -> Uuid {
    let actor = seed_actor(pool, username).await;
    let client = pool.get().await.expect("pool.get");
    query_create_role_grant(&client, &actor, ROLE_ADMIN, None, None, None, None)
        .await
        .expect("grant admin");
    actor
}

/// What the stock signup does with a username invite: create the account and
/// claim the invite. Returns the new actor.
async fn sign_up(pool: &Pool, username: &str) -> Uuid {
    let actor = seed_actor(pool, username).await;
    let client = pool.get().await.expect("pool.get");
    let (invite_id, account_id): (Uuid, Uuid) = {
        let row = client
            .query_one(
                "SELECT i.id, ac.account_id FROM invite i, actor ac
                 WHERE i.username = $1 AND i.claimed_at IS NULL AND ac.id = $2",
                &[&username, &actor],
            )
            .await
            .expect("the invite signup would match");
        (row.get(0), row.get(1))
    };
    assert!(
        query_invite_claim_unscoped(&client, &invite_id, &account_id)
            .await
            .expect("claim")
    );
    actor
}

async fn use_count(pool: &Pool, invite_id: Uuid) -> i32 {
    let client = pool.get().await.expect("pool.get");
    client
        .query_one(
            "SELECT use_count FROM cord_invite WHERE id = $1",
            &[&invite_id],
        )
        .await
        .expect("row")
        .get(0)
}

async fn count(pool: &Pool, sql: &str) -> i64 {
    let client = pool.get().await.expect("pool.get");
    client.query_one(sql, &[]).await.expect("count").get(0)
}

/// Move the link's expiry to `seconds` ago.
async fn backdate(pool: &Pool, invite_id: Uuid, seconds: i64) {
    let client = pool.get().await.expect("pool.get");
    client
        .execute(
            "UPDATE cord_invite SET expires_at = NOW() - $2::bigint * INTERVAL '1 second'
             WHERE id = $1",
            &[&invite_id, &seconds],
        )
        .await
        .expect("backdate");
}

/// Whether the stock signup would find an invite for `username` (with no
/// email), the predicate it admits or refuses on.
async fn signup_admits(pool: &Pool, username: &str) -> bool {
    let client = pool.get().await.expect("pool.get");
    query_invite_find_unclaimed_match_for_update(&client, None, username)
        .await
        .expect("match")
        .is_some()
}

/// The account that claimed the spine invite `invite_id`, if any.
async fn claimed_by(pool: &Pool, invite_id: Uuid) -> Option<Uuid> {
    let client = pool.get().await.expect("pool.get");
    client
        .query_one("SELECT claimed_by FROM invite WHERE id = $1", &[&invite_id])
        .await
        .expect("the invite")
        .get(0)
}

async fn account_of(pool: &Pool, actor_id: Uuid) -> Uuid {
    let client = pool.get().await.expect("pool.get");
    client
        .query_one("SELECT account_id FROM actor WHERE id = $1", &[&actor_id])
        .await
        .expect("the actor")
        .get(0)
}

/// The link `invite_id`'s redemption counts, as the managers' list reports
/// them: (redeemed, accepted).
async fn redemption_counts(pool: &Pool, cord_id: Uuid, invite_id: Uuid) -> (i64, i64) {
    let client = pool.get().await.expect("pool.get");
    let listed = query_cord_invite_list(&client, cord_id)
        .await
        .expect("list")
        .into_iter()
        .find(|l| l.invite.id == invite_id)
        .expect("listed");
    (listed.redemption_count, listed.redemption_accepted_count)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn newcomer_links_admit_new_people() {
    let pool = fresh_db_pool().await;
    let admin = seed_admin(&pool, "admin").await;
    let cord = create_cord(&pool, "the porch", admin).await;

    let (token, link_id, redeemed) =
        scenario_a_redemption_reserves_a_username(&pool, &cord, admin).await;
    let ordinary = scenario_an_ordinary_link_is_for_existing_accounts(&pool, &cord, admin).await;
    let existing =
        scenario_accept_after_redemption_spends_nothing(&pool, &token, link_id, &redeemed).await;
    scenario_existing_accounts_and_early_newcomers(&pool, &cord, admin, existing, &ordinary).await;
    scenario_accept_within_the_grace(&pool, &cord, admin).await;
    scenario_revoke_releases_waiting_reservations(&pool, &cord, admin).await;
    scenario_a_deleted_signup_invite_leaves_the_record(&pool, &cord, admin).await;
    scenario_a_signup_that_claimed_no_invite(&pool, &cord, admin).await;
    scenario_an_account_that_took_a_waiting_username(&pool, &cord, admin).await;
    scenario_an_older_email_invite_won_the_claim(&pool, &cord, admin).await;
    let (demoted, demoted_link) = scenario_a_demoted_minter_closes_the_link(&pool, &cord).await;
    scenario_a_scoped_admin_grant_is_no_authority(&pool, &cord, admin, demoted, &demoted_link)
        .await;
    scenario_a_soft_deleted_minter_closes_the_link(&pool, &cord).await;
    scenario_a_purged_minter_fails_closed(&pool, &cord).await;
    for removal in [
        MinterRemoval::Delete,
        MinterRemoval::Revoke,
        MinterRemoval::Purge,
    ] {
        scenario_a_removal_of_the_minter_mid_redeem(&pool, &cord, removal).await;
    }
    for stall in [AcceptStall::Cord, AcceptStall::Link] {
        for signup in [NewcomerSignup::Claimed, NewcomerSignup::Unclaimed] {
            scenario_a_purge_of_the_minter_mid_accept(&pool, &cord, stall, signup).await;
        }
    }
    scenario_a_purge_a_leave_and_an_accept(&pool, &cord).await;
    scenario_a_purge_of_a_minter_who_renamed_the_cord_mid_accept(&pool).await;
    scenario_a_purge_of_the_newcomer_mid_fallback_accept(&pool, &cord).await;
    scenario_dead_links_never_say_whether_a_username_is_free(&pool, &cord, admin).await;
    scenario_redemptions_never_overshoot_max_uses(&pool, &cord, admin).await;
    scenario_the_expiry_sweep(&pool, &cord, admin).await;
    scenario_the_upkeep_tasks(&pool, &cord, admin).await;
}

/// A redemption creates the spine invite and the row and spends one use;
/// the same username again is idempotent; taken or reserved usernames are
/// refused. Returns the link (two uses) and the redemption.
async fn scenario_a_redemption_reserves_a_username(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
) -> (String, Uuid, CordInviteRedeemed) {
    let (token, link_id) = mint(pool, cord.id, admin, Some(3600), Some(2), true).await;
    let redeemed = redeem(pool, &token, "newcomer").await.expect("redeem");
    assert!(redeemed.created);
    assert_eq!(redeemed.cord.id, cord.id);
    assert_eq!(redeemed.invite_id, link_id);
    assert_eq!(redeemed.redemption.cord_invite_id, link_id);
    assert_eq!(redeemed.redemption.username, "newcomer");
    assert_eq!(redeemed.redemption.accepted_at, None);
    let signup_invite_id = redeemed.redemption.signup_invite_id.expect("signup invite");
    {
        let client = pool.get().await.expect("pool.get");
        let row = client
            .query_one(
                "SELECT username, email, created_by, claimed_at IS NULL FROM invite WHERE id = $1",
                &[&signup_invite_id],
            )
            .await
            .expect("the spine invite");
        assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("newcomer"));
        assert_eq!(row.get::<_, Option<String>>(1), None, "a username invite");
        assert_eq!(
            row.get::<_, Option<Uuid>>(2),
            Some(admin),
            "created in the minter's name"
        );
        assert!(row.get::<_, bool>(3), "unclaimed");
    }
    assert_eq!(use_count(pool, link_id).await, 1);

    // -- the same username again is idempotent --------------------------------
    let again = redeem(pool, &token, "newcomer")
        .await
        .expect("redeem again");
    assert!(!again.created);
    assert_eq!(again.redemption, redeemed.redemption);
    assert_eq!(use_count(pool, link_id).await, 1, "no new use");
    assert_eq!(
        count(pool, "SELECT COUNT(*) FROM invite").await,
        1,
        "no new invite"
    );

    // -- taken or reserved usernames ------------------------------------------
    seed_actor(pool, "taken").await;
    {
        let client = pool.get().await.expect("pool.get");
        query_create_invite(&client, None, Some("reserved"), &admin)
            .await
            .expect("an admin's own invite");
        query_create_invite(&client, Some("both@example.com"), Some("paired"), &admin)
            .await
            .expect("an invite naming an email and a username");
        client
            .execute(
                "UPDATE account SET deleted_at = NOW() WHERE username = 'taken'",
                &[],
            )
            .await
            .expect("soft-delete");
    }
    for username in ["taken", "reserved", "paired"] {
        assert!(
            matches!(
                redeem(pool, &token, username).await,
                Err(CordInviteError::UsernameUnavailable)
            ),
            "{username}"
        );
    }
    assert_eq!(
        use_count(pool, link_id).await,
        1,
        "a refusal spends nothing"
    );
    (token, link_id, redeemed)
}

/// Returns an ordinary link, for the scenario after the next.
async fn scenario_an_ordinary_link_is_for_existing_accounts(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
) -> String {
    let (ordinary, _) = mint(pool, cord.id, admin, Some(3600), Some(5), false).await;
    assert!(matches!(
        redeem(pool, &ordinary, "someone").await,
        Err(CordInviteError::NotForNewcomers)
    ));
    assert!(matches!(
        redeem(pool, "not a real token", "someone").await,
        Err(CordInviteError::NotFound)
    ));
    ordinary
}

/// Accept after redemption spends nothing, even when exhausted. Returns an
/// existing account the exhausted link refused.
async fn scenario_accept_after_redemption_spends_nothing(
    pool: &Pool,
    token: &str,
    link_id: Uuid,
    redeemed: &CordInviteRedeemed,
) -> Uuid {
    redeem(pool, token, "second").await.expect("the last use");
    assert_eq!(status_of(pool, token).await, CordInviteStatus::Exhausted);
    assert!(matches!(
        redeem(pool, token, "third").await,
        Err(CordInviteError::Exhausted)
    ));
    // still idempotent once exhausted: the reservation stands
    assert!(
        !redeem(pool, token, "second")
            .await
            .expect("idempotent")
            .created
    );
    let newcomer = sign_up(pool, "newcomer").await;
    let joined = accept(pool, token, newcomer).await.expect("accept");
    assert_eq!(joined.role, CordRole::Member);
    assert!(joined.role_grant_id.is_some());
    assert_eq!(joined.redemption_id, Some(redeemed.redemption.id));
    assert_eq!(
        joined.claimed_signup_invite_id, None,
        "the signup claimed the invite, not the accept"
    );
    assert_eq!(use_count(pool, link_id).await, 2, "accept spent nothing");
    {
        let redemptions = redemption_list(pool, link_id).await;
        assert_eq!(redemptions.len(), 2);
        assert_eq!(redemptions[0].accepted_by, Some(newcomer));
        assert!(redemptions[0].accepted_at.is_some());
        assert_eq!(redemptions[1].accepted_by, None);
        assert_eq!(
            redemption_counts(pool, redeemed.cord.id, link_id).await,
            (2, 1)
        );
    }
    // accepting again changes nothing
    let again = accept(pool, token, newcomer).await.expect("accept again");
    assert_eq!((again.role_grant_id, again.redemption_id), (None, None));
    // someone who didn't redeem it can't use the exhausted link
    let existing = seed_actor(pool, "existing").await;
    assert!(matches!(
        accept(pool, token, existing).await,
        Err(CordInviteError::Exhausted)
    ));
    // a redeemed username that signs up after exhaustion still joins
    let second = sign_up(pool, "second").await;
    let joined = accept(pool, token, second).await.expect("accept");
    assert!(joined.redemption_id.is_some());
    assert_eq!(use_count(pool, link_id).await, 2);
    existing
}

/// An existing account accepts a newcomer link normally, and a newcomer who
/// got in another way completes their redemption.
async fn scenario_existing_accounts_and_early_newcomers(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
    existing: Uuid,
    ordinary: &str,
) {
    let (open_link, open_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    let joined = accept(pool, &open_link, existing).await.expect("accept");
    assert_eq!(joined.redemption_id, None);
    assert_eq!(
        use_count(pool, open_id).await,
        1,
        "an ordinary accept spends a use"
    );

    // -- a newcomer who got in another way completes their redemption --------
    redeem(pool, &open_link, "early").await.expect("redeem");
    let early = sign_up(pool, "early").await;
    accept(pool, ordinary, early)
        .await
        .expect("in through the ordinary link");
    let already = accept(pool, &open_link, early).await.expect("accept");
    assert_eq!(already.role_grant_id, None, "already a member");
    assert!(
        already.redemption_id.is_some(),
        "the redemption is completed"
    );
    assert_eq!(
        use_count(pool, open_id).await,
        2,
        "only the redemption's use"
    );
}

/// Expiry after redemption: accept within the grace.
async fn scenario_accept_within_the_grace(pool: &Pool, cord: &CordRow, admin: Uuid) {
    let (expiring, expiring_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    redeem(pool, &expiring, "late").await.expect("redeem");
    redeem(pool, &expiring, "tardy").await.expect("redeem");
    backdate(pool, expiring_id, 1).await;
    assert!(matches!(
        redeem(pool, &expiring, "later").await,
        Err(CordInviteError::Expired)
    ));
    // within the grace, a waiting username is answered as accept would take it
    let late_again = redeem(pool, &expiring, "late")
        .await
        .expect("a waiting username within the grace");
    assert!(!late_again.created);
    assert_eq!(use_count(pool, expiring_id).await, 2, "no new use");
    let late = sign_up(pool, "late").await;
    let joined = accept(pool, &expiring, late).await.expect("accept");
    assert!(joined.redemption_id.is_some());
    assert_eq!(use_count(pool, expiring_id).await, 2);
    let grace = i64::from(CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS);
    // past the grace, a dead link answers a waiting username as any other
    backdate(pool, expiring_id, grace + 1).await;
    assert!(matches!(
        redeem(pool, &expiring, "tardy").await,
        Err(CordInviteError::Expired)
    ));
    backdate(pool, expiring_id, grace - 60).await;
    assert!(
        !redeem(pool, &expiring, "tardy")
            .await
            .expect("still within the grace")
            .created
    );
    let tardy = sign_up(pool, "tardy").await;
    backdate(pool, expiring_id, grace + 1).await;
    assert!(matches!(
        accept(pool, &expiring, tardy).await,
        Err(CordInviteError::Expired)
    ));
    backdate(pool, expiring_id, grace - 60).await;
    assert!(
        accept(pool, &expiring, tardy)
            .await
            .expect("within the grace")
            .redemption_id
            .is_some()
    );
}

/// Revoke deletes waiting reservations and refuses the accept.
async fn scenario_revoke_releases_waiting_reservations(pool: &Pool, cord: &CordRow, admin: Uuid) {
    let (revokable, revokable_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    let cancelled = redeem(pool, &revokable, "cancelled").await.expect("redeem");
    let cancelled_invite = cancelled.redemption.signup_invite_id.expect("invite");
    redeem(pool, &revokable, "midway").await.expect("redeem");
    let midway = sign_up(pool, "midway").await;
    assert!(signup_admits(pool, "cancelled").await);
    {
        let client = pool.get().await.expect("pool.get");
        let revoked = query_cord_invite_revoke(&client, cord.id, revokable_id)
            .await
            .expect("revoke")
            .expect("exists");
        assert!(revoked.revoked);
        assert_eq!(
            revoked.released_signup_invite_ids,
            vec![cancelled_invite],
            "only the unclaimed reservation"
        );
        let again = query_cord_invite_revoke(&client, cord.id, revokable_id)
            .await
            .expect("revoke again")
            .expect("exists");
        assert!(!again.revoked);
        assert!(again.released_signup_invite_ids.is_empty());
    }
    assert!(
        !signup_admits(pool, "cancelled").await,
        "the stock signup refuses the released username"
    );
    let redemptions = redemption_list(pool, revokable_id).await;
    assert_eq!(
        redemptions
            .iter()
            .map(|r| (r.username.as_str(), r.signup_invite_id.is_some()))
            .collect::<Vec<_>>(),
        [("cancelled", false), ("midway", true)],
        "the redemptions stay as the record"
    );
    assert_eq!(use_count(pool, revokable_id).await, 2);
    assert!(matches!(
        redeem(pool, &revokable, "cancelled").await,
        Err(CordInviteError::Revoked)
    ));
    // the newcomer who signed up before the revoke has an account, not the cord
    assert!(matches!(
        accept(pool, &revokable, midway).await,
        Err(CordInviteError::Revoked)
    ));
    // someone who takes the released username afterward gets nothing from it
    let taker = seed_actor(pool, "cancelled").await;
    assert!(matches!(
        accept(pool, &revokable, taker).await,
        Err(CordInviteError::Revoked)
    ));
}

/// An admin deleting the signup invite leaves the record.
async fn scenario_a_deleted_signup_invite_leaves_the_record(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
) {
    let (deletable, deletable_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    let first = redeem(pool, &deletable, "dropped").await.expect("redeem");
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "DELETE FROM invite WHERE id = $1",
                &[&first.redemption.signup_invite_id.expect("invite")],
            )
            .await
            .expect("delete the unclaimed invite");
    }
    let redemptions = redemption_list(pool, deletable_id).await;
    assert_eq!(redemptions.len(), 1, "the redemption stays");
    assert_eq!(redemptions[0].signup_invite_id, None);
    assert_eq!(
        use_count(pool, deletable_id).await,
        1,
        "its use stays spent"
    );
    let fresh = redeem(pool, &deletable, "dropped")
        .await
        .expect("a fresh redemption");
    assert!(fresh.created);
    assert_ne!(fresh.redemption.id, first.redemption.id);
    assert_eq!(use_count(pool, deletable_id).await, 2);
}

/// A signup that claimed no invite (open signup) still completes the
/// redemption, claiming its invite.
async fn scenario_a_signup_that_claimed_no_invite(pool: &Pool, cord: &CordRow, admin: Uuid) {
    let (single, single_id) = mint(pool, cord.id, admin, Some(3600), Some(1), true).await;
    let opener_redeemed = redeem(pool, &single, "opener").await.expect("redeem");
    let opener_invite = opener_redeemed.redemption.signup_invite_id.expect("invite");
    // open signup creates the account and claims nothing
    let opener = seed_actor(pool, "opener").await;
    // someone else's account doesn't reach the reservation
    let bystander = seed_actor(pool, "bystander").await;
    assert!(matches!(
        accept(pool, &single, bystander).await,
        Err(CordInviteError::Exhausted)
    ));
    let joined = accept(pool, &single, opener).await.expect("accept");
    assert!(joined.role_grant_id.is_some());
    assert_eq!(joined.redemption_id, Some(opener_redeemed.redemption.id));
    assert_eq!(joined.claimed_signup_invite_id, Some(opener_invite));
    assert_eq!(use_count(pool, single_id).await, 1, "no second use");
    assert_eq!(
        claimed_by(pool, opener_invite).await,
        Some(account_of(pool, opener).await),
        "the accept claimed the reservation"
    );
    assert_eq!(
        redemption_list(pool, single_id).await[0].accepted_by,
        Some(opener)
    );
}

/// An account that took a waiting username ends its waiting.
async fn scenario_an_account_that_took_a_waiting_username(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
) {
    // open signup gives someone the name without claiming its invite, so the
    // reservation could never be signed up; a re-redeem of the name falls
    // through to the new-username path instead of answering `created: false`
    let (drifting, drifting_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    let drifter_redeemed = redeem(pool, &drifting, "drifter").await.expect("redeem");
    let drifter_invite = drifter_redeemed
        .redemption
        .signup_invite_id
        .expect("invite");
    // the spine stores usernames lowercase, and the redeem query compares
    // the input case-insensitively
    let drifter = seed_actor(pool, "drifter").await;
    assert!(matches!(
        redeem(pool, &drifting, "Drifter").await,
        Err(CordInviteError::UsernameUnavailable)
    ));
    assert_eq!(
        use_count(pool, drifting_id).await,
        1,
        "a refusal spends nothing"
    );
    // accept still completes the reservation through the fallback by username
    let joined = accept(pool, &drifting, drifter).await.expect("accept");
    assert_eq!(joined.redemption_id, Some(drifter_redeemed.redemption.id));
    assert_eq!(joined.claimed_signup_invite_id, Some(drifter_invite));
    assert_eq!(use_count(pool, drifting_id).await, 1, "no second use");
}

/// An older email-only invite won the signup's claim.
async fn scenario_an_older_email_invite_won_the_claim(pool: &Pool, cord: &CordRow, admin: Uuid) {
    {
        let client = pool.get().await.expect("pool.get");
        query_create_invite(&client, Some("mail@example.com"), None, &admin)
            .await
            .expect("an older email invite");
    }
    let (mailed, mailed_id) = mint(pool, cord.id, admin, Some(3600), Some(1), true).await;
    let mailer_redeemed = redeem(pool, &mailed, "mailer").await.expect("redeem");
    let mailer = seed_actor(pool, "mailer").await;
    {
        // the stock signup, given that email, claims the oldest match
        let client = pool.get().await.expect("pool.get");
        let matched = query_invite_find_unclaimed_match_for_update(
            &client,
            Some("mail@example.com"),
            "mailer",
        )
        .await
        .expect("match")
        .expect("an invite matches");
        assert_ne!(
            Some(matched.id),
            mailer_redeemed.redemption.signup_invite_id,
            "the email invite wins"
        );
        assert!(
            query_invite_claim_unscoped(&client, &matched.id, &account_of(pool, mailer).await)
                .await
                .expect("claim")
        );
    }
    let joined = accept(pool, &mailed, mailer).await.expect("accept");
    assert_eq!(joined.redemption_id, Some(mailer_redeemed.redemption.id));
    assert_eq!(
        joined.claimed_signup_invite_id,
        mailer_redeemed.redemption.signup_invite_id
    );
    assert_eq!(use_count(pool, mailed_id).await, 1, "no second use");
    assert_eq!(
        redemption_counts(pool, cord.id, mailed_id).await,
        (1, 1),
        "the reserved use went to its newcomer"
    );
}

/// A demoted minter closes the link. Returns the minter and the link.
async fn scenario_a_demoted_minter_closes_the_link(pool: &Pool, cord: &CordRow) -> (Uuid, String) {
    let demoted = seed_admin(pool, "demoted").await;
    let (demoted_link, _) = mint(pool, cord.id, demoted, Some(3600), Some(3), true).await;
    redeem(pool, &demoted_link, "before").await.expect("redeem");
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE role_grant SET revoked_at = NOW() WHERE actor_id = $1 AND role = $2",
                &[&demoted, &ROLE_ADMIN],
            )
            .await
            .expect("demote");
    }
    assert!(matches!(
        redeem(pool, &demoted_link, "after").await,
        Err(CordInviteError::NewcomersClosed)
    ));
    // a closed link still answers a waiting username, whose accept it would take
    assert!(
        !redeem(pool, &demoted_link, "before")
            .await
            .expect("a waiting username on a closed link")
            .created
    );
    (demoted, demoted_link)
}

async fn scenario_a_scoped_admin_grant_is_no_authority(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
    demoted: Uuid,
    demoted_link: &str,
) {
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "INSERT INTO role_grant (actor_id, role, scope_kind, scope_id)
                 VALUES ($1, $2, 'cord', $3)",
                &[&demoted, &ROLE_ADMIN, &cord.id],
            )
            .await
            .expect("a scoped admin, written directly");
        for (actor, holds) in [(demoted, false), (admin, true)] {
            assert_eq!(
                query_actor_holds_global_admin(&client, actor)
                    .await
                    .expect("read"),
                holds
            );
            assert_eq!(
                query_actor_holds_global_admin_unlocked(&client, actor)
                    .await
                    .expect("read"),
                holds
            );
        }
    }
    assert!(matches!(
        redeem(pool, demoted_link, "after").await,
        Err(CordInviteError::NewcomersClosed)
    ));
}

async fn scenario_a_soft_deleted_minter_closes_the_link(pool: &Pool, cord: &CordRow) {
    let departed = seed_admin(pool, "departed").await;
    let (departed_link, _) = mint(pool, cord.id, departed, Some(3600), Some(3), true).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE account SET deleted_at = NOW()
                 WHERE id = (SELECT account_id FROM actor WHERE id = $1)",
                &[&departed],
            )
            .await
            .expect("soft-delete the minter");
    }
    assert!(matches!(
        redeem(pool, &departed_link, "someone").await,
        Err(CordInviteError::NewcomersClosed)
    ));
}

/// A purged minter (a null `created_by`) fails closed.
async fn scenario_a_purged_minter_fails_closed(pool: &Pool, cord: &CordRow) {
    let purged = seed_admin(pool, "purged").await;
    let (purged_link, purged_id) = mint(pool, cord.id, purged, Some(3600), Some(3), true).await;
    redeem(pool, &purged_link, "kept").await.expect("redeem");
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute("DELETE FROM actor WHERE id = $1", &[&purged])
            .await
            .expect("purge the minter");
        let created_by: Option<Uuid> = client
            .query_one(
                "SELECT created_by FROM cord_invite WHERE id = $1",
                &[&purged_id],
            )
            .await
            .expect("row")
            .get(0);
        assert_eq!(created_by, None);
    }
    assert!(matches!(
        redeem(pool, &purged_link, "someone").await,
        Err(CordInviteError::NewcomersClosed)
    ));
    assert!(
        !redeem(pool, &purged_link, "kept")
            .await
            .expect("a waiting username outlives its minter")
            .created
    );
    assert_eq!(use_count(pool, purged_id).await, 1);
}

/// How the minter loses its authority in
/// [`scenario_a_removal_of_the_minter_mid_redeem`].
#[derive(Debug, Clone, Copy)]
enum MinterRemoval {
    /// An account delete: the last-admin guard, then the tombstones and the
    /// invite cascade, as the spine's `account_delete` writes them.
    Delete,
    /// A revoke of the minter's global `admin`: the revokee's account lock,
    /// the last-admin guard, then the revoke, as the spine's
    /// `role_grant_revoke` writes them.
    Revoke,
    /// A purge: every admin's account row, the last-admin guard, the
    /// account's `FOR UPDATE`, the invite void, then the delete, whose
    /// cascade sets the link's `created_by` null: the steps of the spine's
    /// `account_purge` that reach the minter and the link.
    Purge,
}

/// A redemption racing a removal of its minter, on real row locks: the
/// redemption waits on the minter's account row the removal holds, then is
/// refused as a closed link, and the two never deadlock.
///
/// The removal takes the minter's account row first and its `admin` grant
/// last (the last-admin guard locks account, actor, grant), so the
/// redemption takes the minter's account row before it reads the grant under
/// its share lock: the other order would hold the grant while waiting on the
/// account the removal holds while it waits on the grant. A purge's cascade
/// also writes the link's row (its `created_by` goes null), so the
/// redemption takes the account row before the link's too. The replay stalls
/// the removal between the two (a holder of the minter's actor row, which the
/// guard locks between them), starts the redemption, then releases the stall.
async fn scenario_a_removal_of_the_minter_mid_redeem(
    pool: &Pool,
    cord: &CordRow,
    removal: MinterRemoval,
) {
    let username = match removal {
        MinterRemoval::Delete => "deleted_mid_redeem",
        MinterRemoval::Revoke => "revoked_mid_redeem",
        MinterRemoval::Purge => "purged_mid_redeem",
    };
    let minter = seed_admin(pool, username).await;
    let minter_account = account_of(pool, minter).await;
    let (link, link_id) = mint(pool, cord.id, minter, Some(3600), Some(3), true).await;
    let grant_id: Uuid = {
        let client = pool.get().await.expect("pool.get");
        client
            .query_one(
                "SELECT id FROM role_grant
                 WHERE actor_id = $1 AND role = $2 AND scope_id IS NULL",
                &[&minter, &ROLE_ADMIN],
            )
            .await
            .expect("the admin grant")
            .get(0)
    };
    let waiting_before = lock_waiters(pool).await;

    // the stall: the guard locks the minter's actor row after its account row
    let mut stall_client = pool.get().await.expect("pool.get");
    let stall = stall_client.transaction().await.expect("begin");
    stall
        .execute(
            "SELECT 1 FROM actor WHERE id = $1 FOR NO KEY UPDATE",
            &[&minter],
        )
        .await
        .expect("hold the minter's actor");

    let removing = tokio::spawn({
        let pool = pool.clone();
        async move {
            let mut client = pool.get().await.expect("pool.get");
            let tx = client.transaction().await.expect("begin");
            let exclude = match removal {
                MinterRemoval::Delete => None,
                MinterRemoval::Purge => {
                    query_account_lock_admins_for_purge(&tx, &minter_account)
                        .await
                        .expect("lock the admins for purge");
                    None
                }
                MinterRemoval::Revoke => {
                    query_account_lock_for_revoke(&tx, &minter)
                        .await
                        .expect("lock for revoke");
                    Some(grant_id)
                }
            };
            query_count_active_accounts_with_global_role_locked(&tx, ROLE_ADMIN, exclude.as_ref())
                .await
                .expect("the last-admin guard");
            match removal {
                MinterRemoval::Delete => {
                    query_account_soft_delete(&tx, &minter_account, None)
                        .await
                        .expect("soft-delete the account")
                        .expect("a live account");
                    assert!(
                        query_actor_soft_delete(&tx, &minter, None)
                            .await
                            .expect("soft-delete the actor")
                    );
                    query_invite_delete_unclaimed_by_creators(&tx, &[minter])
                        .await
                        .expect("the invite cascade");
                }
                MinterRemoval::Revoke => {
                    query_revoke_role_grant(&tx, grant_id, minter, None, None)
                        .await
                        .expect("revoke")
                        .expect("a live grant");
                }
                MinterRemoval::Purge => {
                    assert!(
                        query_account_lock_for_purge(&tx, &minter_account)
                            .await
                            .expect("lock for purge")
                    );
                    query_invite_delete_unclaimed_by_creators(&tx, &[minter])
                        .await
                        .expect("the invite void");
                    query_purge_account(&tx, &minter_account)
                        .await
                        .expect("purge the account")
                        .expect("an account");
                }
            }
            tx.commit().await.expect("commit the removal");
        }
    });
    await_lock_waiters(
        pool,
        waiting_before + 1,
        &format!("{removal:?}: the removal stalls at the minter's actor"),
    )
    .await;

    let redeeming = tokio::spawn({
        let pool = pool.clone();
        async move { redeem(&pool, &link, "newcomer_mid_removal").await }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock_waiters(pool).await < waiting_before + 2 {
        assert!(
            !redeeming.is_finished(),
            "{removal:?}: the redemption waits on the minter's account row, \
             but finished first: {:?}",
            redeeming.await
        );
        assert!(
            Instant::now() < deadline,
            "{removal:?}: the redemption waits"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    stall.commit().await.expect("release the stall");
    removing.await.expect("the removal, no deadlock");
    let redeemed = redeeming.await.expect("the redemption task");
    assert!(
        matches!(redeemed, Err(CordInviteError::NewcomersClosed)),
        "{removal:?}: the redemption that waited is refused as a closed link: {redeemed:?}"
    );
    assert_eq!(
        use_count(pool, link_id).await,
        0,
        "{removal:?}: no use spent"
    );
    assert_eq!(
        count(
            pool,
            "SELECT COUNT(*) FROM invite WHERE username = 'newcomer_mid_removal'"
        )
        .await,
        0,
        "{removal:?}: no reservation"
    );
}

/// Where [`scenario_a_purge_of_the_minter_mid_accept`] holds the accept.
#[derive(Debug, Clone, Copy)]
enum AcceptStall {
    /// A holder of the cord's row.
    Cord,
    /// A holder of the link's row.
    Link,
}

/// How the newcomer in [`scenario_a_purge_of_the_minter_mid_accept`] signed
/// up.
#[derive(Debug, Clone, Copy)]
enum NewcomerSignup {
    /// Their signup claimed the redemption's spine invite.
    Claimed,
    /// Their signup claimed none (open signup), so accept finds the
    /// redemption by username and locks it with its unclaimed invite, the
    /// invite the purge deletes among the minter's unclaimed ones.
    Unclaimed,
}

/// Wait until `n` backends of this database wait on a lock, or `task` has
/// finished without waiting.
async fn await_lock_waiters_or_finished<T>(
    pool: &Pool,
    n: i64,
    task: &tokio::task::JoinHandle<T>,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !task.is_finished() && lock_waiters(pool).await < n {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// An accept racing a purge of the link's minter, on real row locks: the
/// purge deletes the minter's unclaimed invites (one may be the redemption's,
/// which a fallback accept locks with its redemption), its cascade deletes
/// the minter's actor, which the accept's grant names as its `granted_by`,
/// and sets the link's `created_by` null, writing the link's row. Accept
/// key-shares the minter's account row before the link's lock, and the
/// purge takes that row `FOR UPDATE` before any of those writes, so one waits
/// for the other and neither is refused: both commit, and the accept joins.
///
/// The replay holds the accept at `stall`, runs the purge until it waits or
/// finishes, then releases the stall.
async fn scenario_a_purge_of_the_minter_mid_accept(
    pool: &Pool,
    cord: &CordRow,
    stall: AcceptStall,
    signup: NewcomerSignup,
) {
    let label = format!("{stall:?} {signup:?}");
    let suffix = format!("{stall:?}_{signup:?}").to_lowercase();
    let minter = seed_admin(pool, &format!("purged_{suffix}")).await;
    let minter_account = account_of(pool, minter).await;
    let (link, link_id) = mint(pool, cord.id, minter, Some(3600), Some(3), true).await;
    let username = format!("joining_{suffix}");
    redeem(pool, &link, &username).await.expect("redeem");
    let newcomer = match signup {
        NewcomerSignup::Claimed => sign_up(pool, &username).await,
        NewcomerSignup::Unclaimed => seed_actor(pool, &username).await,
    };
    let waiting_before = lock_waiters(pool).await;

    let mut stall_client = pool.get().await.expect("pool.get");
    let stall_tx = stall_client.transaction().await.expect("begin");
    let (sql, id) = match stall {
        AcceptStall::Cord => (
            "SELECT 1 FROM cell WHERE id = $1 FOR NO KEY UPDATE",
            cord.id,
        ),
        AcceptStall::Link => (
            "SELECT 1 FROM cord_invite WHERE id = $1 FOR NO KEY UPDATE",
            link_id,
        ),
    };
    stall_tx.execute(sql, &[&id]).await.expect("the stall");

    let accepting = tokio::spawn({
        let pool = pool.clone();
        let link = link.clone();
        async move { accept(&pool, &link, newcomer).await }
    });
    await_lock_waiters(
        pool,
        waiting_before + 1,
        &format!("{label}: the accept waits at the stall"),
    )
    .await;

    let purging = tokio::spawn({
        let pool = pool.clone();
        async move {
            let mut client = pool.get().await.expect("pool.get");
            let tx = client.transaction().await?;
            query_account_lock_admins_for_purge(&tx, &minter_account).await?;
            query_count_active_accounts_with_global_role_locked(&tx, ROLE_ADMIN, None).await?;
            purge_account_steps(&tx, minter, minter_account).await?;
            tx.commit().await
        }
    });
    await_lock_waiters_or_finished(
        pool,
        waiting_before + 2,
        &purging,
        &format!("{label}: the purge waits or commits"),
    )
    .await;

    stall_tx.commit().await.expect("release the stall");
    let purged = purging.await.expect("the purge task");
    let accepted = accepting.await.expect("the accept task");
    assert!(purged.is_ok(), "{label}: the purge commits: {purged:?}");
    let accepted = accepted.unwrap_or_else(|e| panic!("{label}: the accept joins: {e:?}"));
    assert!(accepted.role_grant_id.is_some(), "{label}: a new grant");
    let created_by: Option<Uuid> = {
        let client = pool.get().await.expect("pool.get");
        client
            .query_one(
                "SELECT created_by FROM cord_invite WHERE id = $1",
                &[&link_id],
            )
            .await
            .expect("row")
            .get(0)
    };
    assert_eq!(
        created_by, None,
        "{label}: the purge nulled the link's minter"
    );
}

/// Three writers on real row locks, none waiting forever: a purge of a link's
/// minter, which holds every admin's account row from its first statement; a
/// member leaving the cord who is an admin, which holds the cord's row and
/// then waits on its own account row the purge holds; and an accept of the
/// link, which waits on the cord's row. Accept takes the cord's row before
/// its lock on the minter, so it holds nothing the purge waits on: the purge
/// commits, then the leave, then the accept, its link's minter gone.
async fn scenario_a_purge_a_leave_and_an_accept(pool: &Pool, cord: &CordRow) {
    let minter = seed_admin(pool, "purged_three_way").await;
    let minter_account = account_of(pool, minter).await;
    let (link, link_id) = mint(pool, cord.id, minter, Some(3600), Some(3), false).await;
    let leaver = seed_admin(pool, "leaving_three_way").await;
    add_member(pool, cord.id, leaver, CordRole::Member).await;
    let joiner = seed_actor(pool, "joining_three_way").await;
    let waiting_before = lock_waiters(pool).await;

    // the purge's first statements, held open
    let mut purge_client = pool.get().await.expect("pool.get");
    let purge = purge_client.transaction().await.expect("begin");
    query_account_lock_admins_for_purge(&purge, &minter_account)
        .await
        .expect("lock the admins for purge");
    query_count_active_accounts_with_global_role_locked(&purge, ROLE_ADMIN, None)
        .await
        .expect("the last-admin guard");

    let leaving = tokio::spawn({
        let pool = pool.clone();
        let cord_id = cord.id;
        async move { in_tx!(pool, |tx| query_cord_leave(&tx, cord_id, leaver).await) }
    });
    await_lock_waiters(
        pool,
        waiting_before + 1,
        "the leave holds the cord and waits on its account",
    )
    .await;
    let accepting = tokio::spawn({
        let pool = pool.clone();
        async move { accept(&pool, &link, joiner).await }
    });
    await_lock_waiters(pool, waiting_before + 2, "the accept waits").await;

    let purged = purge_account_steps(&purge, minter, minter_account).await;
    let purged = match purged {
        Ok(()) => purge.commit().await,
        Err(e) => Err(e),
    };
    let left = leaving.await.expect("the leave task");
    let accepted = accepting.await.expect("the accept task");
    assert!(purged.is_ok(), "the purge commits: {purged:?}");
    assert!(left.is_ok(), "the leave commits: {left:?}");
    let accepted = accepted.expect("the accept joins");
    assert!(accepted.role_grant_id.is_some(), "a new grant");
    assert_eq!(use_count(pool, link_id).await, 1);
}

/// An accept racing a purge of a link's minter who renamed the link's cord,
/// on real row locks: the purge holds the minter's account `FOR UPDATE`, and
/// the accept holds the cord's row while it waits on that account. The
/// cord's row names no actor, so the purge's cascade never writes it: the
/// purge commits, then the accept joins, its link's minter gone.
async fn scenario_a_purge_of_a_minter_who_renamed_the_cord_mid_accept(pool: &Pool) {
    let minter = seed_admin(pool, "renaming_minter").await;
    let minter_account = account_of(pool, minter).await;
    let cord = create_cord(pool, "the renamed porch", minter).await;
    in_tx!(pool, |tx| query_cord_rename(
        &tx,
        cord.id,
        "the porch, renamed",
    )
    .await)
    .expect("rename")
    .expect("the cord exists");
    let (link, link_id) = mint(pool, cord.id, minter, Some(3600), Some(3), false).await;
    let joiner = seed_actor(pool, "joining_renamed").await;
    let waiting_before = lock_waiters(pool).await;

    // the purge's locks up to the minter's account, held open
    let mut purge_client = pool.get().await.expect("pool.get");
    let purge = purge_client.transaction().await.expect("begin");
    query_account_lock_admins_for_purge(&purge, &minter_account)
        .await
        .expect("lock the admins for purge");
    query_count_active_accounts_with_global_role_locked(&purge, ROLE_ADMIN, None)
        .await
        .expect("the last-admin guard");
    assert!(
        query_account_lock_for_purge(&purge, &minter_account)
            .await
            .expect("lock for purge")
    );

    let accepting = tokio::spawn({
        let pool = pool.clone();
        async move { accept(&pool, &link, joiner).await }
    });
    await_lock_waiters(
        pool,
        waiting_before + 1,
        "the accept holds the cord and waits on the minter's account",
    )
    .await;

    let purged = match purge_account_steps(&purge, minter, minter_account).await {
        Ok(()) => purge.commit().await,
        Err(e) => Err(e),
    };
    let accepted = accepting.await.expect("the accept task");
    assert!(purged.is_ok(), "the purge commits: {purged:?}");
    let accepted = accepted.expect("the accept joins");
    assert!(accepted.role_grant_id.is_some(), "a new grant");
    assert_eq!(use_count(pool, link_id).await, 1);
}

/// A fallback accept racing a purge of the newcomer accepting, on real row
/// locks: the newcomer's signup claimed no invite, so the accept claims the
/// redemption's for their account, a write naming the account the purge
/// holds `FOR UPDATE`. Accept claims before its grant names the newcomer's
/// actor, so it holds nothing the purge's delete of that actor waits on: the
/// purge commits, and the accept fails on the account the purge removed,
/// rather than one of the two deadlocking.
async fn scenario_a_purge_of_the_newcomer_mid_fallback_accept(pool: &Pool, cord: &CordRow) {
    let minter = seed_admin(pool, "minting_for_the_purged").await;
    let (link, link_id) = mint(pool, cord.id, minter, Some(3600), Some(3), true).await;
    redeem(pool, &link, "purged_mid_accept_newcomer")
        .await
        .expect("redeem");
    // open signup: an account with the username, the invite left unclaimed
    let newcomer = seed_actor(pool, "purged_mid_accept_newcomer").await;
    let newcomer_account = account_of(pool, newcomer).await;
    let waiting_before = lock_waiters(pool).await;

    // the purge's locks up to the newcomer's account, held open
    let mut purge_client = pool.get().await.expect("pool.get");
    let purge = purge_client.transaction().await.expect("begin");
    query_account_lock_admins_for_purge(&purge, &newcomer_account)
        .await
        .expect("lock the target for purge");
    assert!(
        query_account_lock_for_purge(&purge, &newcomer_account)
            .await
            .expect("lock for purge")
    );

    let accepting = tokio::spawn({
        let pool = pool.clone();
        async move { accept(&pool, &link, newcomer).await }
    });
    await_lock_waiters(
        pool,
        waiting_before + 1,
        "the accept waits on the newcomer's account",
    )
    .await;

    let purged = match purge_account_steps(&purge, newcomer, newcomer_account).await {
        Ok(()) => purge.commit().await,
        Err(e) => Err(e),
    };
    let accepted = accepting.await.expect("the accept task");
    assert!(purged.is_ok(), "the purge commits: {purged:?}");
    let Err(CordInviteError::Db(CellError::Db(e))) = &accepted else {
        panic!("the accept fails on the removed account: {accepted:?}");
    };
    assert_eq!(
        e.code(),
        Some(&SqlState::FOREIGN_KEY_VIOLATION),
        "the accept fails on the removed account, not a deadlock: {e:?}"
    );
    assert_eq!(use_count(pool, link_id).await, 1, "only the redeem's use");
}

/// A closed or expired link never says whether a username is free.
async fn scenario_dead_links_never_say_whether_a_username_is_free(
    pool: &Pool,
    cord: &CordRow,
    admin: Uuid,
) {
    let grace = i64::from(CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS);
    // someone else's waiting reservation, through another link
    let (holding, _) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    redeem(pool, &holding, "holder").await.expect("redeem");
    let closer = seed_admin(pool, "closer").await;
    let (closed, _) = mint(pool, cord.id, closer, Some(3600), Some(3), true).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE role_grant SET revoked_at = NOW() WHERE actor_id = $1 AND role = $2",
                &[&closer, &ROLE_ADMIN],
            )
            .await
            .expect("demote");
    }
    let (in_grace, in_grace_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    backdate(pool, in_grace_id, 1).await;
    let (past_grace, past_grace_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    backdate(pool, past_grace_id, grace + 1).await;
    let (revoked, revoked_id) = mint(pool, cord.id, admin, Some(3600), Some(3), true).await;
    {
        let client = pool.get().await.expect("pool.get");
        query_cord_invite_revoke(&client, cord.id, revoked_id)
            .await
            .expect("revoke")
            .expect("exists");
    }
    // a live account's name, a soft-deleted one's, a reservation, and a free
    // name all get the link's own refusal, never `UsernameUnavailable`
    for username in ["admin", "taken", "holder", "unheard"] {
        assert!(
            matches!(
                redeem(pool, &closed, username).await,
                Err(CordInviteError::NewcomersClosed)
            ),
            "closed: {username}"
        );
        assert!(
            matches!(
                redeem(pool, &in_grace, username).await,
                Err(CordInviteError::Expired)
            ),
            "expired within the grace: {username}"
        );
        assert!(
            matches!(
                redeem(pool, &past_grace, username).await,
                Err(CordInviteError::Expired)
            ),
            "expired past the grace: {username}"
        );
        assert!(
            matches!(
                redeem(pool, &revoked, username).await,
                Err(CordInviteError::Revoked)
            ),
            "revoked: {username}"
        );
    }
}

async fn scenario_redemptions_never_overshoot_max_uses(pool: &Pool, cord: &CordRow, admin: Uuid) {
    let max_uses = 3;
    let (busy, busy_id) = mint(pool, cord.id, admin, Some(3600), Some(max_uses), true).await;
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let pool = pool.clone();
            let token = busy.clone();
            tokio::spawn(
                async move { redeem(&pool, &token, &format!("rush{i}")).await.map(|_| ()) },
            )
        })
        .collect();
    let mut redeemed = 0;
    let mut exhausted = 0;
    for handle in handles {
        match handle.await.expect("join") {
            Ok(()) => redeemed += 1,
            Err(CordInviteError::Exhausted) => exhausted += 1,
            Err(e) => panic!("unexpected: {e:?}"),
        }
    }
    assert_eq!((redeemed, exhausted), (3, 7));
    assert_eq!(use_count(pool, busy_id).await, 3);
    assert_eq!(
        count(
            pool,
            "SELECT COUNT(*) FROM invite WHERE username LIKE 'rush%'"
        )
        .await,
        3,
        "one spine invite per redemption"
    );

    // the same username at once: one reservation, one use
    let (same, same_id) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let pool = pool.clone();
            let token = same.clone();
            tokio::spawn(async move { redeem(&pool, &token, "twin").await })
        })
        .collect();
    let mut created = 0;
    for handle in handles {
        if handle.await.expect("join").expect("redeem").created {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    assert_eq!(use_count(pool, same_id).await, 1);
}

/// The expiry sweep releases what expired links still reserve.
async fn scenario_the_expiry_sweep(pool: &Pool, cord: &CordRow, admin: Uuid) {
    let grace = i64::from(CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS);
    let audit = Arc::new(AuditEmitter::new(pool.clone(), Arc::default()));
    // expired past the grace: two waiting, one signed up but not accepted,
    // one accepted
    let (swept, swept_id) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    let gone = redeem(pool, &swept, "gone").await.expect("redeem");
    let lost = redeem(pool, &swept, "lost").await.expect("redeem");
    redeem(pool, &swept, "joiner").await.expect("redeem");
    redeem(pool, &swept, "settled").await.expect("redeem");
    sign_up(pool, "joiner").await;
    let settled = sign_up(pool, "settled").await;
    accept(pool, &swept, settled).await.expect("accept");
    backdate(pool, swept_id, grace + 1).await;
    // expired within the grace, and live
    let (graced, graced_id) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    redeem(pool, &graced, "graced").await.expect("redeem");
    backdate(pool, graced_id, grace - 60).await;
    let (live, _) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    redeem(pool, &live, "waiting").await.expect("redeem");
    // expired past the grace, but busy when the pass runs
    let (busy_link, busy_link_id) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    let held = redeem(pool, &busy_link, "held").await.expect("redeem");
    backdate(pool, busy_link_id, grace + 1).await;

    let released = {
        let mut client = pool.get().await.expect("pool.get");
        let tx = client.transaction().await.expect("begin");
        tx.execute(
            "SELECT 1 FROM cord_invite WHERE id = $1 FOR NO KEY UPDATE",
            &[&busy_link_id],
        )
        .await
        .expect("hold the busy link's lock");
        // a pass that waits on the lock instead of skipping it would hang
        let released = tokio::time::timeout(
            Duration::from_secs(10),
            cord_invite_sweep_pass(pool, &audit),
        )
        .await
        .expect("the pass skips a busy link");
        tx.rollback().await.expect("rollback");
        released
    };
    let gone_invite = gone.redemption.signup_invite_id.expect("invite");
    let lost_invite = lost.redemption.signup_invite_id.expect("invite");
    assert_eq!(
        released,
        [CordInviteExpireReleased {
            cord_id: cord.id,
            invite_id: swept_id,
            released_signup_invite_ids: vec![gone_invite, lost_invite],
        }],
        "only the waiting reservations of the link expired past the grace, the busy one skipped"
    );
    for username in ["gone", "lost"] {
        assert!(
            !signup_admits(pool, username).await,
            "the stock signup refuses the released {username}"
        );
    }
    // the untouched: within the grace, live (the demoted minter's too), busy
    for username in ["graced", "waiting", "before", "held"] {
        assert!(signup_admits(pool, username).await, "{username}");
    }
    assert_eq!(
        redemption_list(pool, swept_id)
            .await
            .iter()
            .map(|r| (
                r.username.as_str(),
                r.signup_invite_id.is_some(),
                r.accepted_by
            ))
            .collect::<Vec<_>>(),
        [
            ("gone", false, None),
            ("lost", false, None),
            ("joiner", true, None),
            ("settled", true, Some(settled)),
        ],
        "the redemptions stay as the record; claimed and accepted ones keep their invites"
    );
    // audited with no actor, account, or IP
    {
        let client = pool.get().await.expect("pool.get");
        let rows = client
            .query(
                "SELECT actor_id, account_id, ip, metadata::text FROM audit_log
                 WHERE event_type = 'cord_invite_expire'",
                &[],
            )
            .await
            .expect("audit rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<_, Option<Uuid>>(0), None);
        assert_eq!(rows[0].get::<_, Option<Uuid>>(1), None);
        assert_eq!(rows[0].get::<_, Option<String>>(2), None);
        let metadata: serde_json::Value =
            serde_json::from_str(&rows[0].get::<_, String>(3)).expect("metadata");
        assert_eq!(
            metadata,
            serde_json::json!({
                "cord_id": cord.id,
                "invite_id": swept_id,
                "released_signup_invite_ids": [gone_invite, lost_invite],
            })
        );
    }
    // the busy link waits for the next pass
    let released = cord_invite_sweep_pass(pool, &audit).await;
    assert_eq!(
        released
            .iter()
            .map(|r| (r.invite_id, r.released_signup_invite_ids.clone()))
            .collect::<Vec<_>>(),
        [(
            busy_link_id,
            vec![held.redemption.signup_invite_id.expect("invite")]
        )]
    );
    assert!(!signup_admits(pool, "held").await);
    // then nothing is left to release
    assert!(cord_invite_sweep_pass(pool, &audit).await.is_empty());
    assert_eq!(
        count(
            pool,
            "SELECT COUNT(*) FROM audit_log WHERE event_type = 'cord_invite_expire'"
        )
        .await,
        2
    );
    // the late signups the sweep released the usernames of get no reservation
    let gone_actor = seed_actor(pool, "gone").await;
    assert!(matches!(
        accept(pool, &swept, gone_actor).await,
        Err(CordInviteError::Expired)
    ));
}

/// A `ConnectionCloser` recording every close it is asked for, by target.
#[derive(Default)]
struct RecordingCloser {
    closed: std::sync::Mutex<Vec<ConnectionCloseTarget>>,
}

impl RecordingCloser {
    fn record(&self, target: ConnectionCloseTarget) -> usize {
        self.closed.lock().expect("lock").push(target);
        0
    }
}

impl ConnectionCloser for RecordingCloser {
    fn close_connections(&self, target: &ConnectionCloseTarget) -> usize {
        self.record(target.clone())
    }
}

impl fuz_http::ShutdownCloser for RecordingCloser {
    fn close_all_connections(&self) -> usize {
        0
    }
    fn wait_closed(&self) -> fuz_http::ShutdownFuture<'_> {
        Box::pin(std::future::ready(()))
    }
}

/// The upkeep tasks' startup passes run both sweeps, the invite expiry sweep
/// and the spine's auth cleanup, through the emitter and the closer they're
/// handed (the auth cleanup closes the expired session it deletes, and not
/// the account's live one), and cancelling the token ends them.
async fn scenario_the_upkeep_tasks(pool: &Pool, cord: &CordRow, admin: Uuid) {
    const TIMEOUT: Duration = Duration::from_secs(30);
    // what the invite sweep releases: a link expired past the grace
    let grace = i64::from(CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS);
    let (link, link_id) = mint(pool, cord.id, admin, Some(3600), Some(5), true).await;
    redeem(pool, &link, "unkept").await.expect("redeem");
    backdate(pool, link_id, grace + 1).await;
    assert!(signup_admits(pool, "unkept").await);
    // what the auth cleanup audits: a role-grant offer already expired
    let recipient = seed_actor(pool, "upkeep_recipient").await;
    let offer = {
        let client = pool.get().await.expect("pool.get");
        query_role_grant_offer_create(
            &client,
            CreateRoleGrantOfferInput {
                from_actor_id: admin,
                to_account_id: account_of(pool, recipient).await,
                to_actor_id: None,
                role: "upkeep_role".to_owned(),
                scope_kind: None,
                scope_id: None,
                message: None,
                expires_at: std::time::SystemTime::now() - Duration::from_secs(3600),
            },
        )
        .await
        .expect("create offer")
    };

    // what the auth cleanup deletes and closes: a session already expired,
    // beside a live one on the same account
    pool.get()
        .await
        .expect("pool.get")
        .execute(
            "INSERT INTO auth_session (id, account_id, expires_at) VALUES
               ('upkeep-expired', $1, NOW() - INTERVAL '1 hour'),
               ('upkeep-live', $1, NOW() + INTERVAL '1 hour')",
            &[&account_of(pool, recipient).await],
        )
        .await
        .expect("seed sessions");

    let audit = Arc::new(AuditEmitter::new(pool.clone(), Arc::default()));
    let (tx, mut audited) = tokio::sync::mpsc::unbounded_channel::<Arc<AuditLogEvent>>();
    audit.add_listener(Arc::new(move |event| {
        // a send after the scenario's last `recv` has nowhere to go
        let _ = tx.send(event);
        Box::pin(async {})
    }));
    let recording = Arc::new(RecordingCloser::default());
    let connection_closer: Arc<dyn ConnectionCloser> = Arc::<RecordingCloser>::clone(&recording);
    let shutdown = fuz_http::CancellationToken::new();
    let tasks = spawn_upkeep(
        upkeep_options(fuz_auth::AuthCleanupOptions {
            pool: pool.clone(),
            audit: Arc::clone(&audit),
            connection_closer,
        }),
        &shutdown,
    );
    assert_eq!(tasks.len(), 2, "the auth cleanup and the invite sweep");

    // each task's startup pass audits what it swept, in either order
    let (mut invite_expired, mut offer_expired) = (None, None);
    while invite_expired.is_none() || offer_expired.is_none() {
        let event = tokio::time::timeout(TIMEOUT, audited.recv())
            .await
            .expect("both startup passes audit what they swept")
            .expect("the listener is registered");
        let metadata = event.metadata.clone().expect("metadata");
        if event.event_type == CORD_INVITE_EXPIRE_AUDIT_EVENT
            && metadata["invite_id"] == serde_json::json!(link_id)
        {
            invite_expired = Some(metadata["invite_id"].clone());
        } else if event.event_type == "role_grant_offer_expire"
            && metadata["offer_id"] == serde_json::json!(offer.id)
        {
            offer_expired = Some(metadata["offer_id"].clone());
        }
    }
    assert_eq!(invite_expired, Some(serde_json::json!(link_id)));
    assert_eq!(offer_expired, Some(serde_json::json!(offer.id)));
    // the auth cleanup sweeps sessions before it audits offers, so its
    // session close has run by the time the offer's audit arrives
    assert_eq!(
        *recording.closed.lock().expect("lock"),
        [ConnectionCloseTarget::Session("upkeep-expired".to_owned())],
        "the auth cleanup closes the expired session it deleted, and only that"
    );
    assert!(!signup_admits(pool, "unkept").await);
    assert!(tasks.iter().all(|task| !task.is_finished()));

    shutdown.cancel();
    for task in tasks {
        tokio::time::timeout(TIMEOUT, task)
            .await
            .expect("cancelling the token ends each task")
            .expect("the task ends cleanly");
    }
}
