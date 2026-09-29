/**
 * cord's typed RPC client.
 *
 * `create_cord_rpc` builds a JSON-RPC-over-HTTP client for `POST /api/rpc`,
 * typed by the generated `FrontendActionsApi`. The auth shell needs nothing
 * pushed from the server, so it runs over HTTP alone; authed calls carry the
 * same-origin session cookie. The WebSocket transport joins when the live
 * room needs server-pushed events.
 *
 * Two handles share one transport:
 *
 * - `api` — the throwing Proxy: `await api.foo(input)` returns the value or
 *   throws an `Error` carrying the JSON-RPC `{code, data}`
 * - `api_result` — the Result Proxy: `await api_result.foo(input)` returns
 *   `Result<{value}, {error}>`, for call sites that inspect `error.data.reason`
 *   without try/catch
 *
 * @module
 */

import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';
import { create_frontend_rpc_client } from '@fuzdev/fuz_app/actions/frontend_rpc_client.ts';
import { create_context } from '@fuzdev/fuz_ui/context_helpers.ts';

import { all_cord_action_specs } from '../action_specs.ts';
import type { FrontendActionsApi } from '../action_metatypes.ts';

/** The typed RPC handles, provided once at the root layout. */
export interface CordRpc {
	/** The throwing Proxy; the default for call sites. */
	api: ThrowingApi<FrontendActionsApi>;
	/** The Result Proxy, for call sites that branch on the error. */
	api_result: FrontendActionsApi;
}

/** Svelte context for the `CordRpc` handles. */
export const cord_rpc_context = create_context<CordRpc>();

/**
 * Build cord's RPC handles. Construction allocates and makes no network call,
 * so the root layout builds it once and shares it through `cord_rpc_context`.
 * A method missing from `all_cord_action_specs` resolves to `undefined` on
 * the Proxy, so the spec list must track the server's action registry.
 */
export const create_cord_rpc = (): CordRpc => {
	const { api, api_result } = create_frontend_rpc_client<FrontendActionsApi>({
		specs: all_cord_action_specs,
		path: '/api/rpc'
	});
	return { api, api_result };
};
