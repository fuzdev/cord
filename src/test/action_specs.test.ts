import { assert, describe, test } from 'vitest';

import { all_cord_action_specs } from '#lib/action_specs.ts';
import { ActionSpecs } from '#lib/action_collections.ts';
import { ActionMethod } from '#lib/action_metatypes.ts';

const methods = all_cord_action_specs.map((spec) => spec.method);

describe('all_cord_action_specs', () => {
	test('methods are unique', () => {
		assert.strictEqual(new Set(methods).size, methods.length);
	});

	test('the generated collections cover the same specs', () => {
		assert.deepEqual(Object.keys(ActionSpecs).sort(), [...methods].sort());
		for (const spec of all_cord_action_specs) {
			const generated: unknown = ActionSpecs[spec.method as keyof typeof ActionSpecs];
			assert.strictEqual(generated, spec, spec.method);
		}
	});

	test('the generated method enum covers the same methods', () => {
		assert.deepEqual([...ActionMethod.options].sort(), [...methods].sort());
	});
});
