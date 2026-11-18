import { assert, describe, test } from 'vitest';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

import {
	ERROR_CORD_SPACE_FORBIDDEN,
	ERROR_CORD_SPACE_NAME_INVALID,
	ERROR_CORD_SPACE_NOT_FOUND,
	type CordSpaceJson
} from '#lib/cord_action_specs.ts';
import { CORD_SPACE_NAME_RULE } from '#lib/ui/cord_errors.ts';
import { CordSpacesState } from '#lib/ui/cord_spaces_state.svelte.ts';

import { rpc_error, stub_api } from './stub_api.ts';

const PORCH = '11111111-1111-4111-8111-111111111111' as Uuid;
const YARD = '22222222-2222-4222-8222-222222222222' as Uuid;
const HEARTH = '33333333-3333-4333-8333-333333333333' as Uuid;
const LOFT = '44444444-4444-4444-8444-444444444444' as Uuid;

const space = (id: Uuid, name: string, cord_id: Uuid = PORCH): CordSpaceJson => ({
	id,
	cord_id,
	kind: 'line',
	name,
	created_at: '2026-01-01T00:00:00Z'
});

/** A space list per cord the stub API keeps, so writes show up on the next fetch. */
const create_spaces_api = (initial: Record<string, Array<CordSpaceJson>>) => {
	const by_cord = new Map(Object.entries(initial));
	const calls: Array<string> = [];
	const api = stub_api({
		cord_space_list: (input: { cord_id: string }) => {
			calls.push(`list:${input.cord_id}`);
			return Promise.resolve({ spaces: by_cord.get(input.cord_id) ?? [] });
		},
		cord_space_create: (input: { cord_id: Uuid; name: string }) => {
			calls.push(`create:${input.name}`);
			const created = space(LOFT, input.name, input.cord_id);
			by_cord.set(input.cord_id, [...(by_cord.get(input.cord_id) ?? []), created]);
			return Promise.resolve({ space: created });
		},
		cord_space_update: (input: { cord_id: string; space_id: Uuid; name: string }) => {
			calls.push(`update:${input.space_id}`);
			const spaces = (by_cord.get(input.cord_id) ?? []).map((s) =>
				s.id === input.space_id ? { ...s, name: input.name } : s
			);
			by_cord.set(input.cord_id, spaces);
			return Promise.resolve({ space: spaces.find((s) => s.id === input.space_id) });
		},
		cord_space_delete: (input: { cord_id: string; space_id: Uuid }) => {
			calls.push(`delete:${input.space_id}`);
			by_cord.set(
				input.cord_id,
				(by_cord.get(input.cord_id) ?? []).filter((s) => s.id !== input.space_id)
			);
			return Promise.resolve({ ok: true });
		}
	});
	return { api, calls };
};

/** A list API whose answers the test gives by hand, one per call, in call order. */
const create_held_lists = () => {
	const answers: Array<(spaces: Array<CordSpaceJson>) => void> = [];
	const cord_space_list = () =>
		new Promise((resolve) => {
			answers.push((spaces) => resolve({ spaces }));
		});
	return { answers, cord_space_list };
};

/** Lets a load the state started on its own settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('CordSpacesState', () => {
	test('fetch loads a cord’s spaces, which answer only for that cord', async () => {
		const { api } = create_spaces_api({ [PORCH]: [space(HEARTH, 'hearth')] });
		const state = new CordSpacesState({ api });
		assert.isNull(state.loaded_spaces(PORCH), 'nothing loaded yet');
		await state.fetch(PORCH);
		assert.isTrue(state.list.succeeded);
		assert.deepEqual(
			state.loaded_spaces(PORCH)?.map((s) => s.name),
			['hearth']
		);
		assert.strictEqual(state.get_space(PORCH, HEARTH)?.name, 'hearth');
		assert.isNull(state.loaded_spaces(YARD));
		assert.isUndefined(state.get_space(YARD, HEARTH));
	});

	test('a response for a cord the state moved off is dropped', async () => {
		const finish: Record<string, () => void> = {};
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: (input: { cord_id: Uuid }) =>
					new Promise((resolve) => {
						finish[input.cord_id] = () =>
							resolve({ spaces: [space(HEARTH, `${input.cord_id} hearth`, input.cord_id)] });
					})
			})
		});
		const porch = state.fetch(PORCH);
		const yard = state.fetch(YARD);
		finish[PORCH]!();
		await porch;
		assert.isNull(state.loaded_spaces(PORCH), 'the porch’s answer came after the move');
		assert.isNull(state.loaded_spaces(YARD));
		finish[YARD]!();
		await yard;
		assert.strictEqual(state.loaded_spaces(YARD)?.[0]?.name, `${YARD} hearth`);
	});

	test('moving to another cord forgets the last one’s list and write errors', async () => {
		const { api } = create_spaces_api({ [PORCH]: [space(HEARTH, 'hearth')] });
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: (input) => api.cord_space_list(input),
				cord_space_update: () => Promise.reject(rpc_error(ERROR_CORD_SPACE_FORBIDDEN))
			})
		});
		await state.fetch(PORCH);
		assert.isFalse(await state.rename_space(HEARTH, 'den'));
		assert.isNotNull(state.rename.error(HEARTH));
		const moving = state.fetch(YARD);
		assert.deepEqual(state.spaces, []);
		assert.isNull(state.loaded_cord_id);
		assert.isNull(state.rename.error(HEARTH));
		await moving;
		assert.deepEqual(state.loaded_spaces(YARD), []);
	});

	test('refresh skips a load of the same cord already running', async () => {
		const { api, calls } = create_spaces_api({ [PORCH]: [] });
		const state = new CordSpacesState({ api });
		await Promise.all([state.refresh(PORCH), state.refresh(PORCH)]);
		assert.deepEqual(calls, [`list:${PORCH}`]);
		await state.refresh(PORCH);
		assert.deepEqual(calls, [`list:${PORCH}`, `list:${PORCH}`], 'a settled load runs again');
	});

	test('create_space adds a line at the end of the list without a reload', async () => {
		const { api, calls } = create_spaces_api({ [PORCH]: [space(HEARTH, 'hearth')] });
		const state = new CordSpacesState({ api });
		await state.fetch(PORCH);
		const created = await state.create_space('line', 'loft');
		assert.strictEqual(created?.id, LOFT);
		assert.deepEqual(
			state.spaces.map((s) => s.name),
			['hearth', 'loft']
		);
		assert.deepEqual(calls, [`list:${PORCH}`, 'create:loft']);
	});

	test('a refused create maps its reason to words and changes nothing', async () => {
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: () => Promise.resolve({ spaces: [] }),
				cord_space_create: () => Promise.reject(rpc_error(ERROR_CORD_SPACE_NAME_INVALID))
			})
		});
		await state.fetch(PORCH);
		assert.isUndefined(await state.create_space('line', ' '));
		assert.strictEqual(state.create.error, CORD_SPACE_NAME_RULE);
		assert.deepEqual(state.spaces, []);
	});

	test('a create that lands after a move to another cord leaves that cord’s list alone', async () => {
		let finish_create!: () => void;
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: () => Promise.resolve({ spaces: [] }),
				cord_space_create: () =>
					new Promise((resolve) => {
						finish_create = () => resolve({ space: space(LOFT, 'loft') });
					})
			})
		});
		await state.fetch(PORCH);
		const creating = state.create_space('line', 'loft');
		await state.fetch(YARD);
		finish_create();
		assert.isUndefined(await creating);
		assert.deepEqual(state.loaded_spaces(YARD), []);
	});

	test('a create that lands while a load runs reloads, so the older load can’t drop the new space', async () => {
		const lists = create_held_lists();
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: lists.cord_space_list,
				cord_space_create: () => Promise.resolve({ space: space(LOFT, 'loft') })
			})
		});
		const first = state.fetch(PORCH);
		lists.answers[0]!([]);
		await first;
		const stale = state.fetch(PORCH); // reads the list before the create
		assert.ok(await state.create_space('line', 'loft'));
		assert.strictEqual(lists.answers.length, 3, 'the create started a reload');
		lists.answers[1]!([]); // the stale answer, without the loft
		await stale;
		assert.deepEqual(
			state.loaded_spaces(PORCH)?.map((s) => s.id),
			[LOFT]
		);
		lists.answers[2]!([space(LOFT, 'loft')]);
		await settle();
		assert.deepEqual(
			state.loaded_spaces(PORCH)?.map((s) => s.id),
			[LOFT]
		);
	});

	test('rename_space replaces the space in the list', async () => {
		const { api, calls } = create_spaces_api({ [PORCH]: [space(HEARTH, 'hearth')] });
		const state = new CordSpacesState({ api });
		await state.fetch(PORCH);
		assert.isTrue(await state.rename_space(HEARTH, 'den'));
		assert.strictEqual(state.get_space(PORCH, HEARTH)?.name, 'den');
		assert.deepEqual(calls, [`list:${PORCH}`, `update:${HEARTH}`], 'no load ran, so no reload');
	});

	test('a rename refused for a space that’s gone reloads the list', async () => {
		let lists = 0;
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: () =>
					Promise.resolve({ spaces: lists++ === 0 ? [space(HEARTH, 'hearth')] : [] }),
				cord_space_update: () => Promise.reject(rpc_error(ERROR_CORD_SPACE_NOT_FOUND))
			})
		});
		await state.fetch(PORCH);
		assert.isFalse(await state.rename_space(HEARTH, 'den'));
		assert.strictEqual(state.rename.error(HEARTH), "That space isn't here anymore.");
		// the reload the refusal started
		await settle();
		assert.deepEqual(state.loaded_spaces(PORCH), []);
	});

	test('a rename that lands while a load runs reloads, so the older load can’t undo it', async () => {
		const lists = create_held_lists();
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: lists.cord_space_list,
				cord_space_update: () => Promise.resolve({ space: space(HEARTH, 'den') })
			})
		});
		const first = state.fetch(PORCH);
		lists.answers[0]!([space(HEARTH, 'hearth')]);
		await first;
		const stale = state.fetch(PORCH); // reads the list before the rename
		assert.isTrue(await state.rename_space(HEARTH, 'den'));
		assert.strictEqual(lists.answers.length, 3, 'the rename started a reload');
		lists.answers[1]!([space(HEARTH, 'hearth')]); // the stale answer, with the old name
		await stale;
		assert.strictEqual(state.get_space(PORCH, HEARTH)?.name, 'den');
		lists.answers[2]!([space(HEARTH, 'den')]);
		await settle();
		assert.strictEqual(state.get_space(PORCH, HEARTH)?.name, 'den');
	});

	test('delete_space drops the space from the list', async () => {
		const { api, calls } = create_spaces_api({
			[PORCH]: [space(HEARTH, 'hearth'), space(LOFT, 'loft')]
		});
		const state = new CordSpacesState({ api });
		await state.fetch(PORCH);
		assert.isTrue(await state.delete_space(HEARTH));
		assert.deepEqual(
			state.spaces.map((s) => s.id),
			[LOFT]
		);
		assert.deepEqual(calls, [`list:${PORCH}`, `delete:${HEARTH}`], 'no load ran, so no reload');
	});

	test('a delete that lands while a load runs reloads, so the older load can’t bring it back', async () => {
		const lists = create_held_lists();
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: lists.cord_space_list,
				cord_space_delete: () => Promise.resolve({ ok: true })
			})
		});
		const first = state.fetch(PORCH);
		lists.answers[0]!([space(HEARTH, 'hearth')]);
		await first;
		const stale = state.fetch(PORCH); // reads the list before the delete
		assert.isTrue(await state.delete_space(HEARTH));
		assert.strictEqual(lists.answers.length, 3, 'the delete started a reload');
		lists.answers[1]!([space(HEARTH, 'hearth')]); // the stale answer, with the hearth
		await stale;
		assert.deepEqual(state.loaded_spaces(PORCH), []);
		lists.answers[2]!([]);
		await settle();
		assert.deepEqual(state.loaded_spaces(PORCH), []);
	});

	test('deleting a space the server no longer has counts as deleted', async () => {
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: () => Promise.resolve({ spaces: [space(HEARTH, 'hearth')] }),
				cord_space_delete: () => Promise.reject(rpc_error(ERROR_CORD_SPACE_NOT_FOUND))
			})
		});
		await state.fetch(PORCH);
		assert.isTrue(await state.delete_space(HEARTH));
		assert.deepEqual(state.spaces, []);
		assert.isNull(state.delete.error(HEARTH));
	});

	test('a delete refused for another reason keeps the space and its error', async () => {
		const state = new CordSpacesState({
			api: stub_api({
				cord_space_list: () => Promise.resolve({ spaces: [space(HEARTH, 'hearth')] }),
				cord_space_delete: () => Promise.reject(rpc_error(ERROR_CORD_SPACE_FORBIDDEN))
			})
		});
		await state.fetch(PORCH);
		assert.isFalse(await state.delete_space(HEARTH));
		assert.strictEqual(state.spaces.length, 1);
		assert.strictEqual(
			state.delete.error(HEARTH),
			"Only the cord's owners and admins can manage its spaces."
		);
	});

	test('reset forgets everything', async () => {
		const { api } = create_spaces_api({ [PORCH]: [space(HEARTH, 'hearth')] });
		const state = new CordSpacesState({ api });
		await state.fetch(PORCH);
		state.reset();
		assert.isNull(state.cord_id);
		assert.isNull(state.loaded_spaces(PORCH));
		assert.isTrue(state.list.initial);
	});
});
