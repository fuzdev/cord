import { assert, describe, test } from 'vitest';
import { parse_or_throw } from '@fuzdev/fuz_schema/schema.ts';
import { inject_backend_handle } from '@fuzdev/fuz_app/testing/cross_backend/provided_context.ts';
import { open_session_ws } from '@fuzdev/fuz_app/testing/cross_backend/ws_endpoint.ts';
import { is_response_for } from '@fuzdev/fuz_app/testing/transports/ws_client.ts';
import type { WsTransportClient } from '@fuzdev/fuz_app/testing/transports/ws_transport.ts';

import { CordCreateOutput, CordListOutput, ERROR_CORD_NOT_FOUND } from '#lib/cord_action_specs.ts';
import { CORD_OWNER } from '#lib/cord_roles.ts';
import { call, create_cord, setup_test } from './cord_cross_helpers.ts';

/**
 * cord's verbs over `/api/ws`, the transport the frontend prefers while its
 * socket is open: the one action registry behind both `/api/rpc` and the
 * socket answers the same, reads and writes alike, with each socket acting
 * as the session it upgraded on. The socket itself — the cookie upgrade, a
 * refused anonymous or foreign-origin one, and the 4001 close on every
 * revocation — is fuz_app's spine suites' (`spine_suites.cross.test.ts`),
 * and the close when a session expires is its `credential_deadline` suite.
 */

const handle = inject_backend_handle();
const target = { base_url: handle.config.base_url, ws_path: handle.config.ws_path };

/** A JSON-RPC error frame, as the socket answers a refused request. */
interface ErrorFrame {
	error: { code: number; data?: { reason?: string } };
}

/** Send a request whose answer may be an error, and read the frame. */
const request_frame = async (
	socket: WsTransportClient,
	id: string,
	method: string,
	params: unknown
): Promise<unknown> => {
	const response = socket.wait_for(is_response_for(id));
	await socket.send({ jsonrpc: '2.0', id, method, params });
	return response;
};

describe('cord verbs over the WebSocket', () => {
	test('a session socket carries cord writes and reads', async () => {
		const fixture = await setup_test();
		const socket = await open_session_ws(target, fixture, { admitted: false });
		try {
			const { cord } = parse_or_throw(
				CordCreateOutput,
				await socket.request('create', 'cord_create', { name: 'the porch' })
			);
			assert.strictEqual(cord.role, CORD_OWNER);
			const { cords } = parse_or_throw(
				CordListOutput,
				await socket.request('list', 'cord_list', {})
			);
			assert.deepEqual(
				cords.map((c) => c.id),
				[cord.id]
			);
		} finally {
			await socket.close();
		}
	});

	test('a refusal reads the same over the socket as over HTTP', async () => {
		const fixture = await setup_test();
		const guest = await fixture.create_account({ username: 'guest' });
		const cord = await create_cord(fixture);
		const over_http = await call(fixture, guest, 'cord_get', { cord_id: cord.id });
		assert.ok(!over_http.ok);

		// the guest's socket acts as the guest: the cord is masked as missing
		const socket = await open_session_ws(target, guest, { admitted: false });
		try {
			const frame = (await request_frame(socket, 'get', 'cord_get', {
				cord_id: cord.id
			})) as ErrorFrame;
			assert.strictEqual(frame.error.data?.reason, ERROR_CORD_NOT_FOUND);
			assert.strictEqual(frame.error.code, over_http.error?.code);
		} finally {
			await socket.close();
		}
	});
});
