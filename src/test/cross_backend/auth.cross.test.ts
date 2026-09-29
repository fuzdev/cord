import { assert, describe, inject, test } from 'vitest';
import {
	default_cross_process_setup,
	reconstruct_bootstrapped_handle
} from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { cross_rpc_call } from '@fuzdev/fuz_app/testing/cross_backend/cell_cross_helpers.ts';
import { DEFAULT_TEST_PASSWORD } from '@fuzdev/fuz_app/testing/test_credentials.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import {
	ERROR_ALREADY_BOOTSTRAPPED,
	ERROR_AUTHENTICATION_REQUIRED,
	ERROR_CREDENTIAL_TYPE_REQUIRED,
	ERROR_FORBIDDEN_ORIGIN,
	ERROR_INSUFFICIENT_PERMISSIONS,
	ERROR_NO_MATCHING_INVITE
} from '@fuzdev/fuz_app/http/error_schemas.ts';

import './cross_test_types.ts';
import { CORD_SESSION_COOKIE_NAME } from './cord_backend_config.ts';

/**
 * cord's auth flows end to end against a spawned `testing_cordd` over real
 * HTTP: bootstrap, login and logout, invite-gated signup, the admin RPC gate,
 * the origin allowlist, and the audit stream's gate. Each test starts from a
 * fresh keeper (`_testing_reset`); `create_account` mints members through the
 * production invite and signup path.
 */

const handle = reconstruct_bootstrapped_handle(inject('backend_handle'));
const setup_test = default_cross_process_setup(handle);
const { rpc_path, bootstrap } = handle.config;

const AUDIT_STREAM_PATH = '/api/admin/audit/stream';

const json_post = (body: unknown, headers: Record<string, string> = {}): RequestInit => ({
	method: 'POST',
	headers: { 'content-type': 'application/json', ...headers },
	body: JSON.stringify(body)
});

/** The response's flat REST error string, or `undefined`. */
const read_error = async (res: Response): Promise<unknown> => {
	const body = (await res.json().catch(() => undefined)) as { error?: unknown } | undefined;
	return body?.error;
};

/** The `name=value` session cookie from a transport's jar, if present. */
const find_session_cookie = (cookies: ReadonlyArray<string>): string | undefined =>
	cookies.find((c) => c.startsWith(`${CORD_SESSION_COOKIE_NAME}=`));

describe('bootstrap', () => {
	test('the global setup claimed the fresh instance with the bootstrap token', () => {
		// `bootstrap_backend` throws unless `POST /api/account/bootstrap` with
		// the token returns 200 and a session, so reaching here means it did
		assert.strictEqual(handle.keeper_account.username, bootstrap.username);
		assert.ok(
			find_session_cookie(handle.keeper_cookies),
			'bootstrap set the keeper session cookie'
		);
	});

	test('a second bootstrap is refused and creates no account', async () => {
		const fixture = await setup_test();
		const transport = fixture.fresh_transport();
		const res = await transport(
			'/api/account/bootstrap',
			json_post({
				token: bootstrap.token,
				username: 'second_keeper',
				password: DEFAULT_TEST_PASSWORD
			})
		);
		assert.strictEqual(res.status, 403);
		assert.strictEqual(await read_error(res), ERROR_ALREADY_BOOTSTRAPPED);
		assert.isUndefined(find_session_cookie(transport.cookies()), 'no session is issued');

		const login = await fixture.fresh_transport()(
			'/api/account/login',
			json_post({ username: 'second_keeper', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(login.status, 401, 'the refused bootstrap created no account');
	});
});

describe('login and logout', () => {
	test('login opens a session and logout invalidates it', async () => {
		const fixture = await setup_test();
		await fixture.create_account({ username: 'member' });

		const transport = fixture.fresh_transport();
		const before = await transport('/api/account/status', { method: 'GET' });
		assert.strictEqual(before.status, 401, 'anonymous before login');

		const login = await transport(
			'/api/account/login',
			json_post({ username: 'member', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(login.status, 200);
		const session_cookie = find_session_cookie(transport.cookies());
		assert.ok(session_cookie, 'login set the session cookie');

		const status = await transport('/api/account/status', { method: 'GET' });
		assert.strictEqual(status.status, 200);
		const status_body = (await status.json()) as { account?: { username?: string } };
		assert.strictEqual(status_body.account?.username, 'member');

		const logout = await transport('/api/account/logout', json_post({}));
		assert.strictEqual(logout.status, 200);

		// replay the pre-logout cookie on a clean jar: the server-side session
		// is gone, not just the browser's copy
		const replay = await fixture.fresh_transport()('/api/account/status', {
			method: 'GET',
			headers: { cookie: session_cookie }
		});
		assert.strictEqual(replay.status, 401, 'the logged-out session no longer authenticates');
		const replay_rpc = await cross_rpc_call(
			fixture.fresh_transport(),
			rpc_path,
			'account_verify',
			undefined,
			{ cookie: session_cookie }
		);
		assert.ok(!replay_rpc.ok, 'the logged-out session is refused over RPC too');
		assert.strictEqual(replay_rpc.error?.code, JSONRPC_ERROR_CODES.unauthenticated);
	});

	test('a wrong password is refused without a session', async () => {
		const fixture = await setup_test();
		await fixture.create_account({ username: 'member' });
		const transport = fixture.fresh_transport();
		const res = await transport(
			'/api/account/login',
			json_post({ username: 'member', password: 'not-the-password' })
		);
		assert.strictEqual(res.status, 401);
		assert.isUndefined(find_session_cookie(transport.cookies()));
	});
});

describe('invite-gated signup', () => {
	test('an admin-minted invite admits its signup; an uninvited signup is refused', async () => {
		const fixture = await setup_test();

		const invite = await cross_rpc_call(
			fixture.transport,
			rpc_path,
			'invite_create',
			{ username: 'invited' },
			fixture.create_session_headers()
		);
		assert.ok(invite.ok, `invite_create: ${JSON.stringify(invite.error)}`);

		const invited_transport = fixture.fresh_transport();
		const signup = await invited_transport(
			'/api/account/signup',
			json_post({ username: 'invited', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(signup.status, 200, await signup.clone().text());
		assert.ok(find_session_cookie(invited_transport.cookies()), 'signup opens a session');
		const verify = await cross_rpc_call(
			invited_transport,
			rpc_path,
			'account_verify',
			undefined,
			{}
		);
		assert.ok(verify.ok, `the new account is signed in: ${JSON.stringify(verify.error)}`);

		const stranger_transport = fixture.fresh_transport();
		const refused = await stranger_transport(
			'/api/account/signup',
			json_post({ username: 'stranger', password: DEFAULT_TEST_PASSWORD })
		);
		assert.strictEqual(refused.status, 403);
		assert.strictEqual(await read_error(refused), ERROR_NO_MATCHING_INVITE);
		assert.isUndefined(find_session_cookie(stranger_transport.cookies()));
	});

	test('a member cannot mint invites', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const res = await cross_rpc_call(
			fixture.fresh_transport(),
			rpc_path,
			'invite_create',
			{ username: 'friend' },
			member.create_session_headers()
		);
		assert.ok(!res.ok);
		assert.strictEqual(res.error?.code, JSONRPC_ERROR_CODES.forbidden);
	});
});

describe('admin RPC gate', () => {
	test('a member is forbidden on the admin actions the keeper can call', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const calls: Array<[method: string, params: unknown]> = [
			['admin_account_list', undefined],
			['admin_session_list', undefined],
			['invite_list', undefined],
			['audit_log_list', {}],
			['app_settings_get', undefined]
		];
		for (const [method, params] of calls) {
			const as_keeper = await cross_rpc_call(
				fixture.transport,
				rpc_path,
				method,
				params,
				fixture.create_session_headers()
			);
			assert.ok(as_keeper.ok, `keeper ${method}: ${JSON.stringify(as_keeper.error)}`);

			const as_member = await cross_rpc_call(
				fixture.fresh_transport(),
				rpc_path,
				method,
				params,
				member.create_session_headers()
			);
			assert.ok(!as_member.ok, `member ${method} must be refused`);
			assert.strictEqual(as_member.error?.code, JSONRPC_ERROR_CODES.forbidden, method);
		}
	});
});

describe('origin allowlist', () => {
	test('a disallowed Origin is refused on every browser-facing surface', async () => {
		const fixture = await setup_test();
		// each carries the keeper's session, so a refusal can't be an auth failure
		const headers = { ...fixture.create_session_headers(), origin: 'https://elsewhere.example' };
		const requests: Array<[path: string, init: RequestInit]> = [
			['/api/account/login', json_post({ username: 'keeper', password: 'x' }, headers)],
			['/api/account/bootstrap', json_post({ token: 'x', username: 'x', password: 'x' }, headers)],
			['/api/account/signup', json_post({ username: 'x', password: 'x' }, headers)],
			['/api/account/status', { method: 'GET', headers }],
			[rpc_path, json_post({ jsonrpc: '2.0', id: 1, method: 'account_verify' }, headers)],
			[AUDIT_STREAM_PATH, { method: 'GET', headers }]
		];
		for (const [path, init] of requests) {
			const res = await fixture.fresh_transport()(path, init);
			assert.strictEqual(res.status, 403, path);
			assert.strictEqual(await read_error(res), ERROR_FORBIDDEN_ORIGIN, path);
		}
	});
});

describe('audit stream gate', () => {
	test('anonymous is 401', async () => {
		const fixture = await setup_test();
		const res = await fixture.fresh_transport()(AUDIT_STREAM_PATH, { method: 'GET' });
		assert.strictEqual(res.status, 401);
		assert.strictEqual(await read_error(res), ERROR_AUTHENTICATION_REQUIRED);
	});

	test('a member is 403', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const res = await fixture.fresh_transport()(AUDIT_STREAM_PATH, {
			method: 'GET',
			headers: member.create_session_headers()
		});
		assert.strictEqual(res.status, 403);
		assert.strictEqual(await read_error(res), ERROR_INSUFFICIENT_PERMISSIONS);
	});

	test("the keeper's bearer token is refused, since the stream is session-only", async () => {
		const fixture = await setup_test();
		// no Origin: a bearer credential is discarded in a browser context
		const res = await fixture.fresh_transport({ origin: null })(AUDIT_STREAM_PATH, {
			method: 'GET',
			headers: fixture.create_bearer_headers()
		});
		assert.strictEqual(res.status, 403);
		assert.strictEqual(await read_error(res), ERROR_CREDENTIAL_TYPE_REQUIRED);
	});
});
