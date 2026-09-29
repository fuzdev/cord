//! Postgres-backed tests for newcomer links
//! (`cord_server::cord_invite_redemption_queries`, and accept after a
//! redemption in `cord_server::cord_invite_queries`).
//!
//! What only the database can show: a redemption creates the spine's
//! username invite in the minter's name, records the redemption, and spends
//! one use; redeeming the same username again changes nothing while the
//! link is live, and answers as a fresh redemption once it isn't; a taken or
//! reserved username is refused; the minter's global `admin` is re-read at
//! every redemption, so a demoted, deleted, or purged minter closes the
//! link; concurrent redemptions never spend more uses than the link has;
//! the newcomer's accept after signing up spends nothing, even from an
//! exhausted link or within the grace after expiry, and finds the
//! redemption by username when their signup claimed no invite; and a
//! revoke deletes the waiting reservations, so the stock signup no longer
//! admits them, as does the expiry sweep's pass
//! (`cord_server::cord_invite_sweep`) for links expired past the grace,
//! skipping a link another transaction holds.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and its `CORD_SCHEMA_READY_TEST_URL` override). It resets the `public`
//! schema, so the scenarios run in one test function:
//!
//! ```bash
//! cargo test -p cord_server --test cord_newcomers -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::{fresh_db_pool, redemption_list, seed_actor};
use cord_server::cord_authz::CordRole;
use cord_server::cord_invite_queries::{
    CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS, CordInviteAcceptError, CordInviteAccepted,
    CordInviteStatus, generate_cord_invite_token, hash_cord_invite_token, query_cord_invite_accept,
    query_cord_invite_by_token_hash, query_cord_invite_create, query_cord_invite_revoke,
};
use cord_server::cord_invite_redemption_queries::{
    CordInviteRedeemError, CordInviteRedeemed, query_actor_holds_global_admin,
    query_actor_holds_global_admin_unlocked, query_cord_invite_redeem,
};
use cord_server::cord_invite_sweep::{CordInviteExpireReleased, cord_invite_sweep_pass};
use cord_server::cord_queries::query_cord_create;
use fuz_auth::invite_queries::{
    query_create_invite, query_invite_claim_unscoped, query_invite_find_unclaimed_match_for_update,
};
use fuz_auth::{AuditEmitter, ROLE_ADMIN, query_create_role_grant};
use fuz_db::Pool;
use uuid::Uuid;

/// Redeem in its own transaction, as the action does.
async fn redeem(
    pool: &Pool,
    token: &str,
    username: &str,
) -> Result<CordInviteRedeemed, CordInviteRedeemError> {
    let mut client = pool.get().await.expect("pool.get");
    let tx = client.transaction().await.expect("begin");
    let result = query_cord_invite_redeem(&tx, &hash_cord_invite_token(token), username).await;
    if result.is_ok() {
        tx.commit().await.expect("commit");
    } else {
        tx.rollback().await.expect("rollback");
    }
    result
}

/// Accept in its own transaction, as the action does.
async fn accept(
    pool: &Pool,
    token: &str,
    actor_id: Uuid,
) -> Result<CordInviteAccepted, CordInviteAcceptError> {
    let mut client = pool.get().await.expect("pool.get");
    let tx = client.transaction().await.expect("begin");
    let result = query_cord_invite_accept(&tx, &hash_cord_invite_token(token), actor_id).await;
    if result.is_ok() {
        tx.commit().await.expect("commit");
    } else {
        tx.rollback().await.expect("rollback");
    }
    result
}

/// Mint a link, returning its raw token and id.
async fn mint(
    pool: &Pool,
    cord_id: Uuid,
    by: Uuid,
    max_uses: u32,
    admits_newcomers: bool,
) -> (String, Uuid) {
    let client = pool.get().await.expect("pool.get");
    let minted = generate_cord_invite_token();
    let row = query_cord_invite_create(
        &client,
        cord_id,
        &minted.token_hash,
        by,
        Some(3600),
        Some(max_uses),
        admits_newcomers,
    )
    .await
    .expect("create invite");
    (minted.token, row.id)
}

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

async fn status_of(pool: &Pool, token: &str) -> CordInviteStatus {
    let client = pool.get().await.expect("pool.get");
    query_cord_invite_by_token_hash(&client, &hash_cord_invite_token(token))
        .await
        .expect("read")
        .expect("exists")
        .status()
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn newcomer_links_admit_new_people() {
    let pool = fresh_db_pool().await;
    let admin = seed_admin(&pool, "admin").await;
    let cord = {
        let client = pool.get().await.expect("pool.get");
        query_cord_create(&client, "the porch", admin)
            .await
            .expect("create")
            .cord
    };

    // -- a redemption: the spine invite, the row, one use ---------------------
    let (token, link_id) = mint(&pool, cord.id, admin, 2, true).await;
    let redeemed = redeem(&pool, &token, "newcomer").await.expect("redeem");
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
    assert_eq!(use_count(&pool, link_id).await, 1);

    // -- the same username again is idempotent --------------------------------
    let again = redeem(&pool, &token, "newcomer")
        .await
        .expect("redeem again");
    assert!(!again.created);
    assert_eq!(again.redemption, redeemed.redemption);
    assert_eq!(use_count(&pool, link_id).await, 1, "no new use");
    assert_eq!(
        count(&pool, "SELECT COUNT(*) FROM invite").await,
        1,
        "no new invite"
    );

    // -- taken or reserved usernames ------------------------------------------
    seed_actor(&pool, "taken").await;
    {
        let client = pool.get().await.expect("pool.get");
        query_create_invite(&client, None, Some("reserved"), Some(&admin))
            .await
            .expect("an admin's own invite");
        query_create_invite(&client, Some("both@example.com"), Some("paired"), None)
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
                redeem(&pool, &token, username).await,
                Err(CordInviteRedeemError::UsernameUnavailable)
            ),
            "{username}"
        );
    }
    assert_eq!(
        use_count(&pool, link_id).await,
        1,
        "a refusal spends nothing"
    );

    // -- an ordinary link is for existing accounts ----------------------------
    let (ordinary, _) = mint(&pool, cord.id, admin, 5, false).await;
    assert!(matches!(
        redeem(&pool, &ordinary, "someone").await,
        Err(CordInviteRedeemError::NotForNewcomers)
    ));
    assert!(matches!(
        redeem(&pool, "not a real token", "someone").await,
        Err(CordInviteRedeemError::NotFound)
    ));

    // -- accept after redemption spends nothing, even when exhausted ----------
    redeem(&pool, &token, "second").await.expect("the last use");
    assert_eq!(status_of(&pool, &token).await, CordInviteStatus::Exhausted);
    assert!(matches!(
        redeem(&pool, &token, "third").await,
        Err(CordInviteRedeemError::Exhausted)
    ));
    // still idempotent once exhausted: the reservation stands
    assert!(
        !redeem(&pool, &token, "second")
            .await
            .expect("idempotent")
            .created
    );
    let newcomer = sign_up(&pool, "newcomer").await;
    let joined = accept(&pool, &token, newcomer).await.expect("accept");
    assert_eq!(joined.role, CordRole::Member);
    assert!(joined.role_grant_id.is_some());
    assert_eq!(joined.redemption_id, Some(redeemed.redemption.id));
    assert_eq!(
        joined.claimed_signup_invite_id, None,
        "the signup claimed the invite, not the accept"
    );
    assert_eq!(use_count(&pool, link_id).await, 2, "accept spent nothing");
    {
        let redemptions = redemption_list(&pool, link_id).await;
        assert_eq!(redemptions.len(), 2);
        assert_eq!(redemptions[0].accepted_by, Some(newcomer));
        assert!(redemptions[0].accepted_at.is_some());
        assert_eq!(redemptions[1].accepted_by, None);
        let client = pool.get().await.expect("pool.get");
        let link = query_cord_invite_by_token_hash(&client, &hash_cord_invite_token(&token))
            .await
            .expect("read")
            .expect("exists");
        assert_eq!(
            (link.redemption_count, link.redemption_accepted_count),
            (2, 1)
        );
    }
    // accepting again changes nothing
    let again = accept(&pool, &token, newcomer).await.expect("accept again");
    assert_eq!((again.role_grant_id, again.redemption_id), (None, None));
    // someone who didn't redeem it can't use the exhausted link
    let existing = seed_actor(&pool, "existing").await;
    assert!(matches!(
        accept(&pool, &token, existing).await,
        Err(CordInviteAcceptError::Exhausted)
    ));
    // a redeemed username that signs up after exhaustion still joins
    let second = sign_up(&pool, "second").await;
    let joined = accept(&pool, &token, second).await.expect("accept");
    assert!(joined.redemption_id.is_some());
    assert_eq!(use_count(&pool, link_id).await, 2);

    // -- an existing account accepts a newcomer link normally -----------------
    let (open_link, open_id) = mint(&pool, cord.id, admin, 3, true).await;
    let joined = accept(&pool, &open_link, existing).await.expect("accept");
    assert_eq!(joined.redemption_id, None);
    assert_eq!(
        use_count(&pool, open_id).await,
        1,
        "an ordinary accept spends a use"
    );

    // -- a newcomer who got in another way completes their redemption --------
    redeem(&pool, &open_link, "early").await.expect("redeem");
    let early = sign_up(&pool, "early").await;
    accept(&pool, &ordinary, early)
        .await
        .expect("in through the ordinary link");
    let already = accept(&pool, &open_link, early).await.expect("accept");
    assert_eq!(already.role_grant_id, None, "already a member");
    assert!(
        already.redemption_id.is_some(),
        "the redemption is completed"
    );
    assert_eq!(
        use_count(&pool, open_id).await,
        2,
        "only the redemption's use"
    );

    // -- expiry after redemption: accept within the grace ---------------------
    let (expiring, expiring_id) = mint(&pool, cord.id, admin, 3, true).await;
    redeem(&pool, &expiring, "late").await.expect("redeem");
    redeem(&pool, &expiring, "tardy").await.expect("redeem");
    backdate(&pool, expiring_id, 1).await;
    assert!(matches!(
        redeem(&pool, &expiring, "later").await,
        Err(CordInviteRedeemError::Expired)
    ));
    // a dead link answers a waiting username as any other
    assert!(matches!(
        redeem(&pool, &expiring, "late").await,
        Err(CordInviteRedeemError::Expired)
    ));
    let late = sign_up(&pool, "late").await;
    let joined = accept(&pool, &expiring, late).await.expect("accept");
    assert!(joined.redemption_id.is_some());
    assert_eq!(use_count(&pool, expiring_id).await, 2);
    // past the grace, the ordinary expired refusal
    let grace = i64::from(CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS);
    backdate(&pool, expiring_id, grace - 60).await;
    let tardy = sign_up(&pool, "tardy").await;
    backdate(&pool, expiring_id, grace + 1).await;
    assert!(matches!(
        accept(&pool, &expiring, tardy).await,
        Err(CordInviteAcceptError::Expired)
    ));
    backdate(&pool, expiring_id, grace - 60).await;
    assert!(
        accept(&pool, &expiring, tardy)
            .await
            .expect("within the grace")
            .redemption_id
            .is_some()
    );

    // -- revoke deletes waiting reservations and refuses the accept ----------
    let (revokable, revokable_id) = mint(&pool, cord.id, admin, 3, true).await;
    let cancelled = redeem(&pool, &revokable, "cancelled")
        .await
        .expect("redeem");
    let cancelled_invite = cancelled.redemption.signup_invite_id.expect("invite");
    redeem(&pool, &revokable, "midway").await.expect("redeem");
    let midway = sign_up(&pool, "midway").await;
    assert!(signup_admits(&pool, "cancelled").await);
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
        !signup_admits(&pool, "cancelled").await,
        "the stock signup refuses the released username"
    );
    let redemptions = redemption_list(&pool, revokable_id).await;
    assert_eq!(
        redemptions
            .iter()
            .map(|r| (r.username.as_str(), r.signup_invite_id.is_some()))
            .collect::<Vec<_>>(),
        [("cancelled", false), ("midway", true)],
        "the redemptions stay as the record"
    );
    assert_eq!(use_count(&pool, revokable_id).await, 2);
    assert!(matches!(
        redeem(&pool, &revokable, "cancelled").await,
        Err(CordInviteRedeemError::Revoked)
    ));
    // the newcomer who signed up before the revoke has an account, not the cord
    assert!(matches!(
        accept(&pool, &revokable, midway).await,
        Err(CordInviteAcceptError::Revoked)
    ));
    // someone who takes the released username afterward gets nothing from it
    let taker = seed_actor(&pool, "cancelled").await;
    assert!(matches!(
        accept(&pool, &revokable, taker).await,
        Err(CordInviteAcceptError::Revoked)
    ));

    // -- an admin deleting the signup invite leaves the record ----------------
    let (deletable, deletable_id) = mint(&pool, cord.id, admin, 3, true).await;
    let first = redeem(&pool, &deletable, "dropped").await.expect("redeem");
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
    let redemptions = redemption_list(&pool, deletable_id).await;
    assert_eq!(redemptions.len(), 1, "the redemption stays");
    assert_eq!(redemptions[0].signup_invite_id, None);
    assert_eq!(
        use_count(&pool, deletable_id).await,
        1,
        "its use stays spent"
    );
    let fresh = redeem(&pool, &deletable, "dropped")
        .await
        .expect("a fresh redemption");
    assert!(fresh.created);
    assert_ne!(fresh.redemption.id, first.redemption.id);
    assert_eq!(use_count(&pool, deletable_id).await, 2);

    // -- a signup that claimed no invite (open signup) --------------------------
    let (single, single_id) = mint(&pool, cord.id, admin, 1, true).await;
    let opener_redeemed = redeem(&pool, &single, "opener").await.expect("redeem");
    let opener_invite = opener_redeemed.redemption.signup_invite_id.expect("invite");
    // open signup creates the account and claims nothing
    let opener = seed_actor(&pool, "opener").await;
    // someone else's account doesn't reach the reservation
    let bystander = seed_actor(&pool, "bystander").await;
    assert!(matches!(
        accept(&pool, &single, bystander).await,
        Err(CordInviteAcceptError::Exhausted)
    ));
    let joined = accept(&pool, &single, opener).await.expect("accept");
    assert!(joined.role_grant_id.is_some());
    assert_eq!(joined.redemption_id, Some(opener_redeemed.redemption.id));
    assert_eq!(joined.claimed_signup_invite_id, Some(opener_invite));
    assert_eq!(use_count(&pool, single_id).await, 1, "no second use");
    assert_eq!(
        claimed_by(&pool, opener_invite).await,
        Some(account_of(&pool, opener).await),
        "the accept claimed the reservation"
    );
    assert_eq!(
        redemption_list(&pool, single_id).await[0].accepted_by,
        Some(opener)
    );

    // -- an older email-only invite won the signup's claim ----------------------
    {
        let client = pool.get().await.expect("pool.get");
        query_create_invite(&client, Some("mail@example.com"), None, Some(&admin))
            .await
            .expect("an older email invite");
    }
    let (mailed, mailed_id) = mint(&pool, cord.id, admin, 1, true).await;
    let mailer_redeemed = redeem(&pool, &mailed, "mailer").await.expect("redeem");
    let mailer = seed_actor(&pool, "mailer").await;
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
            query_invite_claim_unscoped(&client, &matched.id, &account_of(&pool, mailer).await)
                .await
                .expect("claim")
        );
    }
    let joined = accept(&pool, &mailed, mailer).await.expect("accept");
    assert_eq!(joined.redemption_id, Some(mailer_redeemed.redemption.id));
    assert_eq!(
        joined.claimed_signup_invite_id,
        mailer_redeemed.redemption.signup_invite_id
    );
    assert_eq!(use_count(&pool, mailed_id).await, 1, "no second use");
    let link = {
        let client = pool.get().await.expect("pool.get");
        query_cord_invite_by_token_hash(&client, &hash_cord_invite_token(&mailed))
            .await
            .expect("read")
            .expect("exists")
    };
    assert_eq!(
        (link.redemption_count, link.redemption_accepted_count),
        (1, 1),
        "the reserved use went to its newcomer"
    );

    // -- a demoted minter closes the link -------------------------------------
    let demoted = seed_admin(&pool, "demoted").await;
    let (demoted_link, _) = mint(&pool, cord.id, demoted, 3, true).await;
    redeem(&pool, &demoted_link, "before")
        .await
        .expect("redeem");
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
        redeem(&pool, &demoted_link, "after").await,
        Err(CordInviteRedeemError::NewcomersClosed)
    ));
    // a closed link answers a waiting username as any other
    assert!(matches!(
        redeem(&pool, &demoted_link, "before").await,
        Err(CordInviteRedeemError::NewcomersClosed)
    ));

    // -- a scoped admin grant is no authority ---------------------------------
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
        redeem(&pool, &demoted_link, "after").await,
        Err(CordInviteRedeemError::NewcomersClosed)
    ));

    // -- a soft-deleted minter closes the link --------------------------------
    let departed = seed_admin(&pool, "departed").await;
    let (departed_link, _) = mint(&pool, cord.id, departed, 3, true).await;
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
        redeem(&pool, &departed_link, "someone").await,
        Err(CordInviteRedeemError::NewcomersClosed)
    ));

    // -- a purged minter (a null created_by) fails closed ---------------------
    let purged = seed_admin(&pool, "purged").await;
    let (purged_link, purged_id) = mint(&pool, cord.id, purged, 3, true).await;
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
        redeem(&pool, &purged_link, "someone").await,
        Err(CordInviteRedeemError::NewcomersClosed)
    ));
    assert_eq!(use_count(&pool, purged_id).await, 0);

    // -- concurrency: redemptions never overshoot max_uses --------------------
    let max_uses = 3;
    let (busy, busy_id) = mint(&pool, cord.id, admin, max_uses, true).await;
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
            Err(CordInviteRedeemError::Exhausted) => exhausted += 1,
            Err(e) => panic!("unexpected: {e:?}"),
        }
    }
    assert_eq!((redeemed, exhausted), (3, 7));
    assert_eq!(use_count(&pool, busy_id).await, 3);
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM invite WHERE username LIKE 'rush%'"
        )
        .await,
        3,
        "one spine invite per redemption"
    );

    // the same username at once: one reservation, one use
    let (same, same_id) = mint(&pool, cord.id, admin, 5, true).await;
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
    assert_eq!(use_count(&pool, same_id).await, 1);

    // -- the expiry sweep releases what expired links still reserve -----------
    let audit = Arc::new(AuditEmitter::new(pool.clone()));
    // expired past the grace: two waiting, one signed up but not accepted,
    // one accepted
    let (swept, swept_id) = mint(&pool, cord.id, admin, 5, true).await;
    let gone = redeem(&pool, &swept, "gone").await.expect("redeem");
    let lost = redeem(&pool, &swept, "lost").await.expect("redeem");
    redeem(&pool, &swept, "joiner").await.expect("redeem");
    redeem(&pool, &swept, "settled").await.expect("redeem");
    sign_up(&pool, "joiner").await;
    let settled = sign_up(&pool, "settled").await;
    accept(&pool, &swept, settled).await.expect("accept");
    backdate(&pool, swept_id, grace + 1).await;
    // expired within the grace, and live
    let (graced, graced_id) = mint(&pool, cord.id, admin, 5, true).await;
    redeem(&pool, &graced, "graced").await.expect("redeem");
    backdate(&pool, graced_id, grace - 60).await;
    let (live, _) = mint(&pool, cord.id, admin, 5, true).await;
    redeem(&pool, &live, "waiting").await.expect("redeem");
    // expired past the grace, but busy when the pass runs
    let (busy_link, busy_link_id) = mint(&pool, cord.id, admin, 5, true).await;
    let held = redeem(&pool, &busy_link, "held").await.expect("redeem");
    backdate(&pool, busy_link_id, grace + 1).await;

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
            std::time::Duration::from_secs(10),
            cord_invite_sweep_pass(&pool, &audit),
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
            !signup_admits(&pool, username).await,
            "the stock signup refuses the released {username}"
        );
    }
    // the untouched: within the grace, live (the demoted minter's too), busy
    for username in ["graced", "waiting", "before", "held"] {
        assert!(signup_admits(&pool, username).await, "{username}");
    }
    assert_eq!(
        redemption_list(&pool, swept_id)
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
    let released = cord_invite_sweep_pass(&pool, &audit).await;
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
    assert!(!signup_admits(&pool, "held").await);
    // then nothing is left to release
    assert!(cord_invite_sweep_pass(&pool, &audit).await.is_empty());
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM audit_log WHERE event_type = 'cord_invite_expire'"
        )
        .await,
        2
    );
    // the late signups the sweep released the usernames of get no reservation
    let gone_actor = seed_actor(&pool, "gone").await;
    assert!(matches!(
        accept(&pool, &swept, gone_actor).await,
        Err(CordInviteAcceptError::Expired)
    ));
}
