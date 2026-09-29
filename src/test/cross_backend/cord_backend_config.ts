/**
 * The cross-backend `BackendConfig` for cord's one backend, the Rust `cordd`,
 * spawned as its test sibling `testing_cordd` (the fast argon2 hasher, the
 * `_testing_reset` action, the daemon-token credential, and disabled rate
 * limiters over the same `run_app`).
 *
 * The shared shape (the `/api/*` paths, the bootstrap block, cookie keys,
 * origins, and the `FUZ_TESTING_RESET_DB_ON_STARTUP` self-wipe) comes from
 * fuz_app's `make_default_rust_backend_config`; this declares only what
 * differs.
 *
 * The binary needs a real Postgres database (`tokio-postgres` can't reach
 * PGlite). Create it once:
 *
 * ```bash
 * createdb cord_test_rust
 * ```
 *
 * `CORD_CROSS_BACKEND_TEST_URL` overrides the URL. The binary drops and
 * recreates the database's `public` schema on every boot, so point it only at
 * a throwaway database.
 *
 * @module
 */

import type { BackendConfig } from '@fuzdev/fuz_app/testing/cross_backend/backend_config.ts';
import { build_test_backend_paths } from '@fuzdev/fuz_app/testing/cross_backend/build_test_backend_paths.ts';
import {
	make_default_rust_backend_config,
	rust_default_capabilities
} from '@fuzdev/fuz_app/testing/cross_backend/default_backend_configs.ts';

/** The default test database; `CORD_CROSS_BACKEND_TEST_URL` overrides it. */
export const CORD_CROSS_BACKEND_DEFAULT_URL = 'postgres://localhost/cord_test_rust';

/** Must match `CORD_SESSION_COOKIE_NAME` in `crates/cord_server/src/router.rs`. */
export const CORD_SESSION_COOKIE_NAME = 'cord_session';

/**
 * `testing_cordd` over real HTTP. `cargo run` keeps cargo the source of
 * truth, so a stale `target/` binary never serves silently; the first spawn
 * builds.
 *
 * Capabilities: cord mounts `/ready` and the admin audit SSE stream but no
 * cell or fact surface.
 */
export const rust_backend_config = (): BackendConfig => {
	const name = 'rust';
	const paths = build_test_backend_paths(`cord_cross_${name}`);
	return make_default_rust_backend_config({
		name,
		port: 14261,
		cookie_name: CORD_SESSION_COOKIE_NAME,
		capabilities: {
			...rust_default_capabilities,
			ready: true,
			sse: true,
			cell_crud: false,
			cell_relations: false,
			fact_serving: false
		},
		// `-p`: the workspace's `default-members` is `cord_server` alone
		start_command: ['cargo', 'run', '-p', 'testing_cord_server', '--bin', 'testing_cordd'],
		database_url: process.env.CORD_CROSS_BACKEND_TEST_URL || CORD_CROSS_BACKEND_DEFAULT_URL,
		// `RUST_LOG` targets are crate names
		rust_log: 'info,cord_server=info,testing_cord_server=info',
		paths,
		extra_env: {
			// pin loopback IPv4; the helper's `localhost` can resolve to `::1`
			HOST: '127.0.0.1',
			// the daemon token lands at `{root}/run/daemon_token`, which is
			// `paths.daemon_token_path`, the keeper credential the harness reads
			CORD_DAEMON_TOKEN_DIR: paths.root
		}
	});
};
