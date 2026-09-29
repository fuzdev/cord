# Cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting;
nothing described below works yet.

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

- a SvelteKit static frontend shell (`src/routes/`)
- a Rust workspace with one starter crate, `crates/cord`, a placeholder CLI
  that will grow into the server

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

The Rust side uses cargo directly:

```bash
cargo run -p cord
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Built with the fuz stack: TypeScript,
[Svelte](https://github.com/sveltejs/svelte),
[SvelteKit](https://github.com/sveltejs/kit),
[fuz_css](https://github.com/fuzdev/fuz_css),
[fuz_ui](https://github.com/fuzdev/fuz_ui),
[Gro](https://github.com/fuzdev/gro), and Rust.

## License

[MIT](LICENSE)
