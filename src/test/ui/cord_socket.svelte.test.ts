import { afterEach, assert, describe, test, vi } from 'vitest';
import { assert_rejects } from '@fuzdev/fuz_util/testing.ts';
import {
	DEFAULT_BACKOFF_FACTOR,
	DEFAULT_HEARTBEAT_INTERVAL,
	DEFAULT_RECONNECT_DELAY
} from '@fuzdev/fuz_app/actions/socket.svelte.ts';
import { DEFAULT_WS_RECEIVE_TIMEOUT } from '@fuzdev/fuz_app/actions/register_action_ws.ts';
import { JSONRPC_ERROR_CODES } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import { CordAuthState, type SessionProbe } from '#lib/ui/cord_auth_state.svelte.ts';
import {
	CORD_SIGNED_OUT_MESSAGE,
	CORD_SOCKET_ACCOUNT_CHANGED_MESSAGE,
	CORD_SOCKET_ACCOUNT_CHANGED_REASON,
	CORD_SOCKET_LOST_MESSAGE,
	CORD_SOCKET_LOST_REASON,
	CORD_SOCKET_WAIT_TIMEOUT_MESSAGE,
	CORD_SOCKET_WAIT_TIMEOUT_REASON
} from '#lib/ui/cord_errors.ts';
import {
	CORD_SOCKET_RECHECK_RECONNECTS,
	CORD_SOCKET_REOPEN_WINDOW,
	CORD_SOCKET_WAIT_TIMEOUT,
	CordSocket
} from '#lib/ui/cord_socket.svelte.ts';

import { FakeSocketClient } from './fake_socket_client.ts';
import { FakeTimers } from './fake_timers.ts';

const ADA = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const BEA = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';

interface SocketHarness {
	socket: CordSocket;
	/** Every client the socket built, oldest first. */
	clients: Array<FakeSocketClient>;
	/** The session probes the socket made, by the account each asked about. */
	probes: { count: number; account_ids: Array<string> };
	ended: { count: number };
	timers: FakeTimers;
}

/**
 * A `CordSocket` over fake clients and timers. `probe` answers each session
 * probe; `end_session` runs when one finds the session gone.
 */
const create_socket = (
	options: {
		probe?: (account_id: string, signal: AbortSignal) => Promise<SessionProbe>;
		end_session?: (socket: CordSocket) => Promise<void>;
		get_account_id?: () => string | null;
		browser?: boolean;
	} = {}
): SocketHarness => {
	const clients: Array<FakeSocketClient> = [];
	const probes: SocketHarness['probes'] = { count: 0, account_ids: [] };
	const ended = { count: 0 };
	const timers = new FakeTimers();
	const socket: CordSocket = new CordSocket({
		probe_session: (account_id, signal) => {
			probes.count++;
			probes.account_ids.push(account_id);
			return options.probe?.(account_id, signal) ?? Promise.resolve('valid');
		},
		end_session: async () => {
			ended.count++;
			await options.end_session?.(socket);
		},
		get_account_id: options.get_account_id,
		create_client: () => {
			const client = new FakeSocketClient();
			clients.push(client);
			return client;
		},
		browser: options.browser ?? true,
		timers
	});
	return { socket, clients, probes, ended, timers };
};

/** A thrown RPC error's `message`. */
const to_message = (error: unknown): unknown => (error as { message?: unknown }).message;

/** A thrown RPC error's `code` and `data.reason`. */
const to_code_and_reason = (error: unknown): [unknown, unknown] => {
	const e = error as { code?: unknown; data?: { reason?: unknown } };
	return [e.code, e.data?.reason];
};

/** Settle `promise` into its value or its rejection, without throwing. */
const settled = async (
	promise: Promise<unknown>
): Promise<{ value?: unknown; error?: unknown }> => {
	try {
		return { value: await promise };
	} catch (error) {
		return { error };
	}
};

/** Let pending promise callbacks run. */
const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

const json_response = (status: number, body: unknown): Response =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('CordSocket lifecycle', () => {
	test('opens no socket off the browser', async () => {
		const { socket, clients } = create_socket({ browser: false });
		socket.set_account(ADA);
		assert.strictEqual(clients.length, 0);
		assert.strictEqual(socket.status, 'initial');
		assert.isFalse(socket.connected);
		assert.isFalse(socket.will_reconnect);
		await assert_rejects(() => socket.request('heartbeat', undefined));
	});

	test('opens nothing while signed out', () => {
		const { socket, clients } = create_socket();
		socket.set_account(null);
		assert.strictEqual(clients.length, 0);
		assert.strictEqual(socket.status, 'initial');
	});

	test('connects when an account signs in', () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		assert.strictEqual(clients.length, 1);
		const client = clients[0]!;
		assert.strictEqual(client.connect_count, 1);
		assert.strictEqual(socket.status, 'connecting');
		assert.isTrue(socket.will_reconnect, 'signed in, requests wait for the socket');
		client.open();
		assert.isTrue(socket.connected);
		assert.strictEqual(socket.status, 'connected');
	});

	test('the same account again changes nothing', () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		clients[0]!.open();
		socket.set_account(ADA);
		assert.strictEqual(clients.length, 1);
		assert.strictEqual(clients[0]!.disconnect_count, 0);
		assert.isTrue(socket.connected);
	});

	test('disconnects when the account signs out', () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		socket.set_account(null);
		assert.strictEqual(client.disconnect_count, 1);
		assert.strictEqual(clients.length, 1);
		assert.isFalse(socket.connected);
		assert.isFalse(socket.will_reconnect, 'signed out, requests go over HTTP');
		assert.strictEqual(socket.status, 'initial');
	});

	test('replaces the client when another account signs in', () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		clients[0]!.open();
		socket.set_account(BEA);
		assert.strictEqual(clients.length, 2);
		assert.strictEqual(clients[0]!.disconnect_count, 1);
		assert.strictEqual(clients[1]!.connect_count, 1);
		assert.strictEqual(socket.status, 'connecting', 'reads the new client');
	});

	test('message handlers follow the current client only', () => {
		const { socket, clients } = create_socket();
		const seen: Array<string> = [];
		socket.add_message_handler((event) => seen.push(String(event.data)));
		socket.set_account(ADA);
		clients[0]!.push({ n: 1 });
		socket.set_account(BEA);
		assert.strictEqual(clients[0]!.message_handler_count, 0, 'unsubscribed from the old client');
		clients[0]!.push({ n: 2 });
		clients[1]!.push({ n: 3 });
		assert.deepEqual(seen, ['{"n":1}', '{"n":3}']);
	});

	test('requests go to the current client', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		clients[0]!.open();
		const result = socket.request('heartbeat', undefined);
		assert.strictEqual(clients[0]!.requests[0]?.method, 'heartbeat');
		clients[0]!.requests[0]!.resolve(null);
		assert.isNull(await result);
	});
});

describe('CordSocket session recheck', () => {
	test('a non-terminal close reconnects on the client’s own backoff, asking nothing', () => {
		const { socket, clients, probes } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		client.close(1006);
		assert.strictEqual(socket.status, 'reconnecting');
		assert.isTrue(socket.will_reconnect);
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(probes.count, 0);
		assert.strictEqual(clients.length, 1, 'the client reconnects itself');
		client.open();
		assert.isTrue(socket.connected);
	});

	test('a 4001 whose session is gone ends it and opens nothing', async () => {
		const { socket, clients, probes, ended } = create_socket({
			probe: () => Promise.resolve('invalid'),
			// what the root layout's account effect does once the session ends
			end_session: (s) => {
				s.set_account(null);
				return Promise.resolve();
			}
		});
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		client.close(4001);
		assert.isTrue(socket.revoked);
		socket.recheck_session_if_in_doubt();
		await socket.recheck_session();
		assert.strictEqual(probes.count, 1);
		assert.strictEqual(ended.count, 1);
		assert.strictEqual(clients.length, 1, 'no reconnect');
		assert.strictEqual(client.connect_count, 1);
		assert.strictEqual(socket.status, 'initial');
	});

	test('a 4001 whose session is still live reopens on a fresh client', async () => {
		const { socket, clients, ended } = create_socket({ probe: () => Promise.resolve('valid') });
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2);
		assert.strictEqual(clients[1]!.connect_count, 1);
		assert.isFalse(socket.revoked);
	});

	test('a 4001 the server can’t answer for keeps the session and reopens', async () => {
		const { socket, clients, ended } = create_socket({
			probe: () => Promise.resolve('unknown')
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2);
	});

	test('a probe that throws reads as unknown', async () => {
		const { socket, clients, ended } = create_socket({
			probe: () => Promise.reject(new Error('boom'))
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2);
	});

	test('reconnects that keep failing ask, and each further failure asks again', async () => {
		const { socket, clients, probes } = create_socket({
			probe: () => Promise.resolve('unknown')
		});
		socket.set_account(ADA);
		const client = clients[0]!;
		for (let i = 1; i < CORD_SOCKET_RECHECK_RECONNECTS; i++) client.close(1006);
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(probes.count, 0);
		client.close(1006);
		socket.recheck_session_if_in_doubt();
		await socket.recheck_session();
		assert.strictEqual(probes.count, 1);
		assert.strictEqual(clients.length, 1, 'a socket not revoked keeps its client');
		client.close(1006);
		socket.recheck_session_if_in_doubt();
		await socket.recheck_session();
		assert.strictEqual(probes.count, 2);
	});

	test('reconnects refused for a dead session end it', async () => {
		const { socket, clients, ended } = create_socket({
			probe: () => Promise.resolve('invalid'),
			end_session: (s) => {
				s.set_account(null);
				return Promise.resolve();
			}
		});
		socket.set_account(ADA);
		const client = clients[0]!;
		for (let i = 0; i < CORD_SOCKET_RECHECK_RECONNECTS; i++) client.close(1006);
		socket.recheck_session_if_in_doubt();
		await socket.recheck_session();
		assert.strictEqual(ended.count, 1);
		assert.strictEqual(client.disconnect_count, 1, 'its reconnects stop');
		assert.strictEqual(clients.length, 1);
	});

	test('rechecks in flight share one probe', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients, probes } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const first = socket.recheck_session();
		const second = socket.recheck_session();
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(probes.count, 1);
		answer('valid');
		await Promise.all([first, second]);
		assert.strictEqual(clients.length, 2, 'one reopen');
	});

	test('an answer that arrives after the account changed is dropped', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients, ended } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const recheck = socket.recheck_session();
		socket.set_account(BEA);
		answer('invalid');
		await recheck;
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2, 'only the new account’s client');
	});

	test('a recheck for a replaced client never absorbs its successor’s 4001', async () => {
		const answers: Array<(probe: SessionProbe) => void> = [];
		const { socket, clients, probes } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answers.push(resolve);
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const for_ada = socket.recheck_session();
		socket.set_account(BEA);
		clients[1]!.open();
		clients[1]!.close(4001);
		assert.isTrue(socket.revoked);
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(probes.count, 2, 'Bea’s 4001 asks on its own');
		assert.deepEqual(probes.account_ids, [ADA, BEA]);
		answers[0]!('valid');
		await for_ada;
		assert.strictEqual(clients.length, 2, 'Ada’s answer opens nothing for Bea');
		answers[1]!('valid');
		await flush();
		assert.strictEqual(clients.length, 3);
		assert.isFalse(socket.revoked);
		assert.strictEqual(clients[2]!.connect_count, 1);
	});

	test('a recheck that ends for a replaced client settles its successor’s 4001 itself', async () => {
		const answers: Array<(probe: SessionProbe) => void> = [];
		const { socket, clients, probes } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answers.push(resolve);
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const for_ada = socket.recheck_session();
		socket.set_account(BEA);
		clients[1]!.open();
		// no `recheck_session_if_in_doubt`: the recheck settles its
		// successor's doubt itself, without leaning on the layout's effect
		clients[1]!.close(4001);
		assert.deepEqual(probes.account_ids, [ADA]);
		answers[0]!('valid');
		await for_ada;
		assert.deepEqual(probes.account_ids, [ADA, BEA], 'Bea’s doubt is asked about');
		answers[1]!('valid');
		await flush();
		assert.strictEqual(clients.length, 3);
		assert.isFalse(socket.revoked);
	});

	test('a probe that never answers gives up as unknown after the wait timeout', async () => {
		const signals: Array<AbortSignal> = [];
		const { socket, clients, ended, timers } = create_socket({
			probe: (_account_id, signal) => {
				signals.push(signal);
				return new Promise(() => {});
			}
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const recheck = socket.recheck_session();
		await flush();
		timers.advance(CORD_SOCKET_WAIT_TIMEOUT - 1);
		await flush();
		assert.strictEqual(clients.length, 1);
		timers.advance(1);
		await recheck;
		assert.isTrue(signals[0]!.aborted, 'the probe’s request is aborted');
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2, 'the revoked socket reopens');
		assert.strictEqual(timers.pending_count, 0);
	});

	test('a live session for another account settles the session and follows it', async () => {
		let signed_in: string | null = ADA;
		const { socket, clients, probes, ended } = create_socket({
			probe: () => Promise.resolve('changed'),
			get_account_id: () => signed_in,
			// what `check_session` does when the cookie is now Bea's
			end_session: () => {
				signed_in = BEA;
				return Promise.resolve();
			}
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		assert.deepEqual(probes.account_ids, [ADA]);
		assert.strictEqual(ended.count, 1);
		assert.strictEqual(clients.length, 2, 'one socket, for Bea, and none reopened for Ada');
		assert.strictEqual(clients[0]!.disconnect_count, 1);
		assert.strictEqual(clients[1]!.connect_count, 1);
	});

	test('a session check that keeps the same account reopens a revoked socket', async () => {
		const { socket, clients, ended } = create_socket({
			probe: () => Promise.resolve('invalid'),
			get_account_id: () => ADA
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		assert.strictEqual(ended.count, 1);
		assert.strictEqual(clients.length, 2, 'not left closed as revoked');
		assert.isFalse(socket.revoked);
	});

	test('a socket revoked again soon after a reopen reopens on a backoff', async () => {
		const { socket, clients, timers } = create_socket({ probe: () => Promise.resolve('valid') });
		socket.set_account(ADA);
		const revoke_and_recheck = async (): Promise<FakeSocketClient> => {
			const client = clients.at(-1)!;
			client.open();
			client.close(4001);
			await socket.recheck_session();
			return clients.at(-1)!;
		};

		const first = await revoke_and_recheck();
		assert.strictEqual(first.connect_count, 1, 'the first reopen is at once');

		timers.advance(1_000);
		const second = await revoke_and_recheck();
		assert.strictEqual(second.connect_count, 0, 'a quick second revoke waits');
		assert.strictEqual(socket.status, 'initial');
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(clients.length, 3, 'nothing asks again while it waits');
		timers.advance(DEFAULT_RECONNECT_DELAY - 1);
		assert.strictEqual(second.connect_count, 0);
		timers.advance(1);
		assert.strictEqual(second.connect_count, 1);

		const third = await revoke_and_recheck();
		timers.advance(DEFAULT_RECONNECT_DELAY * DEFAULT_BACKOFF_FACTOR - 1);
		assert.strictEqual(third.connect_count, 0, 'and each further one waits longer');
		timers.advance(1);
		assert.strictEqual(third.connect_count, 1);

		timers.advance(CORD_SOCKET_REOPEN_WINDOW);
		const fourth = await revoke_and_recheck();
		assert.strictEqual(fourth.connect_count, 1, 'a revoke after the window reopens at once');
	});

	test('a reopen held on its backoff is dropped with the account', async () => {
		const { socket, clients, timers } = create_socket({ probe: () => Promise.resolve('valid') });
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		await socket.recheck_session();
		clients[1]!.open();
		clients[1]!.close(4001);
		await socket.recheck_session();
		assert.strictEqual(timers.pending_count, 1);
		socket.set_account(null);
		assert.strictEqual(timers.pending_count, 0);
		assert.strictEqual(clients[2]!.connect_count, 0);
	});
});

describe('CordSocket superseded', () => {
	test('a 4004 fails what was on the wire as lost and leaves the socket closed, not trying', async () => {
		const { socket, clients, probes } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		client.close(4004);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
		assert.isTrue(socket.superseded);
		assert.strictEqual(socket.status, 'closed');
		assert.strictEqual(client.connect_count, 1, 'nothing reopens it on its own');
		socket.recheck_session_if_in_doubt();
		assert.strictEqual(probes.count, 0, 'the session isn’t in doubt');
	});

	test('the next request after a 4004 reopens the socket and goes over it', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		client.close(4004);
		const listed = socket.request('cord_list', {});
		await flush();
		assert.strictEqual(client.connect_count, 2);
		assert.isFalse(socket.superseded);
		assert.strictEqual(socket.status, 'connecting');
		assert.strictEqual(clients.length, 1, 'the same client, not a fresh one');
		client.open();
		assert.strictEqual(client.requests.at(-1)!.method, 'cord_list');
		client.requests.at(-1)!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});
});

describe('CordSocket requests', () => {
	test('signed out, a request fails at once as unauthenticated', async () => {
		const { socket } = create_socket();
		const { error } = await settled(socket.request('cord_list', {}));
		assert.deepEqual(to_code_and_reason(error), [JSONRPC_ERROR_CODES.unauthenticated, undefined]);
	});

	test('a request made before the layout’s `set_account` opens the socket itself', async () => {
		const { socket, clients } = create_socket({ get_account_id: () => ADA });
		assert.isTrue(socket.will_reconnect);
		const listed = socket.request('cord_list', {});
		assert.strictEqual(clients.length, 1);
		clients[0]!.open();
		clients[0]!.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});

	test('a request while the socket connects waits, then goes over it', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const listed = socket.request('cord_list', {});
		await flush();
		const client = clients[0]!;
		assert.strictEqual(client.requests.length, 0);
		assert.strictEqual(client.queued.length, 1);
		client.open();
		assert.strictEqual(client.requests[0]!.method, 'cord_list');
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});

	test('a request during an outage waits and sends after the reconnect', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		client.close(1006);
		const listed = socket.request('cord_list', {});
		await flush();
		assert.strictEqual(client.requests.length, 0);
		client.open();
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
		assert.strictEqual(clients.length, 1);
	});

	test('a request while the socket connects or reconnects never restarts it', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		void settled(socket.request('cord_list', {}));
		await flush();
		assert.strictEqual(client.connect_count, 1, 'a connect under way is left alone');
		client.open();
		client.close(1006);
		void settled(socket.request('cord_list', {}));
		await flush();
		assert.strictEqual(client.connect_count, 1, 'the reconnect stays on its backoff');
	});

	test('a request that waits too long fails as a timeout, unsent', async () => {
		const { socket, clients, timers } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		const listed = settled(socket.request('cord_list', {}));
		await flush();
		timers.advance(CORD_SOCKET_WAIT_TIMEOUT - 1);
		assert.strictEqual(client.queued.length, 1);
		timers.advance(1);
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.timeout,
			CORD_SOCKET_WAIT_TIMEOUT_REASON
		]);
		assert.strictEqual(to_message(error), CORD_SOCKET_WAIT_TIMEOUT_MESSAGE);
		assert.strictEqual(client.queued.length, 0);
		client.open();
		assert.strictEqual(client.requests.length, 0, 'never sent');
	});

	test('the wait timeout spares a request the socket has sent', async () => {
		const { socket, clients, timers } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		const listed = socket.request('cord_list', {});
		await flush();
		client.open();
		timers.advance(CORD_SOCKET_WAIT_TIMEOUT);
		assert.deepEqual(client.cancels, [], 'not cancelled on the wire');
		client.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});

	test('a caller’s abort drops a waiting request unsent', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		const controller = new AbortController();
		const listed = settled(socket.request('cord_list', {}, { signal: controller.signal }));
		await flush();
		controller.abort();
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [JSONRPC_ERROR_CODES.request_cancelled, undefined]);
		client.open();
		assert.strictEqual(client.requests.length, 0, 'never sent');
		assert.deepEqual(client.cancels, []);
	});

	test('a sign-out rejects the waiting requests, unsent', async () => {
		const { socket, clients, timers } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		const listed = settled(socket.request('cord_list', {}));
		await flush();
		socket.set_account(null);
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [JSONRPC_ERROR_CODES.unauthenticated, undefined]);
		assert.strictEqual(to_message(error), CORD_SIGNED_OUT_MESSAGE);
		assert.strictEqual(client.requests.length, 0);
		assert.strictEqual(timers.pending_count, 0, 'its wait timer is cleared');
	});

	test('an account change rejects the waiting requests, and the next account never sends them', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const listed = settled(socket.request('cord_list', {}));
		await flush();
		socket.set_account(BEA);
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.unauthenticated,
			CORD_SOCKET_ACCOUNT_CHANGED_REASON
		]);
		assert.strictEqual(to_message(error), CORD_SOCKET_ACCOUNT_CHANGED_MESSAGE);
		clients[1]!.open();
		assert.strictEqual(clients[0]!.requests.length, 0);
		assert.strictEqual(clients[1]!.requests.length, 0);
	});

	test('a request on the wire when the connection drops fails as lost', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		assert.strictEqual(client.requests.length, 1);
		client.close(1001);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
		assert.strictEqual(to_message(error), CORD_SOCKET_LOST_MESSAGE);
	});

	test('a request on the wire at a 4001 for a session still live fails as lost, not signed out', async () => {
		const { socket, clients, ended } = create_socket({ probe: () => Promise.resolve('valid') });
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		client.close(4001);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
		await socket.recheck_session();
		assert.strictEqual(ended.count, 0);
		assert.strictEqual(clients.length, 2, 'the session stays and the socket reopens');
	});

	test('a request sent from the queue and then cut off by a 4001 fails as lost', async () => {
		const { socket, clients } = create_socket({ probe: () => Promise.resolve('valid') });
		socket.set_account(ADA);
		const client = clients[0]!;
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		client.open();
		assert.strictEqual(client.requests.length, 1);
		client.close(4001);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
		await flush();
		assert.strictEqual(clients.length, 1, 'not sent again');
	});

	test('an error the server answers with passes through', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		const client = clients[0]!;
		client.open();
		const listed = settled(socket.request('cord_list', {}));
		await flush();
		const unavailable = new Error('database unavailable') as Error & { code: number };
		unavailable.code = JSONRPC_ERROR_CODES.service_unavailable;
		client.requests[0]!.reject(unavailable as never);
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			undefined
		]);
	});

	test('a request while revoked waits on the recheck, then goes over the fresh client', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		socket.recheck_session_if_in_doubt();
		const listed = socket.request('cord_list', {});
		await flush();
		answer('valid');
		await flush();
		const fresh = clients[1]!;
		fresh.open();
		assert.strictEqual(fresh.requests[0]!.method, 'cord_list');
		fresh.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});

	test('a request while revoked fails as signed out once the session is found gone', async () => {
		const { socket, clients } = create_socket({
			probe: () => Promise.resolve('invalid'),
			end_session: (s) => {
				s.set_account(null);
				return Promise.resolve();
			}
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const { error } = await settled(socket.request('cord_list', {}));
		assert.deepEqual(to_code_and_reason(error), [JSONRPC_ERROR_CODES.unauthenticated, undefined]);
		assert.strictEqual(clients.length, 1);
	});

	test('an account change during the revoke wait drops the request, unsent', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const listed = settled(socket.request('cord_list', {}));
		await flush();
		socket.set_account(BEA);
		answer('valid');
		const { error } = await listed;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.unauthenticated,
			CORD_SOCKET_ACCOUNT_CHANGED_REASON
		]);
		clients[1]!.open();
		assert.strictEqual(clients[1]!.requests.length, 0, 'never sent as the next account');
	});

	test('a sign-out and back in as the same account during the revoke wait drops the request, unsent', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				})
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		socket.set_account(null);
		socket.set_account(ADA);
		answer('valid');
		await flush();
		const fresh = clients[1]!;
		fresh.open();
		assert.strictEqual(fresh.requests.length, 0, 'never sent in the new session');
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [JSONRPC_ERROR_CODES.unauthenticated, undefined]);
		assert.strictEqual(to_message(error), CORD_SIGNED_OUT_MESSAGE);
	});

	test('the same account again during the revoke wait keeps the request', async () => {
		let answer!: (probe: SessionProbe) => void;
		const { socket, clients } = create_socket({
			probe: () =>
				new Promise((resolve) => {
					answer = resolve;
				}),
			get_account_id: () => ADA
		});
		socket.set_account(ADA);
		clients[0]!.open();
		clients[0]!.close(4001);
		const listed = socket.request('cord_list', {});
		await flush();
		socket.set_account(ADA);
		answer('valid');
		await flush();
		const fresh = clients[1]!;
		fresh.open();
		assert.strictEqual(fresh.requests[0]!.method, 'cord_list');
		fresh.requests[0]!.resolve({ cords: [] });
		assert.deepEqual(await listed, { cords: [] });
	});

	test('a sign-out while a request is on the wire leaves its outcome unknown', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		clients[0]!.open();
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		assert.strictEqual(clients[0]!.requests.length, 1, 'on the wire');
		socket.set_account(null);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
	});

	test('an account change while a request is on the wire leaves its outcome unknown', async () => {
		const { socket, clients } = create_socket();
		socket.set_account(ADA);
		clients[0]!.open();
		const created = settled(socket.request('cord_create', { name: 'the porch' }));
		await flush();
		assert.strictEqual(clients[0]!.requests.length, 1, 'on the wire');
		socket.set_account(BEA);
		const { error } = await created;
		assert.deepEqual(to_code_and_reason(error), [
			JSONRPC_ERROR_CODES.service_unavailable,
			CORD_SOCKET_LOST_REASON
		]);
	});
});

describe('CordSocket over CordAuthState', () => {
	test('a revoked session falls back to signed out without a reload', async () => {
		let status = 200;
		vi.stubGlobal('fetch', () =>
			Promise.resolve(
				status === 200
					? json_response(200, {
							account: { id: ADA, username: 'ada' },
							actor: { id: BEA, name: 'ada' },
							role_grants: []
						})
					: json_response(401, { error: 'authentication_required' })
			)
		);
		const auth_state = new CordAuthState();
		const clients: Array<FakeSocketClient> = [];
		const socket: CordSocket = new CordSocket({
			probe_session: (account_id, signal) => auth_state.probe_session(account_id, signal),
			end_session: async () => {
				await auth_state.check_session();
				// the root layout's account effect
				socket.set_account(auth_state.signed_in ? (auth_state.account?.id ?? null) : null);
			},
			create_client: () => {
				const client = new FakeSocketClient();
				clients.push(client);
				return client;
			},
			browser: true
		});
		await auth_state.check_session();
		assert.isTrue(auth_state.signed_in);
		socket.set_account(auth_state.account!.id);
		clients[0]!.open();

		status = 401;
		clients[0]!.close(4001);
		socket.recheck_session_if_in_doubt();
		await socket.recheck_session();

		assert.isFalse(auth_state.signed_in);
		assert.isNull(auth_state.account);
		assert.strictEqual(socket.status, 'initial');
		assert.strictEqual(clients.length, 1, 'no reconnect');
	});
});

describe('the socket’s heartbeat', () => {
	test('keeps an idle socket inside the proxy’s and the server’s timeouts', () => {
		// fuz_app's client sends a heartbeat once either direction has been idle
		// for the interval, checking every half interval, so on an idle socket
		// a frame goes each way at least this often, plus the answer's round trip
		const longest_quiet = DEFAULT_HEARTBEAT_INTERVAL + Math.floor(DEFAULT_HEARTBEAT_INTERVAL / 2);
		// the bound: `/api/ws` rides the zap nginx recipe's `/api` block, whose
		// `proxy_read_timeout` and `proxy_send_timeout` are 60s
		const nginx_api_proxy_timeout = 60_000;
		assert.isBelow(longest_quiet, nginx_api_proxy_timeout);
		// and the server closes a socket silent past its receive timeout
		assert.isBelow(longest_quiet, DEFAULT_WS_RECEIVE_TIMEOUT);
	});
});
