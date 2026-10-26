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
//! plus cord's own `/health` and `/`.
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

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use fuz_actions::{
    ActionRegistry, PROTOCOL_ACTION_SPECS, RpcRouteState, WsRouteState,
    auth_adapter::build_auth_spec_set, create_rpc_router, register_action_ws,
};
use fuz_auth::{AccountRouteState, AuthGate, RateLimiter, RateLimiterMode};
use fuz_http::{
    BodyCap, DEFAULT_BODY_LIMIT_BYTES, GatedRouter, ParsedProxy, RouteGroup, StackOptions,
    compose_stack,
};
use serde::Serialize;

use crate::app::CordApp;
use crate::env::CordEnv;
use crate::error::StartupError;
use crate::runtime::{ExtraActionSpecsFactory, PreMigrationHook};

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
/// Falls back to a boot-only router (`/health` and `/` only) when
/// `env.database_url` is absent, so the binary can be smoke-tested without a
/// live Postgres.
///
/// `daemon_token_state`, `audit_inflight_tracking`,
/// `extra_action_specs_factory`, and `pre_migration_hook` are the
/// `testing_cordd` seams; production passes `false` for the tracking and
/// `None` for the rest.
///
/// # Errors
///
/// [`StartupError`] when the composition root can't be built (pool, cookie
/// keys), migrations fail, or the action registry doesn't compile.
pub async fn build_router(
    env: &CordEnv,
    password_hasher: Arc<dyn fuz_auth::PasswordHasher>,
    rate_limiters: RateLimiterMode,
    daemon_token_state: Option<fuz_auth::SharedDaemonTokenState>,
    audit_inflight_tracking: bool,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    pre_migration_hook: Option<PreMigrationHook>,
) -> Result<BuiltRouter, StartupError> {
    if env.database_url.is_none() {
        tracing::warn!("DATABASE_URL not set; running in boot-only mode (no /api, no /ready)");
        return Ok(BuiltRouter {
            router: boot_only_router(env),
            app: None,
        });
    }

    let app = CordApp::build(
        env,
        password_hasher,
        rate_limiters,
        daemon_token_state,
        audit_inflight_tracking,
    )?;

    // `testing_cordd`'s env-gated schema reset runs once the pool exists and
    // before migrations.
    if let Some(hook) = pre_migration_hook {
        hook(&app.pool).await?;
    }

    // Migrations land before any handler runs.
    fuz_db::run_migrations(&app.pool, &cord_migration_namespaces()).await?;

    // Only `testing_cordd` supplies a daemon token; point it at the keeper
    // now that the schema exists. The token is static for the process
    // lifetime.
    if let Some(state) = app.daemon_token_state.clone() {
        fuz_auth::resolve_keeper_into(&state, &app.pool).await;
    }

    // Bootstrap opens only while a token file exists and no account does;
    // recompute now that the schema exists.
    let bootstrap_available = fuz_auth::is_bootstrap_available(
        &app.pool,
        app.bootstrap_token_path.as_deref().and_then(|p| p.to_str()),
    )
    .await;
    app.bootstrap_available
        .store(bootstrap_available, std::sync::atomic::Ordering::SeqCst);
    if bootstrap_available {
        tracing::info!("bootstrap is open: POST /api/account/bootstrap creates the first account");
    }

    // Logout, password change, and session or token revocation drop the
    // matching live `/api/ws` connections and admin audit streams (the
    // `RealtimeRevoker` fan-out).
    let revoker = app.socket_revoker();
    fuz_auth::register_socket_revocation_listeners(&app.audit_emitter, &revoker);

    // Audit rows fan out to the open admin audit streams, which close on
    // logout, password change, session revocation, or loss of the admin role.
    fuz_realtime::register_audit_sse_listener(&app.audit_emitter, &app.audit_sse);

    // `/ready`: the schema-drift readiness probe over the committed
    // `expected_schema.json`, embedded at compile time so the binary is
    // self-contained. Parsed here so a malformed fixture fails the boot.
    let ready = fuz_http::ready_router(fuz_http::ReadyRouteState {
        pool: app.pool.clone(),
        expected: Arc::new(expected_ready_schema()?),
    });

    let router = compose_full_router(
        &app,
        env.trusted_proxies.clone(),
        ready,
        extra_action_specs_factory,
        rate_limiters,
    )?;
    Ok(BuiltRouter {
        router,
        app: Some(app),
    })
}

/// The committed `expected_schema.json` (the column map of cord's full
/// migration chain), served by the `/ready` probe. Regenerated and
/// drift-guarded by `tests/schema_ready.rs`.
fn expected_ready_schema() -> Result<fuz_db::ExpectedSchema, StartupError> {
    const EXPECTED_SCHEMA_JSON: &str = include_str!("../expected_schema.json");
    serde_json::from_str(EXPECTED_SCHEMA_JSON)
        .map_err(|e| StartupError::Internal(format!("expected_schema.json failed to parse: {e}")))
}

/// The router when no database is configured: the probes and the fallback,
/// through the same stack as the full router (1 MiB cap, origin-checked).
///
/// There is no `AuthGate` without a database, so the stack options come
/// straight from the env's allowlist and trusted proxies.
fn boot_only_router(env: &CordEnv) -> GatedRouter {
    compose_stack(
        [RouteGroup::new(
            "api",
            Router::new()
                .route("/health", get(health))
                .route("/", get(root)),
            BodyCap::Bytes(DEFAULT_BODY_LIMIT_BYTES),
        )],
        fallback_group(),
        &StackOptions::new(
            env.allowed_origins.clone(),
            Arc::new(env.trusted_proxies.clone()),
        ),
    )
}

/// The fallback group: the `404` for every unrouted path, behind the same cap
/// and origin check as the routed group.
fn fallback_group() -> RouteGroup {
    RouteGroup::new(
        "fallback",
        Router::new().fallback(not_found),
        BodyCap::Bytes(DEFAULT_BODY_LIMIT_BYTES),
    )
}

/// Health-check response body, the spine's canonical `{"status":"ok"}`.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn root() -> &'static str {
    "cordd: the cord server; see /health\n"
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found\n").into_response()
}

/// Compose the full router with every subsystem wired.
fn compose_full_router(
    app: &CordApp,
    trusted_proxies: Vec<ParsedProxy>,
    ready: Router,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    rate_limiters: RateLimiterMode,
) -> Result<GatedRouter, StartupError> {
    // One gate for the whole server: every route state that resolves a
    // credential takes a clone of it, and the stack's options read its
    // allowlist, so the stack's origin check and each handler's in-handler one
    // cannot drift apart.
    let gate = app.auth_gate();
    let stack_options = gate.stack_options(Arc::new(trusted_proxies));
    // One set of per-action limiters shared by the RPC and WebSocket
    // transports and the spine's auth spec set, so a caller gets one budget
    // across them, not one per surface.
    let action_account_rl = rate_limiters.limiter(fuz_auth::DEFAULT_ACTION_ACCOUNT_RATE_LIMIT);
    let action_ip_rl = rate_limiters.limiter(fuz_auth::DEFAULT_ACTION_IP_RATE_LIMIT);
    let registry = build_action_registry(
        app,
        extra_action_specs_factory,
        action_account_rl.clone(),
        action_ip_rl.clone(),
    )?;

    // Account REST, bootstrap, and signup nest under `/api/account`.
    let api_account = fuz_auth::account_router(build_account_state(app, gate.clone()))
        .merge(fuz_auth::bootstrap_routes::bootstrap_router(
            build_bootstrap_state(app, gate.clone()),
        ))
        .merge(fuz_auth::signup_routes::signup_router(build_signup_state(
            app,
            gate.clone(),
            rate_limiters,
        )));

    // RPC and WebSocket nest under `/api`.
    let api = Router::new()
        .merge(create_rpc_router(build_rpc_state(
            app,
            gate.clone(),
            Arc::clone(&registry),
            action_account_rl.clone(),
            action_ip_rl.clone(),
        )))
        .merge(register_action_ws(build_ws_state(
            app,
            gate.clone(),
            registry,
            action_account_rl,
            action_ip_rl,
        )));

    // The admin audit-log SSE stream, mounted at its absolute path.
    let audit_stream = fuz_realtime::audit_stream_router(build_audit_stream_state(app, gate));

    // Every route sits in the one `api` group, capped at 1 MiB and
    // origin-checked; the body-less routes (the probes, `/ready`, the audit
    // stream) are capped too, since the cap inspects only the request body.
    let api_group = RouteGroup::new(
        "api",
        Router::new()
            .route("/health", get(health))
            .route("/", get(root))
            .merge(ready)
            .nest("/api/account", api_account)
            .nest("/api", api)
            .merge(audit_stream),
        BodyCap::Bytes(DEFAULT_BODY_LIMIT_BYTES),
    );

    let trusted_proxies = &stack_options.trusted_proxies;
    if trusted_proxies.is_empty() {
        tracing::info!("no trusted proxies configured; XFF ignored, client IP is the TCP peer");
    } else {
        tracing::info!(
            count = trusted_proxies.len(),
            "trusted proxies configured; XFF resolution enabled"
        );
    }

    Ok(compose_stack([api_group], fallback_group(), &stack_options))
}

fn build_account_state(app: &CordApp, gate: AuthGate) -> AccountRouteState {
    AccountRouteState {
        gate,
        password_hasher: Arc::clone(&app.password_hasher),
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        bootstrap_available: Arc::clone(&app.bootstrap_available),
        login_ip_rate_limiter: app.login_ip_rate_limiter.clone(),
        login_account_rate_limiter: app.login_account_rate_limiter.clone(),
        login_fail_floor_ms: fuz_auth::DEFAULT_LOGIN_FAIL_FLOOR_MS,
        login_fail_jitter_ms: fuz_auth::DEFAULT_LOGIN_FAIL_JITTER_MS,
    }
}

fn build_bootstrap_state(app: &CordApp, gate: AuthGate) -> fuz_auth::BootstrapRouteState {
    let options = Arc::new(fuz_auth::BootstrapOptions {
        pool: app.pool.clone(),
        password_hasher: Arc::clone(&app.password_hasher),
        audit: Arc::clone(&app.audit_emitter),
        bootstrap_available: Arc::clone(&app.bootstrap_available),
        token_store: app.bootstrap_token_path.as_ref().map(|p| {
            let store: Arc<dyn fuz_auth::BootstrapTokenStore> =
                Arc::new(fuz_auth::FileBootstrapTokenStore::new(p.clone()));
            store
        }),
        // Only set under `testing_cordd`: bootstrap teaches the daemon-token
        // state the keeper's account id so the credential resolves to it.
        on_keeper_resolved: app.daemon_token_state.clone().map(|state| {
            let resolved: Arc<dyn fuz_auth::BootstrapKeeperResolved> =
                Arc::new(fuz_auth::DaemonTokenKeeperResolved::new(state));
            resolved
        }),
    });
    fuz_auth::BootstrapRouteState { options, gate }
}

/// Signup is invite-gated by default (`app_settings.open_signup = false`) and
/// rate-limited on both axes anyway, so the limiters already stand if an
/// admin opens signup.
fn build_signup_state(
    app: &CordApp,
    gate: AuthGate,
    rate_limiters: RateLimiterMode,
) -> fuz_auth::SignupRouteState {
    fuz_auth::SignupRouteState {
        options: Arc::new(fuz_auth::SignupOptions {
            pool: app.pool.clone(),
            password_hasher: Arc::clone(&app.password_hasher),
            audit: Arc::clone(&app.audit_emitter),
            // Own instances, not the login buckets: a signup flood must not
            // spend the budget that bounds credential guessing.
            signup_ip_rate_limiter: rate_limiters.limiter(fuz_auth::DEFAULT_LOGIN_IP_RATE_LIMIT),
            signup_account_rate_limiter: rate_limiters
                .limiter(fuz_auth::DEFAULT_LOGIN_ACCOUNT_RATE_LIMIT),
            signup_fail_floor_ms: fuz_auth::DEFAULT_SIGNUP_FAIL_FLOOR_MS,
            signup_fail_jitter_ms: fuz_auth::DEFAULT_SIGNUP_FAIL_JITTER_MS,
        }),
        gate,
    }
}

/// Compile the action registry: the spine's protocol actions, its account,
/// admin, and role-grant-offer actions over cord's role registry (the
/// builtins plus the cord roles, which no spine grant path confers) and
/// cord's offer gate, cord's own actions, then any extra specs from
/// `testing_cordd`.
fn build_action_registry(
    app: &CordApp,
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    action_account_rate_limiter: Option<Arc<RateLimiter>>,
    action_ip_rate_limiter: Option<Arc<RateLimiter>>,
) -> Result<Arc<ActionRegistry>, StartupError> {
    let mut specs: Vec<fuz_actions::ActionSpec> = PROTOCOL_ACTION_SPECS();
    specs.extend(build_auth_spec_set(
        Arc::clone(&app.audit_emitter),
        app.socket_revoker(),
        action_account_rate_limiter,
        action_ip_rate_limiter,
        // a global admin offers, and never a scoped builtin role; the spine
        // refuses those too, so this is defense in depth
        Arc::new(crate::offer_authorize::CordOfferAuthorize),
        Arc::new(crate::roles::cord_role_registry()),
    ));
    specs.extend(crate::cord_actions::cord_action_specs(
        app.cord_invite_redeem_rate_limiter.clone(),
    ));
    // The `testing_cordd` seam for the `_testing_*` actions; production
    // passes no factory, so `fuz_testing` stays out of the `cordd` graph.
    if let Some(factory) = extra_action_specs_factory {
        let runtime = fuz_actions::ExtraActionSpecsRuntime {
            password_hasher: Arc::clone(&app.password_hasher),
            keyring: Arc::clone(&app.keyring),
            daemon_token_state: app.daemon_token_state.clone(),
            session_cookie_name: CORD_SESSION_COOKIE_NAME,
        };
        specs.extend(factory(Arc::new(app.clone()), runtime));
    }
    // Fail loud: booting with an empty registry would answer
    // `method_not_found` to everything.
    let registry = ActionRegistry::compile(specs)
        .map_err(|e| StartupError::Internal(format!("action registry compile failed: {e}")))?;
    Ok(Arc::new(registry))
}

/// State for `GET /api/admin/audit/stream`: admins only, over a cookie
/// session only, since the stream is a browser feed and close-on-revoke keys
/// on the session.
fn build_audit_stream_state(app: &CordApp, gate: AuthGate) -> fuz_realtime::AuditStreamRouteState {
    fuz_realtime::AuditStreamRouteState {
        gate,
        audit_sse: Arc::clone(&app.audit_sse),
        required_role: fuz_auth::ROLE_ADMIN,
        credential_gate: fuz_auth::CredentialGate::SESSION_ONLY,
    }
}

fn build_rpc_state(
    app: &CordApp,
    gate: AuthGate,
    registry: Arc<ActionRegistry>,
    account_rate_limiter: Option<Arc<RateLimiter>>,
    ip_rate_limiter: Option<Arc<RateLimiter>>,
) -> RpcRouteState {
    RpcRouteState {
        gate,
        registry,
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        notification_sender: app.notification_sender(),
        account_rate_limiter,
        ip_rate_limiter,
    }
}

fn build_ws_state(
    app: &CordApp,
    gate: AuthGate,
    registry: Arc<ActionRegistry>,
    account_rate_limiter: Option<Arc<RateLimiter>>,
    ip_rate_limiter: Option<Arc<RateLimiter>>,
) -> WsRouteState {
    WsRouteState {
        gate,
        registry,
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        notification_sender: app.notification_sender(),
        connection_registry: Arc::clone(&app.realtime),
        account_rate_limiter,
        ip_rate_limiter,
        // No role gate: the socket is a member surface, and the only
        // server-initiated pushes today are the spine's recipient-filtered
        // role-grant-offer notifications. Add roles here the moment a
        // privileged broadcast lands.
        required_roles: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
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
    fn test_env(database_url: Option<&str>) -> CordEnv {
        CordEnv::from_vars(|k| match k {
            "DATABASE_URL" => database_url.map(str::to_owned),
            "FUZ_ALLOWED_ORIGINS" => Some("http://localhost:*".to_owned()),
            "SECRET_FUZ_COOKIE_KEYS" => {
                Some("a-cookie-signing-key-that-is-long-enough-to-pass".to_owned())
            }
            _ => None,
        })
        .expect("env parses")
    }

    fn boot_router() -> GatedRouter {
        boot_only_router(&test_env(None))
    }

    fn lazy_app_over(database_url: &str, rate_limiters: RateLimiterMode) -> CordApp {
        let env = test_env(Some(database_url));
        CordApp::build(
            &env,
            Arc::new(fuz_auth::Argon2idHasher::new()),
            rate_limiters,
            None,
            false,
        )
        .expect("app builds without touching the DB")
    }

    fn full_router() -> GatedRouter {
        full_router_over(&lazy_app())
    }

    fn full_router_over(app: &CordApp) -> GatedRouter {
        let ready = fuz_http::ready_router(fuz_http::ReadyRouteState {
            pool: app.pool.clone(),
            expected: Arc::new(expected_ready_schema().expect("fixture parses")),
        });
        compose_full_router(app, Vec::new(), ready, None, RateLimiterMode::Enforced)
            .expect("router composes")
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
        let schema = expected_ready_schema().expect("committed fixture parses");
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
        let registry =
            build_action_registry(&lazy_app(), None, None, None).expect("registry compiles");
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
            vec![fuz_actions::public_readonly_spec(
                "_testing_seam_probe",
                |_params, _ctx| Box::pin(async { Ok(serde_json::json!({})) }),
            )]
        });
        let app = lazy_app();
        let registry =
            build_action_registry(&app, Some(factory), None, None).expect("registry compiles");
        assert!(registry.get("_testing_seam_probe").is_some());
        // the spine surface is still there alongside the extra spec
        assert!(registry.get("account_verify").is_some());

        let without = build_action_registry(&app, None, None, None).expect("registry compiles");
        assert!(without.get("_testing_seam_probe").is_none());
    }

    /// `testing_cordd`'s `_testing_drain_effects` is a barrier only over the
    /// tracked emitter: a fire-and-forget write still in flight (here, stuck
    /// dialing a server that accepts the connection and never answers) holds
    /// the drain, and the untracked production emitter's drain returns at
    /// once.
    #[tokio::test]
    async fn audit_inflight_tracking_makes_the_drain_wait_for_a_write() {
        use std::time::Duration;

        use fuz_auth::{AuditEventType, AuditLogInput};

        let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binds");
        let url = format!(
            "postgres://cord:cord@{}/cord_unused",
            silent.local_addr().expect("has an address")
        );
        let build = |tracking| {
            CordApp::build(
                &test_env(Some(&url)),
                Arc::new(fuz_auth::Argon2idHasher::new()),
                RateLimiterMode::Enforced,
                None,
                tracking,
            )
            .expect("app builds without touching the DB")
        };
        let emit = |app: &CordApp| {
            app.audit_emitter
                .emit(AuditLogInput::for_event(AuditEventType::Logout))
        };

        let tracked = build(true);
        let write = emit(&tracked);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                tracked.audit_emitter.drain_inflight()
            )
            .await
            .is_err(),
            "the drain waits while the write is in flight"
        );
        write.abort();
        tokio::time::timeout(
            Duration::from_secs(5),
            tracked.audit_emitter.drain_inflight(),
        )
        .await
        .expect("the drain resolves once the write ends");

        let untracked = build(false);
        let write = emit(&untracked);
        tokio::time::timeout(
            Duration::from_millis(200),
            untracked.audit_emitter.drain_inflight(),
        )
        .await
        .expect("untracked, the drain returns at once");
        write.abort();
    }
}

#[cfg(test)]
mod client_ip_posture {
    //! The empty-trusted-proxy posture [`super::compose_full_router`] relies
    //! on: `CORD_TRUSTED_PROXIES` is unset by default, so the composed router
    //! layers `client_ip_middleware` over an empty proxy set. These pin that
    //! the combination records the real TCP peer and never believes a forged
    //! `X-Forwarded-For`.

    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::Body;
    use axum::extract::{ConnectInfo, Extension};
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use fuz_http::{ClientIp, ParsedProxy, client_ip_middleware};
    use tower::ServiceExt;

    /// Drive one request through the composer's empty-set layer shape and
    /// return the `ClientIp` the middleware resolved.
    async fn resolved_client_ip(peer: &str, forwarded_for: Option<&str>) -> String {
        let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let proxies: Arc<Vec<ParsedProxy>> = Arc::new(Vec::new());
        let app = Router::new()
            .route(
                "/ip",
                get(move |Extension(ClientIp(ip)): Extension<ClientIp>| {
                    let sink = Arc::clone(&sink);
                    async move {
                        *sink.lock().expect("sink is uncontended") = Some(ip);
                        StatusCode::OK
                    }
                }),
            )
            .layer(from_fn_with_state(proxies, client_ip_middleware));

        let mut builder = Request::builder().uri("/ip").method("GET");
        if let Some(value) = forwarded_for {
            builder = builder.header("x-forwarded-for", value);
        }
        let mut req = builder.body(Body::empty()).expect("request builds");
        // `serve_with_shutdown` supplies `ConnectInfo` per connection; a
        // `oneshot` has no connection, so plumb the peer by hand
        req.extensions_mut().insert(ConnectInfo(
            peer.parse::<SocketAddr>().expect("peer parses"),
        ));

        let response = app.oneshot(req).await.expect("router responds");
        // a 500 would mean the `ClientIp` extension was missing
        assert_eq!(response.status(), StatusCode::OK, "ClientIp was populated");

        seen.lock()
            .expect("sink is uncontended")
            .clone()
            .expect("the middleware always populates ClientIp")
    }

    #[tokio::test]
    async fn no_forwarded_for_resolves_to_the_peer() {
        assert_eq!(
            resolved_client_ip("203.0.113.7:51234", None).await,
            "203.0.113.7"
        );
    }

    #[tokio::test]
    async fn forged_forwarded_for_from_an_untrusted_peer_is_ignored() {
        assert_eq!(
            resolved_client_ip("203.0.113.7:51234", Some("198.51.100.9")).await,
            "203.0.113.7"
        );
    }
}
