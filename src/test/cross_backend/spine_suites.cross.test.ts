/**
 * fuz_app's shipped spine suites against `testing_cordd` — one
 * `describe_spine_cross_suites` call. Each suite gates its cases on the
 * capabilities `cord_backend_config.ts` states, so a mount cord gains or drops
 * changes what runs here, and a suite fuz_app adds reaches cord without a new
 * file. The suites cord can't run are excluded with their reasons; the
 * overrides are the Rust spine's tracked divergences, as on fuz_app's own
 * `testing_spine_stub`.
 *
 * @module
 */

import { ROLE_ADMIN, ROLE_KEEPER } from '@fuzdev/fuz_app/auth/role_schema.ts';
import { inject_backend_handle } from '@fuzdev/fuz_app/testing/cross_backend/provided_context.ts';
import {
	describe_spine_cross_suites,
	type SuiteOverrides
} from '@fuzdev/fuz_app/testing/cross_backend/spine_suites.ts';

import {
	cord_roles,
	cord_rpc_endpoints,
	cord_session_options,
	cord_surface_source
} from './cord_surface_spec.ts';

const handle = inject_backend_handle();

// `testing_cordd` folds in `_testing_reset` and cord's own test seams only
const NO_BACKDOOR = (method: string): string => `testing_cordd mounts no ${method}`;

const overrides: SuiteOverrides = {
	rpc_adversarial: {
		xfail_params: [
			{
				method: 'account_token_create',
				labels: ['missing: scope', 'null: scope', 'missing: lifetime', 'null: lifetime'],
				tracking_id: 'rust-token-create-schema-reason',
				reason: 'Rust adds invalid_token_scope / invalid_token_lifetime to schema refusals'
			}
		]
	},
	// on the derived multi-actor setup, so its seeding stays the one call's
	transport_composition: (derived) => ({
		multi_actor: derived.multi_actor && {
			...derived.multi_actor,
			xfail: {
				tracking_id: 'rust-ws-upgrade-acting-fail-open',
				reason:
					'the Rust upgrade resolves an account, not an actor, so it admits a multi-actor ' +
					'upgrade that names no actor; per-frame dispatch still resolves each frame'
			}
		}
	})
};

describe_spine_cross_suites(handle, {
	surface_source: cord_surface_source,
	session_options: cord_session_options,
	rpc_endpoints: cord_rpc_endpoints,
	roles: cord_roles,
	// cord's own roles have no grant path: keeper for the role-shaped cell
	// grants (a suite that skips here, with no cell verbs), admin for the conferral
	suite_roles: { cell_grant: ROLE_KEEPER, conferral: ROLE_ADMIN },
	exclude: {
		actor_lookup: 'testing_cordd mounts no actor_lookup resolver',
		credential_deadline: NO_BACKDOOR('_testing_shorten_credential_expiry'),
		origin_conformance: NO_BACKDOOR('_testing_origin_check'),
		ws_outbound_overflow: NO_BACKDOOR('_testing_flood_notifications'),
		testing_backdoor:
			'the suite fires every spine backdoor, and testing_cordd mounts only _testing_reset ' +
			"and cord's own seams"
	},
	overrides
});
