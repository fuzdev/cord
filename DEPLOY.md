# Deploying Cord

A Cord instance runs on one dedicated Debian 13 host, provisioned and
deployed by [zap](https://zap.fuz.dev/) from [`zap.ts`](zap.ts): nginx
serves the static frontend and proxies `/api`, `/health`, and `/ready` to
`cordd`, which runs as a hardened systemd service over a local PostgreSQL.

zap is dry-run by default: each command below previews its plan, and
`--wetrun` carries it out. The `deno task` names come from
[`deno.json`](deno.json) and pass `--env-file .env.production zap.ts`.
The zap recipes resolve through `node_modules/@fuzdev/zap`, the link
`npm install` makes to the zap checkout, so `zap` finds them from any
working directory.

## What you need

- `zap` and `deno` on your `PATH` (zap evaluates `zap.ts` under Deno)
- this repo checked out beside fuz's Rust server crates (the `cordd` build,
  see the README) and beside zap, which `package.json` names as the
  `@fuzdev/zap` dev dependency (run `npm install` to link it); keep the
  installed `zap` binary built from that checkout
- the Rust toolchain and `gro`, for the build zap runs locally; `cordd` is
  built on your machine and shipped as-is, so build on Linux x86_64 with a
  glibc no newer than the host's (Debian 13 building for Debian 13 is fine;
  a Mac isn't)
- `sudo` on the host: zap runs every step through it, even as root, and
  some minimal Debian images lack it

## 1. The host and DNS

- A fresh Debian 13 host that serves nothing else: the nginx config zap
  writes assumes it's the only zap-managed site on the host.
- SSH access as root with your key. Provisioning copies root's
  `authorized_keys` to the `cord` user, then turns root login off.
- A DNS `A` record (and `AAAA`, if the host has IPv6) for the domain,
  pointing at the host. It has to resolve before the first apply, since
  certbot proves the domain over HTTP.

## 2. The operator env

```bash
cp .env.production.example .env.production   # gitignored
```

Fill in `DEPLOY_HOST`, `DEPLOY_CERTBOT_EMAIL`, and `SECRET_FUZ_COOKIE_KEYS`
(`openssl rand -hex 32`), and check the rest. This file stays on your
machine; zap generates the host's env file for `cordd` from `zap.ts`.

`deno task validate` evaluates the config offline, touching no host.

## 3. Provision

```bash
deno task apply:bootstrap            # preview, connecting as BOOTSTRAP_USER
deno task apply:bootstrap --wetrun
deno task apply                      # as cord from here on: preview the rest
deno task apply --wetrun             # converge it (a wetrun runs only a previewed plan)
```

The bootstrap run installs nginx, PostgreSQL 18, certbot, fail2ban, and
ufw, creates the `cord` user and database, issues the TLS certificate,
writes the systemd unit and the env file, mints the one-time bootstrap
token, and last of all hardens SSH. After it, root can't log in, and every
run connects as `cord`. A bootstrap run that stops partway is safe to rerun
with `--bootstrap`, since only its last step locks root out.

## 4. Deploy

```bash
deno task deploy                     # preview
deno task deploy --wetrun
```

zap builds locally (`cargo xtask check-release`, the release `cordd`,
`gro build`), uploads `build/` and `dist_artifacts/`, swaps the `current`
release, restarts `cordd`, and polls `https://<domain>/ready`. `/ready`
answers 200 only once `cordd` is up and the database schema matches the
release, so a release that doesn't get there is rolled back. The deploy
refuses when the host has drifted from `zap.ts` (run `deno task apply
--wetrun` first). Commit before deploying: a wetrun from a dirty tree is
refused; `--force` ships it as its own release, `<hash>-dirty-<id>`.

## 5. Claim the instance

```bash
ssh cord@<host> 'cat ~/.cord/secret_bootstrap_token'
```

Open `https://<domain>/admin`, paste the token, and choose a username and
password. That first account is the instance admin. `cordd` deletes the
token once it's used, and the claim can't be made again.

## 6. The first cord

Signed in as the admin, the home page offers "create your first cord".
From the cord's invites tab you can make invite links, including ones
that bring someone new onto the instance; `/admin/invites` mints plain
signup invites.

## 7. Smoke test

- `curl -i https://<domain>/health` and `curl -i https://<domain>/ready`
  both answer 200
- log out and back in
- an invite round trip: from the cord's invites tab, make a link for
  someone new, open it in a private window, choose a username, sign up,
  and join; the new account shows in the cord's members
- `/admin/audit`: the new events list your real IP, not `127.0.0.1`; press
  "stream", leave it idle for more than a minute, and it still reads
  "connected"; a login elsewhere then shows up live
- `journalctl -u cordd` on the host reads as plain text

## Operating notes

- **Backups aren't set up yet.** Until a nightly `pg_dump` lands, take
  dumps by hand: `ssh cord@<host> 'pg_dump -Fc cord' > cord.dump`.
- **Migrations run when `cordd` starts**, and a rollback swaps the release
  back without undoing them, so every migration has to be additive.
- **Voice and video relay** (a TURN server) arrives with the live room;
  this config doesn't install one yet.
- **PostgreSQL** is reached over its Unix socket as the `cord` account (peer
  auth), so no password is involved.
- **nginx**: zap runs `nginx -t` on every config write and on every plan.
- **Logs**: `journalctl -u cordd`.
