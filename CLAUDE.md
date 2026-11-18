# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is underway: a
SvelteKit static frontend over `cordd`, an API-only Rust server on the fuz
spine, with accounts, cords, their members, and invite links (including ones
that bring someone new onto the instance). The server keeps a cord's spaces
(each a line, by kind), but no page shows them yet and nothing happens inside
one: no live room. Don't describe planned features as working.

## Vocabulary and voice

- **Kinds**: a **cord** is the group ("join our cord"); it holds
  **spaces**, each with a kind and one or more views. A **line** is a live
  room (voice, video, screenshare, ephemeral text, no history; members join
  silent and each of mic, camera, and screen stays off until they turn it
  on). **chat** is persisted messages with unread markers. A **forum** holds
  threads with nested comments and reactions (a forum topic is a
  **thread**; sort order is a view choice). A **feed** is a Bluesky feed,
  list, or account the cord pins, shown as a list of posts that each open
  into their thread, or as a timeline.
- **Views**: two switchable desktop layouts (sidebar plus main, column
  dashboard) and a phone single column, all over the same spaces.
- **Accounts**: the instance's own (operator-gated signup, cookie
  sessions); each cord is invite-gated, with three fixed roles (owner,
  admin, member); a space is visible to all members or a listed subset. A
  member may connect a Bluesky or other atproto PDS account for personal
  feed columns and posting, linked to their account, never a way to log in.
  Members control their own presence visibility, and the instance is honest
  about what it can see.
- **Backend**: the API-only server, this frontend served by nginx,
  Postgres, and, once the live room lands, coturn for WebRTC TURN (full
  mesh). The live room is built first; persisted spaces layer on it.
- **Self-hosting**: one instance hosts several cords; deployed by docker
  compose or zap. Not federation, not a hosted service, not
  engagement-driven: never streaks, scores, or reputation, and no push
  notifications yet.

Voice for user-facing copy: describe Cord by what it is, never by contrast
with other products. Warm, low-key, unhurried. For coding conventions, see
Skill(fuz-stack).

## Layout

- `src/routes/` — SvelteKit routes (see Frontend)
- `src/lib/` — `cord_action_specs.ts` (the cord wire's TS twin),
  `cord_roles.ts` (the roles and member rules' twin), `action_specs.ts` and
  the generated `action_*` modules; `ui/` holds `rpc.ts`, `cord_errors.ts`
  (refusal reasons to words), the `*_state.svelte.ts` classes, and the
  components (`CordNav`, the rail; `Tabs` and `Tab`, the route tabs;
  `CordNameForm`, `CordMembers`, `CordInvites`, `CordInviteForm`,
  `CordNewcomerForm`; `MediaCheck` and its `Media*` parts, the a/v check,
  with `media_output.ts` routing sound to a chosen output)
- `src/test/` — vitest tests; `cross_backend/` holds the end-to-end suites
- `crates/cord_server/` — the server library and `cordd`, plus the
  `expected_schema.json` fixture; `tests/` holds the Postgres suites
- `crates/testing_cord_server/` — the test-only `testing_cordd`
- `crates/xtask/` — `cargo xtask check-release`
- `.env.development.example` — the server's local env, copied to the
  gitignored `.env.development`
- `zap.ts`, the `package.json` zap scripts, `.env.production.example`, `DEPLOY.md` — the
  zap deploy (see Deploying)

`cord_server`'s modules (each module doc holds the detail):

- `main.rs` (`cordd`, `cordd db-status`; the strict command line — anything
  else exits `2`), `runtime.rs` (`run_app`, `RunAppOptions`, the env —
  the spine's `BaseServerEnv` with `CORD_TRUSTED_PROXIES` — and
  `upkeep_options`: the spine's auth cleanup and the invite expiry sweep, run
  beside the server and joined on shutdown; the auth cleanup closes the
  sockets and audit streams of each expired session it deletes), over the
  spine's server toolkit (`fuz_actions::server`: signals, the READ COMMITTED
  boot check before migrations, the shutdown close — a 1001 to every live
  WebSocket and the end of every audit stream — and drain), `app.rs`
  (`CordApp`, the composition root: the spine's `AppBackend` beside the
  invite limiter)
- `router.rs` — the routes, the `fuz_http::compose_stack` groups (one
  `api` group and the fallback, both capped at 1 MiB and origin-checked, so
  every path refuses a foreign `Origin` and answers a declared over-cap body
  with `413` first), migration chain, action registry, and the `cord_session`
  cookie
- `roles.rs`, `cord_authz.rs` — the cord roles; the exact-scope predicates
  and member rules (pure)
- `cord_queries.rs`, `cord_actions.rs` — cords, and the helpers the other
  action modules share (audit, the fresh manager-role gate)
- `cord_member_queries.rs`, `cord_member_actions.rs` — membership, the
  live-grant SQL builders, the cord lock
- `cord_space_queries.rs`, `cord_space_actions.rs` — a cord's spaces and
  their cap
- `invite/` — `queries.rs` (links), `redemption_queries.rs` (newcomer
  redemptions), `sweep.rs` (the expiry sweep), `actions.rs` (the verbs)
- `migrations.rs` — the `cord` namespace and its column projections

## Invariants

The load-bearing rules. Each module named holds the full reasoning.

- **Cords are cells.** A cord is a spine cell with `kind = "cord"`: a
  private root, its name in `data`, `created_by = NULL` so the spine's
  owner branch admits no one and role grants are the only authority (the
  creator is in the `cord_create` audit event). Three role-shaped
  `cell_grant` rows are stamped on it at creation, one per role, never one
  per member; they make the spine agree on **viewing** only.
  `can_edit_cell` treats a null-owner cell as admin-only, so cord cells
  change only through Cord's verbs, and nothing may rely on the spine's
  cell edit paths for them. The generic `fuz_cell_actions` verbs aren't
  mounted. (`cord_queries.rs`)
- **Membership** is spine role grants (`cord_owner`, `cord_admin`,
  `cord_member`) scoped to the cord (`scope_kind: "cord"`). `roles.rs`
  registers them with no grant paths, so the spine's role UI,
  `role_grant_assign`, and offers refuse them; only Cord's verbs confer
  them, through `query_cord_role_grant_create`. One role per (actor, cord),
  kept by Cord (the spine's index is per actor, role, and scope): a role
  change revokes and grants within one transaction, and accept never adds a
  grant beside an existing role. (`cord_authz.rs`, `cord_member_queries.rs`)
- **Exact-scope authz for cord roles.** A scope-blind role check would let
  one cord's owner pass for every cord. Use the predicates
  in `cord_authz.rs`. An instance admin is
  `has_scoped_role(ROLE_ADMIN, None)`, a global grant, which the
  dispatcher's role gate also requires.
  Instance admins create cords and see the ops list but get no implicit
  access. Every verb that names a cord answers `cord_not_found` to anyone
  outside it, the same as a missing cord (the IDOR mask).
- **No scoped builtin roles.** `admin` and `keeper` are only granted
  globally; the spine refuses a scoped one on every grant path, ahead of
  the offer gate (the spine's `AdminOfferAuthorize`, which requires a
  global admin).
- **Spaces are cells too.** A space is a cell with `kind = "space"`, a
  private child of the cord's cell (`parent_id` and `root_id` the cord),
  `created_by = NULL`, its own kind and name in `data` (`kind` a closed
  set, only `line` so far). The cord's three role-shaped grants are stamped
  on it, scoped to the cord; per-space visibility will narrow the member
  one. Owners and admins create, rename, and delete; members list. A space
  is always named with its cord, and one that isn't a live space of that
  cord is `cord_space_not_found`, but only after the IDOR mask. Space names
  share the cord name rules and validator, with their own length.
  (`cord_space_queries.rs`)
- **Serialization.** Every membership change (role set, remove, leave,
  accept) locks the cord's cell row, then re-reads membership and applies
  the rules to that read, never the request's snapshot; this relies on
  READ COMMITTED, which `cordd` checks at boot, refusing to serve (exit
  `2`) when `default_transaction_isolation` is anything else. The other
  managing writes (rename, invite create and revoke) take no cord lock and
  read the caller's role afresh inside the transaction; minting a newcomer link also re-reads the caller's global
  `admin` grant, under a share lock. Lock order is link, then
  cord, then redemption; revoke must not take the cord lock, or it would
  deadlock against accept. Redeem takes the link lock and, for a new
  reservation, share-locks the minter's global `admin` grant. The sweep
  takes the link lock with `SKIP LOCKED`. Space create takes the cord lock
  alone, then reads the role and counts live spaces against
  `CORD_SPACE_COUNT_MAX`, so the cap is exact; space rename and delete take
  no cord lock, read the role afresh, and lock only the space's row. The
  read-only verbs trust the snapshot. (`cord_member_queries.rs`,
  `invite/queries.rs`, `cord_space_queries.rs`)
- **Live grants.** A member is a live grant on a live actor and account;
  soft-deleted ones keep their grants but aren't listed, targeted, or
  counted. `live_role_grant_sql` and `live_cord_grant_sql` are the one
  place that tombstone filter lives; every role read goes through them.
- **Tokens** are stored only as a blake3 digest, returned once at mint,
  never listed or audited. A real token's refusals name their reason; any
  other token gets `cord_invite_not_found`, unless it's out of
  `CordInviteToken`'s bounds, which the verbs refuse at params validation
  as the spec does. (`invite/actions.rs` holds what a token tells its
  holder.)
- **Newcomer links**: an instance admin who manages the cord mints one,
  bounded in uses and expiry. The newcomer redeems it signed out for a
  username (a spine username invite in the minter's name, spending a
  use), signs up through the stock invite-gated signup, then accepts
  signed in, spending nothing; accept stays open until
  `CORD_INVITE_NEWCOMER_ACCEPT_GRACE_SECONDS` past expiry, while a revoke
  refuses at once. Revoke and the expiry sweep release waiting
  reservations. Redeem answers whether a username is free, so beyond the
  spine's shared IP class it has its own per-IP limiter
  (`CORD_INVITE_REDEEM_RATE_LIMIT`, charged in the handler; none under
  `testing_cordd`). (`invite/redemption_queries.rs`, `invite/queries.rs`,
  `invite/sweep.rs`, `invite/actions.rs`)
- **Cord names** refuse, before trimming, the ranges in
  `CORD_NAME_FORBIDDEN_CHAR_RANGES` (control, format, and invisible
  characters, except the join characters and emoji variation selectors,
  with rules against padding joins), then trim and bound the length.
  Checking before trimming keeps the twins exact. The list is twinned in
  `cord_queries.rs` and `cord_action_specs.ts`, and a test derives it from
  the Unicode properties both ways.
- **Join links** are `/join#<token>`: the fragment never reaches a server,
  and the join page keeps the link in the tab's `sessionStorage`.
  (`ui/cord_join_state.svelte.ts`)
- **Frontend gates**: session views wait on `session_checked`; signed-in
  views gate on `signed_in`; gates in front of fuz_app's forms never wait on
  `verifying` (true mid-submit, so the form would unmount and lose input);
  admin UI gates on `auth_state.is_admin`, fuz_app's global-grant check.
  (`ui/cord_auth_state.svelte.ts`)
- **Twins and their pins**: `cord_action_specs.ts` twins the Rust specs,
  error reasons, and limits, and `src/test/cord_action_specs.test.ts` sweeps
  the Rust sources so the reasons, the numeric `CORD_*` limits, and the
  `cord_*` methods match exactly; `cord_roles.ts` twins the roles and member
  rules, and `src/test/cord_roles.test.ts` replays `cord_authz.rs`'s own test
  calls; `action_specs.test.ts` pins the generated modules.

## Verbs

- `cord_actions.rs`: `cord_create`, `cord_list`, `cord_get`, `cord_update`,
  `cord_admin_list`
- `cord_member_actions.rs`: `cord_member_list`, `cord_member_role_set`,
  `cord_member_remove`, `cord_leave`
- `invite/actions.rs`: `cord_invite_create`, `cord_invite_list`,
  `cord_invite_revoke`, `cord_invite_preview`, `cord_invite_redeem`,
  `cord_invite_accept`
- `cord_space_actions.rs`: `cord_space_list`, `cord_space_create`,
  `cord_space_update`, `cord_space_delete`

## Recipes

**A cord verb**: the spec and handler in its Rust module, reached from
`cord_action_specs()`; the TS twin in `cord_action_specs.ts`, added to
`all_cord_domain_action_specs`; any new `ERROR_CORD_*` reason or `CORD_*`
limit as a `pub const` on both sides (the twin sweep fails otherwise); words
for new reasons in `ui/cord_errors.ts`; `gro gen`; then a cross test.

**A cord table**: append a `Migration` in `migrations.rs` (never edit one
that has shipped); give it a `*_COLUMNS` const and register it in
`CORD_COLUMN_PROJECTIONS` (the `columns.rs` guard); regenerate
`expected_schema.json` with `UPDATE_SCHEMA_READY=1`; add the table to the
`_testing_reset` truncate in `testing_cord_server`'s `main.rs`.

**A test seam**: a `RunAppOptions` field, which `cordd` passes as `None`
(or `false`).

## Frontend

A static SvelteKit build over `cordd`'s API, using `@fuzdev/fuz_app` for the
auth UI and the RPC client. The root `+layout.svelte` provides
`cord_rpc_context` (`ui/rpc.ts`: the throwing `api` over `/api/rpc`, HTTP
only, reads as `GET` unless their input is sensitive),
`cord_auth_state_context` (`CordAuthState`, also set on
fuz_app's `auth_state_context`), and `cords_state_context` (the signed-in
account's `CordsState`, fetched when an account signs in and reset when it
signs out), and checks the session on mount.

Every page sits in fuz_ui's `AppShell`, with `CordNav` as its left rail:
home, your cords (each selected on its page and any page under it), then
account, admin for instance admins, a/v check, about, and log out; signed
out, log in, sign up, a/v check, about, and the claim link while the
instance has no accounts.
Until the first session check settles, and while a login's account loads,
the rail shows only the links both shapes share. Its log out goes through
`auth_state.logout` and then home. The rail and the home page read the one
`CordsState`, loaded on sign-in and refetched each time the home page
mounts after that. A cord created, left, or joined here shows in the rail at
once; a cord's layout puts its own view of the cord (its name, renamed there,
and the viewer's role, from the member list once loaded) into the rail's
entry, and drops the entry when the server doesn't find the cord for you.
Other changes (another tab's, or another member's) show when you come home,
sign in, or reload, and a cord already in the rail also catches up when you
open it. A refresh keeps the loaded list up; a failed load keeps its error
through a retry, so the rail's retry, shown while there's no list, stays
mounted until the retry settles. Pages keep their in-text links (log in, claim it)
but no link lists of their own.
`/admin` nests its own section list in a `ColumnLayout` beside the rail.

The state classes hold fuz_app `AsyncSlot`s (`KeyedAsyncSlot` for per-row
writes) over the `api`. The member, invite, and join states map errors
through `ui/cord_errors.ts`; `cords_state.svelte.ts` maps its create and
rename refusals through it too (`map_error: to_cord_error_message`), while
its read slots keep `AsyncSlot`'s default messages. Per-cord states drop
responses for a cord they've moved off, and their `fetch` is safe to call
from an `$effect`.

A cord's pages share a layout that owns one `CordPageState`
(`ui/cord_page_state.svelte.ts`: the cord, its members, and the viewer's
role, from the member list once loaded, else from the cord), provided
through `cord_page_state_context`. Its cord, role, and member-list views
answer only for the cord the route names, so a navigation to another cord
drops the last one's in the same update. The layout holds the session
gates, the not-found view, and the loading and error states, each titling
the page, and mounts a tab page only once the route's cord has loaded,
with the viewer's role in it; no tab page mounts, or fetches, against a
cord the route has left. The members page refreshes the list when it
opens, so a role changed elsewhere catches up there. The invites page
builds its own `CordInvitesState`. The invites and settings tabs show only
to owners and admins, and their pages guard themselves too, fetching
nothing for anyone else.

The typed surface is generated: `action_specs.ts` lists every spec the
server mounts (the protocol actions work only over `/api/ws`), and
`action_collections.gen.ts` / `action_metatypes.gen.ts` generate the typed
modules. A new spec group goes in `action_specs.ts` and as a source row in
`action_collections.gen.ts`, then `gro gen`.

Routes, prerendered anonymous and filled in client-side:

- `/` — the intro; signed in, your cords, and a create form for admins
- `/about` — about the project, with the fuz ecosystem links
- `/av` — the a/v check, for anyone: camera, mic level and spectrum, output
  and test tone, a recording, a WebRTC call to itself with its stats, and
  what the browser offers; all local, no server
- `/c/[cord_id]` — a cord's pages, as tabs: the overview (your role, and
  where spaces will go), `members` (the members, and leaving), and for
  owners and admins `invites` (invite links) and `settings` (rename); the
  one section not prerendered, so the web server must fall back to the
  build's `spa.html` for all of it
- `/join` — where an invite link lands (`/join#<token>`)
- `/login`, `/signup` — fuz_app's forms; signup is invite-gated by default
- `/account` — your account and sessions
- `/admin` — the gate (health probe, bootstrap, login), then invites,
  accounts, sessions, the audit log (live from `/api/admin/audit/stream`),
  and `/admin/cords`, the ops list

Library code imports relatively; routes and tests use the `#lib/*` subpath
import.

## Commands

```bash
gro check     # typecheck, test, gen --check, lint, format check
gro typecheck # typecheck only (faster iteration)
gro test      # vitest, the unit project
gro gen       # regenerate the action_* modules
gro build     # the static frontend
```

For Rust, use cargo directly:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-release
```

`git add` and `git commit` are pre-approved here, at sensible stopping
points and following the ecosystem's commit conventions.

## Server runtime

`cordd` serves no static files: nginx serves the frontend in production,
and Vite in dev proxies `/api` and `/health` to `cordd` on port 4061. Without
`DATABASE_URL` it runs boot-only (`/health` and `/`, through the same
stack). `FUZ_ALLOWED_ORIGINS`
is required always, boot-only included, and validated entry by entry
(`fuz_http::AllowedOrigins::parse` — a blank value or any malformed entry
refuses the boot, naming each; a wildcard entry boots only with
`FUZ_ALLOW_ORIGIN_WILDCARDS=true`, boot-only included); with a database, `SECRET_FUZ_COOKIE_KEYS` is required
too, migrations run before serving, and `cordd` refuses to serve unless
Postgres's `default_transaction_isolation` is `read committed`. The
README's table lists every variable; spine-wide names stay unprefixed and
cord-only ones take `CORD_`. `CORD_TRUSTED_PROXIES`
names the reverse proxy whose `X-Forwarded-For` is believed; unset, it is the
loopback hop (`127.0.0.1`, `::1`, a local nginx), and `,` trusts none. A proxy
it doesn't name makes every caller share the proxy's IP, so one client
exhausts the per-IP budgets, invite redemption's included, for everyone.
Exit codes: `0` success, `2` the caller must fix something, `1` everything
else; `check-release` keeps fuz_audit's `0` clean, `65` a policy
violation, and `69`/`70` a tooling failure. Logs go to stdout in color
unless `NO_COLOR` is set; production sets `NO_COLOR=1` so journald gets
plain text.

The spine crates are path deps into a sibling checkout of the fuz Rust
workspace (paths in the root `Cargo.toml`); they aren't published, so cargo
resolves only with that checkout, and GitHub CI runs only the frontend
check. `default-members` is `cord_server` alone, so a bare `cargo build`
never builds `testing_cordd`. Crates opt into the workspace lints
(`[lints] workspace = true`): `unsafe_code` forbidden, clippy
all/pedantic/nursery/cargo, and `unwrap_used`/`expect_used`/`panic` at warn,
allowed in tests by `clippy.toml`. The clippy gate denies warnings, so
`rust-toolchain.toml` pins the toolchain.

## Deploying

`zap.ts` is the whole instance on one dedicated Debian 13 host (the nginx
recipe's rate-limit zone name is fixed, so two zap sites can't share one);
`DEPLOY.md` is the operator runbook. The domain, host, logins, certbot
email, and cookie keys are `zap.env('NAME')` references resolved from the
operator's gitignored `.env.production` (`.env.production.example`), so no
instance value lives in code. zap evaluates the config with a pinned `fgro
eval`, which resolves the recipes through `node_modules`: the
`@fuzdev/zap` dev dependency installs from npm at an exact version, upgraded
deliberately since its recipes render what lands on servers. `gro check`
typechecks `zap.ts`; check eval and schema offline with
`zap config validate --env-file <file> zap.ts` (no host). The `zap` binary and
`fgro` are built from source (see https://zap.fuz.dev/docs/install); keep the
binary on the same 0.x minor as the package.

What's cord-specific, beyond the stock fuz_app recipes:

- **The unit** runs the bare `cordd` (no subcommand) as the `cord` user,
  loopback port 4061, with its env from `shared/.env.production`; its one
  writable path is `~/.cord`, where `cordd` deletes the bootstrap token
  once claimed.
- **The env file** is `fuz_app_env_base` plus hand-written
  `FUZ_ALLOWED_ORIGINS`, `CORD_TRUSTED_PROXIES=127.0.0.1` (zap's
  trusted-proxies recipe only writes `FUZ_<infix>_TRUSTED_PROXIES`), and
  `NO_COLOR=1`.
- **nginx** falls back with `$uri $uri.html $uri/index.html /spa.html`:
  prerendered pages are `<route>.html`, `admin/` has no `index.html` (a
  `$uri/` entry would 403 on `/admin/`), and `spa.html` serves
  `/c/<cord_id>` and its tabs, and the SPA's 404.
  `/api/admin/audit/stream` gets its own exact-match location, unbuffered
  with an hour's read timeout, since the `/api` block's 60s timeout would
  cut the idle stream. `/api/ws` rides the `/api` block and its 60s
  timeout, to revisit when the frontend holds a WebSocket open.
- **The deploy** builds locally (`cargo xtask check-release`, the release
  `cordd`, `gro build`, then the binary into `dist_artifacts/`, after
  `gro build` since it deletes `dist_*`) and gates on `/ready`. A rollback
  doesn't undo migrations, so they stay additive.

## testing_cordd

The same `run_app` with the test-only pieces: `fuz_testing`'s fast hasher,
disabled rate limiters, a daemon token under `CORD_DAEMON_TOKEN_DIR`
(required), `_testing_reset` (a fresh keeper, then a truncate of cord's
tables and the cell tables), `_testing_drain_effects` (a real barrier, since
it builds the audit emitter with in-flight tracking), cord's
`_testing_cord_invite_expire`, and a schema wipe on boot when
`FUZ_TESTING_RESET_DB_ON_STARTUP` is true. It binds `127.0.0.1:4161`. **Every
cord table goes in the `_testing_reset` truncate**, or rows outlive a test. Never ship it: it is the only runtime
dependent of `fuz_testing`, and `cargo xtask check-release` fails if `cordd`
reaches `fuz_testing` or `fuz_audit` (the `testing_` prefix exempts
`testing_cordd` by name; `cord_server` takes `fuz_testing` as a
dev-dependency for its Postgres suites, an edge the audit ignores).

## Testing

`gro test` runs the in-process vitest `unit` project: the twins
(`cord_action_specs.test.ts`, `cord_roles.test.ts`), the generated surface
(`action_specs.test.ts`), and the state classes (`src/test/ui/`).

Rust unit tests live in `#[cfg(test)]` modules and need no database,
except `router.rs`'s `#[ignore]`d redeem-budget test, which drives the
dispatcher over a live connection but no schema. The Postgres suites in
`crates/cord_server/tests/` are `#[ignore]`d and reset the `public` schema
of their database: `schema_ready.rs` (the fixture's drift guard),
`columns.rs` (the column projections), `cords.rs`, `cord_members.rs`,
`cord_invites.rs`, `cord_newcomers.rs`, and `cord_spaces.rs`, sharing
`tests/common/` (the reset-and-migrate cycle, the actor seeding, and the
`columns.rs` assertion come from the spine's `fuz_testing`); each module
doc lists what it covers.
`transaction_isolation.rs` resets no schema: it boots `cordd`'s router on
its own scratch database set to `repeatable read` and asserts the refusal
comes before any migration. `cordd_args.rs` runs unignored with no
database: the real binary's command line and its start-failure exit codes.

```bash
createdb cord_schema_ready_test
cargo test -p cord_server -- --ignored
# regenerate expected_schema.json after a migration change
UPDATE_SCHEMA_READY=1 cargo test -p cord_server --test schema_ready -- --ignored
```

The `cross_backend_rust` vitest project runs
`src/test/cross_backend/*.cross.test.ts` over real HTTP against a spawned
`testing_cordd` through fuz_app's cross-backend harness; each test fires
`_testing_reset`. Its global setup builds the binary first
(`prepare_cargo_binary`, with `-p` since `default-members` excludes it;
`FUZ_TESTING_NO_REBUILD=1` skips the build) and creates the database when it's
missing. It's gated behind `FUZ_TEST_CROSS_BACKEND`, so a bare `gro test`
skips it. `spine_suites` runs fuz_app's shipped spine suites in one
`describe_spine_cross_suites` call, gated on the mount flags
`cord_backend_config.ts` states, with the suites cord can't run excluded with
their reasons; `auth` covers what those leave out (the one-shot bootstrap, the
admin RPC and audit stream gates for a member and a bearer token), `cords` the
cord verbs, `cord_members` invite links and members, `cord_newcomers`
newcomer links, and `cord_spaces` the space verbs, end to end. The config
is `cord_backend_config.ts`, the declared surface `cord_surface_spec.ts`,
the shared steps `cord_cross_helpers.ts`.

```bash
npm run test:cross   # FUZ_TEST_CROSS_BACKEND=1 vitest run --project cross_backend_rust
```

`CORD_SCHEMA_READY_TEST_URL` and `CORD_CROSS_BACKEND_TEST_URL` override the
databases (`cord_schema_ready_test`, `cord_test_rust`). Both suites wipe
their database's `public` schema (`testing_cordd` on every boot), so point
them only at throwaway databases.
