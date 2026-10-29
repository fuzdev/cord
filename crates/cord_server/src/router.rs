//! Top-level `axum::Router` composition for `cordd`.
//!
//! Composes the spine's route factories:
//!
//! - `fuz_auth::account_router` → `/api/account/{status,login,logout,password}`
//! - `fuz_auth::bootstrap_routes::bootstrap_router` → `/api/account/bootstrap`
//! - `fuz_auth::signup_routes::signup_router` → `/api/account/signup`
//! - `fuz_actions::create_rpc_router` → `/api/rpc` (GET + POST)
//! - `fuz_actions::register_action_ws` → `/api/ws`, over the same registry;
//!   the protocol actions in it work only here, not over `/api/rpc`
//! - `fuz_realtime::audit_stream_router` → `/api/admin/audit/stream`
//! - `fuz_http::ready_router` → `/ready`
//!
//! plus the spine's `/health` and `/` (`fuz_actions::server::probe_router`).
//! The spine core behind them is booted by
//! `fuz_actions::server::create_app_backend`.
//!
//! ## Layers
//!
//! The router is `fuz_http::compose_stack`'s: one `api` route group (the
//! probes, `/ready`, the account REST routes, RPC, the WebSocket, and the
//! audit stream) and the fallback group for unrouted paths, each behind the
//! stack's fixed order — request tracing around everything, then the 1 MiB
//! body cap, the origin check, and client-IP resolution. The cap sits outside
//! the origin check, so a declared over-cap body answers `413` ahead of a
//! `403`. Every path is origin-checked, the probes and the fallback included;
//! a request with no `Origin` passes, so probes and non-browser clients are
//! unaffected. No group is carved out, and none owns its own cap. The
//! boot-only router (no database) composes through the same stack.
//!
//! Client-IP resolution is in every group: with no trusted proxies it resolves
//! to the TCP peer and ignores `X-Forwarded-For`, while leaving it off would
//! collapse every caller onto one shared bucket in the IP-keyed rate limiters.
//!
//! ## Migrations
//!
//! cord runs the spine's `fuz_auth::AUTH_MIGRATIONS` and
//! `fuz_cell::CELL_MIGRATIONS` (cords are cells), then its own `cord`
//! namespace ([`crate::migrations::CORD_MIGRATIONS`], the `cord_invite`
//! and `cord_invite_redemption` tables), via `fuz_db::run_migrations`.

use std::sync::Arc;

use axum::Router;
use fuz_actions::server::{
    AppBackendSpec, BootedAppBackend, SpineBackendOptions, StartupError, boot_only,
    compile_action_registry, create_app_backend, fallback_group, probe_router,
};
use fuz_actions::{
    ActionRateLimits, ActionRegistry, PROTOCOL_ACTION_SPECS, create_rpc_router, register_action_ws,
};
use fuz_http::{
    BaseServerEnv, BodyCap, DEFAULT_BODY_LIMIT_BYTES, GatedRouter, RouteGroup, compose_stack,
};

use crate::app::CordApp;
use crate::runtime::{ExtraActionSpecsFactory, SERVER_NAME};

/// Session cookie name for cord.
///
/// A per-app name instead of the spine default, so an instance sharing a
/// domain with another fuz app keeps its own session.
pub const CORD_SESSION_COOKIE_NAME: &str = "cord_session";

/// The migration chain cord applies, in order.
///
/// Shared by [`build_router`]'s migration run, the `cordd db-status`
/// subcommand, and the schema fixture tests, so they can't drift.
#[must_use]
pub const fn cord_migration_namespaces() -> [fuz_db::MigrationNamespace; 3] {
    // auth first: the cell tables reference `actor`; cord's tables reference
    // both `actor` and `cell`
    [
        fuz_auth::AUTH_MIGRATIONS,
        fuz_cell::CELL_MIGRATIONS,
        crate::migrations::CORD_MIGRATIONS,
    ]
}

/// What cord's server is.
///
/// Its cookie, its chain, the committed `expected_schema.json` (the column
/// map of cord's full chain, regenerated and drift-guarded by
/// `tests/schema_ready.rs`) `/ready` embeds, and the admin audit stream.
pub const CORD_BACKEND_SPEC: AppBackendSpec = AppBackendSpec {
    session_cookie_name: CORD_SESSION_COOKIE_NAME,
    migration_namespaces: &cord_migration_namespaces(),
    expected_schema_json: Some(include_str!("../expected_schema.json")),
    audit_sse: Some(fuz_actions::server::AuditSseSpec::DEFAULT),
    browser_gates: fuz_actions::server::BrowserGateSpec::DEFAULT,
};

/// What [`build_router`] builds: the router, and the composition root behind
/// it for the work [`crate::runtime::run_app`] runs beside the server.
#[derive(Debug)]
pub struct BuiltRouter {
    /// The composed router, served by `fuz_http::serve_with_shutdown`, the
    /// only thing that takes it.
    pub router: GatedRouter,
    /// `None` in boot-only mode, with no database.
    pub app: Option<CordApp>,
}

/// Build the top-level router.
///
/// Boots the spine core through `fuz_actions::server::create_app_backend` —
/// the READ COMMITTED check (the membership serialization relies on it, and
/// it runs before anything writes), the pre-migration hook (`testing_cordd`'s
/// env-gated schema reset), the migration chain, the keeper resolved into the
/// daemon-token state `testing_cordd` injects, bootstrap availability, the
/// revocation listeners (sockets and admin audit streams), the audit-row
/// fan-out to those streams, and `/ready` — then composes cord over it.
///
/// Falls back to a boot-only router (`/health` and `/` only) when
/// `DATABASE_URL` is absent, so the binary can be smoke-tested without a live
/// Postgres.
///
/// # Errors
///
/// [`StartupError`] when the spine core can't boot (pool, cookie keys,
/// isolation, migrations) or the action registry doesn't compile.
pub async fn build_router(
    env: &BaseServerEnv,
    backend: SpineBackendOptions,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
) -> Result<BuiltRouter, StartupError> {
    if env.database_url.is_none() {
        return Ok(BuiltRouter {
            router: boot_only(env, &CORD_BACKEND_SPEC, SERVER_NAME)?,
            app: None,
        });
    }

    let BootedAppBackend { backend, ready } =
        create_app_backend(env, &CORD_BACKEND_SPEC, backend).await?;
    // the spec embeds a fixture, so the boot built `/ready`
    let ready = ready.unwrap_or_default();
    if backend
        .bootstrap_available
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        tracing::info!("bootstrap is open: POST /api/account/bootstrap creates the first account");
    }
    let app = CordApp::new(backend);

    let router = compose_full_router(&app, ready, extra_action_specs_factory)?;
    Ok(BuiltRouter {
        router,
        app: Some(app),
    })
}

/// Compose the full router with every subsystem wired.
fn compose_full_router(
    app: &CordApp,
    ready: Router,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
) -> Result<GatedRouter, StartupError> {
    let backend = &app.backend;
    // One gate for the whole server: every route state that resolves a
    // credential takes a clone of it, and the stack's options read the same
    // allowlist, so the stack's origin check and each handler's in-handler one
    // cannot drift apart.
    let gate = backend.auth_gate();
    let stack_options = backend.stack_options();
    // One set of per-action limiters shared by the RPC and WebSocket
    // transports and the spine's auth spec set, so a caller gets one budget
    // across them, not one per surface.
    let limits = ActionRateLimits::defaults(backend.rate_limiters);
    let registry = build_action_registry(app, extra_action_specs_factory)?;

    // Account REST, bootstrap, and signup nest under `/api/account`. Signup
    // is invite-gated by default (`app_settings.open_signup = false`) and
    // rate-limited on both axes anyway, so the limiters already stand if an
    // admin opens signup.
    let api_account = backend.auth_rest_routes(gate.clone()).into_router();

    // RPC and WebSocket nest under `/api`. No role gate on the socket: it is
    // a member surface, and the only server-initiated pushes today are the
    // spine's recipient-filtered role-grant-offer notifications. Add roles
    // here the moment a privileged broadcast lands.
    let dispatch = backend.action_dispatch(gate.clone(), registry, &limits);
    let api = Router::new()
        .merge(create_rpc_router(dispatch.clone()))
        .merge(register_action_ws(
            backend.ws_route_state(dispatch, Vec::new()),
        ));

    // The admin audit-log SSE stream, mounted at its absolute path: admins
    // only, over a cookie session only (the spine defaults), since the stream
    // is a browser feed and close-on-revoke keys on the session.
    let audit_stream = backend.audit_stream_router(gate)?;

    // Every route sits in the one `api` group, capped at 1 MiB and
    // origin-checked; the body-less routes (the probes, `/ready`, the audit
    // stream) are capped too, since the cap inspects only the request body.
    let api_group = RouteGroup::new(
        "api",
        probe_router(SERVER_NAME)
            .merge(ready)
            .nest("/api/account", api_account)
            .nest("/api", api)
            .merge(audit_stream),
        BodyCap::Bytes(DEFAULT_BODY_LIMIT_BYTES),
    );

    Ok(compose_stack([api_group], fallback_group(), &stack_options))
}

/// Compile the action registry: the spine's protocol actions, its account,
/// admin, and role-grant-offer actions over cord's role registry (the
/// builtins plus the cord roles, which no spine grant path confers) and
/// cord's offer gate, cord's own actions, then any extra specs from
/// `testing_cordd`.
fn build_action_registry(
    app: &CordApp,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
) -> Result<Arc<ActionRegistry>, StartupError> {
    let mut specs: Vec<fuz_actions::ActionSpec> = PROTOCOL_ACTION_SPECS();
    specs.extend(app.backend.auth_spec_set(
        // a global admin offers; the spine refuses a scoped builtin role
        // before this gate runs
        Arc::new(fuz_auth::AdminOfferAuthorize),
        Arc::new(crate::roles::cord_role_registry()),
    ));
    specs.extend(crate::cord_actions::cord_action_specs(
        app.cord_invite_redeem_rate_limiter.clone(),
    ));
    // The `testing_cordd` seam for the `_testing_*` actions; production
    // passes no factory, so `fuz_testing` stays out of the `cordd` graph.
    if let Some(factory) = extra_action_specs_factory {
        specs.extend(factory(
            Arc::new(app.clone()),
            app.backend.extra_action_specs_runtime(),
        )?);
    }
    // Fail loud: booting with an empty registry would answer
    // `method_not_found` to everything.
    compile_action_registry(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use fuz_auth::RateLimiterMode;
    use fuz_http::InputShape;
    use tower::ServiceExt;

    const FOREIGN_ORIGIN: &str = "https://elsewhere.example";

    /// Send one request, with the TCP peer `serve_with_shutdown` would supply,
    /// and return the status and body.
    async fn send(
        router: GatedRouter,
        method: &str,
        uri: &str,
        origin: Option<&str>,
    ) -> (StatusCode, String) {
        send_with(router, method, uri, origin, None).await
    }

    /// [`send`], optionally declaring a `Content-Length` (with no body behind
    /// it — the cap reads only the header).
    async fn send_with(
        router: GatedRouter,
        method: &str,
        uri: &str,
        origin: Option<&str>,
        content_length: Option<usize>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        if let Some(length) = content_length {
            builder = builder.header("content-length", length.to_string());
        }
        let mut req = builder.body(Body::empty()).expect("request builds");
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:50000"
                .parse::<std::net::SocketAddr>()
                .expect("peer parses"),
        ));
        let res = router
            .into_router_for_testing()
            .oneshot(req)
            .await
            .expect("router responds");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .expect("body reads");
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("utf8 body"),
        )
    }

    /// A full app over a lazy pool: `create_pool` doesn't connect, so nothing
    /// here contacts a database unless a handler queries it.
    fn lazy_app() -> CordApp {
        lazy_app_over(
            "postgres://localhost/cord_unused",
            RateLimiterMode::Enforced,
        )
    }

    /// The env a test router runs on; `database_url` `None` is boot-only.
    fn test_env(database_url: Option<&str>) -> BaseServerEnv {
        BaseServerEnv::from_vars(
            &|k| match k {
                "DATABASE_URL" => database_url.map(str::to_owned),
                "FUZ_ALLOWED_ORIGINS" => Some("http://localhost:*".to_owned()),
                "SECRET_FUZ_COOKIE_KEYS" => {
                    Some("a-cookie-signing-key-that-is-long-enough-to-pass".to_owned())
                }
                // no trusted proxies: the test peer is the client
                "CORD_TRUSTED_PROXIES" => Some(",".to_owned()),
                _ => None,
            },
            crate::runtime::ENV_OPTIONS,
        )
        .expect("env parses")
    }

    /// The boot-only branch's router, through [`boot_only`] (the env opted
    /// in to `test_env`'s wildcard allowlist).
    fn boot_router() -> GatedRouter {
        let env = BaseServerEnv {
            allow_origin_wildcards: true,
            ..test_env(None)
        };
        boot_only(&env, &CORD_BACKEND_SPEC, SERVER_NAME).expect("the boot-only router builds")
    }

    /// The spine core built without booting (its pool is lazy), over
    /// `database_url` and `rate_limiters`.
    fn lazy_app_over(database_url: &str, rate_limiters: RateLimiterMode) -> CordApp {
        let backend = fuz_actions::server::build_app_backend(
            &test_env(Some(database_url)),
            &CORD_BACKEND_SPEC,
            &SpineBackendOptions {
                rate_limiters,
                ..SpineBackendOptions::production(Arc::new(fuz_auth::Argon2idHasher::new()))
            },
        )
        .expect("the core builds without touching the DB");
        CordApp::new(backend)
    }

    fn full_router() -> GatedRouter {
        full_router_over(&lazy_app())
    }

    fn full_router_over(app: &CordApp) -> GatedRouter {
        let ready = fuz_http::ready_router(fuz_http::ReadyRouteState {
            pool: app.backend.pool.clone(),
            expected: Arc::new(expected_ready_schema()),
        });
        compose_full_router(app, ready, None).expect("router composes")
    }

    fn expected_ready_schema() -> fuz_db::ExpectedSchema {
        fuz_actions::server::parse_expected_schema(
            CORD_BACKEND_SPEC
                .expected_schema_json
                .expect("the spec embeds a fixture"),
        )
        .expect("committed fixture parses")
    }

    /// The boot-only branch refuses a wildcard allowlist without the opt-in,
    /// as the full boot does, before any database exists — and boots with it.
    #[tokio::test]
    async fn boot_only_refuses_a_wildcard_origin_without_the_opt_in() {
        let options = || SpineBackendOptions::production(Arc::new(fuz_auth::Argon2idHasher::new()));
        // `test_env`'s allowlist is `http://localhost:*`, with no opt-in
        let message = match build_router(&test_env(None), options(), None).await {
            Err(StartupError::Config(message)) => message,
            Err(other) => other.to_string(),
            Ok(_) => String::new(),
        };
        assert!(
            message.starts_with("FUZ_ALLOWED_ORIGINS holds a wildcard entry"),
            "{message:?}"
        );
        let opted_in = BaseServerEnv {
            allow_origin_wildcards: true,
            ..test_env(None)
        };
        let built = build_router(&opted_in, options(), None).await;
        assert!(built.is_ok_and(|built| built.app.is_none()));
    }

    #[tokio::test]
    async fn boot_only_router_serves_health() {
        let (status, body) = send(boot_router(), "GET", "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn boot_only_router_mounts_no_api_or_ready() {
        for uri in ["/api/rpc", "/api/account/status", "/ready"] {
            let (status, _) = send(boot_router(), "GET", uri, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri} must not be mounted");
        }
    }

    #[tokio::test]
    async fn full_router_serves_health_and_falls_back_to_not_found() {
        let (status, body) = send(full_router(), "GET", "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"status":"ok"}"#);
        let (status, _) = send(full_router(), "GET", "/api/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn full_router_rejects_disallowed_origins() {
        // the origin gate runs before any handler, so no DB is needed
        for (method, uri) in [
            ("POST", "/api/rpc"),
            ("GET", "/api/rpc"),
            ("POST", "/api/account/login"),
            ("POST", "/api/account/bootstrap"),
            ("POST", "/api/account/signup"),
            ("GET", "/api/ws"),
            ("GET", "/api/admin/audit/stream"),
        ] {
            let (status, _) = send(full_router(), method, uri, Some(FOREIGN_ORIGIN)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
        }
    }

    /// Both routers, full and boot-only, labelled for the assertion messages.
    fn both_routers() -> [(&'static str, GatedRouter); 2] {
        [("full", full_router()), ("boot-only", boot_router())]
    }

    #[tokio::test]
    async fn a_foreign_origin_is_refused_on_the_probes_and_unrouted_paths() {
        for (label, router) in both_routers() {
            for uri in ["/health", "/", "/nope", "/api/nope"] {
                let (status, body) = send(router.clone(), "GET", uri, Some(FOREIGN_ORIGIN)).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{label} {uri}: {body}");
                assert_eq!(body, r#"{"error":"forbidden_origin"}"#, "{label} {uri}");
            }
        }
    }

    #[tokio::test]
    async fn an_absent_or_allowed_origin_passes_the_check() {
        for (label, router) in both_routers() {
            for origin in [None, Some("http://localhost:5173")] {
                let (status, _) = send(router.clone(), "GET", "/health", origin).await;
                assert_eq!(status, StatusCode::OK, "{label} {origin:?}");
                let (status, _) = send(router.clone(), "GET", "/nope", origin).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{label} {origin:?}");
            }
        }
    }

    #[tokio::test]
    async fn a_declared_over_cap_body_is_413_ahead_of_the_origin_check() {
        let over_cap = Some(DEFAULT_BODY_LIMIT_BYTES + 1);
        for (label, router) in both_routers() {
            for uri in ["/health", "/nope", "/api/nope"] {
                let (status, body) =
                    send_with(router.clone(), "POST", uri, Some(FOREIGN_ORIGIN), over_cap).await;
                assert_eq!(
                    status,
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "{label} {uri}: {body}"
                );
            }
        }
        // and on the API itself
        for uri in ["/api/rpc", "/api/account/login"] {
            let (status, body) =
                send_with(full_router(), "POST", uri, Some(FOREIGN_ORIGIN), over_cap).await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{uri}: {body}");
        }
    }

    #[tokio::test]
    async fn audit_stream_requires_authentication() {
        // no credential resolves without a DB round trip, so the gate refuses
        // before any query
        for origin in [None, Some("http://localhost:5173")] {
            let (status, body) =
                send(full_router(), "GET", "/api/admin/audit/stream", origin).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "origin {origin:?}");
            assert!(body.contains("authentication_required"), "{body}");
        }
    }

    #[test]
    fn redeem_gets_its_own_limiter_only_when_enforced() {
        assert!(lazy_app().cord_invite_redeem_rate_limiter.is_some());
        // `testing_cordd`'s mode, so the cross suite redeems unthrottled
        let disabled = lazy_app_over(
            "postgres://localhost/cord_unused",
            RateLimiterMode::DisabledForTesting,
        );
        assert!(disabled.cord_invite_redeem_rate_limiter.is_none());
    }

    /// POST one JSON-RPC request from `peer` and return the HTTP status and
    /// the response body.
    async fn post_rpc(
        router: GatedRouter,
        peer: &str,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/rpc")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds");
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            peer.parse::<std::net::SocketAddr>().expect("peer parses"),
        ));
        let res = router
            .into_router_for_testing()
            .oneshot(req)
            .await
            .expect("router responds");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .expect("body reads");
        (
            status,
            serde_json::from_slice(&bytes).expect("a JSON-RPC response"),
        )
    }

    /// Redeem through the whole router and the spine's dispatcher, which
    /// needs a live connection (it opens redeem's transaction before the
    /// handler runs) but no schema: an empty token is refused at params
    /// validation, after the limiter charges and before any query.
    #[tokio::test]
    #[ignore = "requires Postgres at CORD_SCHEMA_READY_TEST_URL"]
    async fn redeem_refuses_past_its_budget_per_ip() {
        use crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS;

        let url = std::env::var("CORD_SCHEMA_READY_TEST_URL")
            .unwrap_or_else(|_| "postgres://localhost/cord_schema_ready_test".to_owned());
        let router = full_router_over(&lazy_app_over(&url, RateLimiterMode::Enforced));
        let redeem = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "cord_invite_redeem",
            "params": { "token": "", "username": "newcomer" },
        });
        for attempt in 0..CORD_INVITE_REDEEM_RATE_LIMIT_MAX_ATTEMPTS {
            let (status, body) = post_rpc(router.clone(), "203.0.113.7:50000", &redeem).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "attempt {attempt}: {body}");
            assert_eq!(body["error"]["code"], -32602, "{body}");
            assert!(body["error"].get("data").is_none(), "no reason: {body}");
        }
        let (status, body) = post_rpc(router.clone(), "203.0.113.7:50000", &redeem).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert_eq!(body["error"]["code"], -32006, "{body}");
        assert!(
            body["error"]["data"]["retry_after"].as_u64().is_some(),
            "{body}"
        );
        // another IP keeps its own budget
        let (status, body) = post_rpc(router, "198.51.100.9:50000", &redeem).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    #[test]
    fn expected_schema_parses_and_covers_the_migration_chain() {
        let schema = expected_ready_schema();
        // the tracker must be expected, so a never-migrated DB fails readiness
        for table in [
            "schema_version",
            "account",
            "actor",
            "auth_session",
            "cell",
            "cell_grant",
            "cord_invite",
        ] {
            assert!(
                schema.contains_key(table),
                "expected_schema.json lacks {table}"
            );
        }
    }

    #[test]
    fn action_registry_compiles_with_the_spine_surface() {
        let registry = build_action_registry(&lazy_app(), None).expect("registry compiles");
        for method in ["heartbeat", "account_verify", "admin_account_list"] {
            assert!(registry.get(method).is_some(), "missing action {method}");
        }
        // every cord spec, however the list grows
        for spec in crate::cord_actions::cord_action_specs(None) {
            assert!(
                registry.get(spec.method).is_some(),
                "missing action {}",
                spec.method
            );
        }
    }

    #[test]
    fn extra_action_specs_factory_folds_into_the_registry() {
        let factory: ExtraActionSpecsFactory = Box::new(|_app, runtime| {
            assert_eq!(runtime.session_cookie_name, CORD_SESSION_COOKIE_NAME);
            Ok(vec![fuz_actions::public_readonly_spec(
                "_testing_seam_probe",
                InputShape::Object,
                |_params, _ctx| Box::pin(async { Ok(serde_json::json!({})) }),
            )])
        });
        let app = lazy_app();
        let registry = build_action_registry(&app, Some(factory)).expect("registry compiles");
        assert!(registry.get("_testing_seam_probe").is_some());
        // the spine surface is still there alongside the extra spec
        assert!(registry.get("account_verify").is_some());

        let without = build_action_registry(&app, None).expect("registry compiles");
        assert!(without.get("_testing_seam_probe").is_none());
    }

    /// The probes are the spine's, named for `cordd`.
    #[tokio::test]
    async fn the_root_names_the_server() {
        for (label, router) in both_routers() {
            let (status, body) = send(router, "GET", "/", None).await;
            assert_eq!(status, StatusCode::OK, "{label}");
            assert_eq!(body, "cordd (rust) — see /health\n", "{label}");
        }
    }
}
