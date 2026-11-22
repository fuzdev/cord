//! The expiry sweep: releases the waiting reservations of newcomer links
//! that expired past the accept grace.
//!
//! A redemption reserves its username as a spine username invite, which the
//! stock signup admits whoever signs up with. Revoking the link releases the
//! reservations still waiting on their signup; expiry alone doesn't, and
//! once a link has been expired for longer than
//! [`CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS`], accept refuses its
//! newcomers, so its waiting reservations can no longer lead anyone into the
//! cord. The sweep deletes them, so the stock signup stops admitting their
//! usernames on an invite-gated instance.
//!
//! [`crate::runtime::run_app`] runs [`spawn_cord_invite_sweep`] beside the
//! server, on the spine's periodic task loop
//! ([`fuz_http::spawn_periodic`]): a pass at startup, after migrations, then
//! one every [`CORD_INVITE_SWEEP_INTERVAL`], until shutdown.
//!
//! A pass ([`cord_invite_sweep_pass`]) selects the links expired past the
//! grace (by the database clock) that still have waiting reservations, then
//! releases each in its own transaction: it takes the link's lock with
//! `SKIP LOCKED`, the link lock accept and redeem take, so it orders
//! against them as revoke does, and a link that's busy waits for the next
//! pass. It then releases through
//! [`query_cord_invite_redemption_release`] and audits `cord_invite_expire`
//! with the released spine invite ids, in the same transaction. The event
//! has no actor, account, or IP: no request is behind it.
//!
//! A live link whose minter is no longer an instance admin isn't swept.
//! Redeem refuses it new reservations, but the ones it already made were
//! authorized when made, and accept doesn't re-read the minter, so they're
//! still pending. They go when the link expires past the grace (a newcomer
//! link expires within
//! [`crate::invite::queries::CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX`]),
//! or sooner if the link is revoked.
//!
//! Failures never stop the server: a failed link is logged and the pass
//! moves on, and a pass that can't reach the database is logged and retried
//! on the next tick.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::{Object, Pool};
use fuz_auth::{AuditEmitter, AuditLogInput, AuditRouting, CollectingAuditPostCommit};
use fuz_cell::CellError;
use fuz_db::{DbError, commit_checked};
use fuz_http::CancellationToken;
use serde_json::json;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::invite::queries::{
    CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS, query_cord_invite_try_lock_expired_beyond,
};
use crate::invite::redemption_queries::{
    query_cord_invite_expired_with_waiting_redemptions, query_cord_invite_redemption_release,
};

/// How often the expiry sweep runs after its startup pass (10 minutes).
pub const CORD_INVITE_SWEEP_INTERVAL: Duration = Duration::from_mins(10);

/// The audit event a sweep writes for each link it releases.
pub const CORD_INVITE_EXPIRE_AUDIT_EVENT: &str = "cord_invite_expire";

/// One link's reservations, released by a sweep pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordInviteExpireReleased {
    pub cord_id: Uuid,
    pub invite_id: Uuid,
    /// The spine username invites deleted, never empty.
    pub released_signup_invite_ids: Vec<Uuid>,
}

/// Spawn the expiry sweep, which runs until `shutdown` is cancelled: a pass
/// now, then one every `interval`.
///
/// Shutdown drops a pass in progress, which rolls back the link it was on;
/// the next start's pass picks it up. The handle resolves once the sweep has
/// stopped.
///
/// # Panics
///
/// If `interval` is zero, or when called outside a tokio runtime.
pub fn spawn_cord_invite_sweep(
    pool: Pool,
    audit: Arc<AuditEmitter>,
    interval: Duration,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    fuz_http::spawn_periodic("cord invite sweep", interval, shutdown, move || {
        let pool = pool.clone();
        let audit = Arc::clone(&audit);
        async move {
            let released = cord_invite_sweep_pass(&pool, &audit).await;
            if !released.is_empty() {
                tracing::info!(
                    links = released.len(),
                    "released the waiting reservations of expired newcomer links"
                );
            }
            // a pass logs its own failures and never fails the loop
            Ok::<(), Infallible>(())
        }
    })
}

/// One sweep pass, returning what it released.
///
/// Releases the waiting reservations of every newcomer link expired past
/// [`CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS`], each link in its own
/// transaction, auditing `cord_invite_expire` for each.
///
/// Never fails: a database it can't reach, or a link that fails, is logged
/// at `warn` and left for the next pass.
pub async fn cord_invite_sweep_pass(
    pool: &Pool,
    audit: &Arc<AuditEmitter>,
) -> Vec<CordInviteExpireReleased> {
    let mut client = match pool.get().await {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!(
                error = %fuz_sys::cli::format_error_chain(&e),
                "cord invite sweep: no database connection"
            );
            return Vec::new();
        }
    };
    let invite_ids = match query_cord_invite_expired_with_waiting_redemptions(
        &client,
        CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS,
    )
    .await
    {
        Ok(invite_ids) => invite_ids,
        Err(e) => {
            tracing::warn!(
                error = %fuz_sys::cli::format_error_chain(&e),
                "cord invite sweep: selecting expired links failed"
            );
            return Vec::new();
        }
    };
    let mut released = Vec::new();
    for invite_id in invite_ids {
        match sweep_link(&mut client, audit, invite_id).await {
            Ok(Some(link)) => released.push(link),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    error = %fuz_sys::cli::format_error_chain(&e),
                    %invite_id,
                    "cord invite sweep: releasing a link's reservations failed"
                );
            }
        }
    }
    released
}

/// Why releasing one link failed: a statement inside its transaction, or
/// the commit.
#[derive(Debug, thiserror::Error)]
enum SweepLinkError {
    /// A driver or decode failure inside the transaction.
    #[error(transparent)]
    Query(#[from] CellError),
    /// The `COMMIT` failed, or [`commit_checked`]'s pre-commit probe found
    /// the transaction aborted and rolled it back.
    #[error("commit failed")]
    Commit(#[source] DbError),
}

/// Release one link's waiting reservations in their own transaction, or
/// `None` when the link is busy, no longer qualifies, or had nothing left
/// to release.
async fn sweep_link(
    client: &mut Object,
    audit: &Arc<AuditEmitter>,
    invite_id: Uuid,
) -> Result<Option<CordInviteExpireReleased>, SweepLinkError> {
    let tx = client.transaction().await.map_err(CellError::Db)?;
    // dropping `tx` uncommitted rolls it back
    let Some(cord_id) = query_cord_invite_try_lock_expired_beyond(
        &tx,
        invite_id,
        CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS,
    )
    .await?
    else {
        return Ok(None);
    };
    let released_signup_invite_ids = query_cord_invite_redemption_release(&tx, invite_id).await?;
    if released_signup_invite_ids.is_empty() {
        // a signup claimed what was waiting since the pass selected the link
        return Ok(None);
    }
    // the listener fan-out (the admin audit stream) runs once the row commits
    let post_commit = CollectingAuditPostCommit::default();
    // no request behind it, so no actor, account, or IP
    audit
        .emit_in_tx(
            &tx,
            AuditLogInput::for_custom_event(
                CORD_INVITE_EXPIRE_AUDIT_EVENT,
                AuditRouting::InTx,
                AuditRouting::Pool,
            )
            .metadata(json!({
                "cord_id": cord_id,
                "invite_id": invite_id,
                "released_signup_invite_ids": released_signup_invite_ids,
            })),
            &post_commit,
        )
        .await
        .map_err(CellError::Db)?;
    commit_checked(tx).await.map_err(SweepLinkError::Commit)?;
    post_commit.drain().await;
    Ok(Some(CordInviteExpireReleased {
        cord_id,
        invite_id,
        released_signup_invite_ids,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool whose every checkout fails fast (nothing listens on port 1).
    fn unreachable_pool() -> Pool {
        fuz_db::create_pool(
            "postgres://127.0.0.1:1/cord_unreachable",
            &fuz_db::PoolOptions::default(),
        )
        .expect("the pool is lazy")
    }

    #[tokio::test]
    async fn a_pass_without_a_database_logs_and_releases_nothing() {
        let pool = unreachable_pool();
        let audit = Arc::new(AuditEmitter::new(pool.clone(), Arc::default()));
        assert!(cord_invite_sweep_pass(&pool, &audit).await.is_empty());
    }

    #[tokio::test]
    async fn the_sweep_survives_an_outage_and_stops_on_shutdown() {
        let pool = unreachable_pool();
        let audit = Arc::new(AuditEmitter::new(pool.clone(), Arc::default()));
        let shutdown = CancellationToken::new();
        let sweep =
            spawn_cord_invite_sweep(pool, audit, Duration::from_millis(10), shutdown.clone());
        // several failing passes, then shutdown
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!sweep.is_finished(), "an outage doesn't end the sweep");
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(10), sweep)
            .await
            .expect("the sweep stops once shutdown is cancelled")
            .expect("the sweep task ends cleanly");
    }
}
