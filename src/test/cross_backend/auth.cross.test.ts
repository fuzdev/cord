import { assert, describe, test } from 'vitest';
import { inject_backend_handle } from '@fuzdev/fuz_app/testing/cross_backend/provided_context.ts';
import { default_cross_process_setup } from '@fuzdev/fuz_app/testing/cross_backend/setup.ts';
import { cross_rpc_call } from '@fuzdev/fuz_app/testing/cross_backend/cross_rpc.ts';
import { SPINE_SSE_PATH } from '@fuzdev/fuz_app/testing/cross_backend/spine_surface_constants.ts';
import { DEFAULT_TEST_PASSWORD } from '@fuzdev/fuz_app/testing/test_credentials.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';
import {
	ERROR_ALREADY_BOOTSTRAPPED,
	ERROR_CREDENTIAL_TYPE_REQUIRED,
	ERROR_INSUFFICIENT_PERMISSIONS
} from '@fuzdev/fuz_app/http/error_schemas.ts';

import { CORD_SESSION_COOKIE_NAME } from './cord_backend_config.ts';
import { json_post } from './cord_cross_helpers.ts';

/**
 * cord's auth checks the shared spine suites (`spine_suites.cross.test.ts`)
 * leave out, end to end against a spawned `testing_cordd` over real HTTP: the
 * one-shot bootstrap, which the one call can't drive, and the role gates on
 * cord's admin RPC actions and audit stream for a member and a full bearer
 * token. Login, logout, signup, and the origin allowlist are the shared
 * suites'. Each test starts from a fresh keeper (`_testing_reset`);
 * `create_account` mints members through the production invite and signup
 * path.
 */

const handle = inject_backend_handle();
const setup_test = default_cross_process_setup(handle);
const { rpc_path, bootstrap } = handle.config;

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

describe('invite minting', () => {
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

describe('audit stream gate', () => {
	test('a member is 403', async () => {
		const fixture = await setup_test();
		const member = await fixture.create_account({ username: 'member' });
		const res = await fixture.fresh_transport()(SPINE_SSE_PATH, {
			method: 'GET',
			headers: member.create_session_headers()
		});
		assert.strictEqual(res.status, 403);
		assert.strictEqual(await read_error(res), ERROR_INSUFFICIENT_PERMISSIONS);
	});

	test("the keeper's bearer token is refused, since the stream is session-only", async () => {
		const fixture = await setup_test();
		// no Origin: a bearer credential is discarded in a browser context
		const res = await fixture.fresh_transport({ origin: null })(SPINE_SSE_PATH, {
			method: 'GET',
			headers: fixture.create_bearer_headers()
		});
		assert.strictEqual(res.status, 403);
		assert.strictEqual(await read_error(res), ERROR_CREDENTIAL_TYPE_REQUIRED);
	});
});
