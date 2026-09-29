# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting.
The repo is a SvelteKit static frontend shell plus a Rust workspace whose
one crate, `crates/cord_server` (bin `cordd`), is the server skeleton over
the fuz spine: accounts, sessions, bootstrap, invite-gated signup, JSON-RPC,
WebSocket, health and readiness. No Cord feature exists yet; treat the
current code as scaffolding, not product, and don't describe planned
features as working.

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
  the static adapter
- `src/test/` — vitest tests (not co-located)
- `static/` — files copied as-is into the frontend build
- `crates/cord_server/` — the server crate, library plus the `cordd`
  binary (see Rust below)
- `.env.development.example` — the server's local env, copied to the
  gitignored `.env.development`

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
```

The workspace depends on the fuz Rust spine crates (`fuz_actions`,
`fuz_auth`, `fuz_db`, `fuz_http`, `fuz_realtime`, `fuz_sys`) as path deps
into a sibling checkout of the fuz Rust workspace, at the paths in the
root `Cargo.toml`; they aren't published, so cargo only resolves with that
checkout present. For the same reason GitHub CI runs only the frontend
check, and the Rust gates run locally.

### cord_server

An API-only Axum server over the spine; it serves no static files (the
frontend is served by nginx in production and by Vite in dev, which
proxies `/api` and `/health` to `cordd`). Modules:

- `main.rs` — `cordd`: tracing init, the Argon2id hasher, `run_app`;
  `cordd db-status` is the spine's read-only migration report
- `runtime.rs` — `run_app` / `RunAppOptions` (hasher, default address
  `127.0.0.1:4061`, drain timeout, rate-limiter mode), signal handling and
  graceful drain via `fuz_http`
- `env.rs` — `CordEnv`, read once through an injectable `from_vars`
  (tests pass a map; no process-env mutation)
- `app.rs` — `CordApp`, the composition root (pool, keyring, audit
  emitter, rate limiters, the realtime connection registry)
- `router.rs` — the route composition: `/health`, `/ready`
  (`fuz_http::ready_router` over the committed `expected_schema.json`),
  `/api/account/*` (status, login, logout, password, bootstrap, signup),
  `/api/rpc` (GET + POST, protocol + account + admin actions), `/api/ws`;
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

`CORD_SCHEMA_READY_TEST_URL` overrides their database URL. There is no
testing sibling binary yet, so the spine's test-only seams (the fast
hasher, `_testing_*` actions, the daemon token) aren't wired.

The root `Cargo.toml` is the workspace: shared package metadata, workspace
dependencies, the release profile, and the fuz ecosystem's lints
(`unsafe_code = "forbid"`, clippy all/pedantic/nursery/cargo at warn,
`unwrap_used`/`expect_used`/`panic` at warn, allowed in tests by
`clippy.toml`). Crates opt in with `[lints] workspace = true`. CI denies
warnings, which is why `rust-toolchain.toml` pins the toolchain. Exit
codes: `0` success, `2` the caller must fix something, `1` everything else.
