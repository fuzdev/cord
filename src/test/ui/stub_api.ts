import type { ThrowingApi } from '@fuzdev/fuz_app/actions/rpc_client.ts';

import type { FrontendActionsApi } from '#lib/action_metatypes.ts';

/** A throwing API stub with only the methods a test supplies; any other call throws. */
export const stub_api = (
	methods: Record<string, (input: any) => Promise<unknown>>
): ThrowingApi<FrontendActionsApi> =>
	new Proxy(methods, {
		get: (target, key: string) =>
			target[key] ??
			(() => {
				throw new Error(`unexpected call: ${key}`);
			})
	}) as unknown as ThrowingApi<FrontendActionsApi>;

/** An error shaped as the throwing RPC proxy throws it. */
export const rpc_error = (reason: string, code = -32000): Error =>
	Object.assign(new Error(`refused: ${reason}`), { code, data: { reason } });
