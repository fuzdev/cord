import type { Gen } from '@fuzdev/gro/gen.ts';
import {
	ImportBuilder,
	compose_gen_file,
	generate_frontend_action_handlers,
	generate_typed_action_event_alias
} from '@fuzdev/fuz_app/actions/action_codegen.ts';

import { all_cord_action_specs } from './action_specs.ts';

/**
 * Generates `frontend_action_types.ts`: `FrontendActionHandlers`, the typed
 * handler map `ui/rpc.ts` dispatches through, with each method's phases from
 * its spec's `initiator` (a server push gets `receive`, a call the frontend
 * makes gets its send and response phases). The protocol actions are left
 * out: the dispatcher handles them.
 *
 * @nodocs
 */
export const gen: Gen = ({ origin_path }) => {
	const imports = new ImportBuilder();
	return compose_gen_file({
		origin_path,
		imports,
		blocks: [
			generate_typed_action_event_alias(imports),
			generate_frontend_action_handlers(all_cord_action_specs, imports)
		]
	});
};
