# Cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is underway:
the server runs, an instance admin can create cords, and people can be
invited in, but nothing happens inside a cord yet, so the features
described below don't work yet.

Think of the coiled telephone cord stretched from the kitchen wall into
your bedroom, or everyone on the party line at once. Cord is meant to feel
like that: warm, low-key, a room you already know.

## What it is

A **cord** is your group ("join our cord"). Inside it, everything is a
**space**, and each space has a kind:

- **line**: a live room with voice, video, screenshare, and text that
  isn't saved. You join silent, and your mic, camera, and screen each stay
  off until you turn them on.
- **chat**: messages that stay, with unread markers so you can catch up.
- **forum**: threads with nested comments and reactions, sorted by
  activity, newest, or top.
- **feed**: a Bluesky feed, list, or account the cord pins so everyone sees
  the same thing, read as a list of posts that each open into their
  thread, or as a timeline.

The same spaces can be seen through different views: on desktop, a sidebar
with the active space beside it, or a dashboard of spaces in columns side
by side; on a phone, a single column.

Accounts belong to your instance, the operator decides who can sign up,
and each cord is joined by invite. Each cord has an owner, admins, and
members, and a space can be open to the whole cord or to a chosen few.

Any member can connect a Bluesky account (or any atproto PDS account) to get
personal feed columns and to post, reply, like, and repost from inside
Cord. Nobody needs one to hang out, and the cord's shared feeds show for
everyone.

Cord is built around the people in it. Members decide who sees them online,
globally and per cord. The instance aims to be plain about what its operator
can and can't see. There are no streaks or scores and no push notifications
to start; unread markers and highlighted mentions wait for when you come
back. Your cord's chats and forums live on your instance, in its own
database.

## What's here today

- a SvelteKit static frontend (`src/routes/`) with the account pages:
  logging in and out, invite-gated signup, claiming a fresh instance, your
  account and its sessions, and an admin area for minting signup invites
  and managing accounts and sessions
- cords: the instance's admins create cords and become their owners; the
  home page lists your cords (or, for an admin with none, offers to create
  the first), each cord has a page with its name, your role, and its
  members, its owners and admins can rename it, and the admin area lists
  every cord with its member count
- invites and members: owners and admins share invite links, including
  ones that welcome someone new to the instance, and the cord's members
  can be given roles, removed, or leave, as long as the cord keeps an
  owner. A cord's spaces and everything inside them come next
- the server, `crates/cord_server` (the `cordd` binary): accounts and
  sessions, invite-gated signup, first-account bootstrap, the cord,
  invite, and member actions, a JSON-RPC endpoint, a WebSocket endpoint,
  and health and readiness checks

The live room comes first, since hanging out is the point; chat, forums, and
Bluesky feeds layer on after it.

## Planned shape

- **Server**: an API-only Rust server built on
  [fuz](https://www.fuz.dev/)'s Rust server crates (accounts and sessions,
  database, actions, realtime).
- **Frontend**: a SvelteKit static build, served by the instance's nginx.
- **Storage**: Postgres.
- **Calls**: WebRTC in a full mesh, which suits small groups, with a TURN
  server (coturn) the operator runs.
- **Self-hosting**: one instance can host several cords, each invite-gated.
  A zap config for a single Debian host is here today (see
  [Deploying](#deploying)); docker compose behind your own reverse proxy
  is planned, along with a self-hosting guide. Only the optional Bluesky
  connection needs the instance at a public HTTPS origin; everything else
  runs fine on a LAN. There is no hosted service.

## Developing

Prerequisites:

- Node.js, the version in `package.json` `engines`
- the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it
  automatically)
- [Gro](https://github.com/fuzdev/gro) for the `gro` commands, installed
  globally (`npm i -g @fuzdev/gro`); the `npm run` scripts work without it
- for the server only, fuz's Rust server crates checked out beside this
  repo (see [The server](#the-server)); the frontend builds without them

```bash
npm i
gro dev    # or `npm run dev`, the frontend dev server
gro check  # typecheck, test, gen check, lint, format check
gro build  # production build of the static frontend
```

The build in `build/` is static files plus `spa.html`, which whatever
serves them must fall back to for paths with no prerendered page (a cord's
page, `/c/<id>`).

### The server

`cordd` is built on fuz's Rust server crates, which aren't published yet.
Building it needs a checkout of them beside this repo, at the paths the
root `Cargo.toml` names; without one, cargo can't resolve the workspace,
so the server doesn't build from this repo alone. The frontend builds and
checks on its own.

The Rust side uses cargo directly:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo xtask check-release   # no production binary links test-only code
```

The workspace's crates live in `crates/`: `cord_server` (the `cordd`
server), `testing_cord_server` (`testing_cordd`, a test-only build of the
same server with a fast password hasher, a test reset action, and no rate
limits; never deploy it), and `xtask`, whose `check-release` fails if
`cordd` depends on any test-only crate.

To run `cordd` locally you need Postgres. Copy `.env.development.example`
to `.env.development`, fill in `SECRET_FUZ_COOKIE_KEYS`
(`openssl rand -base64 32`), load it into your shell, and run the server:

```bash
createdb cord
cp .env.development.example .env.development   # then edit it
set -a; . ./.env.development; set +a           # load it (bash)
cargo run -p cord_server                       # cordd, on 127.0.0.1:4061
```

`cordd` reads only its process environment:

| Variable                   | Purpose                                                                |
| -------------------------- | ---------------------------------------------------------------------- |
| `DATABASE_URL`             | Postgres connection; unset runs a boot-only server with just `/health` |
| `FUZ_ALLOWED_ORIGINS`      | always required: comma-separated origins (`http://localhost:*`)        |
| `SECRET_FUZ_COOKIE_KEYS`   | required with a database: cookie-signing keys, 32+ characters each     |
| `FUZ_BOOTSTRAP_TOKEN_PATH` | token file that opens first-account bootstrap                          |
| `HOST`, `PORT`             | bind address, default `127.0.0.1:4061`                                 |
| `CORD_TRUSTED_PROXIES`     | reverse-proxy IPs or CIDRs trusted for `X-Forwarded-For`               |

It runs its migrations on start. With the server up, the frontend dev
server proxies `/api` (WebSocket included) and `/health` to port 4061, so
the two work together on one origin; if you move `cordd` with `PORT`,
change the proxy target in `vite.config.ts` too. On a fresh database,
write a token to the file at `FUZ_BOOTSTRAP_TOKEN_PATH` before starting
`cordd`; it must be owner-only (mode `0600`), which
`(umask 077; openssl rand -hex 32 > <path>)` gives you. Then open `/admin`
in the frontend and submit the token to create the first account, which is
the admin. After that, the admin mints invites at `/admin/invites`, and new
accounts sign up at `/signup` with a username or email that matches an
invite (or freely, if the admin turns on open signup there).

The server's routes:

- `GET /health`: liveness
- `GET /ready`: readiness; checks the database schema against
  `crates/cord_server/expected_schema.json`
- `/api/account/{status,login,logout,password}`: sessions and the account
- `POST /api/account/bootstrap`: first-account creation, token-gated
- `POST /api/account/signup`: invite-gated signup
- `/api/rpc` (GET and POST): JSON-RPC 2.0 with the account, admin,
  role-grant offer, and cord actions
- `/api/ws`: the WebSocket for realtime actions
- `GET /api/admin/audit/stream`: the audit log as a live server-sent event
  stream, for admins signed in with a session

`cordd db-status` prints a read-only report of the database's tables and
migrations.

### Tests that need Postgres

`gro test` and `cargo test --workspace` need no database. These tests do,
and each wipes the `public` schema of its database, so give them throwaway
ones:

```bash
# end-to-end tests of auth, cords, members, invites, and newcomer links:
# spawn testing_cordd and drive it over HTTP
createdb cord_test_rust
npm run test:cross

# the schema fixture, column guards, and the cord, member, invite, and
# newcomer queries
createdb cord_schema_ready_test
cargo test -p cord_server -- --ignored
```

`CORD_CROSS_BACKEND_TEST_URL` and `CORD_SCHEMA_READY_TEST_URL` override
their database URLs.

## Deploying

[`zap.ts`](zap.ts) describes a whole instance on one dedicated Debian
host: nginx serving the frontend, PostgreSQL, and `cordd` as a hardened
systemd service, provisioned and deployed with
[zap](https://zap.fuz.dev/). [DEPLOY.md](DEPLOY.md) is the runbook.
Deploying needs zap checked out beside this repo (`deno.json` maps in its
recipes), along with fuz's server crates for the `cordd` build.

## Built with

The fuz stack: TypeScript,
[Svelte](https://github.com/sveltejs/svelte),
[SvelteKit](https://github.com/sveltejs/kit),
[fuz_css](https://github.com/fuzdev/fuz_css),
[fuz_ui](https://github.com/fuzdev/fuz_ui),
[fuz_app](https://github.com/fuzdev/fuz_app),
[Gro](https://github.com/fuzdev/gro), and Rust.

## License

[MIT](LICENSE)
