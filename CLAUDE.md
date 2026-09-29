# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting.
The repo is a SvelteKit static frontend plus a Rust workspace whose server
crate, `crates/cord_server` (bin `cordd`), is the server skeleton over the
fuz spine: accounts, sessions, bootstrap, invite-gated signup, JSON-RPC,
WebSocket, health and readiness, plus cords themselves (create, list, read,
rename), their invite links for people who already have an account, and
their members (list, role changes, remove, leave); see Cords below. The
frontend is the auth shell over that server (login, signup, bootstrap,
account, admin) with the cord list, a minimal cord page, and the admin
cords list; it has no invite, join, or member screens yet. Nothing happens
inside a cord yet (no spaces), and a link can't bring someone new onto the
instance; don't describe planned features as working.

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
  the static adapter except the dynamic cord page (see Frontend below)
- `src/lib/` — frontend library code: `ui/rpc.ts` (the RPC client),
  `ui/cord_auth_state.svelte.ts` (the session state),
  `ui/cords_state.svelte.ts` (cord list, one cord, the admin list),
  `ui/CordNameForm.svelte`, `cord_action_specs.ts` and `cord_roles.ts`
  (the cord wire), `action_specs.ts` and the generated `action_*` modules
- `src/test/` — vitest tests (not co-located); `src/test/cross_backend/`
  holds the end-to-end suites against a spawned server (see Testing below)
- `static/` — files copied as-is into the frontend build
- `crates/cord_server/` — the server crate, library plus the `cordd`
  binary (see Rust below), and its migration fixture
  `expected_schema.json`
- `crates/testing_cord_server/` — the test-only `testing_cordd` binary
- `crates/xtask/` — `cargo xtask check-release`, aliased in
  `.cargo/config.toml`
- `.env.development.example` — the server's local env, copied to the
  gitignored `.env.development`

## Cords

A cord is a spine cell (`fuz_cell`) with `kind = "cord"`: a private root
(no parent), its name in `data`, and `created_by = NULL`. The null owner
column is deliberate: cords are structural, so the spine's owner branch
admits no one and role grants are the only authority. The creator is
recorded in the `cord_create` audit event instead.

- **Membership** is spine role grants: `cord_owner`, `cord_admin`, or
  `cord_member`, each scoped to the cord cell (`scope_kind: "cord"`,
  `scope_id` the cord id). One role per (actor, cord), kept by Cord's
  verbs, not the spine (its unique index is per actor, role, and scope):
  `cord_create` grants a new cord's first owner, accepting an invite adds
  a grant only for someone not already in the cord, and a role change
  revokes the member's cord grants and creates the new one within one
  transaction. `roles.rs` registers the three in cord's `RoleRegistry`
  with no grant paths, so the spine's admin role UI, `role_grant_assign`,
  and the role-grant offer flow refuse them; only Cord's verbs confer
  them, calling the spine's `query_create_role_grant` directly.
- **Visibility** is stamped: in the same transaction as the cord, three
  role-shaped `cell_grant` rows go on the cord cell (`cord_member` viewer,
  `cord_admin` and `cord_owner` editor, each scoped to the cord), one per
  role and never one per member. They make the spine agree with Cord on
  **viewing** only: `can_view_cell`, `cell_list`, and fact serving admit
  the cord's members (and also a global `admin`, through the spine's admin
  bypass, though nothing cord mounts serves cells through them). They
  enable **no** spine edit path: `can_edit_cell` treats a
  `created_by = NULL` cell as admin-only and ignores editor grants. Cord
  cells change only through Cord's verbs; later slices must not rely on
  the spine's cell edit paths for them.
- **Authz rule: exact-scope role checks; never `has_role` for cord
  roles.** `RequestActorContext::has_role` ignores scope, so an owner of
  one cord would pass for every cord. `cord_authz.rs` holds the pure
  exact-scope predicates (`cord_role_in`, `is_cord_member`,
  `can_manage_cord`, `cord_memberships`) over the request's role-grant
  snapshot, and the member rules (`CordRole::can_set_member_roles`,
  `CordRole::can_remove`, `leaves_no_owner`). Instance-admin checks use
  `has_scoped_role(ROLE_ADMIN, None)` (a global grant); the dispatcher's
  spec-level role gate also admits only a global grant.
- **Instance admins and cords**: admins create cords and see the ops list,
  but get no implicit access; a cord they aren't in stays out of their
  normal UI, and every verb that names a cord they aren't in answers them
  `cord_not_found`.
- **Membership changes serialize per cord.** Every change (role set,
  remove, leave, invite accept) locks the cord's cell row
  (`FOR NO KEY UPDATE`), then reads the membership afresh and applies the
  rules to that read, not to the request's snapshot. So the last-owner rule
  can't be raced, and a change whose caller was demoted by a change that
  committed first is judged by the caller's new role. This relies on READ
  COMMITTED (the default): each statement after the lock wait takes a
  fresh snapshot. A member is an actor with an active cord grant on a live
  actor and a live account; soft-deleted ones keep their grants but aren't
  listed, targeted, or counted as owners.
- **The other managing writes re-read the caller's role.** Rename and
  invite create and revoke take no cord lock; they read the caller's cord
  role with a plain SELECT inside their transaction
  (`query_cord_manager_role`), since the request's role-grant snapshot is
  taken before the transaction opens. An admin demoted or removed before
  that read can't rename the cord or mint a link to rejoin through; a
  demotion committing while the verb runs is ordered after it. Revoke
  must not lock the cord: accept locks the link and then the cord, so the
  reverse order would deadlock. The read-only verbs (`cord_get`,
  `cord_invite_list`, `cord_member_list`) trust the snapshot.
- **No scoped builtin roles.** A builtin role (`admin`, `keeper`) is
  only ever granted globally. The spine enforces this on every grant path
  (`role_grant_offer_create`, `role_grant_assign`,
  `role_grant_offer_accept` refuse a scoped `admin` with
  `role_grant_builtin_scoped`, and `keeper` is never grantable over the
  web), and its admin gates and admin bypasses read only the global
  grant. Cord keeps its own gate as defense in depth:
  `offer_authorize.rs`'s `CordOfferAuthorize`, threaded into
  `build_auth_spec_set` in place of the spine's `AdminOfferAuthorize`,
  requires a global-admin caller and refuses any offer of a builtin role
  that carries a scope. Cross-backend tests pin the spine's refusals.

The verbs (`cord_actions.rs`, domain actions over `cord_queries.rs`; the
generic `fuz_cell_actions` verb set is not mounted):

- `cord_create` — instance admins only; the cell, the caller's
  `cord_owner` grant, and the stamped grants in one transaction; audits
  `cord_create` with the cord id, name, and owner `role_grant_id`
- `cord_list` — the caller's cords with their role in each, from their
  cord-scoped role grants
- `cord_get` — members only; anyone else gets `cord_not_found`, the same
  answer as a missing cord (the IDOR mask)
- `cord_update` — rename; the owner or admins, by their role read afresh
  (`cord_not_found` for non-members, `cord_update_forbidden` for members);
  audits `cord_update`
- `cord_admin_list` — instance admins only: every cord with its member
  count (live actors on live accounts), for operations

Invite links (`cord_invite_actions.rs` over `cord_invite_queries.rs`) are
rows in cord's own `cord_invite` table, the first table in its `cord`
migration namespace (`migrations.rs`, chained after the spine's auth and
cell namespaces). A link stores only the blake3 digest of its token, made
with the spine's session-token helpers; the raw token is returned once. A
link confers `cord_member` (a CHECK holds the role column to it), may
expire (from 60 seconds to 365 days) and may carry a use limit (1 to
1000); the table refuses a use past the limit. Its status is derived:
revoked, else expired (against the database clock), else exhausted, else
active. `admits_newcomers` marks a link that may bring someone with no
account onto the instance; nothing sets it yet, so every link works for
existing accounts only. Links belong to the cord: they cascade with its
cell and outlive their minter's membership.

- `cord_invite_create` — the owner or admins; optional
  `expires_in_seconds` and `max_uses`; returns the link and its token;
  audits `cord_invite_create` (never the token); rate-limited per account,
  the spine's account class, as on `role_grant_assign`
- `cord_invite_list` — the owner or admins; the cord's links with their
  status, newest first, never their tokens
- `cord_invite_revoke` — the owner or admins; idempotent (a second revoke
  succeeds and changes nothing); audits `cord_invite_revoke` when it
  revokes
- `cord_invite_preview` — any signed-in account holding a token: the
  link's status, the caller's role in the cord if any, and the cord's id
  and name only when the link is active or the caller is in the cord
- `cord_invite_accept` — cookie session only; locks the link, then the
  cord; a caller already in the cord gets their role back and spends no
  use, whatever the link's state; otherwise an active link grants
  `cord_member` and spends a use; audits the spine's `role_grant_create`
  and `cord_invite_accept`. The grant's `granted_by` is the link's minter,
  who may since have left the cord.

The token is the secret, so a real token's refusals name their reason
(`cord_invite_revoked`, `cord_invite_expired`, `cord_invite_exhausted`,
all `conflict`), and any other string gets `cord_invite_not_found`. A
member who can't manage links gets `cord_invite_forbidden`.

The member verbs (`cord_member_actions.rs` over `cord_member_queries.rs`):

- `cord_member_list` — any member: each member's actor id, username,
  display name (the actor's name, `null` when blank), and role, owners
  first, then by username
- `cord_member_role_set` — owners only; sets any member, the caller
  included, to owner, admin, or member; several owners are fine, none is
  not
- `cord_member_remove` — an owner removes anyone else, an admin removes
  plain members, a member removes no one; nobody removes themself
  (`cord_member_remove_self`, `invalid_params`), whatever their role, so
  a departure is audited as a leave. An owner may still step down
  through `cord_member_role_set`, while another owner remains
- `cord_leave` — any member

None of them may leave a cord without an owner (`cord_last_owner`,
`conflict`). Other refusals: `cord_member_forbidden` for a caller whose
role can't make the change, `cord_member_not_found` for a target outside
the cord. Each change audits the spine's `role_grant_revoke` per revoked
grant (with `reason` `cord_member_role_set`, `cord_member_remove`, or
`cord_leave`) and, for a role change, `role_grant_create`. Everyone
outside a cord gets `cord_not_found` from every invite and member verb
that names it, as from `cord_get`.

A cord name refuses, anywhere, the control characters (Cc) and the set
(Cf ∪ `Default_Ignorable_Code_Point` ∪ {U+2028, U+2029}) − {U+200C,
U+200D} − [U+FE00, U+FE0F]: every format character and every character
Unicode says to render invisibly, plus the line and paragraph separators,
except the join characters (ZWNJ and ZWJ, which Persian spelling, Indic
conjuncts, and joined emoji need) and the emoji variation selectors. A join
character may not sit at either end of the name or beside whitespace or
another join character, so it can't pad. The tag characters stay refused,
at the cost of the three subdivision flags (England, Scotland, Wales). The
name is then trimmed and must be 1 to 100 characters by code point.
Checking before trimming keeps the twins exact where JS and Rust trim
different characters. The set is an explicit range list,
`CORD_NAME_FORBIDDEN_CHAR_RANGES`, in `cord_queries.rs` and
`cord_action_specs.ts`; a unit test checks the two are identical and that
the list equals the formula over the Unicode properties, both directions.
The TS check also refuses lone surrogates, which a Rust `String` can't
hold.

The TS twin of the wire is `src/lib/cord_action_specs.ts` (with the roles
in `cord_roles.ts`); keep them in step with the Rust handlers.

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
  `is_instance_admin` is true only for a global `admin` grant, the rule the
  server applies; `roles` ignores scope, so don't gate admin UI on it.

The typed surface is generated. `action_specs.ts` lists the specs the
server mounts (protocol, account, admin, role-grant offers, and cord's own
from `cord_action_specs.ts`; the protocol actions run only over `/api/ws`),
and
`action_collections.gen.ts` / `action_metatypes.gen.ts` generate
`action_collections.ts` and `action_metatypes.ts` (`FrontendActionsApi`)
from the same sources; `gro gen` regenerates them, and `gro check` fails on
drift. When the server mounts a new spec group, add it to `action_specs.ts`
and as a source row in `action_collections.gen.ts`, then run `gro gen`.
`src/test/action_specs.test.ts` pins the list and the generated modules to
each other; `src/test/ui/` tests `ui/`.

Routes, prerendered anonymous and filled in client-side from the session:

- `/` — the intro; signed in, your cords (linking to each cord's page) and
  links to your account and (for instance admins) the admin area. With no
  cords, an instance admin gets a "create your first cord" form (and a
  "new cord" form once they have some); anyone else is told to ask for an
  invite
- `/c/[cord_id]` — a cord's page: its name, your role, a placeholder for
  spaces, and a rename form for its owner and admins. The one route not
  prerendered (`prerender = false`, `ssr = false`); the static build's
  `spa.html` fallback serves it, so the web server must fall back to
  `spa.html` for paths with no prerendered file
- `/login`, `/signup` — fuz_app's `LoginForm` and `SignupForm`; signup is
  invite-gated by default, matching an invite by username or email, and
  open when an admin turns on open signup
- `/account` — your account, `LogoutButton`, and `AccountSessions`
- `/admin` — the gate: a `/health` probe, then fuz_app's `BootstrapForm`
  while bootstrap is open, `LoginForm` when signed out, a notice for
  non-admins; admins get the shell with `invites`, `accounts`, `sessions`,
  and `audit log` (fuz_app's `Admin*` components over
  `provide_admin_rpc_contexts`), and `cords` (`/admin/cords`, the
  `cord_admin_list` ops table); the audit log's live toggle streams from
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
depends on the fuz Rust spine crates (`fuz_actions`, `fuz_auth`,
`fuz_cell`, `fuz_db`, `fuz_http`, `fuz_realtime`, `fuz_sys`, plus the
dev-only `fuz_testing` and `fuz_audit`) as path deps into a sibling
checkout of the fuz Rust workspace, at the paths in the root
`Cargo.toml`; they aren't published, so cargo only resolves with that
checkout present. For the same reason
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
  `/api/rpc` (GET + POST, protocol + account + admin + cord actions),
  `/api/ws`,
  `/api/admin/audit/stream` (`fuz_realtime`'s audit SSE: admin role,
  cookie session only, closed on logout, password change, session revocation,
  or loss of the admin role);
  the migration chain (`cord_migration_namespaces`: the spine's auth and
  cell namespaces, then cord's own); the action registry, with the
  spine's auth spec set built over cord's `RoleRegistry`; the session
  cookie `cord_session`
- `roles.rs` — the cord roles, `CORD_SCOPE_KIND`, `cord_role_registry`
- `migrations.rs` — cord's `cord` migration namespace (`CORD_MIGRATIONS`,
  the `cord_invite` table) and its column-projection registry
  (`CORD_COLUMN_PROJECTIONS`)
- `cord_authz.rs` — `CordRole`, the exact-scope predicates, and the member
  rules (pure, unit tested)
- `cord_queries.rs` — cords over the `cell` / `cell_grant` / `role_grant`
  tables: create (cell + owner grant + stamped grants), get, by ids,
  rename, the admin list with member counts; `validate_cord_name`
- `cord_member_queries.rs` — membership over `role_grant`: the cord lock,
  the member read, and the role set, remove, and leave transactions
- `cord_invite_queries.rs` — the `cord_invite` table: tokens and their
  digests, create, list, revoke, lookup, and the accept transaction
- `cord_actions.rs` — the cord verbs' specs and handlers, the helpers the
  other action modules share, and `cord_action_specs` (every cord spec)
- `cord_invite_actions.rs`, `cord_member_actions.rs` — the invite and
  member verbs
- `offer_authorize.rs` — `CordOfferAuthorize`, the role-grant offer gate
  (global admins only; no scoped builtin roles)
- `error.rs` — `StartupError` with `.exit_code()` (`2` config, `1`
  otherwise) and `.hint()`

Without `DATABASE_URL`, `cordd` runs boot-only (`/health` and `/`).
With one, it requires `FUZ_ALLOWED_ORIGINS` (always) and
`SECRET_FUZ_COOKIE_KEYS`, runs migrations, then serves. The other vars:
`HOST`/`PORT`, `FUZ_BOOTSTRAP_TOKEN_PATH`, `CORD_TRUSTED_PROXIES`.
Spine-wide var names stay unprefixed; cord-only ones take `CORD_`.

Unit tests live in `#[cfg(test)]` modules and need no database: env
parsing, exit codes, the keyring guard, the action registry, cord authz,
the member rules, invite status and limits, name validation, and router
posture driven in-process with
`tower::ServiceExt::oneshot` over a lazy pool. The Postgres-backed
integration tests in `crates/cord_server/tests/` are `#[ignore]`d; they
reset the `public` schema of their database: `schema_ready.rs` (the
`expected_schema.json` drift guard), `columns.rs` (the column-projection
guard), `cords.rs` (a created cord's cell, owner grant, and stamped
grants, and the spine's `cell_list` admitting exactly its members),
`cord_members.rs` (one role per member through role changes, the
permission tiers and last-owner rule, no self-removal, the managing
verbs' fresh role read against a stale snapshot, two owners leaving at
once, an accept racing a removal and a role change on one actor), and
`cord_invites.rs` (the stored digest and the table's CHECKs, accept and
its statuses, the cascades, concurrent accepts never overshooting the use
limit). `tests/common/` holds their shared setup:

```bash
createdb cord_schema_ready_test
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
authenticated by that daemon token, then truncates `cord_invite` and the
cell tables so no cord or link outlives a test), cord's own
`_testing_cord_invite_expire` (backdates a link's expiry, under the same
daemon-token gate), and a pre-migration hook that drops
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
audit stream's gate); `cords.cross.test.ts` covers the cord verbs (an
admin creates and owns a cord, non-admins can't create, non-members can't
see or probe a cord, a global admin outside a cord sees it only in the ops
list, the spine can't confer cord roles or mint a scoped `admin`, the
admin list is admin-only); `cord_members.cross.test.ts` covers invite
links and members (joining through a link, accept taking a cookie session
only, revoked, expired, and exhausted links, one answer for an unknown
token, who manages links and how a demoted or removed admin loses that,
the role, remove, and leave tiers, no self-removal, the last-owner rule,
and `cord_not_found` from every verb to anyone outside the cord);
`origin.cross.test.ts` and
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
