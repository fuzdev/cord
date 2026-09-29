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
import { DEFAULT_TEST_PASSWORD } from '@fuzdev/fuz_app/testing/test_credentials.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import { ROLE_ADMIN } from '@fuzdev/fuz_app/auth/role_schema.ts';
import {
	AdminAccountListOutput,
	AppSettingsUpdateOutput,
	AuditLogListOutput,
	InviteListOutput
} from '@fuzdev/fuz_app/auth/admin_action_specs.ts';
import { RoleGrantRevokeOutput } from '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts';
import { ERROR_NO_MATCHING_INVITE } from '@fuzdev/fuz_app/http/error_schemas.ts';

import './cross_test_types.ts';
import {
	CordCreateOutput,
	CordInviteAcceptOutput,
	CordInviteCreateOutput,
	CordInviteListOutput,
	CordInvitePreviewOutput,
	CordInviteRedeemOutput,
	CordInviteRevokeOutput,
	CordListOutput,
	CordMemberRoleSetOutput,
	CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX,
	ERROR_CORD_INVITE_EXHAUSTED,
	ERROR_CORD_INVITE_EXPIRED,
	ERROR_CORD_INVITE_INVALID,
	ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
	ERROR_CORD_INVITE_REVOKED,
	ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN,
	ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED,
	ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
	ERROR_CORD_INVITE_NOT_FOUND,
	ERROR_CORD_INVITE_USERNAME_INVALID,
	ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
	ERROR_CORD_NOT_FOUND
} from '#lib/cord_action_specs.ts';
import { CORD_MEMBER, CORD_OWNER } from '#lib/cord_roles.ts';

/**
 * Newcomer links end to end against a spawned `testing_cordd`: an instance
 * admin mints a link that can bring someone with no account onto the
 * instance; the newcomer previews it and redeems it for a username without
 * signing in, signs up through the stock invite-gated signup with that
 * username, and accepts the link signed in. Each test starts from a fresh
 * keeper (an instance admin) and no cords (`_testing_reset`).
 */

const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));
const setup_test = default_cross_process_setup(handle);
const { rpc_path } = handle.config;

type Fixture = Awaited<ReturnType<typeof setup_test>>;
type Caller = Fixture | TestAccountFixture;

/** Call `method` as `caller` on a clean cookie jar. */
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

/** Call `method` with no credential at all. */
const call_anonymous = (
	fixture: Fixture,
	method: string,
	params: unknown
): ReturnType<typeof cross_rpc_call> =>
	cross_rpc_call(fixture.fresh_transport(), rpc_path, method, params, {});

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

const json_post = (body: unknown): RequestInit => ({
	method: 'POST',
	headers: { 'content-type': 'application/json' },
	body: JSON.stringify(body)
});

/** Create a cord as the keeper, who owns it. */
const create_cord = async (fixture: Fixture, name = 'the porch') =>
	expect_output(await call(fixture, fixture, 'cord_create', { name }), CordCreateOutput).cord;

/** Mint a link on `cord_id` as `caller`. */
const mint = async (
	fixture: Fixture,
	caller: Caller,
	cord_id: string,
	options: { expires_in_seconds?: number; max_uses?: number; admits_newcomers?: boolean } = {}
) =>
	expect_output(
		await call(fixture, caller, 'cord_invite_create', { cord_id, ...options }),
		CordInviteCreateOutput
	);

/** Mint a newcomer link as the keeper. */
const mint_newcomer = (fixture: Fixture, cord_id: string, max_uses = 3) =>
	mint(fixture, fixture, cord_id, { expires_in_seconds: 3600, max_uses, admits_newcomers: true });

/** The cord's links as the keeper lists them. */
const list_invites = async (fixture: Fixture, cord_id: string) =>
	expect_output(await call(fixture, fixture, 'cord_invite_list', { cord_id }), CordInviteListOutput)
		.invites;

type Transport = ReturnType<Fixture['fresh_transport']>;

/** Sign up through the stock signup on a fresh transport, which it leaves signed in. */
const sign_up = async (
	fixture: Fixture,
	body: { username: string; email?: string }
): Promise<Transport> => {
	const transport = fixture.fresh_transport();
	const res = await transport(
		'/api/account/signup',
		json_post({ ...body, password: DEFAULT_TEST_PASSWORD })
	);
	assert.strictEqual(res.status, 200, await res.clone().text());
	return transport;
};

/** Accept `token` on a signed-in transport. */
const accept_on = async (transport: Transport, token: string) =>
	expect_output(
		await cross_rpc_call(transport, rpc_path, 'cord_invite_accept', { token }, {}),
		CordInviteAcceptOutput
	);

/** Redeem `token` for `username` with no credential. */
const redeem_anonymous = async (fixture: Fixture, token: string, username: string) =>
	expect_output(
		await call_anonymous(fixture, 'cord_invite_redeem', { token, username }),
		CordInviteRedeemOutput
	);

/** Preview `token` with no credential. */
const preview_anonymous = async (fixture: Fixture, token: string) =>
	expect_output(
		await call_anonymous(fixture, 'cord_invite_preview', { token }),
		CordInvitePreviewOutput
	);

/** Audit events of `event_type`, oldest first, read by the keeper. */
const audit_events = async (fixture: Fixture, event_type: string) =>
	expect_output(
		await call(fixture, fixture, 'audit_log_list', { event_type }),
		AuditLogListOutput
	).events.reverse();

describe('the newcomer path', () => {
	test('preview, redeem, sign up, log in, accept', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const minted = await mint_newcomer(fixture, cord.id, 2);
		assert.isTrue(minted.invite.admits_newcomers);

		// the join page, signed out: the link, its cord, and that it admits newcomers
		const preview = expect_output(
			await call_anonymous(fixture, 'cord_invite_preview', { token: minted.token }),
			CordInvitePreviewOutput
		);
		assert.deepEqual(preview, {
			status: 'active',
			admits_newcomers: true,
			newcomers_open: true,
			cord: { id: cord.id, name: 'the porch' },
			role: null
		});

		// redeem for a username, signed out; the same username again changes nothing
		const redeemed = expect_output(
			await call_anonymous(fixture, 'cord_invite_redeem', {
				token: minted.token,
				username: 'NewComer'
			}),
			CordInviteRedeemOutput
		);
		assert.deepEqual(redeemed, {
			cord: { id: cord.id, name: 'the porch' },
			username: 'newcomer',
			created: true
		});
		const again = expect_output(
			await call_anonymous(fixture, 'cord_invite_redeem', {
				token: minted.token,
				username: 'newcomer'
			}),
			CordInviteRedeemOutput
		);
		assert.isFalse(again.created);
		const [after_redeem] = await list_invites(fixture, cord.id);
		assert.deepInclude(after_redeem, {
			use_count: 1,
			redemption_count: 1,
			redemption_accepted_count: 0
		});

		// the reservation is an ordinary spine invite in the minter's name
		const spine_invites = expect_output(
			await call(fixture, fixture, 'invite_list', {}),
			InviteListOutput
		).invites;
		assert.deepEqual(
			spine_invites.map((i) => [i.username, i.created_by, i.claimed_at]),
			[['newcomer', fixture.actor.id, null]]
		);

		// the stock signup admits the reserved username
		const signup = await fixture.fresh_transport()(
			'/api/account/signup',
			json_post({ username: 'newcomer', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(signup.status, 200, await signup.clone().text());

		// log in, then accept
		const transport = fixture.fresh_transport();
		const login = await transport(
			'/api/account/login',
			json_post({ username: 'newcomer', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(login.status, 200);
		// signed in now, the preview reads as before
		const signed_in_preview = expect_output(
			await cross_rpc_call(transport, rpc_path, 'cord_invite_preview', { token: minted.token }, {}),
			CordInvitePreviewOutput
		);
		assert.deepEqual(signed_in_preview, preview);
		const accepted = expect_output(
			await cross_rpc_call(transport, rpc_path, 'cord_invite_accept', { token: minted.token }, {}),
			CordInviteAcceptOutput
		);
		assert.deepEqual(accepted, { cord: { ...cord, role: CORD_MEMBER }, joined: true });
		const cords = expect_output(
			await cross_rpc_call(transport, rpc_path, 'cord_list', {}, {}),
			CordListOutput
		).cords;
		assert.deepEqual(
			cords.map((c) => [c.id, c.role]),
			[[cord.id, CORD_MEMBER]]
		);

		// the accept spent nothing: the redemption had
		const [after_accept] = await list_invites(fixture, cord.id);
		assert.deepInclude(after_accept, {
			use_count: 1,
			redemption_count: 1,
			redemption_accepted_count: 1
		});

		// audited: the redemption carries no actor and never the token; the
		// accept names the redemption it completed
		const redeems = await audit_events(fixture, 'cord_invite_redeem');
		assert.strictEqual(redeems.length, 1, 'the idempotent redeem wrote nothing');
		assert.strictEqual(redeems[0]!.actor_id, null);
		assert.deepInclude(redeems[0]!.metadata, {
			cord_id: cord.id,
			invite_id: minted.invite.id,
			username: 'newcomer'
		});
		assert.ok(!JSON.stringify(redeems).includes(minted.token));
		const redemption_id = (redeems[0]!.metadata as { redemption_id: string }).redemption_id;
		const accepts = await audit_events(fixture, 'cord_invite_accept');
		assert.strictEqual(accepts.length, 1);
		assert.deepInclude(accepts[0]!.metadata, {
			cord_id: cord.id,
			redemption_id,
			claimed_signup_invite_id: null
		});
	});

	test('a redeemed newcomer joins even once the link is used up', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const { token } = await mint_newcomer(fixture, cord.id, 1);
		expect_output(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'newcomer' }),
			CordInviteRedeemOutput
		);
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'another' }),
			ERROR_CORD_INVITE_EXHAUSTED,
			JSONRPC_ERROR_CODES.conflict
		);
		const transport = fixture.fresh_transport();
		const signup = await transport(
			'/api/account/signup',
			json_post({ username: 'newcomer', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(signup.status, 200, await signup.clone().text());
		const accepted = expect_output(
			await cross_rpc_call(transport, rpc_path, 'cord_invite_accept', { token }, {}),
			CordInviteAcceptOutput
		);
		assert.isTrue(accepted.joined);
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, { use_count: 1, status: 'exhausted', redemption_accepted_count: 1 });
	});

	test('a waiting username is redeemed again within the grace after expiry', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const minted = await mint_newcomer(fixture, cord.id);
		assert.isTrue((await redeem_anonymous(fixture, minted.token, 'newcomer')).created);
		// backdates the expiry by a second, well inside the accept grace
		const expire = await cross_rpc_call(
			fixture.fresh_transport({ origin: null }),
			rpc_path,
			'_testing_cord_invite_expire',
			{ invite_id: minted.invite.id },
			fixture.create_daemon_token_headers()
		);
		assert.ok(expire.ok, JSON.stringify(expire.error));

		// a retry after a failed signup: the reservation stands, spending nothing
		assert.deepEqual(await redeem_anonymous(fixture, minted.token, 'newcomer'), {
			cord: { id: cord.id, name: 'the porch' },
			username: 'newcomer',
			created: false
		});
		// a new username needs a live link
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', {
				token: minted.token,
				username: 'another'
			}),
			ERROR_CORD_INVITE_EXPIRED,
			JSONRPC_ERROR_CODES.conflict
		);
		const transport = await sign_up(fixture, { username: 'newcomer' });
		assert.isTrue((await accept_on(transport, minted.token)).joined);
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, {
			status: 'expired',
			use_count: 1,
			redemption_count: 1,
			redemption_accepted_count: 1
		});
	});

	test('an existing account accepts a newcomer link as any other, spending a use', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const guest = await fixture.create_account({ username: 'guest' });
		const { token } = await mint_newcomer(fixture, cord.id);
		const accepted = expect_output(
			await call(fixture, guest, 'cord_invite_accept', { token }),
			CordInviteAcceptOutput
		);
		assert.isTrue(accepted.joined);
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, { use_count: 1, redemption_count: 0 });
	});
});

describe('minting newcomer links', () => {
	test('a cord owner who is not an instance admin cannot', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const owner = await fixture.create_account({ username: 'owner' });
		const { token } = await mint(fixture, fixture, cord.id);
		expect_output(
			await call(fixture, owner, 'cord_invite_accept', { token }),
			CordInviteAcceptOutput
		);
		expect_output(
			await call(fixture, fixture, 'cord_member_role_set', {
				cord_id: cord.id,
				actor_id: owner.actor.id,
				role: CORD_OWNER
			}),
			CordMemberRoleSetOutput
		);
		const bounds = { expires_in_seconds: 3600, max_uses: 3 };
		assert_refused(
			await call(fixture, owner, 'cord_invite_create', {
				cord_id: cord.id,
				...bounds,
				admits_newcomers: true
			}),
			ERROR_CORD_INVITE_NEWCOMER_FORBIDDEN,
			JSONRPC_ERROR_CODES.forbidden
		);
		// an ordinary link is theirs to make
		const ordinary = await mint(fixture, owner, cord.id, { ...bounds, admits_newcomers: false });
		assert.isFalse(ordinary.invite.admits_newcomers);

		// an instance admin outside the cord gets the cord's mask, not the
		// newcomer refusal
		const admin = await fixture.create_account({ username: 'second_admin', roles: [ROLE_ADMIN] });
		assert_refused(
			await call(fixture, admin, 'cord_invite_create', {
				cord_id: cord.id,
				...bounds,
				admits_newcomers: true
			}),
			ERROR_CORD_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);
		assert.deepEqual(
			(await list_invites(fixture, cord.id)).map((i) => i.admits_newcomers),
			[false, false]
		);
	});

	test('a newcomer link needs both an expiry and a use limit', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		for (const bounds of [{}, { expires_in_seconds: 3600 }, { max_uses: 3 }]) {
			assert_refused(
				await call(fixture, fixture, 'cord_invite_create', {
					cord_id: cord.id,
					...bounds,
					admits_newcomers: true
				}),
				ERROR_CORD_INVITE_NEWCOMER_UNBOUNDED,
				JSONRPC_ERROR_CODES.invalid_params
			);
		}
		assert.deepEqual(await list_invites(fixture, cord.id), []);
	});
});

describe('redeeming', () => {
	test('an ordinary link, an unknown token, and a bad or taken username are refused', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		await fixture.create_account({ username: 'taken' });
		const ordinary = await mint(fixture, fixture, cord.id);
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', {
				token: ordinary.token,
				username: 'newcomer'
			}),
			ERROR_CORD_INVITE_NOT_FOR_NEWCOMERS,
			JSONRPC_ERROR_CODES.conflict
		);
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token: 'nope', username: 'newcomer' }),
			ERROR_CORD_INVITE_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);

		const { token } = await mint_newcomer(fixture, cord.id);
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'Taken' }),
			ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
			JSONRPC_ERROR_CODES.conflict
		);
		for (const username of ['ab', '9lives', 'trailing_']) {
			assert_refused(
				await call_anonymous(fixture, 'cord_invite_redeem', { token, username }),
				ERROR_CORD_INVITE_USERNAME_INVALID,
				JSONRPC_ERROR_CODES.invalid_params
			);
		}
		// the spine's own invite reserves a username too
		const invite = await call(fixture, fixture, 'invite_create', { username: 'reserved' });
		assert.ok(invite.ok, JSON.stringify(invite.error));
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'reserved' }),
			ERROR_CORD_INVITE_USERNAME_UNAVAILABLE,
			JSONRPC_ERROR_CODES.conflict
		);
		assert.deepEqual(
			(await list_invites(fixture, cord.id)).map((i) => [i.use_count, i.redemption_count]),
			[
				[0, 0],
				[0, 0]
			],
			'no refusal spent a use'
		);
	});
});

describe('the anonymous preview', () => {
	test('names the cord only for an active newcomer link', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const ordinary = await mint(fixture, fixture, cord.id);
		const newcomer = await mint_newcomer(fixture, cord.id);
		const revoked = await mint_newcomer(fixture, cord.id);
		expect_output(
			await call(fixture, fixture, 'cord_invite_revoke', {
				cord_id: cord.id,
				invite_id: revoked.invite.id
			}),
			CordInviteRevokeOutput
		);

		const anonymous = async (token: string) =>
			expect_output(
				await call_anonymous(fixture, 'cord_invite_preview', { token }),
				CordInvitePreviewOutput
			);
		assert.deepEqual(await anonymous(newcomer.token), {
			status: 'active',
			admits_newcomers: true,
			newcomers_open: true,
			cord: { id: cord.id, name: 'the porch' },
			role: null
		});
		assert.deepEqual(await anonymous(ordinary.token), {
			status: 'active',
			admits_newcomers: false,
			newcomers_open: false,
			cord: null,
			role: null
		});
		assert.deepEqual(await anonymous(revoked.token), {
			status: 'revoked',
			admits_newcomers: true,
			newcomers_open: false,
			cord: null,
			role: null
		});
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_preview', { token: 'nope' }),
			ERROR_CORD_INVITE_NOT_FOUND,
			JSONRPC_ERROR_CODES.not_found
		);

		// signed in, an active ordinary link names its cord as before
		const guest = await fixture.create_account({ username: 'guest' });
		const signed_in = expect_output(
			await call(fixture, guest, 'cord_invite_preview', { token: ordinary.token }),
			CordInvitePreviewOutput
		);
		assert.deepEqual(signed_in.cord, { id: cord.id, name: 'the porch' });
	});
});

describe('a signup that claims no reservation', () => {
	test('under open signup, the accept claims it and spends no second use', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const { token, invite } = await mint_newcomer(fixture, cord.id, 1);
		await redeem_anonymous(fixture, token, 'newcomer');
		expect_output(
			await call(fixture, fixture, 'app_settings_update', { open_signup: true }),
			AppSettingsUpdateOutput
		);
		const transport = await sign_up(fixture, { username: 'newcomer' });
		// open signup claimed nothing
		const before = expect_output(
			await call(fixture, fixture, 'invite_list', {}),
			InviteListOutput
		).invites;
		assert.deepEqual(
			before.map((i) => [i.username, i.claimed_at]),
			[['newcomer', null]]
		);

		const accepted = await accept_on(transport, token);
		assert.isTrue(accepted.joined);
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, {
			id: invite.id,
			use_count: 1,
			status: 'exhausted',
			redemption_count: 1,
			redemption_accepted_count: 1
		});
		const after = expect_output(
			await call(fixture, fixture, 'invite_list', {}),
			InviteListOutput
		).invites;
		assert.strictEqual(after[0]!.claimed_by_username, 'newcomer', 'the accept claimed it');
		// audited with the invite it claimed
		const [accept_event] = await audit_events(fixture, 'cord_invite_accept');
		assert.deepInclude(accept_event!.metadata, { claimed_signup_invite_id: before[0]!.id });
	});

	test('when an older invite matching the email won, the accept completes the redemption', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const email_invite = await call(fixture, fixture, 'invite_create', {
			email: 'newcomer@example.com'
		});
		assert.ok(email_invite.ok, JSON.stringify(email_invite.error));
		const { token } = await mint_newcomer(fixture, cord.id, 1);
		await redeem_anonymous(fixture, token, 'newcomer');
		const transport = await sign_up(fixture, {
			username: 'newcomer',
			email: 'newcomer@example.com'
		});
		/** Each spine invite's email or username, and who claimed it, sorted. */
		const claimed = async () =>
			expect_output(await call(fixture, fixture, 'invite_list', {}), InviteListOutput).invites
				.map((i) => [i.email ?? i.username ?? '', i.claimed_by_username])
				.sort((a, b) => a[0]!.localeCompare(b[0]!));
		assert.deepEqual(await claimed(), [
			['newcomer', null],
			['newcomer@example.com', 'newcomer']
		]);

		const accepted = await accept_on(transport, token);
		assert.isTrue(accepted.joined);
		assert.deepEqual(await claimed(), [
			['newcomer', 'newcomer'],
			['newcomer@example.com', 'newcomer']
		]);
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, { use_count: 1, redemption_count: 1, redemption_accepted_count: 1 });
	});

	test('someone already in the cord completes their redemption, audited with no grant', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const newcomer_link = await mint_newcomer(fixture, cord.id);
		const ordinary = await mint(fixture, fixture, cord.id);
		await redeem_anonymous(fixture, newcomer_link.token, 'newcomer');
		const transport = await sign_up(fixture, { username: 'newcomer' });
		assert.isTrue((await accept_on(transport, ordinary.token)).joined);

		const accepted = await accept_on(transport, newcomer_link.token);
		assert.isFalse(accepted.joined);
		const redemption_id = (
			(await audit_events(fixture, 'cord_invite_redeem'))[0]!.metadata as {
				redemption_id: string;
			}
		).redemption_id;
		const accepts = await audit_events(fixture, 'cord_invite_accept');
		assert.deepEqual(
			accepts.map((e) => e.metadata),
			[
				{
					cord_id: cord.id,
					invite_id: ordinary.invite.id,
					role_grant_id: (accepts[0]!.metadata as { role_grant_id: string }).role_grant_id,
					redemption_id: null,
					claimed_signup_invite_id: null
				},
				{
					cord_id: cord.id,
					invite_id: newcomer_link.invite.id,
					role_grant_id: null,
					redemption_id,
					claimed_signup_invite_id: null
				}
			]
		);
		const link = (await list_invites(fixture, cord.id)).find(
			(i) => i.id === newcomer_link.invite.id
		);
		assert.deepInclude(link, { use_count: 1, redemption_accepted_count: 1 });
	});
});

describe('revoking a newcomer link', () => {
	test('deletes its waiting reservation, so the stock signup refuses the username', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const { token, invite } = await mint_newcomer(fixture, cord.id);
		await redeem_anonymous(fixture, token, 'newcomer');
		const revoke = () =>
			call(fixture, fixture, 'cord_invite_revoke', { cord_id: cord.id, invite_id: invite.id });
		expect_output(await revoke(), CordInviteRevokeOutput);
		expect_output(await revoke(), CordInviteRevokeOutput);

		assert.deepEqual(
			expect_output(await call(fixture, fixture, 'invite_list', {}), InviteListOutput).invites,
			[]
		);
		const signup = await fixture.fresh_transport()(
			'/api/account/signup',
			json_post({ username: 'newcomer', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(signup.status, 403);
		assert.strictEqual(
			((await signup.json()) as { error?: unknown }).error,
			ERROR_NO_MATCHING_INVITE
		);

		// a dead link names neither its cord nor which usernames waited on it
		assert.deepEqual(await preview_anonymous(fixture, token), {
			status: 'revoked',
			admits_newcomers: true,
			newcomers_open: false,
			cord: null,
			role: null
		});
		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'newcomer' }),
			ERROR_CORD_INVITE_REVOKED,
			JSONRPC_ERROR_CODES.conflict
		);

		const revokes = await audit_events(fixture, 'cord_invite_revoke');
		assert.strictEqual(revokes.length, 1, 'the second revoke changed nothing');
		const [redeemed] = await audit_events(fixture, 'cord_invite_redeem');
		assert.deepEqual(revokes[0]!.metadata, {
			cord_id: cord.id,
			invite_id: invite.id,
			released_signup_invite_ids: [
				(redeemed!.metadata as { signup_invite_id: string }).signup_invite_id
			]
		});
		const [link] = await list_invites(fixture, cord.id);
		assert.deepInclude(link, { use_count: 1, redemption_count: 1, redemption_accepted_count: 0 });
	});
});

describe('newcomer links and their minter', () => {
	test('close once the minter is no longer an instance admin', async () => {
		const fixture = await setup_test();
		const second = await fixture.create_account({ username: 'second_admin', roles: [ROLE_ADMIN] });
		const cord = expect_output(
			await call(fixture, second, 'cord_create', { name: 'the stoop' }),
			CordCreateOutput
		).cord;
		const { token } = await mint(fixture, second, cord.id, {
			expires_in_seconds: 3600,
			max_uses: 3,
			admits_newcomers: true
		});
		assert.isTrue((await preview_anonymous(fixture, token)).newcomers_open);
		assert.isTrue((await redeem_anonymous(fixture, token, 'early')).created);

		const accounts = expect_output(
			await call(fixture, fixture, 'admin_account_list', {}),
			AdminAccountListOutput
		).accounts;
		const admin_grant = accounts
			.find((a) => a.actor?.id === second.actor.id)!
			.role_grants.find((g) => g.role === ROLE_ADMIN && g.scope_id === null)!;
		expect_output(
			await call(fixture, fixture, 'role_grant_revoke', {
				actor_id: second.actor.id,
				role_grant_id: admin_grant.id
			}),
			RoleGrantRevokeOutput
		);

		assert_refused(
			await call_anonymous(fixture, 'cord_invite_redeem', { token, username: 'newcomer' }),
			ERROR_CORD_INVITE_NEWCOMERS_CLOSED,
			JSONRPC_ERROR_CODES.conflict
		);
		// a username already waiting keeps its reservation, which accept still takes
		assert.isFalse((await redeem_anonymous(fixture, token, 'early')).created);
		// still active, but closed to newcomers, so an anonymous preview names no cord
		assert.deepEqual(await preview_anonymous(fixture, token), {
			status: 'active',
			admits_newcomers: true,
			newcomers_open: false,
			cord: null,
			role: null
		});
		const [link] = expect_output(
			await call(fixture, second, 'cord_invite_list', { cord_id: cord.id }),
			CordInviteListOutput
		).invites;
		assert.deepInclude(link, { use_count: 1, redemption_count: 1 });
	});

	test('a newcomer link expires within the newcomer limit', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const at_limit = await mint(fixture, fixture, cord.id, {
			expires_in_seconds: CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX,
			max_uses: 3,
			admits_newcomers: true
		});
		assert.isTrue(at_limit.invite.admits_newcomers);
		assert_refused(
			await call(fixture, fixture, 'cord_invite_create', {
				cord_id: cord.id,
				expires_in_seconds: CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX + 1,
				max_uses: 3,
				admits_newcomers: true
			}),
			ERROR_CORD_INVITE_INVALID,
			JSONRPC_ERROR_CODES.invalid_params
		);
		// an ordinary link keeps the longer limit
		const ordinary = await mint(fixture, fixture, cord.id, {
			expires_in_seconds: CORD_INVITE_NEWCOMER_EXPIRES_IN_SECONDS_MAX + 1
		});
		assert.isFalse(ordinary.invite.admits_newcomers);
	});
});

describe('redeeming with a session', () => {
	test('behaves as anonymous', async () => {
		const fixture = await setup_test();
		const cord = await create_cord(fixture);
		const { token } = await mint_newcomer(fixture, cord.id);
		const redeemed = expect_output(
			await call(fixture, fixture, 'cord_invite_redeem', { token, username: 'newcomer' }),
			CordInviteRedeemOutput
		);
		assert.deepEqual(redeemed, {
			cord: { id: cord.id, name: 'the porch' },
			username: 'newcomer',
			created: true
		});
		const [redeem_event] = await audit_events(fixture, 'cord_invite_redeem');
		assert.strictEqual(redeem_event!.actor_id, null);
		assert.strictEqual(redeem_event!.account_id, null);
		// the keeper holding a session didn't join or reserve anything for themself
		const cords = expect_output(
			await call(fixture, fixture, 'cord_list', {}),
			CordListOutput
		).cords;
		assert.deepEqual(
			cords.map((c) => [c.id, c.role]),
			[[cord.id, CORD_OWNER]]
		);
	});
});
