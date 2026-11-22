/**
 * cord's declared surface for the cross-process suites — what `testing_cordd`
 * mounts, rebuilt from fuz_app's spine shapes plus cord's own action specs.
 *
 * cord is served by the Rust spine in the usual consumer shape (health, the
 * account router, bootstrap, signup, one RPC endpoint), so
 * `create_rust_spine_consumer_surface_spec` builds it; cord supplies its
 * session cookie name, its role registry, and its domain verbs.
 *
 * @module
 */

import { create_role_registry } from '@fuzdev/fuz_app/auth/role_schema.ts';
import {
	create_rust_spine_consumer_rpc_endpoints,
	create_rust_spine_consumer_surface_spec,
	type RustSpineConsumerRpcEndpointsOptions
} from '@fuzdev/fuz_app/testing/cross_backend/rust_spine_consumer_surface.ts';

import { all_cord_domain_action_specs } from '#lib/cord_action_specs.ts';
import { CordRole } from '#lib/cord_roles.ts';
import { CORD_SESSION_COOKIE_NAME } from './cord_backend_config.ts';

/**
 * cord's role registry, the TS twin of `cord_role_registry`: the cord-scoped
 * membership roles, with no grant path — only cord's own verbs confer them.
 */
export const cord_roles = create_role_registry(
	CordRole.def.map((name) => ({ name, grant_paths: [] }))
);

const surface_options: RustSpineConsumerRpcEndpointsOptions = {
	domain_action_specs: all_cord_domain_action_specs,
	roles: cord_roles
};

/** The declared surface, for `describe_spine_cross_suites`'s `surface_source`. */
export const cord_surface_source = create_rust_spine_consumer_surface_spec({
	...surface_options,
	session_cookie_name: CORD_SESSION_COOKIE_NAME
});

/** The RPC endpoint the surface declares, for `describe_spine_cross_suites`'s `rpc_endpoints`. */
export const cord_rpc_endpoints = create_rust_spine_consumer_rpc_endpoints(surface_options);
