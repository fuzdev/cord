import { assert, vi } from 'vitest';
import { cancel_action_spec } from '@fuzdev/fuz_app/actions/cancel.ts';
import { FrontendWebsocketClient } from '@fuzdev/fuz_app/actions/socket.svelte.ts';
import {
	install_mock_websocket,
	MockWebSocket,
	type MockWebSocketInstall
} from '@fuzdev/fuz_app/testing/mock_websocket.ts';

import type { CordSocketClient } from '#lib/ui/cord_socket.svelte.ts';

/** A request a client sent on one of its sockets, with the means to answer it as the server. */
export interface SentRequest {
	method: string;
	params: unknown;
	resolve: (result: unknown) => void;
	reject: (error: { code: number; message: string; data?: unknown }) => void;
}

/**
 * A real `FrontendWebsocketClient`, as production builds it, over mock
 * sockets the test plays the server for: `open`, `close`, and `push` act on
 * its current socket, and `requests` reads what it sent. The close codes,
 * the queue, cancels, and reconnects are the client's own; a reconnect waits
 * on the client's `setTimeout` (faked by `install_socket_clients`) until
 * `open` or `close` needs the new socket.
 */
export class DrivenSocketClient {
	readonly client: FrontendWebsocketClient;
	readonly #url: string;
	readonly #install: MockWebSocketInstall;
	readonly #connect: ReturnType<typeof vi.spyOn>;
	readonly #disconnect: ReturnType<typeof vi.spyOn>;
	#message_handler_count = 0;

	constructor(url: string, install: MockWebSocketInstall) {
		this.#url = url;
		this.#install = install;
		this.client = new FrontendWebsocketClient(url);
		this.#connect = vi.spyOn(this.client, 'connect');
		this.#disconnect = vi.spyOn(this.client, 'disconnect');
		// count the handlers registered and not yet removed, through the client's own methods
		const add_message_handler = this.client.add_message_handler.bind(this.client);
		this.client.add_message_handler = (handler) => {
			const remove = add_message_handler(handler);
			this.#message_handler_count++;
			let removed = false;
			return () => {
				if (!removed) {
					removed = true;
					this.#message_handler_count--;
				}
				remove();
			};
		};
	}

	/** Message handlers registered on the client and not yet removed. */
	get message_handler_count(): number {
		return this.#message_handler_count;
	}

	/** Calls to `connect`, the client's own reconnects included. */
	get connect_count(): number {
		return this.#connect.mock.calls.length;
	}

	get disconnect_count(): number {
		return this.#disconnect.mock.calls.length;
	}

	/** Every socket this client constructed, oldest first. */
	get sockets(): Array<MockWebSocket> {
		return this.#install.instances.filter((ws) => ws.url === this.#url);
	}

	/** The server accepts the upgrade; the client sends what it queued. */
	open(): void {
		this.#current_socket().fire_open();
	}

	/**
	 * The socket closes with `code`. A 4001 or 4004 is a close frame the server
	 * sends, so it comes only while the socket is open; this throws on one
	 * that couldn't happen.
	 */
	close(code: number): void {
		const ws = this.#current_socket();
		if ((code === 4001 || code === 4004) && ws.readyState !== MockWebSocket.OPEN) {
			throw new Error(`the server can't close with ${code} a socket that isn't open`);
		}
		ws.fire_close(code);
	}

	/** The server pushes a frame on the client's latest socket. */
	push(message: object): void {
		const ws = this.sockets.at(-1);
		assert.ok(ws, 'no socket to push on');
		ws.fire_message(JSON.stringify(message));
	}

	/** Every request sent on the wire, oldest first; cancels and other notifications aside. */
	get requests(): Array<SentRequest> {
		const requests: Array<SentRequest> = [];
		for (const ws of this.sockets) {
			for (const frame of ws.sent) {
				const message = JSON.parse(frame);
				if (message.id === undefined) continue;
				const answer = (body: object): void =>
					ws.fire_message(JSON.stringify({ jsonrpc: '2.0', id: message.id, ...body }));
				requests.push({
					method: message.method,
					params: message.params,
					resolve: (result) => answer({ result }),
					reject: ({ code, message: error_message, data }) =>
						answer({ error: { code, message: error_message, data } })
				});
			}
		}
		return requests;
	}

	/** The methods of the requests the client cancelled on the wire. */
	get cancels(): Array<string> {
		const methods: Map<unknown, string> = new Map();
		const cancels: Array<string> = [];
		for (const ws of this.sockets) {
			for (const frame of ws.sent) {
				const message = JSON.parse(frame);
				if (message.id !== undefined) methods.set(message.id, message.method);
				else if (message.method === cancel_action_spec.method) {
					cancels.push(methods.get(message.params.request_id) ?? '(unknown request)');
				}
			}
		}
		return cancels;
	}

	/** The client's live socket, running out a pending reconnect's delay for it. */
	#current_socket(): MockWebSocket {
		if (!this.client.ws && this.client.status === 'reconnecting') {
			vi.advanceTimersByTime(this.client.current_reconnect_delay);
		}
		const ws = this.client.ws;
		assert.ok(ws instanceof MockWebSocket, `no live socket (status ${this.client.status})`);
		return ws;
	}
}

export interface SocketClients {
	/** Every client built, oldest first. */
	readonly clients: Array<DrivenSocketClient>;
	/** A `CordSocket`'s `create_client`. */
	readonly create_client: () => CordSocketClient;
	/** Put the global `WebSocket` and timers back. */
	readonly restore: () => void;
}

/**
 * Swap in mock sockets and fake the timers the client reconnects and beats
 * on (`setImmediate` stays real, for flushing promises), and build each
 * client a URL of its own so its sockets can be told apart.
 */
export const install_socket_clients = (): SocketClients => {
	const install = install_mock_websocket();
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
	const clients: Array<DrivenSocketClient> = [];
	return {
		clients,
		create_client: () => {
			const driven = new DrivenSocketClient(
				`ws://cord.test/api/ws?client=${clients.length}`,
				install
			);
			clients.push(driven);
			return driven.client;
		},
		restore: () => {
			vi.useRealTimers();
			install.restore();
		}
	};
};
