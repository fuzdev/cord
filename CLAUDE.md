# cord

> A self-hosted place for a friend group or small community to hang out:
> voice, video, and screenshare rooms, live chat, forums, and your Bluesky
> feeds, in one instance you run yourself.

**Status: early.** The design is settled and implementation is starting.
The repo is a SvelteKit static frontend shell plus a Rust workspace whose
one crate, `crates/cord`, is a placeholder CLI that becomes the server
crate `cord_server` (bin `cordd`). No Cord feature exists yet; treat the
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
- `crates/cord/` — the Rust crate (today a starter CLI: argh args,
  thiserror errors with `.exit_code()`/`.hint()`)

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

The root `Cargo.toml` is the workspace: shared package metadata, workspace
dependencies, the release profile, and the fuz ecosystem's lints
(`unsafe_code = "forbid"`, clippy all/pedantic/nursery/cargo at warn,
`unwrap_used`/`expect_used`/`panic` at warn, allowed in tests by
`clippy.toml`). Crates opt in with `[lints] workspace = true`. CI denies
warnings, which is why `rust-toolchain.toml` pins the toolchain. Exit
codes: `0` success, `2` the caller must fix something, `1` everything else.
