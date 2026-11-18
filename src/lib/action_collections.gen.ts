import type { Gen } from '@fuzdev/gro/gen.ts';
import type { ActionSpecUnion } from '@fuzdev/fuz_app/actions/action_spec.ts';
import {
	ImportBuilder,
	compose_gen_file,
	create_namespace_qualifier,
	generate_action_event_datas,
	generate_action_inputs_outputs,
	generate_action_specs_record
} from '@fuzdev/fuz_app/actions/action_codegen.ts';
import { heartbeat_action_spec } from '@fuzdev/fuz_app/actions/heartbeat.ts';
import { cancel_action_spec } from '@fuzdev/fuz_app/actions/cancel.ts';
import { peer_ping_action_spec } from '@fuzdev/fuz_app/actions/peer_ping.ts';
import { all_account_action_specs } from '@fuzdev/fuz_app/auth/account_action_specs.ts';
import { all_admin_action_specs } from '@fuzdev/fuz_app/auth/admin_action_specs.ts';
import { all_role_grant_offer_action_specs } from '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts';

import { all_cord_domain_action_specs } from './cord_action_specs.ts';
import { role_grant_offer_notification_action_specs } from './action_specs.ts';

/**
 * The module fuz_app's role-grant notification specs live in, and the
 * namespace the generated file imports it as. Those specs are exported as
 * `<method>_notification_spec`, not the `<method>_action_spec` that
 * `create_namespace_qualifier` names every spec by, so they are qualified here.
 */
const NOTIFICATION_SPECS_MODULE = '@fuzdev/fuz_app/auth/role_grant_offer_notifications.ts';
const NOTIFICATION_SPECS_NS = 'role_grant_notifications';

/**
 * Generates `action_collections.ts`: the spec record, input/output types, and
 * event data types for every action in `all_cord_action_specs`.
 *
 * Each source is imported as a namespace in the generated file, so a spec's
 * identifier resolves from the module that defines it. The protocol specs are
 * one source per module because fuz_app's `actions/protocol.ts` exports them
 * only bundled. `include_protocol_actions: true` keeps them in the typed maps.
 * The role-grant notifications sit outside the qualifier, which can't name
 * them. Add a row when the server mounts a new spec group.
 *
 * @nodocs
 */
export const gen: Gen = ({ origin_path }) => {
	const imports = new ImportBuilder();
	const { qualify_spec: qualify_source_spec, all_specs: source_specs } = create_namespace_qualifier(
		[
			{
				ns: 'heartbeat_specs',
				module: '@fuzdev/fuz_app/actions/heartbeat.ts',
				specs: [heartbeat_action_spec]
			},
			{
				ns: 'cancel_specs',
				module: '@fuzdev/fuz_app/actions/cancel.ts',
				specs: [cancel_action_spec]
			},
			{
				ns: 'peer_ping_specs',
				module: '@fuzdev/fuz_app/actions/peer_ping.ts',
				specs: [peer_ping_action_spec]
			},
			{
				ns: 'account_specs',
				module: '@fuzdev/fuz_app/auth/account_action_specs.ts',
				specs: all_account_action_specs
			},
			{
				ns: 'admin_specs',
				module: '@fuzdev/fuz_app/auth/admin_action_specs.ts',
				specs: all_admin_action_specs
			},
			{
				ns: 'role_grant_offer_specs',
				module: '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts',
				specs: all_role_grant_offer_action_specs
			},
			{
				ns: 'cord_specs',
				module: './cord_action_specs.ts',
				specs: all_cord_domain_action_specs
			}
		],
		imports
	);
	imports.add(NOTIFICATION_SPECS_MODULE, `* as ${NOTIFICATION_SPECS_NS}`);
	const notification_methods = new Set(
		role_grant_offer_notification_action_specs.map((spec) => spec.method)
	);
	const qualify_spec = (spec: ActionSpecUnion): string =>
		notification_methods.has(spec.method)
			? `${NOTIFICATION_SPECS_NS}.${spec.method}_notification_spec`
			: qualify_source_spec(spec);
	const all_specs = [...source_specs, ...role_grant_offer_notification_action_specs];
	const options = { qualify_spec, include_protocol_actions: true };
	return compose_gen_file({
		origin_path,
		imports,
		blocks: [
			generate_action_specs_record(all_specs, imports, options),
			generate_action_inputs_outputs(all_specs, imports, options),
			generate_action_event_datas(all_specs, imports, options)
		]
	});
};
