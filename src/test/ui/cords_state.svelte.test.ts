import { assert, describe, test } from 'vitest';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import { ERROR_CORD_NAME_INVALID, type CordJson } from '#lib/cord_action_specs.ts';
import { AdminCordsState, CordState, CordsState } from '#lib/ui/cords_state.svelte.ts';
import { CORD_SOCKET_LOST_MESSAGE, CORD_SOCKET_LOST_REASON } from '#lib/ui/cord_errors.ts';

import { rpc_error, stub_api } from './stub_api.ts';

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

	test('a create during an in-flight refresh refetches, so the stale answer keeps the new cord', async () => {
		const resolvers: Array<(v: { cords: Array<CordJson> }) => void> = [];
		const state = new CordsState({
			api: stub_api({
				cord_create: (input: { name: string }) => Promise.resolve({ cord: cord(input.name) }),
				cord_list: () => new Promise((r) => resolvers.push(r))
			})
		});
		const refreshing = state.fetch();
		await state.create_cord('the porch');
		assert.strictEqual(resolvers.length, 2, 'the create refetches while a refresh is in flight');
		resolvers[0]!({ cords: [] });
		resolvers[1]!({ cords: [cord('the porch')] });
		await refreshing;
		await new Promise((r) => setTimeout(r));
		assert.isTrue(state.list.succeeded);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the porch']
		);
	});

	test('a failed create returns undefined and keeps the error', async () => {
		const state = new CordsState({
			api: stub_api({ cord_create: () => Promise.reject(new Error('forbidden')) })
		});
		assert.isUndefined(await state.create_cord('the porch'));
		assert.strictEqual(state.create.error, 'forbidden');
	});

	test('a create refused for its name shows the name rules', async () => {
		const state = new CordsState({
			api: stub_api({ cord_create: () => Promise.reject(rpc_error(ERROR_CORD_NAME_INVALID)) })
		});
		assert.isUndefined(await state.create_cord('the porch'));
		assert.match(state.create.error!, /A cord name is 1 to/);
	});

	test('put_cord replaces the cord with its id in place and leaves out one the list lacks', async () => {
		const porch = cord('the porch');
		const yard = { ...cord('the yard'), id: '22222222-2222-4222-8222-222222222222' } as CordJson;
		const state = new CordsState({
			api: stub_api({ cord_list: () => Promise.resolve({ cords: [porch, yard] }) })
		});
		await state.fetch();
		state.put_cord({ ...porch, name: 'the stoop' });
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the stoop', 'the yard']
		);
		const shed = { ...cord('the shed'), id: '33333333-3333-4333-8333-333333333333' } as CordJson;
		state.put_cord(shed);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the stoop', 'the yard']
		);
	});

	test('remove_cord drops the cord with its id', async () => {
		const porch = cord('the porch');
		const yard = { ...cord('the yard'), id: '22222222-2222-4222-8222-222222222222' } as CordJson;
		const state = new CordsState({
			api: stub_api({ cord_list: () => Promise.resolve({ cords: [porch, yard] }) })
		});
		await state.fetch();
		state.remove_cord(porch.id);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the yard']
		);
		const cords = state.cords;
		state.remove_cord(porch.id);
		assert.strictEqual(state.cords, cords, 'an id the list lacks leaves it as it was');
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

	test('a failed refresh keeps the loaded list, and a retry keeps the error until it settles', async () => {
		let fail = false;
		let resolve_retry!: (v: { cords: Array<CordJson> }) => void;
		let retrying = false;
		const state = new CordsState({
			api: stub_api({
				cord_list: () => {
					if (retrying) return new Promise((r) => (resolve_retry = r));
					return fail
						? Promise.reject(new Error('unreachable'))
						: Promise.resolve({ cords: [cord('the porch')] });
				}
			})
		});
		await state.fetch();
		fail = true;
		await state.fetch();
		assert.isTrue(state.list.failed);
		assert.isTrue(state.loaded);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the porch']
		);
		retrying = true;
		const retry = state.fetch();
		assert.isTrue(state.list.loading);
		assert.strictEqual(state.list.error, 'unreachable');
		resolve_retry({ cords: [cord('the stoop')] });
		await retry;
		assert.isNull(state.list.error);
		assert.deepEqual(
			state.cords.map((c) => c.name),
			['the stoop']
		);
	});

	test('reset during an in-flight fetch keeps its cords out', async () => {
		let resolve_list!: (v: { cords: Array<CordJson> }) => void;
		const state = new CordsState({
			api: stub_api({ cord_list: () => new Promise((r) => (resolve_list = r)) })
		});
		const fetching = state.fetch();
		state.reset();
		resolve_list({ cords: [cord('the porch')] });
		await fetching;
		assert.deepEqual(state.cords, []);
		assert.isFalse(state.loaded);
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

	test('a rename refused for its name shows the name rules', async () => {
		const state = new CordState({
			api: stub_api({
				cord_get: () => Promise.resolve({ cord: cord('the porch') }),
				cord_update: () => Promise.reject(rpc_error(ERROR_CORD_NAME_INVALID))
			})
		});
		await state.fetch(cord('x').id);
		assert.isFalse(await state.rename_cord('the stoop'));
		assert.match(state.rename.error!, /A cord name is 1 to/);
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

	test('a superseded fetch keeps its cord out', async () => {
		const yard = { ...cord('the yard'), id: '22222222-2222-4222-8222-222222222222' } as CordJson;
		const resolvers: Array<(v: { cord: CordJson }) => void> = [];
		const state = new CordState({
			api: stub_api({ cord_get: () => new Promise((r) => resolvers.push(r)) })
		});
		const first = state.fetch(cord('x').id);
		const second = state.fetch(yard.id);
		assert.strictEqual(state.cord_id, yard.id);
		resolvers[1]!({ cord: yard });
		await second;
		resolvers[0]!({ cord: cord('the porch') });
		await first;
		assert.strictEqual(state.cord?.name, 'the yard');
	});

	test('moving to another cord drops a rename of the last one', async () => {
		const yard = { ...cord('the yard'), id: '22222222-2222-4222-8222-222222222222' } as CordJson;
		let resolve_update!: (v: { cord: CordJson }) => void;
		const state = new CordState({
			api: stub_api({
				cord_get: (input: { cord_id: string }) =>
					Promise.resolve({ cord: input.cord_id === yard.id ? yard : cord('the porch') }),
				cord_update: () => new Promise((r) => (resolve_update = r))
			})
		});
		await state.fetch(cord('x').id);
		const renaming = state.rename_cord('the stoop');
		assert.isTrue(state.rename.loading);
		await state.fetch(yard.id);
		assert.isTrue(state.rename.initial);
		resolve_update({ cord: cord('the stoop') });
		assert.isFalse(await renaming);
		assert.strictEqual(state.cord?.name, 'the yard', 'the porch’s rename stays off the yard');
	});
});

describe('the cord read slots', () => {
	test('show a dropped connection in cord’s words, not the error’s own text', async () => {
		const lost = () =>
			Promise.reject(rpc_error(CORD_SOCKET_LOST_REASON, JSONRPC_ERROR_CODES.service_unavailable));
		const api = stub_api({ cord_list: lost, cord_get: lost, cord_admin_list: lost });
		const cords = new CordsState({ api });
		await cords.fetch();
		const one = new CordState({ api });
		await one.fetch(cord('x').id);
		const admin = new AdminCordsState({ api });
		await admin.fetch();
		assert.deepEqual(
			[cords.list.error, one.load.error, admin.list.error],
			[CORD_SOCKET_LOST_MESSAGE, CORD_SOCKET_LOST_MESSAGE, CORD_SOCKET_LOST_MESSAGE]
		);
	});
});
