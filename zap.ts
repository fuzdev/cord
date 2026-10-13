/**
 * zap configuration for a Cord instance on a dedicated Debian 13 host.
 *
 * One host runs nginx, PostgreSQL, and the `cordd` systemd service. nginx
 * terminates TLS, serves the prerendered frontend from `build/`, and proxies
 * `/api`, `/health`, and `/ready` to `cordd` on loopback. The domain and the
 * rest of the operator-side values are `zap.env('VAR')` references resolved from the
 * operator's env file at apply time; see `.env.production.example` and
 * `DEPLOY.md`.
 *
 * The host is dedicated: the nginx recipe's rate-limit zone name is fixed, so
 * two zap-managed sites can't share one nginx.
 *
 * Hardening:
 * - a dedicated `cord` user (not root), with root's SSH keys copied over
 * - SSH hardening (root login disabled, key-only auth), applied last
 * - a hardened systemd unit (`ProtectSystem=strict`, `ProtectHome=read-only`,
 *   the kernel and namespace restrictions; the one writable path is the
 *   config dir, since `cordd` deletes the bootstrap token once it's claimed)
 * - fail2ban and ufw
 * - PostgreSQL over TCP localhost (trust auth for the loopback `cord` role)
 *
 * ## SSH lockout prevention
 *
 * The order of operations is designed for recovery:
 * 1. the `cord` user is created early, with root's keys (alternate access)
 * 2. `verify-post-firewall` aborts if SSH stops working after the firewall
 * 3. `verify-pre-hardening` re-checks the keys and sudo before root login is
 *    disabled, the very last step
 *
 * The first apply on a fresh host connects as `bootstrap_user` (`zap apply
 * --bootstrap`); every later run connects as `cord`.
 *
 * @module
 */

import type { ZapBuilder } from '@fuzdev/zap/zap.ts';
import { fail2ban_jail } from '@fuzdev/zap/recipe_fail2ban.ts';
import { ssh_hardening } from '@fuzdev/zap/recipe_ssh.ts';
import { debian_ssh_reload } from '@fuzdev/zap/recipe_debian_ssh.ts';
import {
	ssl_params,
	rate_limit_zone,
	NGINX_VALIDATE,
	nginx_config_verify
} from '@fuzdev/zap/recipe_nginx.ts';
import {
	debian_nginx_spa_site,
	debian_nginx_remove_default
} from '@fuzdev/zap/recipe_debian_nginx.ts';
import { debian_web_firewall } from '@fuzdev/zap/recipe_debian_firewall.ts';
import { hardened_systemd_unit } from '@fuzdev/zap/recipe_debian_systemd.ts';
import { postgres_apt_repo } from '@fuzdev/zap/recipe_debian_postgres.ts';
import { debian_certbot_stop_nginx } from '@fuzdev/zap/recipe_debian_certbot.ts';
import { certbot_standalone_issue, certbot_webroot_switch } from '@fuzdev/zap/recipe_certbot.ts';
import { spa_csp } from '@fuzdev/zap/recipe_csp.ts';
import { fuz_app_bootstrap_token } from '@fuzdev/zap/recipe_bootstrap.ts';
import { fuz_app_env_base } from '@fuzdev/zap/recipe_env.ts';

//
// Constants
//

/** The service account, its database role, and the SSH login after bootstrap. */
const DEPLOY_USER = 'cord';
/** The PostgreSQL database. */
const DATABASE_NAME = 'cord';
/** The systemd unit, named for the binary it runs. */
const SERVICE_NAME = 'cordd';
/** `cordd`'s loopback port, where nginx proxies. */
const PORT = '4061';

const HOME_DIR = `/home/${DEPLOY_USER}`;
/** Holds the one-shot bootstrap token; mode 700. */
const CONFIG_DIR = `${HOME_DIR}/.cord`;
const APP_DIR = `${HOME_DIR}/app`;
const RELEASES_DIR = `${APP_DIR}/releases`;
const SHARED_DIR = `${APP_DIR}/shared`;
const CURRENT_LINK = `${APP_DIR}/current`;

//
// Templates
//

// The host's env file: the shared fuz_app base, then cord's own lines.
// `CORD_TRUSTED_PROXIES` is written by hand because zap's trusted-proxies
// recipe only emits `FUZ_<infix>_TRUSTED_PROXIES`, a name `cordd` doesn't read.
const ENV_TEMPLATE = `${fuz_app_env_base({
	user: DEPLOY_USER,
	service: DATABASE_NAME,
	config_dir: CONFIG_DIR
})}
# Security - allowed origins for API requests
FUZ_ALLOWED_ORIGINS=https://{{domain}}

# Trust the loopback nginx reverse proxy so cordd resolves the real client IP
# from X-Forwarded-For (nginx connects from 127.0.0.1 and overwrites the header
# with the client's address). Without it every request is attributed to
# 127.0.0.1: the per-IP rate limits, invite redemption's included, collapse
# into one shared bucket, and the audit log's IPs are useless.
CORD_TRUSTED_PROXIES=127.0.0.1

# Plain-text logs for journald (cordd colors its logs unless NO_COLOR is set)
NO_COLOR=1
`;

// Content-Security-Policy for the prerendered SPA: the locked-down fuz_app
// baseline. The frontend loads nothing from other origins. `connect-src`
// carries `wss://{{domain}}` for the WebSocket.
const CSP = spa_csp();

// The rate-limit zone `rate_limit_zone` defines, read back from its own output
// rather than retyped, so the audit-stream location below can reference it and
// a rename in the recipe can't leave a dangling `limit_req`.
const RATE_LIMIT_ZONE_NAME = (() => {
	const match = /zone=([A-Za-z0-9_]+):/.exec(rate_limit_zone());
	if (!match?.[1]) throw Error('rate_limit_zone() output has no zone name');
	return match[1];
})();

// Cord's own nginx locations, injected into the HTTPS server block.
//
// The admin audit log streams server-sent events from
// `/api/admin/audit/stream`. The recipe's `location /api` would buffer the
// stream and close it after 60s without a byte, and the stream is quiet between
// audit events, so an open audit page would drop and reconnect every minute.
// This exact-match location wins over the `/api` prefix and carries the same
// proxy headers (the `X-Forwarded-For` overwrite is what makes
// `CORD_TRUSTED_PROXIES` safe), with buffering off and an hour's read timeout;
// the browser's EventSource reconnects after that and fills the gap. It keeps
// the `/api` rate limit, since an exact match escapes the prefix block's.
//
// No `add_header` here: nginx drops inherited `add_header` directives in any
// block that sets its own, so staying silent keeps the server-level security
// headers.
//
// `/api/ws` stays on the recipe's `/api` block, which carries the upgrade
// headers; its 60s read timeout needs revisiting once the frontend holds a
// WebSocket open.
const EXTRA_LOCATIONS = `    # Admin audit log stream (server-sent events): unbuffered, long-lived.
    location = /api/admin/audit/stream {
        limit_req zone=${RATE_LIMIT_ZONE_NAME} burst=5 nodelay;

        proxy_pass http://127.0.0.1:{{port}};
        proxy_http_version 1.1;
        proxy_set_header Connection '';
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Authorization $http_authorization;

        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 1h;
    }`;

// The SPA fallback. The static build prerenders `/admin` as `admin.html` beside
// an `admin/` directory with no `index.html`, so `$uri.html` must come before
// any directory match (a `$uri/` entry would 403 on `/admin/`), and the
// fallback is the build's `spa.html`, which serves `/c/<cord_id>` and its tabs
// and renders the SPA's 404 for unknown paths.
const SPA_TRY_FILES = '$uri $uri.html $uri/index.html /spa.html';

//
// zap configuration
//

export default (zap: ZapBuilder): void => {
	//
	// Target: the host over SSH, as `cord` once provisioned. A fresh host has no
	// `cord` user yet, so the first apply runs with `--bootstrap` and connects as
	// `bootstrap_user` (root on a stock Debian image) instead.
	//

	zap.target({
		host: zap.env('DEPLOY_HOST'),
		user: DEPLOY_USER,
		bootstrap_user: zap.env('BOOTSTRAP_USER'),
		package_manager: 'apt',
		service_manager: 'systemd'
	});

	//
	// Phase 1: Foundation
	//

	// Set a root password for console recovery out of band (`sudo passwd root`
	// on first console access); it's a one-time step, not convergence state.

	// Swap file, so a small host doesn't OOM during apt or a burst
	zap.swap({
		id: 'swap-file',
		name: '2GB swap file',
		size: '2G'
	});

	//
	// Phase 2: User
	//

	zap.user({
		id: 'create-user',
		name: 'Create cord user with SSH keys and sudo',
		username: DEPLOY_USER,
		shell: '/bin/bash',
		home_mode: '711', // lets nginx traverse the home directory to build/
		ssh_keys: { copy_from: 'root' },
		sudo: 'nopasswd',
		depends_on: ['swap-file']
	});

	//
	// Phase 3: Packages (the planner batches them and updates the cache)
	//

	zap.pkg({ id: 'curl', name: 'curl', from: { apt: 'curl' } });
	zap.pkg({ id: 'fail2ban', name: 'fail2ban intrusion prevention', from: { apt: 'fail2ban' } });
	zap.pkg({ id: 'nginx', name: 'nginx web server', from: { apt: 'nginx' } });
	zap.pkg({ id: 'certbot', name: 'certbot SSL tool', from: { apt: 'certbot' } });
	zap.pkg({ id: 'jq', name: 'jq JSON processor', from: { apt: 'jq' } });
	zap.pkg({ id: 'ufw', name: 'ufw firewall', from: { apt: 'ufw' } });
	zap.pkg({
		id: 'needrestart',
		name: 'needrestart auto-restart helper',
		from: { apt: 'needrestart' }
	});

	// PostgreSQL 18 from the PGDG apt repository
	zap.apt_repo({
		id: 'postgresql-repo',
		name: 'PostgreSQL apt repository',
		...postgres_apt_repo(),
		depends_on: ['curl']
	});

	zap.pkg({
		id: 'postgresql',
		name: 'PostgreSQL 18 database',
		from: { apt: 'postgresql-18' },
		depends_on: ['postgresql-repo']
	});

	//
	// Phase 4: Security
	//

	zap.file({
		id: 'fail2ban-config',
		name: 'fail2ban jail.local',
		path: '/etc/fail2ban/jail.local',
		content: { type: 'inline', content: fail2ban_jail() },
		depends_on: ['fail2ban']
	});

	zap.service({
		id: 'fail2ban-service',
		name: 'fail2ban service',
		service: 'fail2ban',
		enabled: true,
		running: true,
		restart_on: ['fail2ban-config'],
		depends_on: ['fail2ban-config']
	});

	// SSH, HTTP, and HTTPS in; everything else denied
	zap.firewall({
		id: 'ufw-firewall',
		name: 'Configure ufw firewall',
		...debian_web_firewall(),
		depends_on: ['nginx', 'ufw']
	});

	// Checkpoint: SSH still works after the firewall
	zap.verify({
		id: 'verify-post-firewall',
		name: 'Verify SSH after firewall',
		assertions: [{ type: 'command', run: 'true' }],
		depends_on: ['ufw-firewall']
	});

	//
	// Phase 5: PostgreSQL
	//

	// PostgreSQL's stock `default_transaction_isolation` is read committed,
	// which `cordd` requires at boot; nothing here changes it.
	zap.database({
		id: 'postgresql-init',
		name: 'Initialize PostgreSQL database',
		engine: 'postgresql',
		database_name: DATABASE_NAME,
		username: DEPLOY_USER,
		depends_on: ['postgresql', 'verify-post-firewall']
	});

	//
	// Phase 6: Directories
	//

	zap.directory({
		id: 'app-dir',
		name: 'Application directory',
		path: APP_DIR,
		mode: '755',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: ['verify-post-firewall', 'create-user']
	});

	zap.directory({
		id: 'releases-dir',
		name: 'Releases directory',
		path: RELEASES_DIR,
		mode: '755',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: ['app-dir']
	});

	zap.directory({
		id: 'shared-dir',
		name: 'Shared directory',
		path: SHARED_DIR,
		mode: '755',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: ['app-dir']
	});

	// Runtime state: the bootstrap token. It must exist before systemd starts
	// the unit, since a missing `ReadWritePaths` entry fails namespace setup
	// (status 226).
	zap.directory({
		id: 'cord-config-dir',
		name: 'cord config directory',
		path: CONFIG_DIR,
		mode: '700',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: ['create-user']
	});

	//
	// Phase 7: Configuration files
	//

	// The one-shot token for claiming the first account at /admin. Minted once;
	// the check also passes after `cordd` has consumed and deleted it (the
	// `bootstrap_lock` row). See `recipe_bootstrap` for rotating a stuck token.
	zap.script({
		id: 'bootstrap-token',
		name: 'Create bootstrap token',
		...fuz_app_bootstrap_token({
			user: DEPLOY_USER,
			service: DATABASE_NAME,
			config_dir: CONFIG_DIR
		}),
		depends_on: ['cord-config-dir', 'postgresql-init']
	});

	// The env `cordd` runs with, loaded by the unit's `EnvironmentFile`. It
	// names only the token's path, so it doesn't wait on the token.
	zap.file({
		id: 'env-production',
		name: 'Production environment',
		path: `${SHARED_DIR}/.env.production`,
		content: {
			type: 'template',
			template: ENV_TEMPLATE,
			vars: {
				port: PORT,
				domain: zap.env('DEPLOY_DOMAIN'),
				cookie_keys: zap.env('SECRET_FUZ_COOKIE_KEYS')
			}
		},
		mode: '600',
		owner: DEPLOY_USER,
		group: DEPLOY_USER,
		depends_on: ['shared-dir', 'postgresql-init']
	});

	zap.file({
		id: 'systemd-service',
		name: 'systemd service unit',
		path: `/etc/systemd/system/${SERVICE_NAME}.service`,
		content: {
			type: 'inline',
			content: hardened_systemd_unit({
				description: 'cordd, the Cord server',
				exec_start: `${CURRENT_LINK}/dist_artifacts/cordd`,
				user: DEPLOY_USER,
				working_directory: CURRENT_LINK,
				read_write_paths: [CONFIG_DIR],
				env_file: `${SHARED_DIR}/.env.production`,
				hardening_comment: `# Additional sandboxing for an Axum/tokio daemon over local PostgreSQL (TCP).
# AF_UNIX+AF_INET[6] cover the TCP listener and the Postgres connection.
# The only write is deleting the claimed bootstrap token in the config dir.
# MemoryDenyWriteExecute / SystemCallFilter stay off, as on the other
# fuz_app hosts, for the argon2/tokio runtime.`
			})
		},
		depends_on: ['env-production']
	});

	zap.service({
		id: 'cordd-service',
		name: `${SERVICE_NAME} service`,
		service: SERVICE_NAME,
		enabled: true,
		// no `running: true`; the first apply's restart trigger does start it, and
		// it retries every 5s until the first deploy lands a binary
		// an env change (e.g. a cookie-key rotation) must reach the process too
		restart_on: ['systemd-service', 'env-production'],
		depends_on: ['systemd-service', 'cord-config-dir']
	});

	//
	// Phase 8: TLS certificate
	//
	// The first issuance uses `certbot certonly --standalone` (never `--nginx`,
	// so certbot never edits the nginx config): it stops nginx briefly to free
	// port 80, then the nginx config below brings it back with the SSL block.
	// Renewal runs unattended from certbot.timer and can't stop nginx, so
	// `certbot-renewal-webroot` re-issues once over `--webroot` to switch the
	// renewal authenticator.
	//

	zap.directory({
		id: 'certbot-webroot-dir',
		name: 'ACME challenge webroot',
		path: '/var/www/certbot',
		mode: '755',
		owner: 'root',
		group: 'root',
		depends_on: ['create-user']
	});

	zap.script({
		id: 'stop-nginx-for-certbot',
		name: 'Stop nginx for certbot standalone (fresh-host bootstrap only)',
		...debian_certbot_stop_nginx(),
		depends_on: ['nginx', 'certbot']
	});

	zap.script({
		id: 'ssl-certificate',
		name: 'SSL certificate via certbot (bootstrap)',
		...certbot_standalone_issue(),
		depends_on: ['stop-nginx-for-certbot']
	});

	// Idempotent: once the renewal config records webroot, the check passes.
	zap.script({
		id: 'certbot-renewal-webroot',
		name: 'Switch cert renewal to webroot',
		...certbot_webroot_switch(),
		depends_on: ['certbot-webroot-dir', 'nginx-service']
	});

	//
	// Phase 9: nginx
	// zap owns the whole config, SSL directives included; certbot only manages
	// the cert files. The site waits on the certificate so its files exist when
	// nginx starts.
	//

	zap.file({
		id: 'nginx-ssl-params',
		name: 'nginx shared SSL parameters',
		path: '/etc/nginx/snippets/ssl_params.conf',
		content: { type: 'inline', content: ssl_params() },
		validate: NGINX_VALIDATE,
		depends_on: ['nginx']
	});

	zap.file({
		id: 'nginx-rate-limit',
		name: 'nginx rate limit zone',
		path: '/etc/nginx/conf.d/rate_limit.conf',
		content: { type: 'inline', content: rate_limit_zone() },
		validate: NGINX_VALIDATE,
		depends_on: ['nginx']
	});

	zap.file({
		id: 'nginx-config',
		name: 'nginx site config',
		path: `/etc/nginx/sites-available/${zap.env('DEPLOY_DOMAIN')}`,
		content: {
			type: 'template',
			template: debian_nginx_spa_site({
				root: `${CURRENT_LINK}/build`,
				csp: CSP,
				client_max_body_size: '1m',
				client_max_body_size_comment: 'cordd takes no uploads; its JSON bodies stay small.',
				spa_try_files: SPA_TRY_FILES,
				extra_locations: EXTRA_LOCATIONS
			}),
			vars: {
				domain: zap.env('DEPLOY_DOMAIN'),
				port: PORT
			}
		},
		validate: NGINX_VALIDATE,
		depends_on: ['nginx', 'ssl-certificate', 'nginx-ssl-params', 'nginx-rate-limit']
	});

	zap.link({
		id: 'nginx-enabled',
		name: 'Enable nginx site',
		path: `/etc/nginx/sites-enabled/${zap.env('DEPLOY_DOMAIN')}`,
		target: `/etc/nginx/sites-available/${zap.env('DEPLOY_DOMAIN')}`,
		depends_on: ['nginx-config']
	});

	// One-time: remove the default site, then validate the config
	zap.script({
		id: 'nginx-remove-default',
		name: 'Remove default nginx site',
		...debian_nginx_remove_default(),
		depends_on: ['nginx-enabled']
	});

	zap.service({
		id: 'nginx-service',
		name: 'nginx service',
		service: 'nginx',
		enabled: true,
		running: true,
		reload_on: ['nginx-config', 'nginx-ssl-params', 'nginx-rate-limit', 'nginx-enabled'],
		depends_on: ['nginx-remove-default']
	});

	// Re-checks the on-disk config with `nginx -t` on every plan
	zap.verify({
		id: 'nginx-config-valid',
		name: 'nginx config passes nginx -t',
		...nginx_config_verify(),
		depends_on: ['nginx-service']
	});

	//
	// Phase 10: SSH hardening, last
	// Disables root login and password auth; only `cord`, with a key, gets in.
	//

	// The last chance to abort before root login is disabled
	zap.verify({
		id: 'verify-pre-hardening',
		name: 'Final verification before SSH hardening',
		assertions: [
			{
				type: 'path_exists',
				path: `${HOME_DIR}/.ssh/authorized_keys`,
				mode: '600',
				owner: DEPLOY_USER,
				not_empty: true
			},
			{ type: 'command', run: 'sudo whoami', expect: 'root' }
		],
		depends_on: ['nginx-service']
	});

	zap.file({
		id: 'ssh-hardened-config',
		name: 'SSH hardening config',
		path: '/etc/ssh/sshd_config.d/99-hardened.conf',
		content: { type: 'inline', content: ssh_hardening({ user: DEPLOY_USER }) },
		depends_on: ['verify-pre-hardening']
	});

	zap.script({
		id: 'ssh-reload',
		name: 'Reload SSH with hardened config',
		...debian_ssh_reload(),
		depends_on: ['ssh-hardened-config']
	});

	//
	// Deploy
	//

	// Built locally: the release audit first (no production binary may link
	// test-only code), then `cordd`, then the static frontend. The copy comes
	// after `gro build`, which deletes `dist_*` directories as it starts.
	// `/ready` is the gate: it answers 200 only when the live schema matches
	// the release's `expected_schema.json`, so a release that can't serve its
	// schema, or doesn't come up at all, rolls back. Migrations run at boot and
	// aren't undone by a rollback, so they must stay additive.
	zap.deploy({
		build:
			'cargo xtask check-release && cargo build --release --bin cordd && gro build && mkdir -p dist_artifacts && cp target/release/cordd dist_artifacts/cordd',
		artifacts: ['build/', 'dist_artifacts/'],
		path: APP_DIR,
		releases: 5,
		service: SERVICE_NAME,
		health: { domain: zap.env('DEPLOY_DOMAIN'), path: '/ready' }
	});
};
