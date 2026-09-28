# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds in one instance you run yourself.

**Status: design phase.** This repo is a fresh, unmolted fuz_template
clone — the package identity, deps, and source tree are still the
template's. No Cord implementation exists yet; treat the current code as
scaffolding, not product.

The intended shape, briefly: a **cord** is the group; it holds **spaces**
of several kinds — a **line** (a live room: voice, video, screenshare,
ephemeral text, no history), **chat** (persisted messages), **forum**
(threads with nested comments and reactions), and **feed** (a Bluesky
feed, list, or account the cord pins, rendered reddit-style or as a
timeline). Identity is an atproto DID (Bluesky or any PDS OAuth; no local
accounts). The backend is an API-only Rust server over the fuz spine
crates (auth, db, actions, realtime, cells, facts) with a SvelteKit static
frontend, nginx, Postgres, and coturn for WebRTC (full mesh), deployed by
zap. The live room is the first thing built; persisted spaces layer on it.
Not federation, not a hosted service, not engagement-driven.

For coding conventions, see Skill(fuz-stack).

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

For Rust crates, use cargo directly (`cargo check`, `cargo clippy`,
`cargo test`), not Gro.
