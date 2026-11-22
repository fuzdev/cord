import { assert, describe, test } from 'vitest';
import type { TestAccount } from '@fuzdev/fuz_app/testing/test_app.ts';
import { expect_output } from '@fuzdev/fuz_app/testing/cross_backend/cross_rpc.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';

import {
	assert_invalid_params,
	assert_refused,
	audit_events,
	call,
	create_cord,
	mint,
	setup_test,
	type Caller,
	type Fixture
} from './cord_cross_helpers.ts';
import {
	CORD_SPACE_COUNT_MAX,
	CORD_SPACE_NAME_LENGTH_MAX,
	CordInviteAcceptOutput,
	CordMemberRoleSetOutput,
	CordSpaceCreateOutput,
	CordSpaceDeleteOutput,
	CordSpaceListOutput,
	CordSpaceUpdateOutput,
	ERROR_CORD_NOT_FOUND,
	ERROR_CORD_SPACE_LIMIT_REACHED,
	ERROR_CORD_SPACE_FORBIDDEN,
	ERROR_CORD_SPACE_NAME_INVALID,
	ERROR_CORD_SPACE_NOT_FOUND,
	type CordSpaceJson
} from '#lib/cord_action_specs.ts';
import { CORD_ADMIN, CORD_MEMBER, type CordRole } from '#lib/cord_roles.ts';

/**
 * A cord's spaces end to end against a spawned `testing_cordd`: the cord's
 * owner and admins create, rename, and delete spaces, its members list them,
 * and everyone outside the cord gets `cord_not_found` from every space verb,
 * whatever space they name. Each test starts from a fresh keeper and no cords
 * (`_testing_reset`).
 */

/** Bring `account` into the cord through a fresh link, then give them `role`. */
const join_as = async (
	fixture: Fixture,
	cord_id: string,
	account: TestAccount,
	role: CordRole = CORD_MEMBER
): Promise<void> => {
	const { token } = await mint(fixture, fixture, cord_id);
	expect_output(
		await call(fixture, account, 'cord_invite_accept', { token }),
		CordInviteAcceptOutput
	);
	if (role !== CORD_MEMBER) {
		expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id,
				actor_id: account.actor.id,
				role
			}),
			CordMemberRoleSetOutput
		);
	}
};

/** Create a line in `cord_id` as `caller`. */
const create_space = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string,
	name: string
): Promise<CordSpaceJson> =>
	expect_output(
		await call(fixture, caller, 'cord_space_create', { cord_id, kind: 'line', name }),
		CordSpaceCreateOutput
	).space;

/** The cord's spaces, read by `caller`. */
const list_spaces = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string
): Promise<Array<CordSpaceJson>> =>
	expect_output(await call(fixture, caller, 'cord_space_list', { cord_id }), CordSpaceListOutput)
		.spaces;

describe('cord spaces', () => {
	test('the owner creates, lists, renames, and deletes a line', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);

		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), [], 'a new cord has none');
		const space = await create_space(fixture, fixture, cord.id, '  the hearth  ');
		assert.deepEqual(
			{ ...space, id: '', created_at: '' },
			{ id: '', cord_id: cord.id, kind: 'line', name: 'the hearth', created_at: '' }
		);
		const second = await create_space(fixture, fixture, cord.id, 'the stove');
		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), [space, second], 'oldest first');

		const renamed = expect_output(
			await call(fixture, fixture, 'cord_space_update', {
				cord_id: cord.id,
				space_id: space.id,
				name: 'the porch swing'
			}),
			CordSpaceUpdateOutput
		).space;
		assert.deepEqual(renamed, { ...space, name: 'the porch swing' });

		expect_output(
			await call(fixture, fixture, 'cord_space_delete', { cord_id: cord.id, space_id: second.id }),
			CordSpaceDeleteOutput
		);
		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), [renamed]);

		// a deleted space is gone from every verb
		assert_refused(
			await call(fixture, fixture, 'cord_space_update', {
				cord_id: cord.id,
				space_id: second.id,
				name: 'back'
			}),
			ERROR_CORD_SPACE_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);
		assert_refused(
			await call(fixture, fixture, 'cord_space_delete', { cord_id: cord.id, space_id: second.id }),
			ERROR_CORD_SPACE_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);

		// each change is audited in the caller's name
		const metadata = async (event_type: string) =>
			(await audit_events(fixture, event_type)).map((e) => [e.actor_id, e.metadata]);
		assert.deepEqual(await metadata('cord_space_create'), [
			[
				fixture.actor.id,
				{ cord_id: cord.id, space_id: space.id, kind: 'line', name: 'the hearth' }
			],
			[fixture.actor.id, { cord_id: cord.id, space_id: second.id, kind: 'line', name: 'the stove' }]
		]);
		assert.deepEqual(await metadata('cord_space_update'), [
			[fixture.actor.id, { cord_id: cord.id, space_id: space.id, name: 'the porch swing' }]
		]);
		assert.deepEqual(await metadata('cord_space_delete'), [
			[fixture.actor.id, { cord_id: cord.id, space_id: second.id, name: 'the stove' }]
		]);
	});

	test('admins manage spaces; members only see them', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const admin = await fixture.create_account({ username: 'admin' });
		const member = await fixture.create_account({ username: 'member' });
		await join_as(fixture, cord.id, admin, CORD_ADMIN);
		await join_as(fixture, cord.id, member);

		const space = await create_space(fixture, admin, cord.id, 'the hearth');
		assert.deepEqual(await list_spaces(fixture, member, cord.id), [space]);

		for (const [method, params] of [
			['cord_space_create', { cord_id: cord.id, kind: 'line', name: 'mine' }],
			['cord_space_update', { cord_id: cord.id, space_id: space.id, name: 'mine' }],
			['cord_space_delete', { cord_id: cord.id, space_id: space.id }]
		] as const) {
			assert_refused(
				await call(fixture, member, method, params),
				ERROR_CORD_SPACE_FORBIDDEN,
				JSONRPC_ERROR_CODES.forbidden
			);
		}

		expect_output(
			await call(fixture, admin, 'cord_space_update', {
				cord_id: cord.id,
				space_id: space.id,
				name: 'the stove'
			}),
			CordSpaceUpdateOutput
		);
		expect_output(
			await call(fixture, admin, 'cord_space_delete', { cord_id: cord.id, space_id: space.id }),
			CordSpaceDeleteOutput
		);
		assert.deepEqual(await list_spaces(fixture, member, cord.id), []);
	});

	test('outside the cord, every space verb answers cord_not_found, whatever the space', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const space = await create_space(fixture, fixture, cord.id, 'the hearth');
		const stranger = await fixture.create_account({ username: 'stranger' });
		const operator = await fixture.create_account({ username: 'operator', roles: [ROLE_ADMIN] });

		for (const outsider of [stranger, operator]) {
			// the cord and a random id answer the same
			for (const cord_id of [cord.id, crypto.randomUUID()]) {
				for (const [method, params] of [
					['cord_space_list', { cord_id }],
					['cord_space_create', { cord_id, kind: 'line', name: 'taken over' }],
					['cord_space_update', { cord_id, space_id: space.id, name: 'taken over' }],
					['cord_space_update', { cord_id, space_id: crypto.randomUUID(), name: 'taken over' }],
					['cord_space_delete', { cord_id, space_id: space.id }]
				] as const) {
					assert_refused(
						await call(fixture, outsider, method, params),
						ERROR_CORD_NOT_FOUND,
						JSONRPC_ERROR_CODES.not_found
					);
				}
			}
		}
		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), [space], 'nothing changed');
	});

	test('a space is reached only through its own cord', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const other = await create_cord(fixture, 'the stoop');
		const theirs = await create_space(fixture, fixture, other.id, 'next door');

		// the keeper owns both cords, so naming the other cord's space under
		// this one is the only thing wrong
		for (const space_id of [theirs.id, other.id, cord.id]) {
			assert_refused(
				await call(fixture, fixture, 'cord_space_update', {
					cord_id: cord.id,
					space_id,
					name: 'moved'
				}),
				ERROR_CORD_SPACE_NOT_FOUND
			);
			assert_refused(
				await call(fixture, fixture, 'cord_space_delete', { cord_id: cord.id, space_id }),
				ERROR_CORD_SPACE_NOT_FOUND
			);
		}
		assert.deepEqual(await list_spaces(fixture, fixture, other.id), [theirs]);
		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), []);
	});

	test('a bad name carries its reason; a bad kind carries none', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const space = await create_space(fixture, fixture, cord.id, 'the hearth');
		for (const name of [
			'   ',
			'two\nlines',
			'\u200B',
			'\uFEFFx',
			'a\u200D\u200Db',
			'x'.repeat(CORD_SPACE_NAME_LENGTH_MAX + 1)
		]) {
			assert_refused(
				await call(fixture, fixture, 'cord_space_create', { cord_id: cord.id, kind: 'line', name }),
				ERROR_CORD_SPACE_NAME_INVALID,
				JSONRPC_ERROR_CODES.invalid_params
			);
			assert_refused(
				await call(fixture, fixture, 'cord_space_update', {
					cord_id: cord.id,
					space_id: space.id,
					name
				}),
				ERROR_CORD_SPACE_NAME_INVALID,
				JSONRPC_ERROR_CODES.invalid_params
			);
		}
		for (const kind of ['chat', 'Line', null]) {
			assert_invalid_params(
				await call(fixture, fixture, 'cord_space_create', { cord_id: cord.id, kind, name: '' }),
				JSON.stringify(kind)
			);
		}
		assert.deepEqual(await list_spaces(fixture, fixture, cord.id), [space]);
	});

	test(`a cord holds at most ${CORD_SPACE_COUNT_MAX} spaces`, async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const spaces: Array<CordSpaceJson> = [];
		for (let i = 0; i < CORD_SPACE_COUNT_MAX; i++) {
			spaces.push(await create_space(fixture, fixture, cord.id, `room ${i}`));
		}
		const one_more = { cord_id: cord.id, kind: 'line', name: 'one more' };
		assert_refused(
			await call(fixture, fixture, 'cord_space_create', one_more),
			ERROR_CORD_SPACE_LIMIT_REACHED,
			JSONRPC_ERROR_CODES.conflict
		);
		expect_output(
			await call(fixture, fixture, 'cord_space_delete', {
				cord_id: cord.id,
				space_id: spaces[0]!.id
			}),
			CordSpaceDeleteOutput
		);
		expect_output(
			await call(fixture, fixture, 'cord_space_create', one_more),
			CordSpaceCreateOutput
		);
		assert.strictEqual((await list_spaces(fixture, fixture, cord.id)).length, CORD_SPACE_COUNT_MAX);
	});
});
