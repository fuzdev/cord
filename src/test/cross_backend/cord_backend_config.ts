/**
 * The cross-backend `BackendConfig` for cord's one backend, the Rust `cordd`,
 * spawned as its test sibling `testing_cordd` (the fast argon2 hasher, the
 * `_testing_reset` action, the daemon-token credential, and disabled rate
 * limiters over the same `run_app`).
 *
 * The shared shape (the `/api/*` paths, `HOST`, the bootstrap block, cookie
 * keys, and the daemon-token dir) comes from fuz_app's
 * `make_default_rust_backend_config`; this declares only what differs.
 * Origins come from it too: the `http://localhost:*` allowlist with the
 * `FUZ_ALLOW_ORIGIN_WILDCARDS` opt-in its wildcard needs, so cord sets
 * neither.
 *
 * The database is the global setup's: a `pglet_server` of its own, or a fresh
 * PostgreSQL database under `FUZ_TEST_POSTGRES_URL`.
 *
 * @module
 */

import { fileURLToPath } from 'node:url';
import type { BackendConfig } from '@fuzdev/fuz_app/testing/cross_backend/backend_config.ts';
import {
	resolve_cargo_binary_path,
	type CargoBinaryOptions
} from '@fuzdev/fuz_app/testing/cross_backend/cargo_binary.ts';
import {
	make_default_rust_backend_config,
	rust_default_capabilities
} from '@fuzdev/fuz_app/testing/cross_backend/default_backend_configs.ts';

/** Must match `CORD_SESSION_COOKIE_NAME` in `crates/cord_server/src/router.rs`. */
export const CORD_SESSION_COOKIE_NAME = 'cord_session';

/**
 * The `testing_cordd` binary, built by the global setup's `before_spawn`
 * (`prepare_cargo_binary`) ahead of the spawn. `-p` is needed: the
 * workspace's `default-members` is `cord_server` alone.
 */
export const CORD_CARGO_BINARY: CargoBinaryOptions = {
	workspace_dir: fileURLToPath(new URL('../../..', import.meta.url)),
	package: 'testing_cord_server',
	bin: 'testing_cordd'
};

/**
 * `testing_cordd` over real HTTP.
 *
 * Every mount flag is stated against `crates/cord_server/src/router.rs`: one
 * action registry (protocol + `build_auth_spec_set` + cord's verbs) on both
 * `/api/rpc` and `/api/ws`, the admin audit SSE stream, and `/ready`. Cords
 * are cells, but cord mounts its own verbs over them, not the cell verbs, and
 * no fact store.
 *
 * @param database_url - the database the global setup provisioned
 */
export const rust_backend_config = (database_url: string): BackendConfig =>
	make_default_rust_backend_config({
		name: 'rust',
		port: 14261,
		cookie_name: CORD_SESSION_COOKIE_NAME,
		capabilities: {
			...rust_default_capabilities,
			ws: true,
			sse: true,
			cell_crud: false,
			cell_relations: false,
			cell_gated_create: false,
			fact_serving: false,
			fact_serving_admin_hash: false,
			ready: true,
			// `PROTOCOL_ACTION_SPECS` (`peer/ping`) sit in the one registry both
			// `/api/rpc` and `/api/ws` serve
			peer_request: true,
			// `build_auth_spec_set` is in that registry, so the WS endpoint mounts
			// the account actions
			ws_account_actions: true,
			// no role-gated WS endpoint beside `/api/ws`
			ws_role_gated: false,
			// no room verbs mounted yet
			rooms: false,
			// the spine backdoors `testing_cordd` mounts; cord's own
			// `_testing_cord_invite_expire` is the spine suites' `extra_methods`
			testing_backdoors: [
				'_testing_reset',
				'_testing_drain_effects',
				'_testing_db_invariants',
				'_testing_shorten_credential_expiry',
				'_testing_action_manifest'
			]
		},
		start_command: [resolve_cargo_binary_path(CORD_CARGO_BINARY)],
		database_url,
		// `RUST_LOG` targets are crate names
		rust_log: 'info,cord_server=info,testing_cord_server=info',
		// the keeper credential `_testing_reset` authenticates with
		daemon_token_dir_env: 'CORD_DAEMON_TOKEN_DIR'
	});
