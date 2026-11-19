import { assert, describe, test } from 'vitest';
import { error_reason, expect_output } from '@fuzdev/fuz_app/testing/cross_backend/cross_rpc.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import {
	ERROR_INSUFFICIENT_PERMISSIONS,
	ERROR_ROLE_NOT_WEB_GRANTABLE
} from '@fuzdev/fuz_app/http/error_schemas.ts';
import {
	ERROR_ROLE_GRANT_BUILTIN_SCOPED,
	ERROR_ROLE_GRANT_OFFER_ROLE_NOT_GRANTABLE,
	RoleGrantOfferAcceptOutput,
	RoleGrantOfferCreateOutput
} from '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';

import {
	CordAdminListOutput,
	CordGetOutput,
	CordInviteAcceptOutput,
	CordListOutput,
	CordUpdateOutput,
	ERROR_CORD_NAME_INVALID,
	ERROR_CORD_NOT_FOUND
} from '#lib/cord_action_specs.ts';
import { CORD_MEMBER, CORD_OWNER } from '#lib/cord_roles.ts';

import {
	assert_refused,
	audit_events,
	call,
	create_cord,
	mint,
	setup_test,
	type Fixture
} from './cord_cross_helpers.ts';

/**
 * The cord actions end to end against a spawned `testing_cordd`: an instance
 * admin (the keeper) creates a cord and owns it, members who aren't in a cord
 * can neither see nor probe it, and the admin-only actions refuse members.
 * Each test starts from a fresh keeper and an empty cell table
 * (`_testing_reset`); `create_account` signs members up through a
 * production invite.
 */

/** The account's role grants, from `/api/account/status`. */
const read_role_grants = async (
	fixture: Fixture,
	headers: Record<string, string>
): Promise<
	Array<{ id: string; role: string; scope_kind: string | null; scope_id: string | null }>
> => {
	const res = await fixture.fresh_transport()('/api/account/status', { method: 'GET', headers });
	assert.strictEqual(res.status, 200);
	return ((await res.json()) as { role_grants: Array<any> }).role_grants;
};

describe('cord_create', () => {
	test('an instance admin creates a cord and becomes its owner', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, '  the porch  ');
		assert.strictEqual(cord.name, 'the porch', 'the name is trimmed');
		assert.strictEqual(cord.role, CORD_OWNER);

		// the owner grant is a spine role grant scoped exactly to the cord
		const grants = await read_role_grants(fixture, fixture.create_session_headers());
		const cord_grants = grants.filter((g) => g.scope_id === cord.id);
		assert.deepEqual(
			cord_grants.map((g) => [g.role, g.scope_kind]),
			[[CORD_OWNER, 'cord']]
		);

		// the audit event names the cord and the owner grant; the cell itself
		// records no creator
		const events = await audit_events(fixture, 'cord_create');
		assert.deepEqual(
			events.map((e) => e.metadata),
			[{ cord_id: cord.id, name: 'the porch', owner_role_grant_id: cord_grants[0]!.id }]
		);
	});

	test('a member who is not an instance admin cannot create one', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const res = await call(fixture, member, 'cord_create', { name: 'mine' });
		assert_refused(res, ERROR_INSUFFICIENT_PERMISSIONS, JSONRPC_ERROR_CODES.forbidden);

		const admin_list = await call(fixture, fixture, 'cord_admin_list', {});
		assert.deepEqual(expect_output(admin_list, CordAdminListOutput).cords, [], 'no cord was made');
	});

	test('an invalid name is refused', async () => {
		const fixture = await setup_test();
		for (const name of [
			'   ',
			'two\nlines',
			'\u200B',
			'evil\u202Etxt',
			'\uFEFFx',
			'\u200D',
			'a\u200D\u200Db',
			'x'.repeat(101)
		]) {
			const res = await call(fixture, fixture, 'cord_create', { name });
			assert.ok(!res.ok, JSON.stringify(name));
			assert_refused(res, ERROR_CORD_NAME_INVALID, JSONRPC_ERROR_CODES.invalid_params);
		}
	});
});

describe('cord names', () => {
	test('join characters between letters and emoji are kept', async () => {
		const fixture = await setup_test();
		for (const name of ['🏳\uFE0F\u200D🌈 friends', 'می\u200Cخواهم', 'ශ්\u200Dරී']) {
			const cord = await create_cord(fixture, name);
			assert.strictEqual(cord.name, name);
		}
	});
});

describe('reading cords', () => {
	test('the creator sees the cord in cord_list and can read it', async () => {
		const fixture = await setup_test();
		const first = await create_cord(fixture, 'the porch');
		const second = await create_cord(fixture, 'the stoop');

		const list = await call(fixture, fixture, 'cord_list', {});
		assert.deepEqual(expect_output(list, CordListOutput).cords, [first, second]);

		const get = await call(fixture, fixture, 'cord_get', { cord_id: first.id });
		assert.deepEqual(expect_output(get, CordGetOutput).cord, first);
	});

	test('another account neither sees the cord nor can tell it exists', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const member = await fixture.create_account({ username: 'member' });

		const list = await call(fixture, member, 'cord_list', {});
		assert.deepEqual(expect_output(list, CordListOutput).cords, []);

		// the cord and a random id answer the same
		for (const cord_id of [cord.id, crypto.randomUUID()]) {
			const get = await call(fixture, member, 'cord_get', { cord_id });
			assert_refused(get, ERROR_CORD_NOT_FOUND, JSONRPC_ERROR_CODES.not_found);

			const update = await call(fixture, member, 'cord_update', { cord_id, name: 'taken over' });
			assert_refused(update, ERROR_CORD_NOT_FOUND);
		}
	});

	test('the spine cannot confer a cord role', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const member = await fixture.create_account({ username: 'member' });

		const assign = await call(fixture, fixture, 'role_grant_assign', {
			to_account_id: member.account.id,
			role: CORD_MEMBER,
			scope_kind: 'cord',
			scope_id: cord.id
		});
		assert_refused(assign, ERROR_ROLE_NOT_WEB_GRANTABLE);

		const offer = await call(fixture, fixture, 'role_grant_offer_create', {
			to_account_id: member.account.id,
			role: CORD_MEMBER,
			scope_kind: 'cord',
			scope_id: cord.id
		});
		assert_refused(offer, ERROR_ROLE_GRANT_OFFER_ROLE_NOT_GRANTABLE);

		const get = await call(fixture, member, 'cord_get', { cord_id: cord.id });
		assert.strictEqual(error_reason(get), ERROR_CORD_NOT_FOUND);
	});
});

describe('cord_update', () => {
	test('the owner renames the cord', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const res = await call(fixture, fixture, 'cord_update', {
			cord_id: cord.id,
			name: 'the stoop'
		});
		assert.deepEqual(expect_output(res, CordUpdateOutput).cord, { ...cord, name: 'the stoop' });
	});

	test('an invalid name is refused before the gates, so it is no existence oracle', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const member = await fixture.create_account({ username: 'member' });
		const { token } = await mint(fixture, fixture, cord.id);
		expect_output(
			await call(fixture, member, 'cord_invite_accept', { token }),
			CordInviteAcceptOutput
		);

		// a member who may not rename it, the keeper on no cord, and the keeper
		// on the real cord all get the same name refusal
		const replies = [
			await call(fixture, member, 'cord_update', { cord_id: cord.id, name: '' }),
			await call(fixture, fixture, 'cord_update', { cord_id: crypto.randomUUID(), name: '' }),
			await call(fixture, fixture, 'cord_update', { cord_id: cord.id, name: '' })
		];
		for (const res of replies) {
			assert_refused(res, ERROR_CORD_NAME_INVALID, JSONRPC_ERROR_CODES.invalid_params);
		}
		const [first, ...rest] = replies;
		for (const res of rest) {
			assert.strictEqual(res.status, first!.status);
			assert.deepEqual(res.error, first!.error);
		}
	});
});

describe('cord_admin_list', () => {
	test('lists every cord with its member count, for instance admins only', async () => {
		const fixture = await setup_test();
		const first = await create_cord(fixture, 'the porch');
		const second = await create_cord(fixture, 'the stoop');

		const res = await call(fixture, fixture, 'cord_admin_list', {});
		const cords = expect_output(res, CordAdminListOutput).cords;
		assert.deepEqual(
			cords.map((c) => [c.id, c.name, c.member_count]),
			[
				[second.id, 'the stoop', 1],
				[first.id, 'the porch', 1]
			],
			'newest first'
		);

		const member = await fixture.create_account({ username: 'member' });
		const refused = await call(fixture, member, 'cord_admin_list', {});
		assert.ok(!refused.ok);
		assert.strictEqual(refused.error?.code, JSONRPC_ERROR_CODES.forbidden);
	});
});

describe('instance admins and cords', () => {
	test('a global admin who is not in a cord cannot see or change it, but the ops list shows it', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		// the spine's normal path: the keeper offers `admin`, the member accepts
		const admin = await fixture.create_account({ username: 'second_admin', roles: [ROLE_ADMIN] });
		const grants = await read_role_grants(fixture, admin.create_session_headers());
		assert.ok(
			grants.some((g) => g.role === ROLE_ADMIN && g.scope_id === null),
			'the member holds a global admin grant'
		);

		const list = await call(fixture, admin, 'cord_list', {});
		assert.deepEqual(expect_output(list, CordListOutput).cords, []);
		const get = await call(fixture, admin, 'cord_get', { cord_id: cord.id });
		assert.strictEqual(error_reason(get), ERROR_CORD_NOT_FOUND);
		const update = await call(fixture, admin, 'cord_update', {
			cord_id: cord.id,
			name: 'taken over'
		});
		assert.strictEqual(error_reason(update), ERROR_CORD_NOT_FOUND);

		const admin_list = await call(fixture, admin, 'cord_admin_list', {});
		assert.deepEqual(
			expect_output(admin_list, CordAdminListOutput).cords.map((c) => c.id),
			[cord.id]
		);
	});
});

describe('scoped builtin roles', () => {
	test('an offer of admin with a scope is refused', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const member = await fixture.create_account({ username: 'member' });
		const offer = await call(fixture, fixture, 'role_grant_offer_create', {
			to_account_id: member.account.id,
			role: ROLE_ADMIN,
			scope_kind: 'cord',
			scope_id: cord.id
		});
		// the spine refuses a scoped builtin before cord's own offer gate runs
		assert_refused(offer, ERROR_ROLE_GRANT_BUILTIN_SCOPED, JSONRPC_ERROR_CODES.invalid_params);
	});

	test('an instance admin still offers admin globally', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const offer = await call(fixture, fixture, 'role_grant_offer_create', {
			to_account_id: member.account.id,
			role: ROLE_ADMIN
		});
		const offer_id = expect_output(offer, RoleGrantOfferCreateOutput).offer.id;
		const accept = await call(fixture, member, 'role_grant_offer_accept', { offer_id });
		expect_output(accept, RoleGrantOfferAcceptOutput);
		const grants = await read_role_grants(fixture, member.create_session_headers());
		assert.deepEqual(
			grants.map((g) => [g.role, g.scope_id]),
			[[ROLE_ADMIN, null]]
		);
	});

	test('no other path mints a scoped admin', async () => {
		// `role_grant_assign` with a scope is the other candidate path: the
		// spine refuses a scoped builtin role there too, and writes nothing
		const fixture = await setup_test();
		const cord = await create_cord(fixture, 'the porch');
		const member = await fixture.create_account({ username: 'member' });
		const assign = await call(fixture, fixture, 'role_grant_assign', {
			to_account_id: member.account.id,
			role: ROLE_ADMIN,
			scope_kind: 'cord',
			scope_id: cord.id
		});
		assert.ok(!assign.ok, 'a scoped admin assign is refused');
		assert_refused(assign, ERROR_ROLE_GRANT_BUILTIN_SCOPED, JSONRPC_ERROR_CODES.invalid_params);
		const grants = await read_role_grants(fixture, member.create_session_headers());
		assert.deepEqual(grants, [], 'no grant was written');

		// so `cord_create` never meets a scoped admin; with no admin grant at
		// all, the dispatcher's role gate refuses first
		const create = await call(fixture, member, 'cord_create', { name: 'mine' });
		assert.strictEqual(error_reason(create), ERROR_INSUFFICIENT_PERMISSIONS);
	});
});
