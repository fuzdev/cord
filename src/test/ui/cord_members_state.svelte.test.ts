import { assert, describe, test } from 'vitest';
import { create_stub_api, rpc_error } from '@fuzdev/fuz_app/testing/stub_api.ts';
import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import type { Uuid } from '@fuzdev/fuz_app/primitive_schemas.ts';

import {
	ERROR_CORD_LAST_OWNER,
	ERROR_CORD_MEMBER_FORBIDDEN,
	type CordMemberJson
} from '#lib/cord_action_specs.ts';
import type { CordRole } from '#lib/cord_roles.ts';
import { CordMembersState } from '#lib/ui/cord_members_state.svelte.ts';
import type { FrontendActionsApi } from '#lib/action_metatypes.ts';

const CORD_ID = '11111111-1111-4111-8111-111111111111' as Uuid;
const OTHER_CORD_ID = '22222222-2222-4222-8222-222222222222' as Uuid;
const ALICE = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' as Uuid;
const BOB = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb' as Uuid;

const member = (actor_id: string, username: string, role: CordRole): CordMemberJson =>
	({ actor_id, username, display_name: null, role }) as CordMemberJson;

/** A member list the stub API keeps, so writes show up on the next fetch. */
const create_members_api = (initial: Array<CordMemberJson>) => {
	let members = initial;
	const calls: Array<string> = [];
	const api = create_stub_api<ThrowingApi<FrontendActionsApi>>({
		cord_member_list: () => {
			calls.push('list');
			return Promise.resolve({ members });
		},
		cord_member_role_set: (input: { actor_id: string; role: CordRole }) => {
			calls.push(`role_set:${input.actor_id}:${input.role}`);
			members = members.map((m) =>
				m.actor_id === input.actor_id ? { ...m, role: input.role } : m
			);
			return Promise.resolve({ member: members.find((m) => m.actor_id === input.actor_id)! });
		},
		cord_member_remove: (input: { actor_id: string }) => {
			calls.push(`remove:${input.actor_id}`);
			members = members.filter((m) => m.actor_id !== input.actor_id);
			return Promise.resolve({ ok: true });
		},
		cord_leave: () => {
			calls.push('leave');
			return Promise.resolve({ ok: true });
		}
	});
	return { api, calls };
};

describe('CordMembersState', () => {
	test('fetch loads the members and counts the owners', async () => {
		const { api } = create_members_api([
			member(ALICE, 'alice', 'cord_owner'),
			member(BOB, 'bob', 'cord_member')
		]);
		const state = new CordMembersState({ api });
		await state.fetch(CORD_ID);
		assert.isTrue(state.list.succeeded);
		assert.strictEqual(state.cord_id, CORD_ID);
		assert.deepEqual(
			state.members.map((m) => m.username),
			['alice', 'bob']
		);
		assert.strictEqual(state.owner_count, 1);
		assert.strictEqual(state.get_member(BOB)?.role, 'cord_member');
		assert.isUndefined(state.get_member(null));
	});

	test('set_role changes the role and reloads the list', async () => {
		const { api, calls } = create_members_api([
			member(ALICE, 'alice', 'cord_owner'),
			member(BOB, 'bob', 'cord_member')
		]);
		const state = new CordMembersState({ api });
		await state.fetch(CORD_ID);
		assert.isTrue(await state.set_role(BOB, 'cord_owner'));
		assert.strictEqual(state.get_member(BOB)?.role, 'cord_owner');
		assert.strictEqual(state.owner_count, 2);
		assert.deepEqual(calls, ['list', `role_set:${BOB}:cord_owner`, 'list']);
	});

	test('a role change stays pending through the reload, so it runs once', async () => {
		let finish_list!: () => void;
		let lists = 0;
		let role_sets = 0;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () =>
					++lists === 1
						? Promise.resolve({ members: [member(BOB, 'bob', 'cord_member')] })
						: new Promise((resolve) => {
								finish_list = () => resolve({ members: [member(BOB, 'bob', 'cord_admin')] });
							}),
				cord_member_role_set: () => {
					role_sets++;
					return Promise.resolve({ member: member(BOB, 'bob', 'cord_admin') });
				}
			})
		});
		await state.fetch(CORD_ID);
		const changing = state.set_role(BOB, 'cord_admin');
		assert.isTrue(state.role_changing(BOB));
		// the call is done, and the reload is waiting
		await new Promise((resolve) => setTimeout(resolve, 0));
		assert.isFalse(state.role_set.loading(BOB));
		assert.isTrue(state.list.loading);
		assert.isTrue(state.role_changing(BOB), 'pending until the list shows the change');
		assert.strictEqual(
			state.get_member(BOB)?.role,
			'cord_member',
			'the row still shows the old role'
		);
		assert.isFalse(await state.set_role(BOB, 'cord_admin'), 'a second apply does nothing');
		finish_list();
		assert.isTrue(await changing);
		assert.isFalse(state.role_changing(BOB));
		assert.strictEqual(state.get_member(BOB)?.role, 'cord_admin');
		assert.strictEqual(role_sets, 1);
	});

	test('a removal stays pending through the reload, so it runs once', async () => {
		let finish_list!: () => void;
		let lists = 0;
		let removes = 0;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () =>
					++lists === 1
						? Promise.resolve({ members: [member(BOB, 'bob', 'cord_member')] })
						: new Promise((resolve) => {
								finish_list = () => resolve({ members: [] });
							}),
				cord_member_remove: () => {
					removes++;
					return Promise.resolve({ ok: true });
				}
			})
		});
		await state.fetch(CORD_ID);
		const removing = state.remove_member(BOB);
		await new Promise((resolve) => setTimeout(resolve, 0));
		assert.isFalse(state.remove.loading(BOB));
		assert.isTrue(state.removing(BOB));
		assert.isFalse(await state.remove_member(BOB));
		finish_list();
		assert.isTrue(await removing);
		assert.isFalse(state.removing(BOB));
		assert.strictEqual(removes, 1);
	});

	test('a refused role change keeps its reason, in words, and still reloads', async () => {
		let lists = 0;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () => {
					lists++;
					return Promise.resolve({ members: [member(ALICE, 'alice', 'cord_owner')] });
				},
				cord_member_role_set: () => Promise.reject(rpc_error(ERROR_CORD_LAST_OWNER))
			})
		});
		await state.fetch(CORD_ID);
		assert.isFalse(await state.set_role(ALICE, 'cord_member'));
		assert.match(state.role_set.error(ALICE)!, /always keeps an owner/);
		assert.strictEqual(lists, 2);
	});

	test('remove_member removes and reloads; a refusal keeps its reason per member', async () => {
		const { api } = create_members_api([
			member(ALICE, 'alice', 'cord_owner'),
			member(BOB, 'bob', 'cord_member')
		]);
		const state = new CordMembersState({ api });
		await state.fetch(CORD_ID);
		assert.isTrue(await state.remove_member(BOB));
		assert.deepEqual(
			state.members.map((m) => m.username),
			['alice']
		);

		const refused = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () => Promise.resolve({ members: [] }),
				cord_member_remove: () => Promise.reject(rpc_error(ERROR_CORD_MEMBER_FORBIDDEN))
			})
		});
		await refused.fetch(CORD_ID);
		assert.isFalse(await refused.remove_member(BOB));
		assert.match(refused.remove.error(BOB)!, /can't make that change/);
		assert.isNull(refused.remove.error(ALICE));
	});

	test('leave_cord leaves', async () => {
		const { api, calls } = create_members_api([member(ALICE, 'alice', 'cord_member')]);
		const state = new CordMembersState({ api });
		await state.fetch(CORD_ID);
		assert.isTrue(await state.leave_cord());
		assert.deepEqual(calls, ['list', 'leave']);
	});

	test('a refused leave keeps its reason', async () => {
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () => Promise.resolve({ members: [] }),
				cord_leave: () => Promise.reject(rpc_error(ERROR_CORD_LAST_OWNER))
			})
		});
		await state.fetch(CORD_ID);
		assert.isFalse(await state.leave_cord());
		assert.match(state.leave.error!, /always keeps an owner/);
	});

	test('does nothing before a cord is loaded', async () => {
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({})
		});
		assert.isFalse(await state.set_role(BOB, 'cord_admin'));
		assert.isFalse(await state.remove_member(BOB));
		assert.isFalse(await state.leave_cord());
	});

	test('fetching another cord forgets the first cord’s members', async () => {
		let answer: Array<CordMemberJson> = [member(ALICE, 'alice', 'cord_owner')];
		let release!: () => void;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: async () => {
					if (answer.length === 0) await new Promise<void>((r) => (release = r));
					return { members: answer };
				}
			})
		});
		await state.fetch(CORD_ID);
		answer = [];
		const pending = state.fetch(OTHER_CORD_ID);
		assert.deepEqual(state.members, [], 'no stale members while the other cord loads');
		release();
		await pending;
		assert.strictEqual(state.cord_id, OTHER_CORD_ID);
	});

	test('drops a late response for a cord the state has moved off', async () => {
		let release_first!: () => void;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: async (input: { cord_id: string }) => {
					if (input.cord_id === CORD_ID) {
						await new Promise<void>((r) => (release_first = r));
						return { members: [member(ALICE, 'alice', 'cord_owner')] };
					}
					return { members: [member(BOB, 'bob', 'cord_owner')] };
				}
			})
		});
		const first = state.fetch(CORD_ID);
		await state.fetch(OTHER_CORD_ID);
		release_first();
		await first;
		assert.strictEqual(state.cord_id, OTHER_CORD_ID);
		assert.deepEqual(
			state.members.map((m) => m.actor_id),
			[BOB]
		);
		assert.strictEqual(state.loaded_cord_id, OTHER_CORD_ID);
	});

	test('loaded_cord_id names the cord once its list loads, and survives a failed refresh', async () => {
		let fail = false;
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () =>
					fail
						? Promise.reject(new TypeError('fetch failed'))
						: Promise.resolve({ members: [member(ALICE, 'alice', 'cord_owner')] })
			})
		});
		assert.isNull(state.loaded_cord_id);
		const loading = state.fetch(CORD_ID);
		assert.isNull(state.loaded_cord_id, 'not while the first load runs');
		await loading;
		assert.strictEqual(state.loaded_cord_id, CORD_ID);
		fail = true;
		await state.fetch(CORD_ID);
		assert.isTrue(state.list.failed);
		assert.strictEqual(state.loaded_cord_id, CORD_ID, 'the list it loaded stands');
		await state.fetch(OTHER_CORD_ID);
		assert.isNull(state.loaded_cord_id, 'another cord, failed, has no list');
	});

	test('a write finishing after a move to another cord doesn’t reload the first', async () => {
		let release_role_set!: () => void;
		const lists: Array<string> = [];
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: (input: { cord_id: string }) => {
					lists.push(input.cord_id);
					return Promise.resolve({ members: [] });
				},
				cord_member_role_set: async () => {
					await new Promise<void>((r) => (release_role_set = r));
					return { member: member(BOB, 'bob', 'cord_admin') };
				}
			})
		});
		await state.fetch(CORD_ID);
		const setting = state.set_role(BOB, 'cord_admin');
		await state.fetch(OTHER_CORD_ID);
		release_role_set();
		await setting;
		assert.deepEqual(lists, [CORD_ID, OTHER_CORD_ID]);
		assert.strictEqual(state.cord_id, OTHER_CORD_ID);
	});

	test('a write ending after a move off its cord and back leaves a newer one pending', async () => {
		const releases: Array<() => void> = [];
		const held = <T>(value: T): Promise<T> =>
			new Promise<T>((resolve) => releases.push(() => resolve(value)));
		const state = new CordMembersState({
			api: create_stub_api<ThrowingApi<FrontendActionsApi>>({
				cord_member_list: () => Promise.resolve({ members: [member(BOB, 'bob', 'cord_member')] }),
				cord_member_role_set: () => held({ member: member(BOB, 'bob', 'cord_admin') }),
				cord_member_remove: () => held({ ok: true })
			})
		});
		await state.fetch(CORD_ID);
		const first_role = state.set_role(BOB, 'cord_admin');
		const first_remove = state.remove_member(BOB);
		// A, then B, then back to A, while the first calls run
		await state.fetch(OTHER_CORD_ID);
		await state.fetch(CORD_ID);
		assert.isFalse(state.role_changing(BOB), 'the moves cleared the first calls');
		assert.isFalse(state.removing(BOB));
		const second_role = state.set_role(BOB, 'cord_admin');
		const second_remove = state.remove_member(BOB);
		assert.isTrue(state.role_changing(BOB));
		assert.isTrue(state.removing(BOB));
		const [release_first_role, release_first_remove, release_second_role, release_second_remove] =
			releases;
		release_first_role!();
		release_first_remove!();
		await first_role;
		await first_remove;
		assert.isTrue(state.role_changing(BOB), "the first call doesn't clear the second's");
		assert.isTrue(state.removing(BOB));
		release_second_role!();
		release_second_remove!();
		await second_role;
		await second_remove;
		assert.isFalse(state.role_changing(BOB));
		assert.isFalse(state.removing(BOB));
	});
});
