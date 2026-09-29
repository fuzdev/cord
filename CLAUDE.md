# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting.
The repo is a SvelteKit static frontend plus a Rust workspace whose server
crate, `crates/cord_server` (bin `cordd`), is the server skeleton over the
fuz spine: accounts, sessions, bootstrap, invite-gated signup, JSON-RPC,
WebSocket, health and readiness. The frontend is the auth shell over that
server (login, signup, bootstrap, account, admin). No Cord feature exists
yet; treat the current code as scaffolding, not product, and don't describe
planned features as working.

The intended shape, briefly:

- **Kinds**: a **cord** is the group ("join our cord"); it holds
  **spaces**, each with a kind and one or more views. A **line** is a live
  room (voice, video, screenshare, ephemeral text, no history; members join
  silent and each of mic, camera, and screen stays off until they turn it
  on). **chat** is persisted messages with unread markers. A **forum** holds
  threads with nested comments and reactions (a forum topic is a
  **thread**; sort order is a view choice). A **feed** is a Bluesky feed,
  list, or account the cord pins, rendered reddit-style or as a timeline.
- **Views**: two switchable desktop layouts (sidebar plus main, column
  dashboard) and a phone single column, all over the same spaces.
- **Accounts**: the instance's own (operator-gated signup, cookie
  sessions); each cord is invite-gated. Each cord has three fixed roles
  (owner, admin, member); a space is visible to all members or a listed
  subset. Any member may optionally connect a Bluesky or other atproto PDS
  account for personal feed columns and posting; the connection is linked
  to an account, not a way to log in. Nobody needs one, and shared feed
  spaces render for everyone. Members control their own
  presence visibility, and the instance is honest about what it can see.
- **Backend**: an API-only Rust server over the fuz spine crates (auth,
  db, actions, realtime, cells, facts), this SvelteKit static frontend
  served by nginx, Postgres, and coturn for WebRTC TURN (full mesh). The
  frontend ships with each instance, so there is no GitHub Pages deploy.
  The live room is built first; persisted spaces layer on it.
- **Self-hosting**: one instance hosts several cords; deployed by zap or
  docker compose. Not federation, not a hosted service, not
  engagement-driven (never streaks, scores, or reputation; no push at MVP).

Voice for user-facing copy: describe Cord by what it is, never by
contrast with other products. Warm, low-key, unhurried.

For coding conventions, see Skill(fuz-stack).

## Layout

- `src/routes/` — SvelteKit routes; `+layout.ts` prerenders everything for
  the static adapter (see Frontend below)
- `src/lib/` — frontend library code: `ui/rpc.ts` (the RPC client),
  `ui/cord_auth_state.svelte.ts` (the session state), `action_specs.ts` and
  the generated `action_*` modules
- `src/test/` — vitest tests (not co-located); `src/test/cross_backend/`
  holds the end-to-end suites against a spawned server (see Testing below)
- `static/` — files copied as-is into the frontend build
- `crates/cord_server/` — the server crate, library plus the `cordd`
  binary (see Rust below)
- `crates/testing_cord_server/` — the test-only `testing_cordd` binary
- `crates/xtask/` — `cargo xtask check-release`, aliased in
  `.cargo/config.toml`
- `.env.development.example` — the server's local env, copied to the
  gitignored `.env.development`

## Frontend

A static SvelteKit build over `cordd`'s API, using `@fuzdev/fuz_app` for the
auth UI and the RPC client. The root `+layout.svelte` provides the contexts
and checks the session on mount:

- `cord_rpc_context` — `CordRpc` from `ui/rpc.ts`: fuz_app's
  `create_frontend_rpc_client` over `POST /api/rpc`, with the throwing `api`
  and the Result-shaped `api_result` Proxies. HTTP only; the WebSocket
  transport joins when something needs server push.
- `cord_auth_state_context` — `CordAuthState` from
  `ui/cord_auth_state.svelte.ts`: fuz_app's `AuthState` (cookie session
  status, login, logout, bootstrap, signup against `/api/account/*`) plus
  `session_checked`, true once the first check settles. The same instance
  is set on fuz_app's `auth_state_context`, which its components read.
  Session-dependent views show "checking session..." until
  `session_checked`, so the prerendered HTML stays neutral and a signed-in
  visitor never sees the signed-out view flash. Signed-in views gate on
  `signed_in` (verified and the account loaded), which skips the gap after
  a successful login before the account arrives. Gates in front of
  fuz_app's forms don't wait on `verifying`, which is also true
  mid-submit: unmounting a form then loses its input on a failed attempt.

The typed surface is generated. `action_specs.ts` lists the specs the
server mounts (protocol, account, admin, role-grant offers; the protocol
actions run only over `/api/ws`), and
`action_collections.gen.ts` / `action_metatypes.gen.ts` generate
`action_collections.ts` and `action_metatypes.ts` (`FrontendActionsApi`)
from the same sources; `gro gen` regenerates them, and `gro check` fails on
drift. When the server mounts a new spec group, add it to `action_specs.ts`
and as a source row in `action_collections.gen.ts`, then run `gro gen`.
`src/test/action_specs.test.ts` pins the list and the generated modules to
each other; `src/test/ui/` tests `ui/`.

Routes, all prerendered anonymous and filled in client-side from the
session:

- `/` — the intro; signed in, who you are and links to your account and
  (for admins) the admin area
- `/login`, `/signup` — fuz_app's `LoginForm` and `SignupForm`; signup is
  invite-gated by default, matching an invite by username or email, and
  open when an admin turns on open signup
- `/account` — your account, `LogoutButton`, and `AccountSessions`
- `/admin` — the gate: a `/health` probe, then fuz_app's `BootstrapForm`
  while bootstrap is open, `LoginForm` when signed out, a notice for
  non-admins; admins get the shell with `invites`, `accounts`, `sessions`,
  and `audit log` (fuz_app's `Admin*` components over
  `provide_admin_rpc_contexts`); the audit log's live toggle streams from
  `GET /api/admin/audit/stream`

Library code imports relatively; routes and tests import it through the
`#lib/*` subpath import in `package.json`.

## Committing

`git add` and `git commit` are pre-approved here — commit short 1-liners
at sensible stopping points (`fix:` / `docs:` / `test:` / `feat:` /
`refactor:` / `chore:` prefixes, no body, no trailers).

## Gro commands

```bash
gro check     # typecheck, test, lint, format check (run before committing)
gro typecheck # typecheck only (faster iteration)
gro test      # run tests with vitest
gro build     # build for production (static adapter)
gro sync      # regenerate files and run svelte-kit sync
```

IMPORTANT for AI agents: Do NOT run `gro dev` — the developer manages the
dev server.

## Rust

For Rust crates, use cargo directly, not Gro:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-release
```

The workspace's crates are `cord_server` (the production `cordd`),
`testing_cord_server` (the test-only `testing_cordd`), and `xtask`. It
depends on the fuz Rust spine crates (`fuz_actions`, `fuz_auth`, `fuz_db`,
`fuz_http`, `fuz_realtime`, `fuz_sys`, plus the dev-only `fuz_testing`
and `fuz_audit`) as path deps into a sibling checkout of the fuz Rust
workspace, at the paths in the root `Cargo.toml`; they aren't published,
so cargo only resolves with that checkout present. For the same reason
GitHub CI runs only the frontend check, and the Rust gates run locally.
`default-members` is `cord_server` alone, so a bare `cargo build` (or
`cargo build --release`) builds only `cordd` and never leaves
`testing_cordd` in `target/`; `--workspace` (as in the gates above)
covers every crate.

The root `Cargo.toml` is the workspace: shared package metadata, workspace
dependencies, the release profile, and the fuz ecosystem's lints
(`unsafe_code = "forbid"`, clippy all/pedantic/nursery/cargo at warn,
`unwrap_used`/`expect_used`/`panic` at warn, allowed in tests by
`clippy.toml`). Crates opt in with `[lints] workspace = true`. The clippy
gate denies warnings, which is why `rust-toolchain.toml` pins the
toolchain. Exit
codes: `0` success, `2` the caller must fix something, `1` everything else.
`cargo xtask check-release` keeps fuz_audit's sysexits codes: `0` clean,
`65` a policy violation, `69`/`70` a tooling failure.

### cord_server

An API-only Axum server over the spine; it serves no static files (the
frontend is served by nginx in production and by Vite in dev, which
proxies `/api` and `/health` to `cordd`). Modules:

- `main.rs` — `cordd`: tracing init, the Argon2id hasher, `run_app`;
  `cordd db-status` is the spine's read-only migration report
- `runtime.rs` — `run_app` / `RunAppOptions` (hasher, default address
  `127.0.0.1:4061`, drain timeout, rate-limiter mode, and the test-only
  seams `cordd` passes as `None`: `extra_action_specs_factory`,
  `pre_migration_hook`, `daemon_token_state`), signal handling and
  graceful drain via `fuz_http`
- `env.rs` — `CordEnv`, read once through an injectable `from_vars`
  (tests pass a map; no process-env mutation)
- `app.rs` — `CordApp`, the composition root (pool, keyring, audit
  emitter, rate limiters, the realtime connection registry, the audit SSE
  registry)
- `router.rs` — the route composition: `/health`, `/ready`
  (`fuz_http::ready_router` over the committed `expected_schema.json`),
  `/api/account/*` (status, login, logout, password, bootstrap, signup),
  `/api/rpc` (GET + POST, protocol + account + admin actions), `/api/ws`,
  `/api/admin/audit/stream` (`fuz_realtime`'s audit SSE: admin role,
  cookie session only, closed on logout, password change, session revocation,
  or loss of the admin role);
  the migration chain (`cord_migration_namespaces`, today only the spine's
  auth migrations); the session cookie `cord_session`
- `error.rs` — `StartupError` with `.exit_code()` (`2` config, `1`
  otherwise) and `.hint()`

Without `DATABASE_URL`, `cordd` runs boot-only (`/health` and `/`).
With one, it requires `FUZ_ALLOWED_ORIGINS` (always) and
`SECRET_FUZ_COOKIE_KEYS`, runs migrations, then serves. The other vars:
`HOST`/`PORT`, `FUZ_BOOTSTRAP_TOKEN_PATH`, `CORD_TRUSTED_PROXIES`.
Spine-wide var names stay unprefixed; cord-only ones take `CORD_`.

Unit tests live in `#[cfg(test)]` modules and need no database: env
parsing, exit codes, the keyring guard, the action registry, and router
posture driven in-process with `tower::ServiceExt::oneshot` over a lazy
pool. Two Postgres-backed integration tests in `crates/cord_server/tests/`
are `#[ignore]`d; they reset the `public` schema of their database:

```bash
createdb cord_schema_ready_test
# drift guard for expected_schema.json, and the column-projection guard
cargo test -p cord_server -- --ignored
# regenerate expected_schema.json after a migration change
UPDATE_SCHEMA_READY=1 cargo test -p cord_server --test schema_ready -- --ignored
```

`CORD_SCHEMA_READY_TEST_URL` overrides their database URL.

### testing_cordd

`crates/testing_cord_server` builds `testing_cordd`, the same `run_app`
with the test-only pieces swapped in: `fuz_testing`'s fast
`TestingArgon2idHasher`, rate limiters in `DisabledForTesting`, a daemon
token written under `CORD_DAEMON_TOKEN_DIR` (required), the
`_testing_reset` action (wipes the auth tables and seeds a fresh keeper,
authenticated by that daemon token), and a pre-migration hook that drops
and recreates the `public` schema when `FUZ_TESTING_RESET_DB_ON_STARTUP`
is true. It binds `127.0.0.1:4161` by default. Never ship it: only this
crate links `fuz_testing`, the `testing_` prefix marks it unshippable, and
`cargo xtask check-release` (fuz_audit's dep-graph audit) fails if any
production binary reaches `fuz_testing` or `fuz_audit`. When a new test
seam is needed, add it to `RunAppOptions` with `cordd` passing `None`.

## Testing

`gro test` runs the in-process vitest `unit` project. The
`cross_backend_rust` project in `vite.config.ts` runs
`src/test/cross_backend/*.cross.test.ts` over real HTTP against a spawned
`testing_cordd`, through fuz_app's cross-backend harness: its global setup
spawns the binary with
`cargo run -p testing_cord_server --bin testing_cordd` (the `-p` is needed
since `default-members` is `cord_server` alone), bootstraps the keeper with the
bootstrap token, and each test fires `_testing_reset` for a fresh keeper.
`auth.cross.test.ts` covers cord's auth flows (bootstrap, login and
logout, invite-gated signup, the admin RPC gate, the origin allowlist, the
audit stream's gate); `origin.cross.test.ts` and
`audit_stream.cross.test.ts` run fuz_app's shared origin and SSE suites.
The backend config is `cord_backend_config.ts`.

It needs Postgres and is gated behind `FUZ_TEST_CROSS_BACKEND`, so a bare
`gro test` skips it:

```bash
createdb cord_test_rust
npm run test:cross   # FUZ_TEST_CROSS_BACKEND=1 vitest run --project cross_backend_rust
```

`CORD_CROSS_BACKEND_TEST_URL` overrides the database URL
(`postgres://localhost/cord_test_rust`). The binary wipes that database's
`public` schema on every boot, so point it only at a throwaway database.
