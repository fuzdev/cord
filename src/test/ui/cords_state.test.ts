import { assert, describe, test } from 'vitest';

import type { CordJson } from '#lib/cord_action_specs.ts';
import { CordState, CordsState } from '#lib/ui/cords_state.svelte.ts';

import { stub_api } from './stub_api.ts';

const cord = (name: string, role: CordJson['role'] = 'cord_owner'): CordJson =>
	({
		id: '11111111-1111-4111-8111-111111111111',
		name,
		role,
		created_at: '2026-01-01T00:00:00Z'
	}) as CordJson;

describe('CordsState', () => {
	test('fetch loads the caller’s cords', async () => {
		const state = new CordsState({
			api: stub_api({ cord_list: () => Promise.resolve({ cords: [cord('the porch')] }) })
		});
		await state.fetch();
		assert.isTrue(state.list.succeeded);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the porch']
		);
	});

	test('create_cord creates, appends to the list without a reload, and returns the new cord', async () => {
		let lists = 0;
		const state = new CordsState({
			api: stub_api({
				cord_create: (input: { name: string }) => Promise.resolve({ cord: cord(input.name) }),
				cord_list: () => {
					lists++;
					return Promise.resolve({ cords: [cord('the yard')] });
				}
			})
		});
		await state.fetch();
		const result = await state.create_cord('the porch');
		assert.strictEqual(result?.name, 'the porch');
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the yard', 'the porch']
		);
		assert.strictEqual(lists, 1);
	});

	test('a failed create returns undefined and keeps the error', async () => {
		const state = new CordsState({
			api: stub_api({ cord_create: () => Promise.reject(new Error('forbidden')) })
		});
		assert.isUndefined(await state.create_cord('the porch'));
		assert.strictEqual(state.create.error, 'forbidden');
	});

	test('reset forgets the cords', async () => {
		const state = new CordsState({
			api: stub_api({ cord_list: () => Promise.resolve({ cords: [cord('the porch')] }) })
		});
		await state.fetch();
		state.reset();
		assert.deepEqual(state.cords, []);
		assert.isTrue(state.list.initial);
	});
});

describe('CordState', () => {
	test('fetch then rename updates the loaded cord', async () => {
		const state = new CordState({
			api: stub_api({
				cord_get: () => Promise.resolve({ cord: cord('the porch') }),
				cord_update: (input: { name: string }) => Promise.resolve({ cord: cord(input.name) })
			})
		});
		await state.fetch(cord('x').id);
		assert.strictEqual(state.cord?.name, 'the porch');
		assert.isTrue(await state.rename_cord('the stoop'));
		assert.strictEqual(state.cord?.name, 'the stoop');
	});

	test('a failed fetch leaves no cord', async () => {
		const state = new CordState({
			api: stub_api({ cord_get: () => Promise.reject(new Error('cord not found')) })
		});
		await state.fetch(cord('x').id);
		assert.isNull(state.cord);
		assert.isTrue(state.load.failed);
		assert.isFalse(await state.rename_cord('nope'), 'nothing to rename');
	});
});
