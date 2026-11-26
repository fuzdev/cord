//! Every cord verb through the assembled router: `build_router`'s live
//! branch (migrations, the `compose_stack` groups, the credential gate, and
//! the spine's JSON-RPC dispatcher), driven with `POST /api/rpc`.
//!
//! The other suites call the query functions, and each action module's unit
//! tests call its params parser directly, so neither shows that a verb is
//! mounted with the auth spec it declares or that its handler reaches the
//! right parser. Here each verb is sent malformed params twice: anonymously,
//! refused `authentication_required` ahead of the params unless its spec
//! admits anyone, and as the instance admin who owns the cord, refused
//! `invalid_params` with the reason the field declares, or none, before the
//! handler answers anything of its own. A declared reason pins the verb's own
//! parser; a reasonless refusal shows only that some strict parser runs first,
//! so each family's admitted call also pins its result's keys. Each family
//! then answers a caller it admits and masks the cord from one outside it,
//! and the instance-admin and session-only gates refuse who they should.
//! Every refusal pins its HTTP status, its JSON-RPC code, and its whole
//! `data`.
//!
//! One test over one database (`common::DB`, bare: the boot migrates it).

#![expect(
    clippy::expect_used,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{DB, create_cord, mint};
use cord_server::cord_actions::{ERROR_CORD_NAME_INVALID, ERROR_CORD_NOT_FOUND, cord_action_specs};
use cord_server::cord_space_actions::ERROR_CORD_SPACE_NAME_INVALID;
use cord_server::invite::actions::ERROR_CORD_INVITE_USERNAME_INVALID;
use cord_server::router::{BuiltRouter, CORD_SESSION_COOKIE_NAME, build_router};
use fuz_actions::server::SpineBackendOptions;
use fuz_auth::{ROLE_ADMIN, TokenScopeDocument};
use fuz_http::GatedRouter;
use fuz_testing::{
    TESTING_ALLOWED_ORIGIN, TestingDbOptions, fresh_test_db, grant_role, mint_api_token,
    mint_session_cookie, seed_account_actor, testing_keyring,
};
use serde_json::{Value, json};

/// Who sends a request.
#[derive(Clone, Copy, Debug)]
enum Caller {
    /// No credential.
    Anonymous,
    /// An instance admin who owns the cord, over a session cookie.
    Admin,
    /// The admin's account over an API token, with no `Origin`, as a bearer
    /// request must be.
    AdminToken,
    /// An account with no role anywhere, over a session cookie.
    Outsider,
}

/// Whom a verb's auth spec admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Admits {
    /// An authenticated actor only.
    Actors,
    /// Anyone (`optional_identity` or `public`).
    Anyone,
}

/// What a request owes.
#[derive(Debug)]
enum Expect {
    /// `200` with a `result` holding exactly these keys, sorted, which tell
    /// the verb's own handler from another's.
    Ok(&'static [&'static str]),
    /// The HTTP status, the JSON-RPC code, and the whole `error.data`
    /// (`None` when there is none).
    Refused(StatusCode, i64, Option<Value>),
}

const UNAUTHENTICATED: i64 = -32001;
const FORBIDDEN: i64 = -32002;
const NOT_FOUND: i64 = -32003;
const INVALID_PARAMS: i64 = -32602;

fn invalid_params(reason: Option<&str>) -> Expect {
    let data = reason.map(|reason| json!({ "reason": reason }));
    Expect::Refused(StatusCode::BAD_REQUEST, INVALID_PARAMS, data)
}

/// The instance-admin role gate's refusal.
fn not_an_instance_admin() -> Expect {
    let data = json!({ "reason": "insufficient_permissions", "required_roles": [ROLE_ADMIN] });
    Expect::Refused(StatusCode::FORBIDDEN, FORBIDDEN, Some(data))
}

/// The IDOR mask: anyone outside the cord, as for a missing one.
fn cord_not_found() -> Expect {
    let data = json!({ "reason": ERROR_CORD_NOT_FOUND });
    Expect::Refused(StatusCode::NOT_FOUND, NOT_FOUND, Some(data))
}

/// The credentials the callers present.
struct Credentials {
    admin_cookie: String,
    admin_token: String,
    outsider_cookie: String,
}

/// POST one JSON-RPC request as `caller` and check it against `expect`,
/// returning a description of the mismatch, if any.
async fn check(
    router: &GatedRouter,
    credentials: &Credentials,
    (method, caller, params, expect): (&str, Caller, &Value, &Expect),
) -> Option<String> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let builder = Request::builder()
        .method("POST")
        .uri("/api/rpc")
        .header("content-type", "application/json");
    let builder = match caller {
        Caller::Anonymous => builder.header("origin", TESTING_ALLOWED_ORIGIN),
        Caller::Admin => builder
            .header("origin", TESTING_ALLOWED_ORIGIN)
            .header("cookie", &credentials.admin_cookie),
        Caller::AdminToken => builder.header(
            "authorization",
            format!("Bearer {}", credentials.admin_token),
        ),
        Caller::Outsider => builder
            .header("origin", TESTING_ALLOWED_ORIGIN)
            .header("cookie", &credentials.outsider_cookie),
    };
    let req = builder
        .body(Body::from(body.to_string()))
        .expect("request builds");
    let res = router
        .clone()
        .send_for_testing(req, fuz_http::TEST_PEER)
        .await;
    let status = res.status();
    let body: Value = serde_json::from_slice(res.body()).expect("a JSON-RPC response");
    let held = body["id"] == 1
        && match expect {
            Expect::Ok(keys) => {
                let mut got: Vec<&str> =
                    body["result"].as_object().map_or_else(Vec::new, |result| {
                        result.keys().map(String::as_str).collect()
                    });
                got.sort_unstable();
                status == StatusCode::OK && got == *keys
            }
            Expect::Refused(want_status, code, data) => {
                status == *want_status
                    && body["error"]["code"] == *code
                    && body["error"].get("data") == data.as_ref()
            }
        };
    (!held).then(|| {
        format!("{method} as {caller:?} with {params}: expected {expect:?}, got {status} {body}")
    })
}

#[tokio::test]
async fn every_verb_answers_its_auth_spec_and_params_through_the_router() {
    // bare: the boot under test runs the chain
    let db = fresh_test_db(TestingDbOptions {
        migrations: &[],
        ..DB
    })
    .await;
    let url = db.url().to_owned();
    let env = fuz_http::BaseServerEnv::from_vars(
        &|k| match k {
            "DATABASE_URL" => Some(url.clone()),
            // the fixture key `testing_keyring` signs with
            "SECRET_FUZ_COOKIE_KEYS" => Some("k".repeat(fuz_auth::keyring::MIN_KEY_LENGTH)),
            "FUZ_ALLOWED_ORIGINS" => Some(TESTING_ALLOWED_ORIGIN.to_owned()),
            // no trusted proxies: the test peer is the client
            "CORD_TRUSTED_PROXIES" => Some(",".to_owned()),
            _ => None,
        },
        cord_server::runtime::ENV_OPTIONS,
    )
    .expect("the env reads");
    let options = SpineBackendOptions {
        // so no limiter, redeem's own included, answers in a gate's place
        rate_limiters: fuz_auth::RateLimiterMode::DisabledForTesting,
        ..SpineBackendOptions::production(
            Arc::new(fuz_testing::TestingArgon2idHasher::new()),
            fuz_auth::AuthCleanupSchedule::Off,
        )
    };
    let built = build_router(&env, options, None, None)
        .await
        .expect("the router builds");
    let BuiltRouter::Live { router, .. } = built else {
        panic!("a database URL builds the live router");
    };

    let (admin_account, admin) = seed_account_actor(&db, "admin").await;
    grant_role(&db, admin, ROLE_ADMIN).await;
    let (outsider_account, _) = seed_account_actor(&db, "outsider").await;
    let cord = create_cord(&db, "the cord", admin).await.id;
    let (token, _) = mint(&db, cord, admin, None, None, false).await;
    let keyring = testing_keyring();
    let cookie = |account| mint_session_cookie(&db, &keyring, account, CORD_SESSION_COOKIE_NAME);
    let credentials = Credentials {
        admin_cookie: cookie(admin_account).await,
        admin_token: mint_api_token(&db, admin_account, &TokenScopeDocument::full())
            .await
            .token,
        outsider_cookie: cookie(outsider_account).await,
    };

    let id = cord.to_string();
    let other = "11111111-1111-4111-8111-111111111111";
    let mut failures = Vec::new();

    // Each verb, malformed: anonymously, then as the admin.
    let verbs = [
        // cords (`cord_actions.rs`)
        (
            "cord_create",
            Admits::Actors,
            json!({ "name": "" }),
            Some(ERROR_CORD_NAME_INVALID),
        ),
        ("cord_list", Admits::Actors, json!({ "extra": 1 }), None),
        (
            "cord_get",
            Admits::Actors,
            json!({ "cord_id": "not-a-uuid" }),
            None,
        ),
        (
            "cord_update",
            Admits::Actors,
            json!({ "cord_id": id, "name": " " }),
            Some(ERROR_CORD_NAME_INVALID),
        ),
        (
            "cord_admin_list",
            Admits::Actors,
            json!({ "extra": 1 }),
            None,
        ),
        // members (`cord_member_actions.rs`)
        ("cord_member_list", Admits::Actors, json!({}), None),
        (
            "cord_member_role_set",
            Admits::Actors,
            json!({ "cord_id": id, "actor_id": other, "role": "king" }),
            None,
        ),
        (
            "cord_member_remove",
            Admits::Actors,
            json!({ "cord_id": id }),
            None,
        ),
        ("cord_leave", Admits::Actors, json!({ "cord_id": 7 }), None),
        // invite links (`invite/actions.rs`)
        (
            "cord_invite_create",
            Admits::Actors,
            json!({ "cord_id": id, "max_uses": 0 }),
            None,
        ),
        (
            "cord_invite_list",
            Admits::Actors,
            json!({ "cord_id": id, "extra": 1 }),
            None,
        ),
        (
            "cord_invite_revoke",
            Admits::Actors,
            json!({ "cord_id": id }),
            None,
        ),
        (
            "cord_invite_preview",
            Admits::Anyone,
            json!({ "token": "" }),
            None,
        ),
        (
            "cord_invite_redeem",
            Admits::Anyone,
            json!({ "token": token, "username": "1nope" }),
            Some(ERROR_CORD_INVITE_USERNAME_INVALID),
        ),
        (
            "cord_invite_accept",
            Admits::Actors,
            json!({ "token": "" }),
            None,
        ),
        // spaces (`cord_space_actions.rs`)
        (
            "cord_space_list",
            Admits::Actors,
            json!({ "cord_id": id, "extra": 1 }),
            None,
        ),
        (
            "cord_space_create",
            Admits::Actors,
            json!({ "cord_id": id, "kind": "line", "name": "" }),
            Some(ERROR_CORD_SPACE_NAME_INVALID),
        ),
        (
            "cord_space_update",
            Admits::Actors,
            json!({ "cord_id": id, "space_id": other, "name": "" }),
            Some(ERROR_CORD_SPACE_NAME_INVALID),
        ),
        (
            "cord_space_delete",
            Admits::Actors,
            json!({ "cord_id": id }),
            None,
        ),
    ];
    for (method, admits, params, reason) in &verbs {
        let refused = invalid_params(*reason);
        // the pre-authorization 401 comes ahead of the params
        let anonymous = match admits {
            Admits::Actors => Expect::Refused(
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED,
                Some(json!({ "reason": "authentication_required" })),
            ),
            Admits::Anyone => invalid_params(*reason),
        };
        for (caller, expect) in [(Caller::Anonymous, &anonymous), (Caller::Admin, &refused)] {
            let case = (*method, caller, params, expect);
            failures.extend(check(&router, &credentials, case).await);
        }
    }
    // every mounted cord verb has its row
    for spec in cord_action_specs(None) {
        let Some((_, admits, ..)) = verbs.iter().find(|(m, ..)| *m == spec.method) else {
            failures.push(format!("{} has no row", spec.method));
            continue;
        };
        let required = spec.auth.account_required();
        if required != (*admits == Admits::Actors) {
            failures.push(format!("{}: account_required is {required}", spec.method));
        }
    }

    // Each family: a caller it admits, one outside the cord, and the gates.
    let cases = [
        // cords
        (
            "cord_create",
            Caller::Admin,
            json!({ "name": "another cord" }),
            Expect::Ok(&["cord"]),
        ),
        // the instance-admin role gate, ahead of the params
        (
            "cord_create",
            Caller::Outsider,
            json!({ "name": "" }),
            not_an_instance_admin(),
        ),
        (
            "cord_admin_list",
            Caller::Admin,
            json!({}),
            Expect::Ok(&["cords"]),
        ),
        (
            "cord_admin_list",
            Caller::Outsider,
            json!({}),
            not_an_instance_admin(),
        ),
        (
            "cord_list",
            Caller::Outsider,
            json!({}),
            Expect::Ok(&["cords"]),
        ),
        (
            "cord_get",
            Caller::Admin,
            json!({ "cord_id": id }),
            Expect::Ok(&["cord"]),
        ),
        (
            "cord_get",
            Caller::Outsider,
            json!({ "cord_id": id }),
            cord_not_found(),
        ),
        // members
        (
            "cord_member_list",
            Caller::Admin,
            json!({ "cord_id": id }),
            Expect::Ok(&["members"]),
        ),
        (
            "cord_member_list",
            Caller::Outsider,
            json!({ "cord_id": id }),
            cord_not_found(),
        ),
        // invite links
        (
            "cord_invite_create",
            Caller::Admin,
            json!({ "cord_id": id }),
            Expect::Ok(&["invite", "token"]),
        ),
        (
            "cord_invite_create",
            Caller::Outsider,
            json!({ "cord_id": id }),
            cord_not_found(),
        ),
        (
            "cord_invite_preview",
            Caller::Anonymous,
            json!({ "token": token }),
            Expect::Ok(&[
                "admits_newcomers",
                "cord",
                "newcomers_open",
                "role",
                "status",
            ]),
        ),
        // session only: the admin's own API token is refused, ahead of the params
        (
            "cord_invite_accept",
            Caller::AdminToken,
            json!({ "token": "" }),
            Expect::Refused(
                StatusCode::FORBIDDEN,
                FORBIDDEN,
                Some(json!({
                    "reason": "credential_type_required",
                    "required_credential_types": ["session"],
                })),
            ),
        ),
        // spaces
        (
            "cord_space_create",
            Caller::Admin,
            json!({ "cord_id": id, "kind": "line", "name": "the den" }),
            Expect::Ok(&["space"]),
        ),
        (
            "cord_space_list",
            Caller::AdminToken,
            json!({ "cord_id": id }),
            Expect::Ok(&["spaces"]),
        ),
        (
            "cord_space_list",
            Caller::Outsider,
            json!({ "cord_id": id }),
            cord_not_found(),
        ),
    ];
    for (method, caller, params, expect) in &cases {
        let case = (*method, *caller, params, expect);
        failures.extend(check(&router, &credentials, case).await);
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
