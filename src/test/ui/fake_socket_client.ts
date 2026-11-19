import type { SocketStatus } from '@fuzdev/fuz_app/actions/socket.svelte.ts';
import { jsonrpc_errors, type ThrownJsonrpcError } from '@fuzdev/fuz_app/http/jsonrpc_errors.ts';

import type { CordSocketClient, CordSocketRequestOptions } from '#lib/ui/cord_socket.svelte.ts';

/** A request the fake sent or queued, with the means to answer it. */
export interface FakeRequest {
	method: string;
	params: unknown;
	resolve: (result: unknown) => void;
	reject: (error: ThrownJsonrpcError) => void;
}

/**
 * A `CordSocketClient` driven by the test: `open`, `close`, and `push` stand
 * in for the server, and the close codes, the durable queue, and aborts
 * follow `FrontendWebsocketClient`'s rules — a 4001 is terminal and rejects
 * everything, a 4004 rejects everything and stays closed until `connect`,
 * anything else rejects what was on the wire, keeps what's queued, and
 * schedules a reconnect; `open` sends what's queued; an abort drops a queued
 * request unsent and cancels one on the wire.
 */
export class FakeSocketClient implements CordSocketClient {
	status: SocketStatus = 'initial';
	connected = false;
	revoked = false;
	superseded = false;
	reconnect_count = 0;

	connect_count = 0;
	disconnect_count = 0;
	/** Every request sent on the wire, oldest first. */
	readonly requests: Array<FakeRequest> = [];
	/** Requests queued while down, not yet sent. */
	readonly queued: Array<FakeRequest> = [];
	/** The methods of requests aborted on the wire, each sent a `cancel`. */
	readonly cancels: Array<string> = [];
	readonly #in_flight: Set<FakeRequest> = new Set();
	readonly #message_handlers: Set<(event: MessageEvent) => void> = new Set();
	readonly #error_handlers: Set<(event: Event) => void> = new Set();

	connect(): void {
		if (this.revoked) return;
		this.superseded = false;
		this.connect_count++;
		this.status = 'connecting';
	}

	disconnect(): void {
		this.disconnect_count++;
		this.status = 'closed';
		this.connected = false;
		this.reconnect_count = 0;
		this.#reject_all(() => jsonrpc_errors.service_unavailable('fake: client disconnected'));
	}

	/** The server accepts the upgrade; the queue flushes. */
	open(): void {
		this.status = 'connected';
		this.connected = true;
		this.reconnect_count = 0;
		for (const request of this.queued.splice(0)) this.#put_on_wire(request);
	}

	/**
	 * The socket closes with `code`: a 4001 for good, a 4004 until `connect`,
	 * anything else to reconnect. A 4001 or 4004 is a close frame the server
	 * sends, so it comes only while the socket is open, after `open` has sent
	 * what was queued; the fake throws on one that couldn't happen.
	 */
	close(code: number): void {
		if ((code === 4001 || code === 4004) && !this.connected) {
			throw new Error(`fake: the server can't close with ${code} a socket that isn't open`);
		}
		this.connected = false;
		if (code === 4001) {
			this.revoked = true;
			this.status = 'closed';
			this.reconnect_count = 0;
			this.#reject_all(() => jsonrpc_errors.unauthenticated('fake: session revoked'));
			return;
		}
		if (code === 4004) {
			this.superseded = true;
			this.status = 'closed';
			this.reconnect_count = 0;
			this.#reject_all(() => jsonrpc_errors.service_unavailable('fake: connection superseded'));
			return;
		}
		this.reconnect_count++;
		this.status = 'reconnecting';
		for (const request of [...this.#in_flight]) {
			request.reject(jsonrpc_errors.service_unavailable('fake: connection closed'));
		}
	}

	/** The server pushes a frame. */
	push(message: object): void {
		const event = { data: JSON.stringify(message) } as MessageEvent;
		for (const handler of this.#message_handlers) handler(event);
	}

	get message_handler_count(): number {
		return this.#message_handlers.size;
	}

	send(_data: object): boolean {
		return this.connected;
	}

	request(
		method: string,
		params: unknown,
		options: CordSocketRequestOptions = {}
	): Promise<unknown> {
		if (this.revoked) {
			return Promise.reject(jsonrpc_errors.unauthenticated('fake: session revoked'));
		}
		const { signal } = options;
		if (signal?.aborted) return Promise.reject(jsonrpc_errors.request_cancelled('fake: aborted'));
		return new Promise((resolve, reject) => {
			// held from the call until the request settles, as the real client
			// holds it across the queue and the wire
			const on_abort = (): void => {
				const index = this.queued.indexOf(request);
				if (index !== -1) this.queued.splice(index, 1);
				else if (this.#in_flight.has(request)) this.cancels.push(method);
				else return;
				request.reject(jsonrpc_errors.request_cancelled('fake: aborted'));
			};
			const settle = (): void => {
				this.#in_flight.delete(request);
				signal?.removeEventListener('abort', on_abort);
			};
			const request: FakeRequest = {
				method,
				params,
				resolve: (result) => {
					settle();
					resolve(result);
				},
				reject: (error) => {
					settle();
					reject(error);
				}
			};
			signal?.addEventListener('abort', on_abort, { once: true });
			if (this.connected) {
				this.#put_on_wire(request);
				return;
			}
			if (options.queue === false) {
				request.reject(jsonrpc_errors.service_unavailable('fake: not connected'));
				return;
			}
			this.queued.push(request);
		});
	}

	add_message_handler(handler: (event: MessageEvent) => void): () => void {
		this.#message_handlers.add(handler);
		return () => {
			this.#message_handlers.delete(handler);
		};
	}

	add_error_handler(handler: (event: Event) => void): () => void {
		this.#error_handlers.add(handler);
		return () => {
			this.#error_handlers.delete(handler);
		};
	}

	#put_on_wire(request: FakeRequest): void {
		this.#in_flight.add(request);
		this.requests.push(request);
	}

	#reject_all(make_error: () => ThrownJsonrpcError): void {
		for (const request of [...this.#in_flight, ...this.queued.splice(0)]) {
			request.reject(make_error());
		}
	}
}
