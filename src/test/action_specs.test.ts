import { describe_action_spec_invariants } from '@fuzdev/fuz_app/testing/action_spec_invariants.ts';

import { all_cord_action_specs } from '#lib/action_specs.ts';
import { ActionSpecs } from '#lib/action_collections.ts';
import { ActionMethod } from '#lib/action_metatypes.ts';

describe_action_spec_invariants({
	specs: all_cord_action_specs,
	method_enum: ActionMethod,
	collections: ActionSpecs
});
