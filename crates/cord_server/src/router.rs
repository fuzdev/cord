//! Top-level `axum::Router` composition for `cordd`.
//!
//! Composes the spine's route factories:
//!
//! - `fuz_auth::account_router` → `/api/account/{status,login,logout,password}`
//! - `fuz_auth::bootstrap_routes::bootstrap_router` → `/api/account/bootstrap`
//! - `fuz_auth::signup_routes::signup_router` → `/api/account/signup`
//! - `fuz_actions::create_rpc_router` → `/api/rpc` (GET + POST)
//! - `fuz_actions::register_action_ws` → `/api/ws`
//! - `fuz_realtime::audit_stream_router` → `/api/admin/audit/stream`
//! - `fuz_http::ready_router` → `/ready`
//!
//! plus cord's own `/health` and `/`. Middleware applied once at the outer
//! router: client IP resolution, the origin allowlist, and request tracing,
//! with per-group body limits on the API routes. The client-IP layer mounts
//! unconditionally: with no trusted proxies it resolves to the TCP peer and
//! ignores `X-Forwarded-For`, while leaving it off would collapse every caller
//! onto one shared bucket in the IP-keyed rate limiters.
//!
//! ## Migrations
//!
//! cord owns no tables of its own yet; it runs the spine's
//! `fuz_auth::AUTH_MIGRATIONS` via `fuz_db::run_migrations`. A `cord`
//! namespace lands here when the first cord table does.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use fuz_actions::{
    ActionRegistry, PROTOCOL_ACTION_SPECS, RpcRouteState, WsRouteState,
    auth_adapter::build_auth_spec_set, create_rpc_router, register_action_ws,
};
use fuz_auth::{AccountRouteState, RateLimiter, RateLimiterMode};
use fuz_http::{
    DEFAULT_BODY_LIMIT_BYTES, ParsedProxy, body_limit_layer, client_ip_middleware, origin_layer,
    request_tracing_layer,
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
pub const fn cord_migration_namespaces() -> [fuz_db::MigrationNamespace; 1] {
    [fuz_auth::AUTH_MIGRATIONS]
}

/// Build the top-level router.
///
/// Falls back to a boot-only router (`/health` and `/` only) when
/// `env.database_url` is absent, so the binary can be smoke-tested without a
/// live Postgres.
///
/// `daemon_token_state`, `extra_action_specs_factory`, and
/// `pre_migration_hook` are the `testing_cordd` seams; production passes
/// `None` for each.
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
    extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    pre_migration_hook: Option<PreMigrationHook>,
) -> Result<Router, StartupError> {
    if env.database_url.is_none() {
        tracing::warn!("DATABASE_URL not set; running in boot-only mode (no /api, no /ready)");
        return Ok(boot_only_router());
    }

    let app = CordApp::build(env, password_hasher, rate_limiters, daemon_token_state)?;

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
    // matching live `/api/ws` connections.
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

    compose_full_router(
        &app,
        env.trusted_proxies.clone(),
        ready,
        extra_action_specs_factory,
        rate_limiters,
    )
}

/// The committed `expected_schema.json` (the column map of cord's full
/// migration chain), served by the `/ready` probe. Regenerated and
/// drift-guarded by `tests/schema_ready.rs`.
fn expected_ready_schema() -> Result<fuz_db::ExpectedSchema, StartupError> {
    const EXPECTED_SCHEMA_JSON: &str = include_str!("../expected_schema.json");
    serde_json::from_str(EXPECTED_SCHEMA_JSON)
        .map_err(|e| StartupError::Internal(format!("expected_schema.json failed to parse: {e}")))
}

/// The router when no database is configured.
fn boot_only_router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/", get(root))
        .fallback(not_found)
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
) -> Result<Router, StartupError> {
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
    let api_account = fuz_auth::account_router(build_account_state(app))
        .merge(fuz_auth::bootstrap_routes::bootstrap_router(
            build_bootstrap_state(app),
        ))
        .merge(fuz_auth::signup_routes::signup_router(build_signup_state(
            app,
            rate_limiters,
        )))
        .layer(body_limit_layer(DEFAULT_BODY_LIMIT_BYTES));

    // RPC and WebSocket nest under `/api`.
    let api = Router::new()
        .merge(create_rpc_router(build_rpc_state(
            app,
            Arc::clone(&registry),
            action_account_rl.clone(),
            action_ip_rl.clone(),
        )))
        .merge(register_action_ws(build_ws_state(
            app,
            registry,
            action_account_rl,
            action_ip_rl,
        )))
        .layer(body_limit_layer(DEFAULT_BODY_LIMIT_BYTES));

    // The admin audit-log SSE stream, mounted at its absolute path.
    let audit_stream = fuz_realtime::audit_stream_router(build_audit_stream_state(app));

    let router = Router::new()
        .route("/health", get(health))
        .route("/", get(root))
        .merge(ready)
        .nest("/api/account", api_account)
        .nest("/api", api)
        .merge(audit_stream);

    if trusted_proxies.is_empty() {
        tracing::info!("no trusted proxies configured; XFF ignored, client IP is the TCP peer");
    } else {
        tracing::info!(
            count = trusted_proxies.len(),
            "trusted proxies configured; XFF resolution enabled"
        );
    }

    Ok(router
        .layer(from_fn_with_state(
            Arc::new(trusted_proxies),
            client_ip_middleware,
        ))
        .layer(from_fn_with_state(
            Arc::clone(&app.allowed_origins),
            origin_layer,
        ))
        .layer(request_tracing_layer())
        .fallback(not_found))
}

fn build_account_state(app: &CordApp) -> AccountRouteState {
    AccountRouteState {
        pool: app.pool.clone(),
        keyring: Arc::clone(&app.keyring),
        password_hasher: Arc::clone(&app.password_hasher),
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        allowed_origins: Arc::clone(&app.allowed_origins),
        bootstrap_available: Arc::clone(&app.bootstrap_available),
        login_ip_rate_limiter: app.login_ip_rate_limiter.clone(),
        login_account_rate_limiter: app.login_account_rate_limiter.clone(),
        daemon_token_state: app.daemon_token_state.clone(),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
    }
}

fn build_bootstrap_state(app: &CordApp) -> fuz_auth::BootstrapRouteState {
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
    fuz_auth::BootstrapRouteState {
        options,
        keyring: Arc::clone(&app.keyring),
        allowed_origins: Arc::clone(&app.allowed_origins),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
    }
}

/// Signup is invite-gated by default (`app_settings.open_signup = false`) and
/// rate-limited on both axes anyway, so the limiters already stand if an
/// admin opens signup.
fn build_signup_state(app: &CordApp, rate_limiters: RateLimiterMode) -> fuz_auth::SignupRouteState {
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
        keyring: Arc::clone(&app.keyring),
        allowed_origins: Arc::clone(&app.allowed_origins),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
    }
}

/// Compile the action registry: the spine's protocol actions plus its
/// account, admin, and role-grant-offer actions over the builtin roles
/// (`keeper`, `admin`), then any extra specs from `testing_cordd`.
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
        Arc::new(fuz_auth::AdminOfferAuthorize),
        Arc::new(fuz_auth::RoleRegistry::default()),
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
fn build_audit_stream_state(app: &CordApp) -> fuz_realtime::AuditStreamRouteState {
    fuz_realtime::AuditStreamRouteState {
        pool: app.pool.clone(),
        keyring: Arc::clone(&app.keyring),
        daemon_token_state: app.daemon_token_state.clone(),
        audit_sse: Arc::clone(&app.audit_sse),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
        required_role: fuz_auth::ROLE_ADMIN,
        credential_gate: fuz_auth::CredentialGate::SESSION_ONLY,
    }
}

fn build_rpc_state(
    app: &CordApp,
    registry: Arc<ActionRegistry>,
    account_rate_limiter: Option<Arc<RateLimiter>>,
    ip_rate_limiter: Option<Arc<RateLimiter>>,
) -> RpcRouteState {
    RpcRouteState {
        pool: app.pool.clone(),
        keyring: Arc::clone(&app.keyring),
        daemon_token_state: app.daemon_token_state.clone(),
        allowed_origins: Arc::clone(&app.allowed_origins),
        registry,
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        notification_sender: app.notification_sender(),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
        account_rate_limiter,
        ip_rate_limiter,
    }
}

fn build_ws_state(
    app: &CordApp,
    registry: Arc<ActionRegistry>,
    account_rate_limiter: Option<Arc<RateLimiter>>,
    ip_rate_limiter: Option<Arc<RateLimiter>>,
) -> WsRouteState {
    WsRouteState {
        pool: app.pool.clone(),
        keyring: Arc::clone(&app.keyring),
        daemon_token_state: app.daemon_token_state.clone(),
        allowed_origins: Arc::clone(&app.allowed_origins),
        registry,
        audit: Arc::clone(&app.audit_emitter),
        socket_revoker: app.socket_revoker(),
        notification_sender: app.notification_sender(),
        connection_registry: Arc::clone(&app.realtime),
        session_cookie_name: CORD_SESSION_COOKIE_NAME,
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

    /// Send one request, with the TCP peer `serve_with_shutdown` would supply,
    /// and return the status and body.
    async fn send(
        router: Router,
        method: &str,
        uri: &str,
        origin: Option<&str>,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        let mut req = builder.body(Body::empty()).expect("request builds");
        req.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:50000"
                .parse::<std::net::SocketAddr>()
                .expect("peer parses"),
        ));
        let res = router.oneshot(req).await.expect("router responds");
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
        let env = CordEnv::from_vars(|k| match k {
            "DATABASE_URL" => Some("postgres://localhost/cord_unused".to_owned()),
            "FUZ_ALLOWED_ORIGINS" => Some("http://localhost:*".to_owned()),
            "SECRET_FUZ_COOKIE_KEYS" => {
                Some("a-cookie-signing-key-that-is-long-enough-to-pass".to_owned())
            }
            _ => None,
        })
        .expect("env parses");
        CordApp::build(
            &env,
            Arc::new(fuz_auth::Argon2idHasher::new()),
            RateLimiterMode::Enforced,
            None,
        )
        .expect("app builds without touching the DB")
    }

    fn full_router() -> Router {
        let app = lazy_app();
        let ready = fuz_http::ready_router(fuz_http::ReadyRouteState {
            pool: app.pool.clone(),
            expected: Arc::new(expected_ready_schema().expect("fixture parses")),
        });
        compose_full_router(&app, Vec::new(), ready, None, RateLimiterMode::Enforced)
            .expect("router composes")
    }

    #[tokio::test]
    async fn boot_only_router_serves_health() {
        let (status, body) = send(boot_only_router(), "GET", "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn boot_only_router_mounts_no_api_or_ready() {
        for uri in ["/api/rpc", "/api/account/status", "/ready"] {
            let (status, _) = send(boot_only_router(), "GET", uri, None).await;
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
            let (status, _) = send(
                full_router(),
                method,
                uri,
                Some("https://elsewhere.example"),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
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
    fn expected_schema_parses_and_covers_the_migration_chain() {
        let schema = expected_ready_schema().expect("committed fixture parses");
        // the tracker must be expected, so a never-migrated DB fails readiness
        for table in ["schema_version", "account", "actor", "auth_session"] {
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
