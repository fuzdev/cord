# Cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting:
the server skeleton runs, but none of the features described below work
yet.

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
  the same thing, read as reddit-style threads or as a timeline.

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
- the server skeleton, `crates/cord_server` (the `cordd` binary): accounts
  and sessions, invite-gated signup, first-account bootstrap, a JSON-RPC
  endpoint, a WebSocket endpoint, and health and readiness checks, with no
  Cord features on top yet

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
  Deploy with docker compose behind your own reverse proxy, or with zap,
  the fuz stack's deploy tool; both are planned, along with a self-hosting
  guide. Only the optional Bluesky connection needs the instance at a
  public HTTPS origin; everything else runs fine on a LAN. There is no
  hosted service.

## Developing

Prerequisites:

- Node.js, the version in `package.json` `engines`
- the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it
  automatically)
- [Gro](https://github.com/fuzdev/gro) for the `gro` commands, installed
  globally (`npm i -g @fuzdev/gro`); the `npm run` scripts work without it

```bash
npm i
gro dev    # or `npm run dev`, the frontend dev server
gro check  # typecheck, test, lint, format check
gro build  # production build of the static frontend
```

### The server

`cordd` is built on fuz's Rust server crates, which aren't published yet.
Building it needs those crates checked out beside this repo, at the path
named in the root `Cargo.toml`; without them, cargo can't resolve the
workspace. The frontend builds and checks without them.

The Rust side uses cargo directly:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

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
| `FUZ_ALLOWED_ORIGINS`      | required origin allowlist, comma-separated (`http://localhost:*`)      |
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
- `/api/rpc` (GET and POST): JSON-RPC 2.0 with the account and admin
  actions
- `/api/ws`: the WebSocket for realtime actions
- `GET /api/admin/audit/stream`: the audit log as a live server-sent event
  stream, for admins signed in with a session

`cordd db-status` prints a read-only report of the database's tables and
migrations.

Built with the fuz stack: TypeScript,
[Svelte](https://github.com/sveltejs/svelte),
[SvelteKit](https://github.com/sveltejs/kit),
[fuz_css](https://github.com/fuzdev/fuz_css),
[fuz_ui](https://github.com/fuzdev/fuz_ui),
[fuz_app](https://github.com/fuzdev/fuz_app),
[Gro](https://github.com/fuzdev/gro), and Rust.

## License

[MIT](LICENSE)
