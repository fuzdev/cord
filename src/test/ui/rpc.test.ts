import { afterEach, assert, describe, test, vi } from 'vitest';
import { expect_logged_error } from '@fuzdev/fuz_app/testing/logged_errors.ts';

import type { FrontendActionHandlers } from '#lib/frontend_action_types.ts';
import { CORD_SOCKET_WAIT_TIMEOUT, CordSocket } from '#lib/ui/cord_socket.svelte.ts';
import { to_cord_error_message } from '#lib/ui/cord_errors.ts';
import { create_cord_rpc } from '#lib/ui/rpc.ts';

import { FakeTimers } from './fake_timers.ts';
import { install_socket_clients, type SocketClients } from './socket_clients.ts';

// fuz_app's client opens no socket off the browser (`esm-env`'s `BROWSER`, no seam)
vi.mock('esm-env', () => ({ BROWSER: true, DEV: true, NODE: true }));

const ADA = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const GRANT = 'cccccccc-cccc-4ccc-8ccc-cccccccccccc';

/** The mock sockets and timers the test's clients run on, put back after each test. */
let installed: SocketClients | null = null;

/** The RPC client over a socket on real clients over mock sockets, signed in as Ada unless `signed_out`. */
const create_rpc = (options: { handlers?: FrontendActionHandlers; signed_out?: boolean } = {}) => {
	installed ??= install_socket_clients();
	const { clients, create_client } = installed;
	const timers = new FakeTimers();
	const socket = new CordSocket({
		probe_session: () => Promise.resolve('valid'),
		end_session: () => Promise.resolve(),
		create_client,
		browser: true,
		timers
	});
	if (!options.signed_out) socket.set_account(ADA);
	return {
		rpc: create_cord_rpc({ socket, handlers: options.handlers }),
		socket,
		client: clients[0]!,
		timers
	};
};

/** Answers every `fetch` as `/api/rpc` would a `cord_list` with no cords, recording each URL. */
const stub_http = (): Array<string> => {
	const urls: Array<string> = [];
	vi.stubGlobal('fetch', (input: string) => {
		urls.push(input);
		const id = new URL(input, 'http://localhost').searchParams.get('id');
		return Promise.resolve(
			new Response(JSON.stringify({ jsonrpc: '2.0', id, result: { cords: [] } }), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			})
		);
	});
	return urls;
};

/** Let the transport's async message handler run (`setImmediate`, which the fake timers leave real). */
const settle = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

afterEach(() => {
	installed?.restore();
	installed = null;
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe('create_cord_rpc', () => {
	test('signed in, requests go over the socket while it’s open', async () => {
		const urls = stub_http();
		const { rpc, client } = create_rpc();
		client.open();
		const listed = rpc.api.cord_list({});
		await settle();
		assert.strictEqual(client.requests.length, 1);
		assert.strictEqual(client.requests[0]!.method, 'cord_list');
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
		assert.deepEqual(urls, []);
	});

	test('signed in, a request while the socket connects waits for it, never HTTP', async () => {
		const urls = stub_http();
		const { rpc, client } = create_rpc();
		const listed = rpc.api.cord_list({});
		await settle();
		assert.strictEqual(client.requests.length, 0);
		client.open();
		assert.strictEqual(client.requests[0]!.method, 'cord_list');
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
		assert.deepEqual(urls, []);
	});

	test('signed in, a request during an outage sends after the reconnect, never HTTP', async () => {
		const urls = stub_http();
		const { rpc, client } = create_rpc();
		client.open();
		client.close(1006);
		const listed = rpc.api.cord_list({});
		await settle();
		assert.strictEqual(client.requests.length, 0);
		client.open();
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
		assert.deepEqual(urls, []);
	});

	test('signed out, requests go over HTTP, a read as a GET', async () => {
		const urls = stub_http();
		const { rpc, client } = create_rpc({ signed_out: true });
		assert.isUndefined(client);
		assert.deepEqual(await rpc.api.cord_list({}), { cords: [] });
		assert.strictEqual(urls.length, 1);
		assert.match(urls[0]!, /^\/api\/rpc\?.*method=cord_list/);
	});

	test('a sign-out drops a waiting request rather than sending it over HTTP', async () => {
		expect_logged_error('[peer] send request failed: cord_list');
		const urls = stub_http();
		const { rpc, socket, client } = create_rpc();
		const listed = rpc.api.cord_list({}).catch((e: unknown) => e);
		await settle();
		socket.set_account(null);
		const error = await listed;
		assert.strictEqual(
			to_cord_error_message(error),
			"You're signed out now, so this didn't go through."
		);
		assert.deepEqual(urls, [], 'the dropped request isn’t retried over HTTP');
		assert.strictEqual(client.requests.length, 0);
	});

	test('a request that waits out the timeout says it wasn’t sent', async () => {
		expect_logged_error('[peer] send request failed: cord_list');
		stub_http();
		const { rpc, timers } = create_rpc();
		const listed = rpc.api.cord_list({}).catch((e: unknown) => e);
		await settle();
		timers.advance(CORD_SOCKET_WAIT_TIMEOUT);
		assert.match(to_cord_error_message(await listed), /wasn't sent/);
	});

	test('a request lost with the connection says it may have gone through', async () => {
		expect_logged_error('[peer] send request failed: cord_create');
		stub_http();
		const { rpc, client } = create_rpc();
		client.open();
		const created = rpc.api.cord_create({ name: 'the porch' }).catch((e: unknown) => e);
		await settle();
		client.close(1001);
		assert.strictEqual(
			to_cord_error_message(await created),
			'The connection dropped before the server answered, so this may or may not have gone through. Check before trying again.'
		);
	});

	test('a pushed notification dispatches to its receive handler', async () => {
		const received: Array<unknown> = [];
		const { client } = create_rpc({
			handlers: {
				role_grant_revoked: {
					receive: ({ data }) => {
						received.push(data.input);
					}
				}
			}
		});
		client.open();
		const params = {
			role_grant_id: GRANT,
			role: 'admin',
			scope_kind: null,
			scope_id: null,
			reason: null
		};
		client.push({ jsonrpc: '2.0', method: 'role_grant_revoked', params });
		await settle();
		assert.deepEqual(received, [params]);
	});

	test('a listed notification with no handler is dropped quietly', async () => {
		// the logged-error gate fails the test on any error it logs
		const warn = vi.spyOn(console, 'warn');
		const { client } = create_rpc();
		client.open();
		client.push({
			jsonrpc: '2.0',
			method: 'role_grant_revoked',
			params: {
				role_grant_id: GRANT,
				role: 'admin',
				scope_kind: null,
				scope_id: null,
				reason: null
			}
		});
		await settle();
		assert.strictEqual(warn.mock.calls.length, 0);
	});
});
