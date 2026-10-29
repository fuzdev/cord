//! Cord's own `_testing_*` actions, folded into `testing_cordd` beside the
//! spine's. Like those, each is keeper-only over the daemon-token credential
//! and writes the database directly, which the production wire never does.
//!
//! - `_testing_cord_invite_expire`: set an invite link's `expires_at` a
//!   second into the past, so the cross-backend suites can see an expired
//!   link without waiting out the shortest real expiry.

use std::sync::Arc;

use fuz_actions::{ActionHandler, ActionSpec};
use fuz_auth::{AuthSpec, CredentialGate, CredentialType};
use fuz_http::{InputShape, internal_error_with_source, not_found};

/// The spine's `_testing_*` auth axis: keeper-only via the daemon token, no
/// acting actor.
const TESTING_ACTION_AUTH: AuthSpec =
    AuthSpec::account_grain(CredentialGate::Only(&[CredentialType::DaemonToken]));

/// Build the `_testing_cord_invite_expire` action spec: input
/// `{invite_id}`, output `{}`.
pub fn create_testing_cord_invite_expire_action_spec() -> ActionSpec {
    let handler: ActionHandler = Arc::new(|params, ctx| {
        Box::pin(async move {
            fuz_http::reject_unknown_keys(&params, &["invite_id"])?;
            let invite_id = fuz_http::parse_uuid_param(&params, "invite_id")?;
            let updated = ctx
                .db
                .execute(
                    "UPDATE cord_invite SET expires_at = NOW() - INTERVAL '1 second' \
                     WHERE id = $1",
                    &[&invite_id],
                )
                .await
                .map_err(|e| internal_error_with_source("_testing_cord_invite_expire", &e))?;
            if updated == 0 {
                return Err(not_found("cord invite", None));
            }
            Ok(serde_json::json!({}))
        })
    });
    ActionSpec::with_side_effects(
        "_testing_cord_invite_expire",
        TESTING_ACTION_AUTH,
        InputShape::Object,
        handler,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seam_is_keeper_only_over_the_daemon_token() {
        let spec = create_testing_cord_invite_expire_action_spec();
        assert_eq!(spec.auth, TESTING_ACTION_AUTH);
        assert!(!spec.auth.needs_actor());
        assert!(spec.side_effects);
    }
}
