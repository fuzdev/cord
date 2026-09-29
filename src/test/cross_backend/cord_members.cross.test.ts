import { assert, describe, inject, test } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle,
	type TestAccountFixture
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import {
	cross_rpc_call,
	error_reason,
	expect_output
} from '@fuzdev/fuz_app/testing/cross_backend/cell_cross_helpers.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';
import { ERROR_CREDENTIAL_TYPE_REQUIRED } from '@fuzdev/fuz_app/http/error_schemas.ts';
import { AuditLogListOutput } from '@fuzdev/fuz_app/auth/admin_action_specs.ts';

import './cross_test_types.ts';
import {
	CordCreateOutput,
	CordGetOutput,
	CordInviteAcceptOutput,
	CordInviteCreateOutput,
	CordInviteListOutput,
	CordInvitePreviewOutput,
	CordInviteRevokeOutput,
	CordLeaveOutput,
	CordListOutput,
	CordMemberListOutput,
	CordMemberRemoveOutput,
	CordMemberRoleSetOutput,
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_FORBIDDEN,
	ERROR_CORD_INVITE_INVALID,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_LAST_OWNER,
	ERROR_CORD_MEMBER_FORBIDDEN,
	ERROR_CORD_MEMBER_NOT_FOUND,
	ERROR_CORD_MEMBER_REMOVE_SELF,
	ERROR_CORD_NOT_FOUND
} from '#lib/cord_action_specs.ts';
import { CORD_ADMIN, CORD_MEMBER, CORD_OWNER, type CordRole } from '#lib/cord_roles.ts';

/**
 * Cord invite links and members end to end against a spawned
 * `testing_cordd`: the keeper (an instance admin) creates a cord and owns
 * it, other accounts join through links, and the member verbs hold their
 * permission tiers and the last-owner rule. Everyone outside a cord gets
 * `cord_not_found` from every verb that names it. Each test starts from a
 * fresh keeper and no cords (`_testing_reset`).
 */

const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));
const setup_test = default_cross_process_setup(handle);
const { rpc_path } = handle.config;

type Fixture = Awaited<ReturnType<typeof setup_test>>;
type Caller = Fixture | TestAccountFixture;

/** Call `method` as `caller` (the keeper's fixture or another account) on a clean cookie jar. */
const call = (
	fixture: Fixture,
	caller: Caller,
	method: string,
	params: unknown
): ReturnType<typeof cross_rpc_call> =>
	cross_rpc_call(
		fixture.fresh_transport(),
		rpc_path,
		method,
		params,
		caller.create_session_headers()
	);

/** Assert a call failed with `reason` (and `code`, when given). */
const assert_refused = (
	res: Awaited<ReturnType<typeof cross_rpc_call>>,
	reason: string,
	code?: number
): void => {
	assert.ok(!res.ok, `expected ${reason}, got ${JSON.stringify(res.result)}`);
	assert.strictEqual(error_reason(res), reason);
	if (code !== undefined) assert.strictEqual(res.error?.code, code);
};

/** Create a cord as the keeper, who owns it. */
const create_cord = async (fixture: Fixture, name = 'the porch') =>
	expect_output(await call(fixture, fixture, 'cord_create', { name }), CordCreateOutput).cord;

/** Mint a link on `cord_id` as `caller`. */
const mint = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string,
	limits: { expires_in_seconds?: number; max_uses?: number } = {}
) =>
	expect_output(
		await call(fixture, caller, 'cord_invite_create', { cord_id, ...limits }),
		CordInviteCreateOutput
	);

/** Bring `account` into the cord through a fresh link, then give them `role`. */
const join_as = async (
	fixture: Fixture,
	cord_id: string,
	account: TestAccountFixture,
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

/** The cord's members as `[actor_id, role]`, read by `caller`. */
const roster = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string
): Promise<Array<[string, CordRole]>> =>
	expect_output(
		await call(fixture, caller, 'cord_member_list', { cord_id }),
		CordMemberListOutput
	).members.map((m) => [m.actor_id, m.role]);

/** Audit events of `event_type`, oldest first, read by the keeper. */
const audit_events = async (fixture: Fixture, event_type: string) =>
	expect_output(
		await call(fixture, fixture, 'audit_log_list', { event_type }),
		AuditLogListOutput
	).events.reverse();

describe('invite links', () => {
	test('an owner mints a link and an existing account joins through it', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const guest = await fixture.create_account({ username: 'guest' });

		const minted = await mint(fixture, fixture, cord.id, {
			expires_in_seconds: 3600,
			max_uses: 2
		});
		assert.strictEqual(minted.token.length, 43);
		assert.deepEqual(
			{ ...minted.invite, id: '', created_at: '', expires_at: '' },
			{
				id: '',
				cord_id: cord.id,
				role: CORD_MEMBER,
				admits_newcomers: false,
				created_by: fixture.actor.id,
				created_at: '',
				expires_at: '',
				max_uses: 2,
				use_count: 0,
				revoked_at: null,
				status: 'active',
				redemption_count: 0,
				redemption_accepted_count: 0
			}
		);

		// the join page's read
		const preview = expect_output(
			await call(fixture, guest, 'cord_invite_preview', { token: minted.token }),
			CordInvitePreviewOutput
		);
		assert.deepEqual(preview, {
			status: 'active',
			admits_newcomers: false,
			newcomers_open: false,
			cord: { id: cord.id, name: 'the porch' },
			role: null
		});

		const accepted = expect_output(
			await call(fixture, guest, 'cord_invite_accept', { token: minted.token }),
			CordInviteAcceptOutput
		);
		assert.deepEqual(accepted, { cord: { ...cord, role: CORD_MEMBER }, joined: true });
		const got = expect_output(
			await call(fixture, guest, 'cord_get', { cord_id: cord.id }),
			CordGetOutput
		);
		assert.strictEqual(got.cord.role, CORD_MEMBER);

		// the member list, as the new member sees it
		const members = expect_output(
			await call(fixture, guest, 'cord_member_list', { cord_id: cord.id }),
			CordMemberListOutput
		).members;
		assert.deepEqual(
			members.map((m) => [m.actor_id, m.username, m.role]),
			[
				[fixture.actor.id, fixture.account.username, CORD_OWNER],
				[guest.actor.id, 'guest', CORD_MEMBER]
			]
		);

		// accepting again changes nothing and spends no use
		const again = expect_output(
			await call(fixture, guest, 'cord_invite_accept', { token: minted.token }),
			CordInviteAcceptOutput
		);
		assert.deepEqual(again, { cord: { ...cord, role: CORD_MEMBER }, joined: false });
		const invites = expect_output(
			await call(fixture, fixture, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		).invites;
		assert.deepEqual(
			invites.map((i) => [i.id, i.use_count, i.status]),
			[[minted.invite.id, 1, 'active']]
		);
		assert.ok(!JSON.stringify(invites).includes(minted.token), 'the list never carries the token');

		// audited as the spine's grant event plus cord's own; no event
		// carries the token
		const grants = (await audit_events(fixture, 'role_grant_create')).filter(
			(e) => e.target_actor_id === guest.actor.id
		);
		assert.strictEqual(grants.length, 1);
		assert.deepInclude(grants[0]!.metadata, {
			role: CORD_MEMBER,
			scope_kind: 'cord',
			scope_id: cord.id,
			source_invite_id: minted.invite.id
		});
		const accepts = await audit_events(fixture, 'cord_invite_accept');
		assert.deepEqual(
			accepts.map((e) => e.metadata),
			[
				{
					cord_id: cord.id,
					invite_id: minted.invite.id,
					role_grant_id: (grants[0]!.metadata as { role_grant_id: string }).role_grant_id,
					redemption_id: null,
					claimed_signup_invite_id: null
				}
			]
		);
		const creates = await audit_events(fixture, 'cord_invite_create');
		assert.strictEqual(creates.length, 1);
		assert.deepInclude(creates[0]!.metadata, { cord_id: cord.id, invite_id: minted.invite.id });
		assert.ok(!JSON.stringify(creates).includes(minted.token));
	});

	test('accepting takes a cookie session; a bearer or daemon token is refused', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const guest = await fixture.create_account({ username: 'guest' });
		const { token } = await mint(fixture, fixture, cord.id);

		const bearer = await cross_rpc_call(
			fixture.fresh_transport({ origin: null }),
			rpc_path,
			'cord_invite_accept',
			{ token },
			guest.create_bearer_headers()
		);
		assert_refused(bearer, ERROR_CREDENTIAL_TYPE_REQUIRED);
		const list = await call(fixture, guest, 'cord_list', {});
		assert.deepEqual(expect_output(list, CordListOutput).cords, [], 'nothing was granted');

		// the daemon token acts as the keeper; the gate refuses it before the
		// handler runs, so no use is spent either
		const daemon = await cross_rpc_call(
			fixture.fresh_transport({ origin: null }),
			rpc_path,
			'cord_invite_accept',
			{ token },
			fixture.create_daemon_token_headers()
		);
		assert_refused(daemon, ERROR_CREDENTIAL_TYPE_REQUIRED);
		const invites = expect_output(
			await call(fixture, fixture, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		).invites;
		assert.deepEqual(
			invites.map((i) => i.use_count),
			[0]
		);

		// preview reads with any credential
		const preview = await cross_rpc_call(
			fixture.fresh_transport({ origin: null }),
			rpc_path,
			'cord_invite_preview',
			{ token },
			guest.create_bearer_headers()
		);
		assert.strictEqual(expect_output(preview, CordInvitePreviewOutput).status, 'active');
	});

	test('revoked, expired, and exhausted links are refused', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const first = await fixture.create_account({ username: 'first' });
		const late = await fixture.create_account({ username: 'late' });

		// exhausted
		const single = await mint(fixture, fixture, cord.id, { max_uses: 1 });
		expect_output(
			await call(fixture, first, 'cord_invite_accept', { token: single.token }),
			CordInviteAcceptOutput
		);
		assert_refused(
			await call(fixture, late, 'cord_invite_accept', { token: single.token }),
			ERROR_CORD_INVITE_EXHAUSTED,
			JSONRPC_ERROR_CODES.conflict
		);

		// revoked, and revoking again is a no-op
		const revokable = await mint(fixture, fixture, cord.id);
		const revoked = expect_output(
			await call(fixture, fixture, 'cord_invite_revoke', {
				cord_id: cord.id,
				invite_id: revokable.invite.id
			}),
			CordInviteRevokeOutput
		).invite;
		assert.strictEqual(revoked.status, 'revoked');
		assert.ok(revoked.revoked_at);
		const revoked_again = expect_output(
			await call(fixture, fixture, 'cord_invite_revoke', {
				cord_id: cord.id,
				invite_id: revokable.invite.id
			}),
			CordInviteRevokeOutput
		).invite;
		assert.deepEqual(revoked_again, revoked);
		assert.strictEqual((await audit_events(fixture, 'cord_invite_revoke')).length, 1);
		assert_refused(
			await call(fixture, late, 'cord_invite_accept', { token: revokable.token }),
			ERROR_CORD_INVITE_REVOKED,
			JSONRPC_ERROR_CODES.conflict
		);

		// expired, through the test binary's backdating seam
		const expiring = await mint(fixture, fixture, cord.id, { expires_in_seconds: 60 });
		const expire = await cross_rpc_call(
			fixture.fresh_transport({ origin: null }),
			rpc_path,
			'_testing_cord_invite_expire',
			{ invite_id: expiring.invite.id },
			fixture.create_daemon_token_headers()
		);
		assert.ok(expire.ok, JSON.stringify(expire.error));
		assert_refused(
			await call(fixture, late, 'cord_invite_accept', { token: expiring.token }),
			ERROR_CORD_INVITE_EXPIRED,
			JSONRPC_ERROR_CODES.conflict
		);

		// a dead link shows its status but not its cord to someone outside it
		for (const [token, status] of [
			[single.token, 'exhausted'],
			[revokable.token, 'revoked'],
			[expiring.token, 'expired']
		] as const) {
			const preview = expect_output(
				await call(fixture, late, 'cord_invite_preview', { token }),
				CordInvitePreviewOutput
			);
			assert.deepEqual(preview, {
				status,
				admits_newcomers: false,
				newcomers_open: false,
				cord: null,
				role: null
			});
		}
		// a member sees their cord and role, whatever the link's state
		const member_preview = expect_output(
			await call(fixture, first, 'cord_invite_preview', { token: single.token }),
			CordInvitePreviewOutput
		);
		assert.deepEqual(member_preview, {
			status: 'exhausted',
			admits_newcomers: false,
			newcomers_open: false,
			cord: { id: cord.id, name: 'the porch' },
			role: CORD_MEMBER
		});

		assert.deepEqual(
			await roster(fixture, fixture, cord.id),
			[
				[fixture.actor.id, CORD_OWNER],
				[first.actor.id, CORD_MEMBER]
			],
			'no dead link let anyone in'
		);
		const statuses = expect_output(
			await call(fixture, fixture, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		).invites.map((i) => i.status);
		assert.deepEqual(statuses, ['expired', 'revoked', 'exhausted']);
	});

	test('an unknown token gets one answer', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const guest = await fixture.create_account({ username: 'guest' });
		const { token } = await mint(fixture, fixture, cord.id);
		for (const unknown of ['', 'nope', token.slice(0, -1), 'x'.repeat(500)]) {
			for (const method of ['cord_invite_preview', 'cord_invite_accept']) {
				assert_refused(
					await call(fixture, guest, method, { token: unknown }),
					ERROR_CORD_INVITE_NOT_FOUND,
					JSONRPC_ERROR_CODES.not_found
				);
			}
		}
	});

	test('only the owner and admins manage links', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const admin = await fixture.create_account({ username: 'cord_admin' });
		const member = await fixture.create_account({ username: 'member' });
		await join_as(fixture, cord.id, admin, CORD_ADMIN);
		await join_as(fixture, cord.id, member);

		const by_admin = await mint(fixture, admin, cord.id);
		assert.strictEqual(by_admin.invite.created_by, admin.actor.id);
		expect_output(
			await call(fixture, admin, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		);

		for (const [method, params] of [
			['cord_invite_create', { cord_id: cord.id }],
			['cord_invite_list', { cord_id: cord.id }],
			['cord_invite_revoke', { cord_id: cord.id, invite_id: by_admin.invite.id }]
		] as const) {
			assert_refused(
				await call(fixture, member, method, params),
				ERROR_CORD_INVITE_FORBIDDEN,
				JSONRPC_ERROR_CODES.forbidden
			);
		}

		// an id from another cord, or none at all, isn't this cord's link
		const other = await create_cord(fixture, 'the stoop');
		const elsewhere = await mint(fixture, fixture, other.id);
		for (const invite_id of [elsewhere.invite.id, crypto.randomUUID()]) {
			assert_refused(
				await call(fixture, admin, 'cord_invite_revoke', { cord_id: cord.id, invite_id }),
				ERROR_CORD_INVITE_NOT_FOUND
			);
		}

		// an admin who loses the role loses link management with it: demoted,
		// then removed, their next create and revoke are refused
		const minted_before = await mint(fixture, fixture, cord.id);
		const links = async () =>
			expect_output(
				await call(fixture, fixture, 'cord_invite_list', { cord_id: cord.id }),
				CordInviteListOutput
			).invites.map((i) => [i.id, i.status]);
		const links_before = await links();
		const manage = (caller: Caller) =>
			Promise.all([
				call(fixture, caller, 'cord_invite_create', { cord_id: cord.id }),
				call(fixture, caller, 'cord_invite_revoke', {
					cord_id: cord.id,
					invite_id: minted_before.invite.id
				})
			]);
		expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: admin.actor.id,
				role: CORD_MEMBER
			}),
			CordMemberRoleSetOutput
		);
		for (const res of await manage(admin)) {
			assert_refused(res, ERROR_CORD_INVITE_FORBIDDEN, JSONRPC_ERROR_CODES.forbidden);
		}
		expect_output(
			await call(fixture, fixture, 'cord_member_remove', {
				cord_id: cord.id,
				actor_id: admin.actor.id
			}),
			CordMemberRemoveOutput
		);
		for (const res of await manage(admin)) {
			assert_refused(res, ERROR_CORD_NOT_FOUND, JSONRPC_ERROR_CODES.not_found);
		}
		assert.deepEqual(await links(), links_before, 'nothing minted or revoked');
	});

	test('limits are bounded', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		for (const limits of [
			{ expires_in_seconds: 59 },
			{ expires_in_seconds: 365 * 24 * 60 * 60 + 1 },
			{ max_uses: 0 },
			{ max_uses: 1001 }
		]) {
			assert_refused(
				await call(fixture, fixture, 'cord_invite_create', { cord_id: cord.id, ...limits }),
				ERROR_CORD_INVITE_INVALID,
				JSONRPC_ERROR_CODES.invalid_params
			);
		}
		for (const limits of [{ expires_in_seconds: null }, { max_uses: -1 }, { max_uses: 1.5 }]) {
			const res = await call(fixture, fixture, 'cord_invite_create', {
				cord_id: cord.id,
				...limits
			});
			assert.ok(!res.ok, JSON.stringify(limits));
			assert.strictEqual(res.error?.code, JSONRPC_ERROR_CODES.invalid_params);
		}
	});
});

describe('members', () => {
	test('role changes follow the permission tiers', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const admin = await fixture.create_account({ username: 'cord_admin' });
		const member = await fixture.create_account({ username: 'member' });
		const outsider = await fixture.create_account({ username: 'outsider' });
		await join_as(fixture, cord.id, admin);
		await join_as(fixture, cord.id, member);

		// the owner promotes
		const promoted = expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: admin.actor.id,
				role: CORD_ADMIN
			}),
			CordMemberRoleSetOutput
		).member;
		assert.deepEqual(promoted, {
			actor_id: admin.actor.id,
			username: 'cord_admin',
			display_name: 'cord_admin',
			role: CORD_ADMIN
		});
		// one role per (account, cord): the old grant is gone
		const status = await fixture.fresh_transport()('/api/account/status', {
			method: 'GET',
			headers: admin.create_session_headers()
		});
		const grants = (
			(await status.json()) as { role_grants: Array<{ role: string; scope_id: string | null }> }
		).role_grants
			.filter((g) => g.scope_id === cord.id)
			.map((g) => g.role);
		assert.deepEqual(grants, [CORD_ADMIN]);
		const revokes = (await audit_events(fixture, 'role_grant_revoke')).filter(
			(e) => e.target_actor_id === admin.actor.id
		);
		assert.deepEqual(
			revokes.map((e) => {
				const m = e.metadata as { role: string; scope_id: string; reason: string };
				return [m.role, m.scope_id, m.reason];
			}),
			[[CORD_MEMBER, cord.id, 'cord_member_role_set']]
		);

		// admins and members can't set roles; the owner can't set an outsider's
		for (const caller of [admin, member]) {
			assert_refused(
				await call(fixture, caller, 'cord_member_role_set', {
					cord_id: cord.id,
					actor_id: member.actor.id,
					role: CORD_ADMIN
				}),
				ERROR_CORD_MEMBER_FORBIDDEN,
				JSONRPC_ERROR_CODES.forbidden
			);
		}
		assert_refused(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: outsider.actor.id,
				role: CORD_MEMBER
			}),
			ERROR_CORD_MEMBER_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);
		// the spine's role names aren't cord roles
		const builtin = await call(fixture, fixture, 'cord_member_role_set', {
			cord_id: cord.id,
			actor_id: member.actor.id,
			role: ROLE_ADMIN
		});
		assert.strictEqual(builtin.error?.code, JSONRPC_ERROR_CODES.invalid_params);

		assert.deepEqual(await roster(fixture, member, cord.id), [
			[fixture.actor.id, CORD_OWNER],
			[admin.actor.id, CORD_ADMIN],
			[member.actor.id, CORD_MEMBER]
		]);
	});

	test('a cord always keeps an owner', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const second = await fixture.create_account({ username: 'second' });
		await join_as(fixture, cord.id, second);

		// the only owner can't leave, be removed, or step down
		assert_refused(
			await call(fixture, fixture, 'cord_leave', { cord_id: cord.id }),
			ERROR_CORD_LAST_OWNER,
			JSONRPC_ERROR_CODES.conflict
		);
		// removing yourself is leaving, which the remove verb refuses outright
		assert_refused(
			await call(fixture, fixture, 'cord_member_remove', {
				cord_id: cord.id,
				actor_id: fixture.actor.id
			}),
			ERROR_CORD_MEMBER_REMOVE_SELF,
			JSONRPC_ERROR_CODES.invalid_params
		);
		assert_refused(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: fixture.actor.id,
				role: CORD_ADMIN
			}),
			ERROR_CORD_LAST_OWNER
		);

		// with a second owner, the first may step down
		expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: second.actor.id,
				role: CORD_OWNER
			}),
			CordMemberRoleSetOutput
		);
		expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: fixture.actor.id,
				role: CORD_MEMBER
			}),
			CordMemberRoleSetOutput
		);
		assert.deepEqual(await roster(fixture, fixture, cord.id), [
			[second.actor.id, CORD_OWNER],
			[fixture.actor.id, CORD_MEMBER]
		]);
		// and has no owner power left
		assert_refused(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: fixture.actor.id,
				role: CORD_OWNER
			}),
			ERROR_CORD_MEMBER_FORBIDDEN
		);
		// the new owner is now the last one
		assert_refused(
			await call(fixture, second, 'cord_leave', { cord_id: cord.id }),
			ERROR_CORD_LAST_OWNER
		);
	});

	test('removal follows the permission tiers', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const admin = await fixture.create_account({ username: 'cord_admin' });
		const other_admin = await fixture.create_account({ username: 'other_admin' });
		const member = await fixture.create_account({ username: 'member' });
		const other = await fixture.create_account({ username: 'other' });
		await join_as(fixture, cord.id, admin, CORD_ADMIN);
		await join_as(fixture, cord.id, other_admin, CORD_ADMIN);
		await join_as(fixture, cord.id, member);
		await join_as(fixture, cord.id, other);

		const remove = (caller: Caller, target: { actor: { id: string } }) =>
			call(fixture, caller, 'cord_member_remove', { cord_id: cord.id, actor_id: target.actor.id });

		// a member removes no one; an admin removes no owner or admin
		assert_refused(await remove(member, other), ERROR_CORD_MEMBER_FORBIDDEN);
		assert_refused(await remove(admin, fixture), ERROR_CORD_MEMBER_FORBIDDEN);
		assert_refused(await remove(admin, other_admin), ERROR_CORD_MEMBER_FORBIDDEN);
		const stranger = await fixture.create_account({ username: 'stranger' });
		assert_refused(await remove(admin, stranger), ERROR_CORD_MEMBER_NOT_FOUND);
		// nobody removes themself, whatever their role; they leave instead
		for (const caller of [fixture, admin, member]) {
			assert_refused(
				await remove(caller, caller),
				ERROR_CORD_MEMBER_REMOVE_SELF,
				JSONRPC_ERROR_CODES.invalid_params
			);
		}

		// an admin removes a member; the owner removes an admin
		expect_output(await remove(admin, member), CordMemberRemoveOutput);
		expect_output(await remove(fixture, other_admin), CordMemberRemoveOutput);
		assert.deepEqual(await roster(fixture, fixture, cord.id), [
			[fixture.actor.id, CORD_OWNER],
			[admin.actor.id, CORD_ADMIN],
			[other.actor.id, CORD_MEMBER]
		]);
		const reasons = (await audit_events(fixture, 'role_grant_revoke'))
			.filter((e) => (e.metadata as { reason?: string }).reason === 'cord_member_remove')
			.map((e) => e.target_actor_id);
		assert.deepEqual(reasons, [member.actor.id, other_admin.actor.id]);

		// the removed member is out: the cord is gone from their view
		assert_refused(
			await call(fixture, member, 'cord_get', { cord_id: cord.id }),
			ERROR_CORD_NOT_FOUND
		);
		const list = await call(fixture, member, 'cord_list', {});
		assert.deepEqual(expect_output(list, CordListOutput).cords, []);
		// and a fresh link brings them back
		await join_as(fixture, cord.id, member);
		assert.strictEqual(
			expect_output(await call(fixture, member, 'cord_get', { cord_id: cord.id }), CordGetOutput)
				.cord.role,
			CORD_MEMBER
		);
	});

	test('any member but the last owner may leave', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const admin = await fixture.create_account({ username: 'cord_admin' });
		const member = await fixture.create_account({ username: 'member' });
		await join_as(fixture, cord.id, admin, CORD_ADMIN);
		await join_as(fixture, cord.id, member);

		for (const leaver of [member, admin]) {
			expect_output(
				await call(fixture, leaver, 'cord_leave', { cord_id: cord.id }),
				CordLeaveOutput
			);
			assert_refused(
				await call(fixture, leaver, 'cord_get', { cord_id: cord.id }),
				ERROR_CORD_NOT_FOUND
			);
			// leaving twice is the same as never having been in
			assert_refused(
				await call(fixture, leaver, 'cord_leave', { cord_id: cord.id }),
				ERROR_CORD_NOT_FOUND
			);
		}
		assert.deepEqual(await roster(fixture, fixture, cord.id), [[fixture.actor.id, CORD_OWNER]]);
		const leaves = (await audit_events(fixture, 'role_grant_revoke')).filter(
			(e) => (e.metadata as { reason?: string }).reason === 'cord_leave'
		);
		assert.deepEqual(
			leaves.map((e) => [e.actor_id, e.target_actor_id]),
			[
				[member.actor.id, member.actor.id],
				[admin.actor.id, admin.actor.id]
			]
		);
	});
});

describe('the IDOR mask', () => {
	test('outside a cord, every verb that names it answers as for no cord at all', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const member = await fixture.create_account({ username: 'member' });
		await join_as(fixture, cord.id, member);
		const { invite } = await mint(fixture, fixture, cord.id);

		// a plain account, and a second instance admin who isn't in the cord
		const stranger = await fixture.create_account({ username: 'stranger' });
		const admin = await fixture.create_account({ username: 'second_admin', roles: [ROLE_ADMIN] });

		for (const caller of [stranger, admin]) {
			for (const cord_id of [cord.id, crypto.randomUUID()]) {
				for (const [method, params] of [
					['cord_invite_create', { cord_id }],
					['cord_invite_list', { cord_id }],
					['cord_invite_revoke', { cord_id, invite_id: invite.id }],
					['cord_member_list', { cord_id }],
					['cord_member_role_set', { cord_id, actor_id: member.actor.id, role: CORD_OWNER }],
					['cord_member_remove', { cord_id, actor_id: member.actor.id }],
					['cord_leave', { cord_id }]
				] as const) {
					assert_refused(
						await call(fixture, caller, method, params),
						ERROR_CORD_NOT_FOUND,
						JSONRPC_ERROR_CODES.not_found
					);
				}
			}
		}
		// nothing changed
		assert.deepEqual(await roster(fixture, fixture, cord.id), [
			[fixture.actor.id, CORD_OWNER],
			[member.actor.id, CORD_MEMBER]
		]);
		const listed = expect_output(
			await call(fixture, fixture, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		).invites;
		assert.ok(listed.every((i) => i.revoked_at === null));
	});
});
