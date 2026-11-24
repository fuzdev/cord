# Cord

[<img src="static/logo.svg" alt="two teal socket faces leaning apart, a plug reaching toward each, joined by one looped cord" align="right" width="192" height="192">](https://cord.fuz.dev/)

> self-hosted voice, video, chat, and forums for connection with your people 🔌 pre-alpha

> ⚠️ **Not ready to be used.** There is no release and no supported install.
> The server runs, an instance admin can create cords, and people can be
> invited in and add spaces, but nothing happens inside a space yet, so the
> features described below don't work. APIs, wire shapes, and the database
> schema break without notice, and its security is not ready to be trusted.

_plug unplug_

Cord is a self-hosted place for your people: a family, a friend group, a
club, a team. Voice, video, and screenshare rooms, live chat, and forums,
with your Bluesky and ActivityPub feeds and the RSS, Atom, and JSON Feed
sources you follow, in one instance you run yourself.

Think of the coiled telephone cord stretched from the kitchen wall into
your bedroom, or everyone on the party line at once. Cord is meant to feel
like that: warm, low-key, a room you already know.

Cord is part of the Fuz stack
([fuz.dev](https://www.fuz.dev/), [@fuzdev](https://github.com/fuzdev)).

AI disclosure: this codebase is LLM-generated with uneven human review.

## What it is

A **cord** is your group ("join our cord"). Inside it, everything is a
**space**, and each space has a kind:

- **line**: a live room with voice, video, screenshare, and text that
  isn't saved. You join silent, and your mic, camera, and screen each stay
  off until you turn them on.
- **chat**: messages that stay, with unread markers so you can catch up.
- **forum**: threads with nested comments and reactions, sorted by
  activity, newest, or top.
- **feed**: a source the cord pins so everyone sees the same thing: a
  Bluesky or ActivityPub feed, list, or account, or an RSS, Atom, or JSON
  Feed source. Read it as a list of posts that each open into their
  thread, or as a timeline.

The same spaces can be seen through different views: on desktop, a sidebar
with the active space beside it, or a dashboard of spaces in columns side
by side; on a phone, a single column.

Accounts belong to your instance, the operator decides who can sign up,
and each cord is joined by invite. Each cord has owners, admins, and
members, and a space can be open to the whole cord or to a chosen few.

Any member can connect a Bluesky account (or any atproto PDS account) or
an ActivityPub account to get personal feed columns and to post, reply,
like, and repost from inside Cord. Nobody needs one to hang out, and the cord's shared feeds show for
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
  owner
- spaces: a cord's owners and admins add lines to it, rename them, and
  delete them; its members see them in the rail and open them as tabs on
  the cord's page, a preview tab and the ones they keep (remembered in the
  browser). The live room inside a line comes next
- an a/v check page, for anyone: your camera, mic, and output, a
  recording, and a WebRTC call to itself, all local to the browser
- the server, `crates/cord_server` (the `cordd` binary): accounts and
  sessions, invite-gated signup, first-account bootstrap, the cord,
  invite, member, and space actions, a JSON-RPC endpoint, a WebSocket
  endpoint, and health and readiness checks

The live room comes first, since hanging out is the point; chat, forums, and
feeds layer on after it.

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
- for the server only, [fuz](https://github.com/fuzdev/fuz)'s Rust server
  crates and [pglet](https://github.com/fuzdev/pglet) checked out beside
  this repo (see [The server](#the-server)); the frontend builds without
  them

```bash
npm i
gro dev    # or `npm run dev`, the frontend dev server
gro check  # typecheck, test, gen check, lint, format check
gro build  # production build of the static frontend
```

The build in `build/` is static files plus `spa.html`, which whatever
serves them must fall back to for paths with no prerendered page (a cord's
pages, `/c/<id>` and its tabs).

### The server

`cordd` is built on fuz's Rust server crates, which aren't published yet.
Building it needs a checkout of them beside this repo, at the paths the
root `Cargo.toml` names, and a checkout of pglet beside that, as `pglet/`
(fuz's test harness depends on it by path, and cargo loads it for any
build); without both, cargo can't resolve the workspace, so the server
doesn't build from this repo alone. The frontend builds and
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
(`openssl rand -hex 32`), load it into your shell, and run the server:

```bash
createdb cord
cp .env.development.example .env.development   # then edit it
set -a; . ./.env.development; set +a           # load it (bash)
cargo run -p cord_server                       # cordd, on 127.0.0.1:4061
```

`cordd` reads only its process environment:

| Variable                           | Purpose                                                                 |
| ---------------------------------- | ----------------------------------------------------------------------- |
| `DATABASE_URL`                     | Postgres connection; unset runs a boot-only server (`/health` and `/`)  |
| `FUZ_ALLOWED_ORIGINS`              | always required: comma-separated origins (`http://localhost:*`)         |
| `FUZ_ALLOW_ORIGIN_WILDCARDS`       | `true` lets a wildcard origin boot (refused otherwise); `true`/`false`  |
| `SECRET_FUZ_COOKIE_KEYS`           | required with a database: cookie-signing keys, 32+ characters each      |
| `FUZ_BOOTSTRAP_TOKEN_PATH`         | token file that opens first-account bootstrap                           |
| `FUZ_DB_CHECKOUT_TIMEOUT_MS`       | a request's pool checkout bound in ms (default 5000); failing it is 503 |
| `FUZ_DB_AUDIT_CHECKOUT_TIMEOUT_MS` | an audit write's pool checkout bound in ms (default 30000)              |
| `HOST`, `PORT`                     | bind address, default `127.0.0.1:4061`                                  |
| `CORD_TRUSTED_PROXIES`             | proxy IPs/CIDRs trusted for `X-Forwarded-For`; default loopback         |
| `NO_COLOR`                         | set, logs are plain text (production sets it for journald)              |

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
- `/api/ws`: the same actions over a WebSocket, and the notifications the
  server pushes; the frontend holds it open while signed in and sends every
  signed-in request over it, using `/api/rpc` only while signed out
- `GET /api/admin/audit/stream`: the audit log as a live server-sent event
  stream, for admins signed in with a session

`cordd db-status` prints a read-only report of the database's tables and
migrations.

### Tests that need Postgres

`gro test` and `cargo test --workspace` need no database: the Rust database
tests run each on an in-process [pglet](https://github.com/fuzdev/pglet)
server of its own. The few that test what only PostgreSQL has, or wait on
a pglet feature, are ignored there; to run every database test on
PostgreSQL, name a server and a role with `CREATEDB`, through which each
test creates and drops a database of its own:

```bash
FUZ_TEST_POSTGRES_URL=postgres://localhost/postgres cargo test --workspace -- --include-ignored
```

The end-to-end tests need Postgres, and wipe the `public` schema of their
database, so give them a throwaway one:

```bash
# end-to-end tests of auth, cords, members, invites, newcomer links, and
# spaces, plus fuz_app's shared spine suites: build testing_cordd, create
# its database (cord_test_rust) when missing, and drive it over HTTP
npm run test:cross
```

`CORD_CROSS_BACKEND_TEST_URL` overrides its database URL.

## Deploying

[`zap.ts`](zap.ts) describes a whole instance on one dedicated Debian
host: nginx serving the frontend, PostgreSQL, and `cordd` as a hardened
systemd service, provisioned and deployed with
[zap](https://zap.fuz.dev/). [DEPLOY.md](DEPLOY.md) is the runbook.
Deploying needs the `zap` binary and `fgro`, both built from source
([install](https://zap.fuz.dev/docs/install)); `package.json` installs zap's
recipes from npm. The `cordd` build also needs fuz's server crates and pglet
beside this repo.

[cord.fuz.dev](https://cord.fuz.dev/) is a static site built from the same
frontend (`npm run build:site`): the intro, about, and the a/v check, with
no server behind it.

## Built with

The fuz stack: TypeScript,
[Svelte](https://github.com/sveltejs/svelte),
[SvelteKit](https://github.com/sveltejs/kit),
[fuz_css](https://github.com/fuzdev/fuz_css),
[fuz_ui](https://github.com/fuzdev/fuz_ui),
[fuz_app](https://github.com/fuzdev/fuz_app),
[Gro](https://github.com/fuzdev/gro), and Rust.

## License

[0BSD](LICENSE)
