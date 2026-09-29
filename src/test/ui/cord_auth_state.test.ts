import { afterEach, assert, describe, test, vi } from 'vitest';

import { CordAuthState } from '#lib/ui/cord_auth_state.svelte.ts';

const json_response = (status: number, body: unknown): Response =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('CordAuthState', () => {
	test('starts unchecked', () => {
		const auth_state = new CordAuthState();
		assert.isFalse(auth_state.session_checked);
		assert.isFalse(auth_state.verified);
	});

	test('marks the session checked after a signed-out check', async () => {
		vi.stubGlobal('fetch', () =>
			Promise.resolve(json_response(401, { error: 'authentication_required' }))
		);
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.session_checked);
		assert.isFalse(auth_state.verified);
	});

	test('marks the session checked after a signed-in check', async () => {
		vi.stubGlobal('fetch', () =>
			Promise.resolve(
				json_response(200, {
					account: { id: 'a', username: 'keeper' },
					actor: { id: 'b', name: 'keeper' },
					role_grants: []
				})
			)
		);
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.session_checked);
		assert.isTrue(auth_state.verified);
		assert.strictEqual(auth_state.account?.username, 'keeper');
	});

	test('marks the session checked when the server is unreachable', async () => {
		vi.stubGlobal('fetch', () => Promise.reject(new TypeError('fetch failed')));
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.session_checked);
		assert.isFalse(auth_state.verified);
	});

	test('signed_in waits for the account after a successful login', async () => {
		let resolve_status!: (response: Response) => void;
		const status = new Promise<Response>((resolve) => {
			resolve_status = resolve;
		});
		const calls: Array<string> = [];
		vi.stubGlobal('fetch', (input: string) => {
			calls.push(input);
			return input === '/api/account/login'
				? Promise.resolve(json_response(200, { ok: true }))
				: status;
		});
		const auth_state = new CordAuthState();
		const login = auth_state.login('keeper', 'correct-horse-battery');
		await vi.waitFor(() => assert.deepEqual(calls, ['/api/account/login', '/api/account/status']));
		// the gap: verified, but the account and roles haven't loaded
		assert.isTrue(auth_state.verified);
		assert.isTrue(auth_state.verifying);
		assert.isNull(auth_state.account);
		assert.isFalse(auth_state.signed_in);
		resolve_status(
			json_response(200, {
				account: { id: 'a', username: 'keeper' },
				actor: { id: 'b', name: 'keeper' },
				role_grants: []
			})
		);
		assert.isTrue(await login);
		assert.isTrue(auth_state.signed_in);
		assert.isFalse(auth_state.verifying);
	});

	test('a failed login leaves the state signed out', async () => {
		vi.stubGlobal('fetch', () =>
			Promise.resolve(json_response(401, { error: 'invalid_credentials' }))
		);
		const auth_state = new CordAuthState();
		assert.isFalse(await auth_state.login('keeper', 'wrong-password-here'));
		assert.isFalse(auth_state.verified);
		assert.isFalse(auth_state.signed_in);
		assert.isFalse(auth_state.verifying);
		assert.isNotNull(auth_state.verify_error);
	});

	test('a signed-out re-check clears the previous identity', async () => {
		const responses = [
			json_response(200, {
				account: { id: 'a', username: 'keeper' },
				actor: { id: 'b', name: 'keeper' },
				role_grants: [{ id: 'g', role: 'admin', created_at: '2026-01-01T00:00:00Z' }]
			}),
			json_response(401, { error: 'authentication_required' })
		];
		vi.stubGlobal('fetch', () => Promise.resolve(responses.shift()));
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.signed_in);
		assert.deepEqual(auth_state.roles, ['admin']);
		await auth_state.check_session();
		assert.isFalse(auth_state.verified);
		assert.isFalse(auth_state.signed_in);
		assert.isNull(auth_state.account);
		assert.isNull(auth_state.actor);
		assert.deepEqual(auth_state.role_grants, []);
		assert.deepEqual(auth_state.roles, []);
	});

	test('is_instance_admin needs a global admin grant', async () => {
		const status = (role_grants: Array<unknown>): Response =>
			json_response(200, {
				account: { id: 'a', username: 'keeper' },
				actor: { id: 'b', name: 'keeper' },
				role_grants
			});
		const grant = (role: string, scope_id: string | null) => ({
			id: 'g',
			role,
			scope_kind: scope_id ? 'cord' : null,
			scope_id,
			created_at: '2026-01-01T00:00:00Z',
			expires_at: null,
			granted_by: null
		});
		const responses = [
			status([grant('admin', null)]),
			status([grant('admin', 'c'), grant('cord_owner', 'c')]),
			status([])
		];
		vi.stubGlobal('fetch', () => Promise.resolve(responses.shift()));
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.is_instance_admin);
		await auth_state.check_session();
		assert.isFalse(auth_state.is_instance_admin, 'a scoped admin grant is not global');
		await auth_state.check_session();
		assert.isFalse(auth_state.is_instance_admin);
	});

	test('logout signs out', async () => {
		const responses = [
			json_response(200, {
				account: { id: 'a', username: 'keeper' },
				actor: { id: 'b', name: 'keeper' },
				role_grants: []
			}),
			json_response(200, { ok: true })
		];
		vi.stubGlobal('fetch', () => Promise.resolve(responses.shift()));
		const auth_state = new CordAuthState();
		await auth_state.check_session();
		assert.isTrue(auth_state.signed_in);
		await auth_state.logout();
		assert.isFalse(auth_state.signed_in);
		assert.isNull(auth_state.account);
	});

	test('logout calls the on_logout listeners until they’re removed', async () => {
		vi.stubGlobal('fetch', () => Promise.resolve(json_response(200, { ok: true })));
		const auth_state = new CordAuthState();
		const calls: Array<string> = [];
		const remove_a = auth_state.on_logout(() => calls.push('a'));
		auth_state.on_logout(() => calls.push(`b:${auth_state.verified}`));
		await auth_state.logout();
		assert.deepEqual(calls, ['a', 'b:false'], 'after the local state is cleared');
		remove_a();
		await auth_state.logout();
		assert.deepEqual(calls, ['a', 'b:false', 'b:false']);
	});
});
