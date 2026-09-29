import type { Gen } from '@fuzdev/gro/gen.ts';
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

/**
 * Generates `action_collections.ts`: the spec record, input/output types, and
 * event data types for every action in `all_cord_action_specs`.
 *
 * Each source is imported as a namespace in the generated file, so a spec's
 * identifier resolves from the module that defines it. The protocol specs are
 * one source per module because fuz_app's `actions/protocol.ts` exports them
 * only bundled. `include_protocol_actions: true` keeps them in the typed maps.
 * Add a row when the server mounts a new spec group.
 *
 * @nodocs
 */
export const gen: Gen = ({ origin_path }) => {
	const imports = new ImportBuilder();
	const { qualify_spec, all_specs } = create_namespace_qualifier(
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
