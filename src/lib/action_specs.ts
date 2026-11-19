/**
 * Action specs for cord's frontend RPC client.
 *
 * Mirrors the action surface `cordd` mounts: the protocol actions, which run
 * over `/api/ws` alone, plus the spine's standard auth surface (account
 * self-service, admin, and role-grant offers) and cord's own actions
 * (`cord_action_specs.ts`), which run over either, and the notifications the
 * server pushes over `/api/ws`.
 * Keep it in step with the server's action registry, which the
 * `action_manifest` cross suite holds it to; the generated
 * `action_collections.ts`, `action_metatypes.ts`, and
 * `frontend_action_types.ts` derive their types from the same sources.
 *
 * @module
 */

import type { ActionSpecUnion } from '@fuzdev/fuz_app/actions/action_spec.ts';
import { protocol_action_specs } from '@fuzdev/fuz_app/actions/protocol.ts';
import { all_account_action_specs } from '@fuzdev/fuz_app/auth/account_action_specs.ts';
import { all_admin_action_specs } from '@fuzdev/fuz_app/auth/admin_action_specs.ts';
import { all_role_grant_offer_action_specs } from '@fuzdev/fuz_app/auth/role_grant_offer_action_specs.ts';
import { role_grant_offer_notification_actions } from '@fuzdev/fuz_app/auth/role_grant_offer_notifications.ts';

import { all_cord_domain_action_specs } from './cord_action_specs.ts';

/**
 * The notifications the spine pushes to an account's sockets as its role
 * grants and offers change. fuz_app exports them as handler-less actions.
 */
export const role_grant_offer_notification_action_specs: Array<ActionSpecUnion> =
	role_grant_offer_notification_actions.map((action) => action.spec);

/**
 * Every action spec the cord server mounts, and the notifications it pushes.
 * The protocol actions (`heartbeat`, `cancel`, `peer/ping`) work only over
 * the `/api/ws` WebSocket; sent over `/api/rpc` they fail with
 * method-not-found or `peer_no_transport`. A pushed notification the client
 * lists but has no handler for is parsed and dropped quietly; one it doesn't
 * list logs a method-not-found warning, so every push the server sends is
 * listed here.
 */
export const all_cord_action_specs: Array<ActionSpecUnion> = [
	...protocol_action_specs,
	...all_account_action_specs,
	...all_admin_action_specs,
	...all_role_grant_offer_action_specs,
	...role_grant_offer_notification_action_specs,
	...all_cord_domain_action_specs
];
