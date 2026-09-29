import type { Gen } from '@fuzdev/gro/gen.ts';
import {
	type ActionMethodEnumKind,
	ImportBuilder,
	compose_gen_file,
	generate_action_method_enums,
	generate_frontend_actions_api
} from '@fuzdev/fuz_app/actions/action_codegen.ts';

import { all_cord_action_specs } from './action_specs.ts';

/**
 * Open unions enumerate every dispatchable method, protocol actions included.
 */
const OPEN_UNION_KINDS: ReadonlySet<ActionMethodEnumKind> = new Set([
	'all',
	'request_response',
	'remote_notification',
	'local_call',
	'frontend',
	'backend'
]);

/**
 * Narrow enums drive typed handler maps, so they leave out the protocol
 * actions, whose handlers the dispatcher owns.
 */
const HANDLER_NARROW_KINDS: ReadonlySet<ActionMethodEnumKind> = new Set([
	'frontend_handled',
	'backend_handled',
	'broadcast'
]);

/**
 * Generates `action_metatypes.ts`: the method enums and the typed
 * `FrontendActionsApi` over `all_cord_action_specs`, which types the RPC
 * client and satisfies fuz_app's admin RPC adapters.
 *
 * @nodocs
 */
export const gen: Gen = ({ origin_path }) => {
	const imports = new ImportBuilder();
	return compose_gen_file({
		origin_path,
		imports,
		blocks: [
			generate_action_method_enums(all_cord_action_specs, imports, {
				emit: OPEN_UNION_KINDS,
				include_protocol_actions: true
			}),
			generate_action_method_enums(all_cord_action_specs, imports, {
				emit: HANDLER_NARROW_KINDS
			}),
			generate_frontend_actions_api(all_cord_action_specs, imports, {
				include_protocol_actions: true
			})
		]
	});
};
