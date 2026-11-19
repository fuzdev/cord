/**
 * Holds the frontend's request specs to what `testing_cordd` mounts: the
 * methods `all_cord_action_specs` lists, each one's input shape, auth axes,
 * role and credential gates, `side_effects`, and rate-limit class, equal the
 * server's registry, the spine's surface and cord's own alike. A spec added,
 * dropped, or changed on one side alone fails here; a method the frontend
 * doesn't list would resolve to `undefined` on the RPC proxy.
 *
 * The server's side is `_testing_action_manifest`, which `testing_cordd`
 * builds over the assembled spec list (its `final_action_specs_factory`
 * seam). The protocol actions are out of both manifests, and the
 * `_testing_*` actions are dropped from the server's: they're test seams,
 * not the frontend's surface.
 *
 * @module
 */

import { test } from 'vitest';
import { is_protocol_action_method } from '@fuzdev/fuz_app/actions/action_codegen.ts';
import type { RequestResponseActionSpec } from '@fuzdev/fuz_app/actions/action_spec.ts';
import { build_action_manifest } from '@fuzdev/fuz_app/testing/cross_backend/action_manifest.ts';
import { assert_action_manifests_equal } from '@fuzdev/fuz_app/testing/cross_backend/action_manifest_parity.ts';
import { inject_backend_handle } from '@fuzdev/fuz_app/testing/cross_backend/provided_context.ts';
import { capture_action_manifest } from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';

import { all_cord_action_specs } from '#lib/action_specs.ts';

const handle = inject_backend_handle();

test("the frontend's request specs match the server's registered specs", async () => {
	const declared = build_action_manifest(
		// requests the server answers: a request it initiates has no handler there
		all_cord_action_specs.filter(
			(spec): spec is RequestResponseActionSpec =>
				spec.kind === 'request_response' &&
				spec.initiator !== 'backend' &&
				!is_protocol_action_method(spec.method)
		)
	);
	const registered = await capture_action_manifest(handle);

	assert_action_manifests_equal(
		declared,
		{ methods: registered.methods.filter((entry) => !entry.method.startsWith('_testing_')) },
		{ a: 'the frontend specs', b: 'testing_cordd' }
	);
});
