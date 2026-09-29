# cord

> a self-hosted hangout for friend groups and small communities: rooms, chat, forums, and Bluesky feeds over the fuz spine

cord is a static web app built with the fuz stack:
TypeScript, Svelte 5 (runes), SvelteKit with the static adapter, fuz_css,
fuz_ui, and Gro as the task runner.

## Gro commands

```bash
gro check     # typecheck, test, lint, format check (run before committing)
gro typecheck # typecheck only (faster iteration)
gro test      # run tests with vitest
gro build     # build for production (static adapter)
gro deploy    # build, commit, and push to deploy branch
gro sync      # regenerate files and run svelte-kit sync
```

IMPORTANT for AI agents: Do NOT run `gro dev` — the developer manages the
dev server.

## Architecture

- `src/routes/` — SvelteKit routes; `+layout.ts` exports `prerender = true`
  for full static generation
- `src/lib/` — library code (components, utilities)
- fuz_css utility classes are generated on demand by the
  `vite_plugin_fuz_css` Vite plugin via the `virtual:fuz.css` module
- Dark/light theme detection runs in `src/app.html` before render to prevent
  theme flash

## Project standards

- TypeScript strict mode
- Svelte 5 with runes API
- tsv (`gro format`) with tabs, 100 char width

## Rust workspace

The repo doubles as a Rust workspace: root `Cargo.toml` with the fuz
ecosystem's canonical lints (`unsafe_code = "forbid"`, clippy
pedantic/nursery at warn) and release profile, crates under `crates/`, and
the toolchain pinned in `rust-toolchain.toml`. Do not use Gro for the Rust
side — run cargo directly:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

CI runs these in the `rust` job of `.github/workflows/check.yml`. Errors
follow thiserror enums with `.exit_code()`/`.hint()` and
`fn main() -> ExitCode`; exit codes: `0` success, `2` caller-must-fix,
`1` everything else.
