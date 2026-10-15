import { assert, describe, test } from 'vitest';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

import {
	ERROR_CORD_NOT_FOUND,
	type CordJson,
	type CordMemberJson
} from '#lib/cord_action_specs.ts';
import type { CordRole } from '#lib/cord_roles.ts';
import { CordPageState } from '#lib/ui/cord_page_state.svelte.ts';

import { rpc_error, stub_api } from './stub_api.ts';

const PORCH_ID = '11111111-1111-4111-8111-111111111111' as Uuid;
const YARD_ID = '22222222-2222-4222-8222-222222222222' as Uuid;
const ALICE = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' as Uuid;

const cord = (id: Uuid, name: string, role: CordRole): CordJson => ({
	id,
	name,
	role,
	created_at: '2026-01-01T00:00:00Z'
});

const member = (actor_id: Uuid, role: CordRole): CordMemberJson => ({
	actor_id,
	username: 'alice',
	display_name: null,
	role
});

/** A `CordPageState` whose route and viewer the test moves, over the given API methods. */
const create_page = (methods: Record<string, (input: any) => Promise<unknown>>) => {
	const route: { cord_id: Uuid | undefined; actor_id: Uuid | null } = $state({
		cord_id: PORCH_ID,
		actor_id: ALICE
	});
	const cord_page = new CordPageState({
		api: stub_api(methods),
		get_cord_id: () => route.cord_id,
		get_self_actor_id: () => route.actor_id
	});
	return { cord_page, route };
};

describe('CordPageState', () => {
	test('a malformed id reads as not found without a request', () => {
		const { cord_page, route } = create_page({});
		route.cord_id = undefined;
		assert.isTrue(cord_page.not_found);
		assert.isUndefined(cord_page.role);
		assert.isFalse(cord_page.can_manage);
	});

	test('a cord the server doesn’t find reads as not found only while the route names it', async () => {
		const { cord_page, route } = create_page({
			cord_get: () => Promise.reject(rpc_error(ERROR_CORD_NOT_FOUND))
		});
		assert.isFalse(cord_page.not_found, 'nothing loaded yet');
		await cord_page.cord_state.fetch(PORCH_ID);
		assert.isTrue(cord_page.not_found);
		route.cord_id = YARD_ID;
		assert.isFalse(
			cord_page.not_found,
			'a failed load of another cord says nothing about this one'
		);
	});

	test('a failure for another reason isn’t a not found', async () => {
		const { cord_page } = create_page({
			cord_get: () => Promise.reject(new Error('unreachable'))
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		assert.isTrue(cord_page.cord_state.load.failed);
		assert.isFalse(cord_page.not_found);
	});

	test('the role comes from the cord until this cord’s member list loads, then from the list', async () => {
		let members = [member(ALICE, 'cord_admin')];
		const { cord_page } = create_page({
			cord_get: (input: { cord_id: Uuid }) =>
				Promise.resolve({ cord: cord(input.cord_id, 'the porch', 'cord_member') }),
			cord_member_list: () => Promise.resolve({ members })
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		assert.strictEqual(cord_page.cord?.name, 'the porch');
		assert.isFalse(cord_page.members_loaded);
		assert.strictEqual(cord_page.role, 'cord_member');
		assert.isFalse(cord_page.can_manage);

		await cord_page.members_state.fetch(PORCH_ID);
		assert.isTrue(cord_page.members_loaded);
		assert.strictEqual(cord_page.role, 'cord_admin', 'the list shows a role change at once');
		assert.isTrue(cord_page.can_manage);

		members = [];
		await cord_page.members_state.fetch(PORCH_ID);
		assert.strictEqual(cord_page.role, 'cord_member', 'a list without the viewer falls back');
	});

	test('a member list left from another cord says nothing about this one', async () => {
		const { cord_page, route } = create_page({
			cord_get: (input: { cord_id: Uuid }) =>
				Promise.resolve({ cord: cord(input.cord_id, 'a cord', 'cord_member') }),
			cord_member_list: () => Promise.resolve({ members: [member(ALICE, 'cord_owner')] })
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		await cord_page.members_state.fetch(PORCH_ID);
		assert.strictEqual(cord_page.role, 'cord_owner');

		route.cord_id = YARD_ID;
		await cord_page.cord_state.fetch(YARD_ID);
		assert.strictEqual(cord_page.cord?.id, YARD_ID);
		assert.isFalse(cord_page.members_loaded);
		assert.strictEqual(
			cord_page.role,
			'cord_member',
			'the porch’s list doesn’t speak for the yard'
		);
	});

	test('moving the route to another cord drops the last one before any fetch', async () => {
		const { cord_page, route } = create_page({
			cord_get: (input: { cord_id: Uuid }) =>
				Promise.resolve({ cord: cord(input.cord_id, 'the porch', 'cord_owner') }),
			cord_member_list: () => Promise.resolve({ members: [member(ALICE, 'cord_owner')] })
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		await cord_page.members_state.fetch(PORCH_ID);
		assert.strictEqual(cord_page.role, 'cord_owner');
		assert.isTrue(cord_page.can_manage);

		route.cord_id = YARD_ID;
		assert.isNull(cord_page.cord, 'the porch doesn’t show under the yard');
		assert.isFalse(cord_page.members_loaded);
		assert.isUndefined(cord_page.role);
		assert.isFalse(cord_page.can_manage);
		assert.isFalse(cord_page.not_found);
		assert.isFalse(cord_page.load_failed);
	});

	test('a failed load of another cord isn’t this one’s', async () => {
		const { cord_page, route } = create_page({
			cord_get: () => Promise.reject(new Error('unreachable'))
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		assert.isTrue(cord_page.load_failed);
		route.cord_id = YARD_ID;
		assert.isFalse(cord_page.load_failed);
	});

	test('fetch_members sends one request while one for the cord is running', async () => {
		let calls = 0;
		const { cord_page } = create_page({
			cord_member_list: () => {
				calls++;
				return Promise.resolve({ members: [member(ALICE, 'cord_owner')] });
			}
		});
		const first = cord_page.fetch_members(PORCH_ID);
		const second = cord_page.fetch_members(PORCH_ID);
		await Promise.all([first, second]);
		assert.strictEqual(calls, 1);
		assert.strictEqual(cord_page.members_state.loaded_cord_id, PORCH_ID);

		await cord_page.fetch_members(PORCH_ID);
		assert.strictEqual(calls, 2, 'a settled list refreshes');
		await cord_page.fetch_members(YARD_ID);
		assert.strictEqual(calls, 3, 'another cord’s list loads');
	});

	test('the viewer’s role follows who the viewer is', async () => {
		const { cord_page, route } = create_page({
			cord_get: () => Promise.resolve({ cord: cord(PORCH_ID, 'the porch', 'cord_member') }),
			cord_member_list: () => Promise.resolve({ members: [member(ALICE, 'cord_owner')] })
		});
		await cord_page.cord_state.fetch(PORCH_ID);
		await cord_page.members_state.fetch(PORCH_ID);
		assert.strictEqual(cord_page.self_actor_id, ALICE);
		assert.strictEqual(cord_page.role, 'cord_owner');
		route.actor_id = null;
		assert.isNull(cord_page.self_actor_id);
		assert.strictEqual(cord_page.role, 'cord_member');
	});
});
