# cord

> Cord is a self-hosted place for your people: a family, a friend group, a
> club, a team. Voice, video, and screenshare rooms, live chat, and forums,
> with your Bluesky and ActivityPub feeds and the RSS, Atom, and JSON Feed
> sources you follow, in one instance you run yourself.

**Status: early.** The design is settled and implementation is underway: a
SvelteKit static frontend over `cordd`, an API-only Rust server on the fuz
spine, with accounts, cords, their members, and invite links (including ones
that bring someone new onto the instance), and a cord's spaces (each a line,
by kind), which its owners and admins add, rename, and delete, shown in the
rail and as tabs. Nothing happens inside a space yet: no live room. Don't
describe planned features as working.

## Vocabulary and voice

- **Kinds**: a **cord** is the group ("join our cord"); it holds
  **spaces**, each with a kind and one or more views. A **line** is a live
  room (voice, video, screenshare, ephemeral text, no history; members join
  silent and each of mic, camera, and screen stays off until they turn it
  on). **chat** is persisted messages with unread markers. A **forum** holds
  threads with nested comments and reactions (a forum topic is a
  **thread**; sort order is a view choice). A **feed** is a source the
  cord pins (a Bluesky or ActivityPub feed, list, or account, or an RSS,
  Atom, or JSON Feed source), shown as a list of posts that each open
  into their thread, or as a timeline.
- **Who it's for**: your people, the ones you already know and chose,
  not strangers or an audience. The tagline is "plug unplug".
- **Views**: two switchable desktop layouts (sidebar plus main, column
  dashboard) and a phone single column, all over the same spaces.
- **Accounts**: the instance's own (operator-gated signup, cookie
  sessions); each cord is invite-gated, with three fixed roles (owner,
  admin, member); a space is visible to all members or a listed subset. A
  member may connect a Bluesky (or other atproto PDS) account or an
  ActivityPub account for personal feed columns and posting, linked to
  their account, never a way to log in.
  Members control their own presence visibility, and the instance is honest
  about what it can see.
- **Backend**: the API-only server, this frontend served by nginx,
  Postgres, and, once the live room lands, coturn for WebRTC TURN (full
  mesh). The live room is built first; persisted spaces layer on it.
- **Self-hosting**: one instance hosts several cords; deployed by zap,
  with docker compose planned. Not federation, not a hosted service, not
  engagement-driven: never streaks, scores, or reputation, and no push
  notifications yet.

Voice for user-facing copy: describe Cord by what it is, never by contrast
with other products. Warm, low-key, unhurried. For coding conventions, see
Skill(fuz-stack).

## Layout

- `src/routes/` — SvelteKit routes (see Frontend)
- `src/lib/` — `cord_action_specs.ts` (the cord wire's TS twin),
  `cord_roles.ts` (the roles and member rules' twin), `action_specs.ts` and
  the generated `action_*` and `frontend_action_types` modules; `ui/` holds
  `rpc.ts`, `cord_socket.svelte.ts` (the signed-in account's WebSocket),
  `cord_errors.ts` (refusal reasons to words, the socket's included),
  `route_params.ts`, `double_click.ts`, the `*_state.svelte.ts` classes,
  and the components (`CordNav`, the rail; `Tabs` and `Tab`, the route tabs, a
  cord's space tabs among them, which `CordSpaceTabStrip` renders;
  `CordNameForm`, for cord and space names;
  `CordMembers`, `CordInvites`, `CordInviteForm`,
  `CordNewcomerForm`; `MediaCheck` and its `Media*` parts, the a/v check,
  with `media_output.ts` routing sound to a chosen output)
- `src/test/` — vitest tests; `cross_backend/` holds the end-to-end suites
- `crates/cord_server/` — the server library and `cordd`, plus the
  `expected_schema.json` fixture; `tests/` holds the database suites
- `crates/testing_cord_server/` — the test-only `testing_cordd`
- `crates/xtask/` — `cargo xtask check-release`
- `.env.development.example` — the server's local env, copied to the
  gitignored `.env.development`
- `zap.ts`, the `package.json` zap scripts, `.env.production.example`, `DEPLOY.md` — the
  zap deploy (see Deploying)
- `src/routes/logos.ts` (`logo_cord`), `static/logo.svg`, `static/favicon.png`
  — the logo: two socket faces, a plug reaching toward each, and one looped
  cord, in teal; the three are one drawing, changed together
- `src/env.ts` (`PUBLIC_CORD_SITE`), `src/hooks.ts` (its `reroute`),
  `src/lib/cord_site.ts` — the static site (see The static site)

`cord_server`'s modules (each module doc holds the detail):

- `main.rs` (`cordd`, `cordd db-status`; the strict command line — anything
  else exits `2`), `runtime.rs` (`run_app`, `RunAppOptions`, the env —
  the spine's `BaseServerEnv` with `CORD_TRUSTED_PROXIES` — and
  `extra_upkeep`: the invite expiry sweep, run with the spine's auth cleanup
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
  set, only `line` so far). A space names no actor, as its cord doesn't:
  `updated_by` stays `NULL` through rename and delete, and the audit
  events are the record, so no purge reaches the row. The cord's three
  role-shaped grants are stamped on it, scoped to the cord; per-space
  visibility will narrow the member one. Owners and admins create, rename,
  and delete; members list. A space is always named with its cord, and one
  that isn't a live space of that cord is `cord_space_not_found`, but only
  after the IDOR mask. Space names share the cord name rules and
  validator, with their own length. (`cord_space_queries.rs`)
- **Serialization.** Every membership change (role set, remove, leave,
  accept) takes the cord's scope lock shared (the spine's
  `query_scope_lock_shared`, keyed by the cord id), holding no row, then
  locks the cord's cell row, then re-reads membership and applies
  the rules to that read, never the request's snapshot; this relies on
  READ COMMITTED, which `cordd` checks at boot, refusing to serve (exit
  `2`) when `default_transaction_isolation` is anything else. The other
  managing writes (rename, invite create and revoke) take no cord lock and
  read the caller's role afresh inside the transaction; minting a newcomer
  link also re-reads the caller's global `admin` grant, under a share
  lock. The scope lock comes first because a cord teardown (not yet
  built) would take it exclusive and revoke the cord's grants before
  touching its row; cord create needs none, granting on a row its own
  transaction inserted.
  Accept locks the cord, then the link's minter's account, then the
  link, then the redemption; redeem locks the minter's account, then the
  link, and no cord. Revoke must not take the cord lock after the link's.
  The minter's account comes before the link because a purge holds it,
  then deletes the minter's unclaimed invites and nulls the link's
  `created_by`; the minter is found by an unlocked read of the link and
  re-checked under the lock. Accept takes it `FOR KEY SHARE`, the weakest
  lock that conflicts with the purge's `FOR UPDATE`, so a login, a revoke,
  or a member verb of another cord doesn't wait behind an accept, and
  after the cord (a purge holds every admin's account, so a member verb
  can wait on it holding the cord). Holding the cord while waiting on an
  account is safe because a purge never writes a cord's row: a cord cell
  names no actor (`created_by` and `updated_by` stay `NULL`; the audit
  events are the record of who created or renamed it). A newcomer's accept
  claims their signup invite before its grant names their actor, so a
  purge of the newcomer is waited out, not deadlocked, and the accept then
  fails; the redemption's `accepted_by` is a record with no foreign key,
  so a purge never writes it. Redeem takes the minter's account
  `FOR SHARE`, and a new reservation then share-locks the minter's global
  `admin` grant (account before grant, the last-admin guard's order). The
  sweep takes the link lock with `SKIP LOCKED`. Space create takes the
  cord lock alone, then reads the role and counts live spaces against
  `CORD_SPACE_COUNT_MAX`, so the cap is exact; space rename and delete
  take no cord lock, read the role afresh, and lock only the space's row.
  The read-only verbs trust the snapshot. (`cord_member_queries.rs`,
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
- **Signed-in requests go only over the socket**, never falling back to
  HTTP: a request waits for a down socket at most
  `CORD_SOCKET_WAIT_TIMEOUT`, nothing waiting is ever sent in the next
  session, and one on the wire at a drop fails as `connection_lost`, its
  outcome unknown. Signed out, requests go over HTTP.
  (`ui/cord_socket.svelte.ts` tells the socket's whole story; `ui/rpc.ts`
  the HTTP side)
- **The socket ends a session only on a 401.** A 4001 close or reconnects
  that keep failing only put the session in doubt; a definitive 401, or a
  live session naming another account, is the one answer that runs
  `check_session`, so a server restart or an outage signs no one out. A live
  session reopens a revoked socket on a fresh client.
  (`ui/cord_socket.svelte.ts`)
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

**A pushed notification**: the `remote_notification` spec
(`initiator: 'backend'`) in `all_cord_action_specs`, so a push is parsed
rather than warned about as an unknown method; `gro gen`; then a `receive`
handler in the `FrontendActionHandlers` passed to `create_cord_rpc`.

## Frontend

A static SvelteKit build over `cordd`'s API, using `@fuzdev/fuz_app` for the
auth UI and the RPC client. The root `+layout.svelte` provides
`cord_rpc_context` (`ui/rpc.ts`: the throwing `api`, and the `socket`),
`cord_auth_state_context` (`CordAuthState`, also set on
fuz_app's `auth_state_context`), `cords_state_context` (the signed-in
account's `CordsState`, fetched when an account signs in and reset when it
signs out), and, reset with it, `cord_spaces_state_context` (the one
`CordSpacesState`) and `cord_space_tabs_context` (the one `CordSpaceTabs`),
which live at the root rather than in the cord layout because the rail
nests them; and it checks the session on mount.

Signed in, every request goes over the account's WebSocket on `/api/ws`;
signed out (an invite link's preview and redemption), over HTTP on
`/api/rpc`. The root layout drives the `CordSocket` from the same account
id as its reset effect and calls its `recheck_session_if_in_doubt`; how the
socket opens, waits, fails, re-checks the session, and reopens, and what
the rail shows meanwhile, is `ui/cord_socket.svelte.ts`'s module doc, and
the HTTP side and pushed notifications are `ui/rpc.ts`'s.

Every page sits in fuz_ui's `AppShell`, with `CordNav` as its left rail:
home, your cords (each selected on its page and any page under it, the one
you're on with its spaces nested under it once they've loaded, the others
collapsed), then
account, admin for instance admins, a/v check, about, and log out; signed
out, log in, sign up, a/v check, about, and the claim link while the
instance has no accounts.
Until the first session check settles, and while a login's account loads,
the rail shows only the links both shapes share. Its log out goes through
`auth_state.logout` and then home. A click on a nested space opens it, and a
double click keeps its tab (below). The rail and the home page read the one
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
writes) over the `api`. The member, invite, space, and join states map errors
through `ui/cord_errors.ts`, and so does every slot in
`cords_state.svelte.ts` (`map_error: to_cord_error_message`); the socket's
reasons and words live there too, and its errors carry the words as their
messages. Per-cord states drop
responses for a cord they've moved off, and their `fetch` is safe to call
from an `$effect`; so is a `reset`, which reads nothing trackably (a keyed
slot's reset reads its entries, and the root layout's reset effect would
otherwise rerun on every write). `CordSpacesState`
(`ui/cord_spaces_state.svelte.ts`) answers by cord (`loaded_spaces`,
`get_space`), so a list left from another cord answers for none; a delete
of a space the server no longer has counts as done.

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

The layout also loads the cord's spaces with the cord, and its
`CordSpaceTabStrip` renders the space tabs and keys `CordSpaceTabs`
(`ui/cord_space_tabs.svelte.ts`) to the viewer's account and the route's
cord. The space tabs work as an editor's file tabs, with
its words: one `preview` tab, which opening another space replaces, and
`permanent` tabs that stay until closed; the `selected` tab, the one
showing, is the route's. The strip is the management tabs (overview,
members, then invites and settings for managers), the permanent tabs in
the order kept, then the preview tab (italic); it never wraps, scrolling
sideways with the selected tab scrolled into view, and an edge fades
while tabs are hidden past it. Opening a space by URL
(a link, a reload, a single click in the rail or the overview) puts it in
the preview tab unless it's permanent; a double click on the preview tab,
or on a space in the rail, keeps it, and the space page's
"keep as a permanent tab" button is the keyboard path. Closing the selected
tab goes to its neighbour (the next, else the previous), else the
overview, and focus lands on the selected tab. Permanent tabs are
remembered in `localStorage` per account and cord (every read and write
guarded; a blocked store keeps them for the page's life), and tabs of
spaces the cord no longer has drop whenever its list loads or changes. The
overview refreshes the space list when it opens.

The typed surface is generated: `action_specs.ts` lists every spec the
server mounts (the protocol actions work only over `/api/ws`) and the
notifications it pushes, and `action_collections.gen.ts` /
`action_metatypes.gen.ts` / `frontend_action_types.gen.ts` generate the
typed modules, the last the `FrontendActionHandlers` map. A new spec group
goes in `action_specs.ts` and as a source row in
`action_collections.gen.ts`, then `gro gen`.

Routes, prerendered anonymous and filled in client-side:

- `/` — the intro; signed in, your cords, and a create form for admins
- `/about` — about the project, with the fuz ecosystem links
- `/av` — the a/v check, for anyone: camera, mic level and spectrum, output
  and test tone, a recording, a WebRTC call to itself with its stats, and
  what the browser offers; all local, no server
- `/c/[cord_id]` — a cord's pages, as tabs: the overview (your role, and
  the cord's spaces, with an add-a-line form for owners and admins),
  `members` (the members, and leaving), for owners and admins `invites`
  (invite links) and `settings` (rename), and the space tabs, each
  `/c/[cord_id]/s/[space_id]` (the space's name and kind, saying the room
  isn't here yet, and renaming and deleting it for owners and admins; an id
  the loaded list lacks shows the not-found view); the one section not
  prerendered, so the web server must fall back to the build's `spa.html`
  for all of it
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
gro check     # typecheck, test, gen --check, format check
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
resolves only with that checkout and a `pglet/` checkout beside it
(`fuz_testing` path-depends on pglet's server crate, and cargo loads it for
any build). GitHub CI's `rust` job checks out fuz and pglet at `main`
beside this repo and runs fmt, clippy, and `cargo test` on pglet, then the
PostgreSQL leg on a Postgres service, beside the frontend check. `default-members` is `cord_server` alone, so a bare `cargo build`
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
  cut the idle stream. `/api/ws` rides the `/api` block, whose upgrade
  headers carry it; the client's heartbeat, sent once either direction has
  been idle for `DEFAULT_HEARTBEAT_INTERVAL` (checked every half interval),
  keeps a frame each way inside its 60s read and send timeouts, and inside
  `cordd`'s own receive timeout (`cord_socket.svelte.test.ts` pins both). The
  `/api` rate limit counts the upgrade, not the requests sent over the
  socket after it.
- **The deploy** builds locally (`cargo xtask check-release`, the release
  `cordd`, `gro build`, then the binary into `dist_artifacts/`, after
  `gro build` since it deletes `dist_*`) and gates on `/ready`. A rollback
  doesn't undo migrations, so they stay additive.

## The static site

cord.fuz.dev is this same frontend built with `PUBLIC_CORD_SITE=true`
(`npm run build:site`) and deployed to GitHub Pages's `deploy` branch with
`npm run deploy:site` (`gro deploy`). It has no server and never points at
one. `src/env.ts` declares the flag, a static public var that refuses
anything but `true`, `false`, or empty, and `vite.config.ts` reads it the
way SvelteKit does. With it on:

- only `CORD_SITE_PATHS` (`cord_site.ts`: the intro, about, and the a/v
  check) prerender, and an unseen site route still fails the build;
- the `reroute` hook sends every other path to a route-less one, so an
  instance's pages are a 404 on the server and in the client alike;
- the adapter writes `404.html` (Pages' fallback) in place of `spa.html`,
  plus `CNAME` and `.nojekyll`, which only this build ships;
- the root layout never checks a session, so the rail keeps the links both
  shapes share and nothing requests `/api`; the home page names the repo
  instead of the session.

The instance build is unchanged when the flag is unset. A new site page goes
in `CORD_SITE_PATHS` and needs a link the crawler can follow.

## testing_cordd

The same `run_app` with the test-only pieces: `fuz_testing`'s fast hasher,
disabled rate limiters, a daemon token under `CORD_DAEMON_TOKEN_DIR`
(required), `_testing_reset` (a fresh keeper, then a truncate of cord's
tables and the cell tables), `_testing_drain_effects` (a real barrier, since
every spine audit emitter tracks its in-flight writes),
`_testing_db_invariants` (the auth schema's invariants, which the
cross-process setup checks after each test),
`_testing_shorten_credential_expiry` (an account's credentials expiring
seconds from now, for the spine's credential-deadline suite),
`_testing_credential_read` and `_testing_origin_check` (the credential and
origin readers the spine's conformance suites drive),
`_testing_flood_notifications` (the outbound-overflow burst), and cord's
`_testing_cord_invite_expire`. Its two action seams are
`RunAppOptions` fields `cordd` passes as `None`:
`extra_action_specs_factory` (the spine's, given the app and the spine's
runtime) mounts the `_testing_*` actions after cord's own, and
`final_action_specs_factory` (cord's, given the assembled spec list) mounts
`_testing_action_manifest` last, so the manifest reports every method
mounted above it. It binds `127.0.0.1:4161`. **Every
cord table goes in the `_testing_reset` truncate**, or rows outlive a test. Never ship it: it is the only runtime
dependent of `fuz_testing`, and `cargo xtask check-release` fails if `cordd`
reaches `fuz_testing` or `fuz_audit` (the `testing_` prefix exempts
`testing_cordd` by name; `cord_server` takes `fuz_testing` as a
dev-dependency for its database suites, an edge the audit ignores).

## Testing

`gro test` runs the in-process vitest `unit` project: the twins
(`cord_action_specs.test.ts`, `cord_roles.test.ts`), the generated surface
(`action_specs.test.ts`), and the state classes, the socket (its
re-checks, reopens, and waiting requests), and the RPC client's routing and
push dispatch (`src/test/ui/`: the socket over real
`FrontendWebsocketClient`s on fuz_app's mock sockets, `socket_clients.ts`,
and `FakeTimers`; the state classes over fuz_app's `create_stub_api`).
Its setup file, `src/test/logged_error_gate.ts`, installs fuz_app's
logged-error gate: a test fails on any error it logs unless it declares it
(`expect_logged_error` for one it provokes, `allow_logged_error` with a
reason for one it tolerates). A test file is named for its module,
`.svelte.` included (`ui/cords_state.svelte.test.ts`).

Every database test opens a database of its own through the spine's
`fuz_testing::fresh_test_db`: an in-process pglet server by default, so
`cargo test --workspace` runs them with nothing to set up, or, with
`FUZ_TEST_POSTGRES_URL` naming a PostgreSQL server and a role with
`CREATEDB`, a database created and dropped there per test. Rust unit tests
live in `#[cfg(test)]` modules; `router.rs`'s redeem-budget test drives the
dispatcher over a bare database. The suites in `crates/cord_server/tests/`
are `schema_ready.rs` (the fixture's drift guard), `columns.rs` (the column
projections), `cords.rs`, `cord_members.rs`, `cord_invites.rs`,
`cord_newcomers.rs`, and `cord_spaces.rs`, sharing `tests/common/` (the
fresh database, the actor and grant seeding, `in_tx!`, the lock-wait
observers, the recording closer, and the `columns.rs` assertion come from
the spine's `fuz_testing`); each module doc lists what it covers.
`request_reads_census.rs` holds `clippy.toml`'s disallows of axum's lenient
query and path readers in place (the spine's shared census).
`transaction_isolation.rs` boots `cordd`'s router on a database set to
`repeatable read` and asserts the refusal comes before any migration.
`cordd_args.rs` needs no database: the real binary's command line and its
start-failure exit codes.

A test of what only PostgreSQL has (a lock wait seen through
`pg_stat_activity`, `ALTER DATABASE … SET`) opens its database with
`DB.postgres_only().open()` (`TestEngine::Postgres`) and carries
`#[ignore = "postgres-only: <why>"]`; one waiting on a pglet feature
carries `#[ignore = "pglet: <feature>"]` and the same engine. The
PostgreSQL leg runs everything:

```bash
FUZ_TEST_POSTGRES_URL=postgres://localhost/postgres cargo test --workspace -- --include-ignored
```

Regenerate `expected_schema.json` after a migration change on the default
engine (the column map is the same on either):

```bash
UPDATE_SCHEMA_READY=1 cargo test -p cord_server --test schema_ready
```

GitHub CI's `rust` job runs the PostgreSQL leg on a Postgres service
(`-p cord_server`, the crate with database tests); `xtask`'s
`tests/db_suite_lists.rs` holds that `-p` list to the crates with database
tests.

A run killed before its tests' drops ran leaves `fuz_testing_*` databases
behind; drop them by that prefix.

The `cross_backend_rust` vitest project runs
`src/test/cross_backend/*.cross.test.ts` over real HTTP against a spawned
`testing_cordd` through fuz_app's cross-backend harness; each test fires
`_testing_reset`. Its global setup builds the binary first
(its `cargo_binaries` entry, with `-p` since `default-members` excludes it;
`FUZ_TESTING_NO_REBUILD=1` skips the build) and opens its database: a
`pglet_server` of its own, built in cord's Cargo workspace, or under
`FUZ_TEST_POSTGRES_URL` a PostgreSQL database `fuz_testing_cord_rust`
dropped after the run. It's gated behind `FUZ_TEST_CROSS_BACKEND`, so a
bare `gro test` skips it. `spine_suites` runs fuz_app's shipped spine suites in one
`describe_spine_cross_suites` call, gated on the mount flags
`cord_backend_config.ts` states, with the suites cord can't run excluded with
their reasons; `auth` covers what those leave out (the one-shot bootstrap, the
admin RPC and audit stream gates for a member and a bearer token), `cords` the
cord verbs, `cord_members` invite links and members, `cord_newcomers`
newcomer links, `cord_spaces` the space verbs, and `cord_socket` cord verbs
over `/api/ws`, end to end (the socket itself, its upgrade and every 4001
close, revocation and expiry alike, is the spine suites'); `action_manifest`
holds `all_cord_action_specs` to the registry `testing_cordd` mounts (each
method's input shape, auth, side effects, and rate-limit class), the
protocol and `_testing_*` actions aside. The config
is `cord_backend_config.ts`, the declared surface `cord_surface_spec.ts`,
the shared steps `cord_cross_helpers.ts`.

```bash
npm run test:cross   # FUZ_TEST_CROSS_BACKEND=1 vitest run --project cross_backend_rust
FUZ_TEST_POSTGRES_URL=postgres://localhost/postgres npm run test:cross   # on PostgreSQL
```
