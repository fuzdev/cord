/**
 * cord's typed RPC client.
 *
 * `create_cord_rpc` builds a JSON-RPC client over two transports, typed by
 * the generated `FrontendActionsApi`, and routes by session. Signed in, every
 * request goes over the account's WebSocket on `/api/ws` and never over
 * HTTP; how a request waits for a down socket, and how it fails when it
 * can't be sent or the connection drops under it, is the socket's story
 * (`ui/cord_socket.svelte.ts`). Signed out, there's no socket, so the calls
 * made then (an invite link's preview and redemption) go over HTTP on
 * `/api/rpc`, where fuz_app derives each call's method from its spec: a read
 * goes as a `GET` (a `POST` when its input carries a secret or personal
 * value, or its URL runs too long), everything else as a `POST`.
 *
 * Both carry the same session cookie and answer with the same JSON-RPC
 * envelopes, and the server's per-action rate limits count calls from both
 * alike; nginx's per-IP request limit, though, sees a socket only as its one
 * upgrade, not the requests sent over it.
 *
 * The server pushes notifications over the socket. Each dispatches to the
 * `receive` handler for its method in `handlers`, the generated
 * `FrontendActionHandlers` map; a listed notification with no handler is
 * parsed and dropped. Today the server pushes only the spine's role-grant
 * and offer notifications, which cord lists but doesn't handle.
 *
 * The handle is `api`, the throwing Proxy: `await api.foo(input)` returns the
 * value or throws an `Error` carrying the JSON-RPC `{code, data}`, whose
 * `data.reason` `ui/cord_errors.ts` reads.
 *
 * @module
 */

import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { create_frontend_rpc_client } from '@fuzdev/fuz_app/actions/frontend_rpc_client.ts';
import type { ActionEventPhase } from '@fuzdev/fuz_app/actions/action_spec.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import { all_cord_action_specs } from '../action_specs.ts';
import type { FrontendActionsApi } from '../action_metatypes.ts';
import type { FrontendActionHandlers } from '../frontend_action_types.ts';
import type { CordSocket } from './cord_socket.svelte.ts';

/** The typed RPC handle, provided once at the root layout. */
export interface CordRpc {
	/** The throwing Proxy. */
	api: ThrowingApi<FrontendActionsApi>;
	/** The WebSocket the signed-in requests go over and the pushes arrive on. */
	socket: CordSocket;
}

/** Svelte context for the `CordRpc` handle. */
export const cord_rpc_context = create_context<CordRpc>();

export interface CreateCordRpcOptions {
	/** The signed-in account's socket; the root layout drives its lifecycle. */
	socket: CordSocket;
	/**
	 * Handlers for pushed notifications (and any other phase), read at each
	 * dispatch, so handlers added to the object later are found.
	 */
	handlers?: FrontendActionHandlers;
}

/**
 * Build cord's RPC handle. Construction allocates and makes no network call,
 * so the root layout builds it once and shares it through `cord_rpc_context`.
 * A method missing from `all_cord_action_specs` resolves to `undefined` on
 * the Proxy, so the spec list must track the server's action registry.
 */
export const create_cord_rpc = (options: CreateCordRpcOptions): CordRpc => {
	const { socket, handlers = {} } = options;
	const { api } = create_frontend_rpc_client<FrontendActionsApi>({
		specs: all_cord_action_specs,
		path: '/api/rpc',
		// queue: a signed-in request waits out a down socket rather than
		// falling back to HTTP, which the transport does only once the
		// connection stops saying `will_reconnect` (signed out)
		ws: { client: socket, queue: true },
		lookup_action_handler: (method, phase) => lookup_handler(handlers, method, phase)
	});
	return { api, socket };
};

const lookup_handler = (
	handlers: FrontendActionHandlers,
	method: string,
	phase: ActionEventPhase
): ((event: any) => any) | undefined =>
	(handlers as Record<string, Partial<Record<ActionEventPhase, (event: any) => any>> | undefined>)[
		method
	]?.[phase];
